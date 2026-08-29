//! HTTP layer against the Tracker REST API.
//!
//! We talk to the API directly instead of using the official Python-era client:
//! see `docs/adr/0004-own-http-client.md`.

pub mod duration;
pub mod error;
pub mod models;
pub mod parse;
pub mod query;

use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};

use serde_json::Value;

use crate::api::error::ApiError;
use crate::api::models::{
    Attachment, Change, ChecklistItem, Comment, DictEntry, Entity, Issue, Link, Page, Person,
    RemoteLink, User, Worklog,
};
use crate::config::OrgKind;

/// Default API root. Overridable so tests can point at a `wiremock` server.
pub const DEFAULT_BASE_URL: &str = "https://api.tracker.yandex.net";

/// Entity fields we ask for. Requesting an explicit set keeps the response small
/// and its shape predictable; the endpoints return only identity otherwise.
/// `entityType` is deliberately absent: it is an attribute of the entity, not
/// one of its fields, and asking for it makes Tracker refuse the whole request
/// with `поля [entityType] не существуют`. It comes back regardless.
const ENTITY_FIELDS: &str = "summary,description,entityStatus,start,end,lead,author,parentEntity";

/// The host part of a URL, for comparing two of them.
fn host_of(url: &str) -> Option<String> {
    let without_scheme = url.split_once("://")?.1;
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    Some(authority.to_ascii_lowercase())
}

/// Everything the client needs to address one organisation as one account.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub base_url: String,
    pub token: String,
    pub org_id: String,
    pub org_kind: OrgKind,
    pub timeout: Duration,
    /// Retry attempts for transport errors, 429 and 5xx. Client errors are never retried.
    pub retries: usize,
}

impl ClientConfig {
    #[must_use]
    pub fn new(token: String, org_id: String, org_kind: OrgKind) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            token,
            org_id,
            org_kind,
            timeout: Duration::from_secs(30),
            retries: 3,
        }
    }
}

/// A configured Tracker client.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    retries: usize,
    /// Which organisation this client talks to.
    ///
    /// The headers carry it already, but nothing can read them back, and one
    /// command can hold two clients: keys resolve per profile, and two profiles
    /// can be two organisations. A bulk change is one request to one of them.
    org: String,
}

impl Client {
    pub fn new(config: &ClientConfig) -> Result<Self, ApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("ytcli/", env!("CARGO_PKG_VERSION"))),
        );

        // A malformed token or org id must fail here, not as a confusing 401 later.
        let mut auth = HeaderValue::try_from(format!("OAuth {}", config.token))
            .map_err(|_| ApiError::Unauthorized)?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);

        let org_header = HeaderName::from_static(config.org_kind.header_name());
        let org_value =
            HeaderValue::try_from(config.org_id.clone()).map_err(|_| ApiError::Forbidden)?;
        headers.insert(org_header, org_value);

        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .default_headers(headers)
            .build()?;

        Ok(Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            retries: config.retries,
            org: config.org_id.clone(),
        })
    }

    /// The organisation id this client was built for.
    #[must_use]
    pub fn org(&self) -> &str {
        &self.org
    }

    /// `GET /v3/myself` — the cheapest call that proves the whole chain works:
    /// token, organisation header, and network.
    pub async fn myself(&self) -> Result<User, ApiError> {
        let value = self.get_value("/v3/myself", "current user").await?;
        Ok(User {
            id: value
                .get("uid")
                .map_or_else(String::new, ToString::to_string),
            login: value
                .get("login")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            display: value
                .get("display")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        })
    }

    /// One issue, both normalised and raw.
    ///
    /// The raw payload travels alongside so that `--json-raw` does not cost a
    /// second request, and so that a field we do not model is still reachable.
    pub async fn issue(&self, key: &str) -> Result<(Issue, Value), ApiError> {
        let raw = self
            .get_value(&format!("/v3/issues/{key}"), &format!("issue {key}"))
            .await?;
        let issue = parse::issue(&raw).ok_or_else(|| ApiError::NotFound(format!("issue {key}")))?;
        Ok((issue, raw))
    }

    /// The links of an issue, with their direction resolved.
    ///
    /// Tracker keeps links on their own endpoint, so the compact issue view
    /// costs two requests. Showing links is worth that: "what blocks this" is
    /// the question that follows "what is this", and making the caller ask twice
    /// costs more than one round trip (ADR 3).
    pub async fn issue_links(&self, key: &str) -> Result<Vec<Link>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/issues/{key}/links"),
                &format!("issue {key} links"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::link).collect())
            .unwrap_or_default())
    }

    /// `GET /v3/issues/{key}/remotelinks` — the links that leave Tracker.
    ///
    /// Its own request rather than a second section of [`Self::issue_links`]:
    /// most issues have none, and making every `issue links` pay for a request
    /// that usually answers `[]` is the wrong trade.
    pub async fn issue_remote_links(&self, key: &str) -> Result<Vec<RemoteLink>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/issues/{key}/remotelinks"),
                &format!("remote links of {key}"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::remote_link).collect())
            .unwrap_or_default())
    }

    /// One page of search results.
    ///
    /// Tracker reports the total in `X-Total-Count`. When it does not, the page
    /// still has to be honest about whether more exists, which is why
    /// [`Page::has_more`] falls back to "a full page probably is not the last".
    pub async fn search(
        &self,
        query: &str,
        page: u32,
        per_page: u32,
    ) -> Result<Page<Issue>, ApiError> {
        let path = format!("/v3/issues/_search?page={page}&perPage={per_page}");
        let body = serde_json::json!({ "query": query });
        let (value, headers) = self.post_value(&path, &body, "issues").await?;

        let items = value
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::issue).collect())
            .unwrap_or_default();

        Ok(Page {
            items,
            page,
            per_page,
            total: headers
                .get("x-total-count")
                .and_then(|count| count.to_str().ok())
                .and_then(|count| count.parse().ok()),
        })
    }

    /// How many issues match, without fetching any of them.
    pub async fn count(&self, query: &str) -> Result<u64, ApiError> {
        let body = serde_json::json!({ "query": query });
        let (value, _) = self
            .post_value("/v3/issues/_count", &body, "issues")
            .await?;

        value
            .as_u64()
            .ok_or_else(|| ApiError::NotFound("issue count".to_owned()))
    }

    /// `POST /v3/issues/` — create an issue, returning it normalised.
    pub async fn create_issue(&self, body: &Value) -> Result<Issue, ApiError> {
        let (value, _) = self.post_value("/v3/issues/", body, "issue").await?;
        parse::issue(&value).ok_or_else(|| ApiError::NotFound("created issue".to_owned()))
    }

    /// `PATCH /v3/issues/{key}` — change fields.
    pub async fn update_issue(&self, key: &str, body: &Value) -> Result<Issue, ApiError> {
        let value = self
            .send_value(
                reqwest::Method::PATCH,
                &format!("/v3/issues/{key}"),
                Some(body),
                &format!("issue {key}"),
            )
            .await?
            .0;
        parse::issue(&value).ok_or_else(|| ApiError::NotFound(format!("issue {key}")))
    }

    /// `POST /v3/issues/{key}/comments` — add a comment.
    pub async fn add_comment(&self, key: &str, text: &str) -> Result<Comment, ApiError> {
        let body = serde_json::json!({ "text": text });
        let (value, _) = self
            .post_value(
                &format!("/v3/issues/{key}/comments"),
                &body,
                &format!("issue {key}"),
            )
            .await?;
        parse::comment(&value).ok_or_else(|| ApiError::NotFound("created comment".to_owned()))
    }

    /// Rewrite a comment that is already there.
    ///
    /// Tracker keeps no history of the previous text and shows the comment as
    /// edited, so this replaces rather than appends: the old wording is gone.
    pub async fn update_comment(
        &self,
        key: &str,
        id: &str,
        text: &str,
    ) -> Result<Comment, ApiError> {
        let body = serde_json::json!({ "text": text });
        let (value, _) = self
            .send_value(
                reqwest::Method::PATCH,
                &format!("/v3/issues/{key}/comments/{id}"),
                Some(&body),
                &format!("comment {id} of issue {key}"),
            )
            .await?;
        parse::comment(&value).ok_or_else(|| ApiError::NotFound(format!("comment {id}")))
    }

    /// Remove a comment.
    pub async fn delete_comment(&self, key: &str, id: &str) -> Result<(), ApiError> {
        self.send_value(
            reqwest::Method::DELETE,
            &format!("/v3/issues/{key}/comments/{id}"),
            None,
            &format!("comment {id} of issue {key}"),
        )
        .await?;
        Ok(())
    }

    /// Correct a worklog entry that is already recorded.
    pub async fn update_worklog(
        &self,
        key: &str,
        id: &str,
        body: &Value,
    ) -> Result<Worklog, ApiError> {
        let (value, _) = self
            .send_value(
                reqwest::Method::PATCH,
                &format!("/v3/issues/{key}/worklog/{id}"),
                Some(body),
                &format!("worklog {id} of issue {key}"),
            )
            .await?;
        parse::worklog(&value).ok_or_else(|| ApiError::NotFound(format!("worklog {id}")))
    }

    /// `GET /v3/issues/{key}/worklog` — every entry, oldest first.
    pub async fn worklogs(&self, key: &str) -> Result<Vec<Worklog>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/issues/{key}/worklog"),
                &format!("issue {key} worklog"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::worklog).collect())
            .unwrap_or_default())
    }

    /// `POST /v3/issues/{key}/worklog` — record time spent.
    pub async fn add_worklog(&self, key: &str, body: &Value) -> Result<Worklog, ApiError> {
        let (value, _) = self
            .post_value(
                &format!("/v3/issues/{key}/worklog"),
                body,
                &format!("issue {key} worklog"),
            )
            .await?;
        parse::worklog(&value).ok_or_else(|| ApiError::NotFound("created worklog".to_owned()))
    }

    /// `DELETE /v3/issues/{key}/worklog/{id}`.
    pub async fn delete_worklog(&self, key: &str, id: &str) -> Result<(), ApiError> {
        self.send_value(
            reqwest::Method::DELETE,
            &format!("/v3/issues/{key}/worklog/{id}"),
            None,
            &format!("worklog {id} of issue {key}"),
        )
        .await?;
        Ok(())
    }

    /// `GET /v3/issues/{key}/checklistItems`.
    pub async fn checklist(&self, key: &str) -> Result<Vec<ChecklistItem>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/issues/{key}/checklistItems"),
                &format!("issue {key} checklist"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::checklist_item).collect())
            .unwrap_or_default())
    }

    /// `POST /v3/issues/{key}/checklistItems` — add a line.
    ///
    /// Tracker answers with the whole issue rather than the item, so the list
    /// comes back out of the issue's own `checklistItems`.
    pub async fn add_checklist_item(
        &self,
        key: &str,
        body: &Value,
    ) -> Result<Vec<ChecklistItem>, ApiError> {
        let (value, _) = self
            .post_value(
                &format!("/v3/issues/{key}/checklistItems"),
                body,
                &format!("issue {key} checklist"),
            )
            .await?;
        Ok(checklist_of(&value))
    }

    /// `PATCH /v3/issues/{key}/checklistItems/{id}` — tick, untick or reword.
    pub async fn update_checklist_item(
        &self,
        key: &str,
        id: &str,
        body: &Value,
    ) -> Result<Vec<ChecklistItem>, ApiError> {
        let (value, _) = self
            .send_value(
                reqwest::Method::PATCH,
                &format!("/v3/issues/{key}/checklistItems/{id}"),
                Some(body),
                &format!("checklist item {id} of issue {key}"),
            )
            .await?;
        Ok(checklist_of(&value))
    }

    /// `DELETE /v3/issues/{key}/checklistItems/{id}`.
    pub async fn delete_checklist_item(&self, key: &str, id: &str) -> Result<(), ApiError> {
        self.send_value(
            reqwest::Method::DELETE,
            &format!("/v3/issues/{key}/checklistItems/{id}"),
            None,
            &format!("checklist item {id} of issue {key}"),
        )
        .await?;
        Ok(())
    }

    /// `POST /v3/issues/{key}/links` — link two issues.
    pub async fn add_link(
        &self,
        key: &str,
        relationship: &str,
        other: &str,
    ) -> Result<(), ApiError> {
        let body = serde_json::json!({ "relationship": relationship, "issue": other });
        self.post_value(
            &format!("/v3/issues/{key}/links"),
            &body,
            &format!("issue {key} links"),
        )
        .await?;
        Ok(())
    }

    /// `DELETE /v3/issues/{key}/links/{id}`.
    pub async fn delete_link(&self, key: &str, id: &str) -> Result<(), ApiError> {
        self.send_value(
            reqwest::Method::DELETE,
            &format!("/v3/issues/{key}/links/{id}"),
            None,
            &format!("link {id} of issue {key}"),
        )
        .await?;
        Ok(())
    }

    /// `DELETE /v3/issues/{key}/attachments/{id}` — remove an attachment.
    ///
    /// Tracker keeps no copy: the file is gone, and the comment or description
    /// that pointed at it is left pointing at nothing.
    pub async fn delete_attachment(&self, key: &str, id: &str) -> Result<(), ApiError> {
        self.send_value(
            reqwest::Method::DELETE,
            &format!("/v3/issues/{key}/attachments/{id}"),
            None,
            &format!("attachment {id} of issue {key}"),
        )
        .await?;
        Ok(())
    }

    /// Transitions available from the issue's current status.
    pub async fn transitions(&self, key: &str) -> Result<Vec<Transition>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/issues/{key}/transitions"),
                &format!("issue {key} transitions"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(Transition::parse).collect())
            .unwrap_or_default())
    }

    /// Perform a transition.
    pub async fn execute_transition(
        &self,
        key: &str,
        transition: &str,
        body: &Value,
    ) -> Result<(), ApiError> {
        self.post_value(
            &format!("/v3/issues/{key}/transitions/{transition}/_execute"),
            body,
            &format!("transition {transition} of issue {key}"),
        )
        .await?;
        Ok(())
    }

    /// Search projects, portfolios or goals.
    ///
    /// The entity endpoints answer with their own envelope (`values`, `hits`,
    /// `pages`) rather than the header-based totals the issue endpoints use, so
    /// the page is assembled from the body here.
    pub async fn entities(
        &self,
        kind: &str,
        input: Option<&str>,
        page: u32,
        per_page: u32,
    ) -> Result<Page<Entity>, ApiError> {
        let path = format!(
            "/v3/entities/{kind}/_search?page={page}&perPage={per_page}&fields={ENTITY_FIELDS}"
        );
        let mut body = serde_json::Map::new();
        if let Some(input) = input {
            body.insert("input".to_owned(), Value::String(input.to_owned()));
        }

        let (value, _) = self
            .post_value(&path, &Value::Object(body), &format!("{kind}s"))
            .await?;

        let items = value
            .get("values")
            .and_then(Value::as_array)
            .map(|entries| entries.iter().filter_map(parse::entity).collect())
            .unwrap_or_default();

        Ok(Page {
            items,
            page,
            per_page,
            total: value.get("hits").and_then(Value::as_u64),
        })
    }

    /// What a portfolio contains: the portfolios and projects under it.
    ///
    /// Two requests, because the entity endpoints are typed and containment is
    /// not: a portfolio holds both. The tally sums the two totals, so `shown N
    /// of M` is the real count even though a page is a page of each.
    pub async fn entities_in(
        &self,
        parent: &str,
        page: u32,
        per_page: u32,
    ) -> Result<Page<Entity>, ApiError> {
        let mut items = Vec::new();
        let mut total = 0;

        for kind in ["portfolio", "project"] {
            let path = format!(
                "/v3/entities/{kind}/_search?page={page}&perPage={per_page}&fields={ENTITY_FIELDS}"
            );
            let body = serde_json::json!({ "filter": { "parentEntity": parent } });
            let (value, _) = self
                .post_value(&path, &body, &format!("{kind}s in {parent}"))
                .await?;

            if let Some(entries) = value.get("values").and_then(Value::as_array) {
                items.extend(entries.iter().filter_map(parse::entity));
            }
            total += value.get("hits").and_then(Value::as_u64).unwrap_or(0);
        }

        Ok(Page {
            items,
            page,
            per_page,
            total: Some(total),
        })
    }

    /// One project, portfolio or goal, by the id the entity endpoints use.
    pub async fn entity(&self, kind: &str, id: &str) -> Result<Entity, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/entities/{kind}/{id}?fields={ENTITY_FIELDS}"),
                &format!("{kind} {id}"),
            )
            .await?;

        parse::entity(&raw).ok_or_else(|| ApiError::NotFound(format!("{kind} {id}")))
    }

    /// The attachments of an issue.
    pub async fn attachments(&self, key: &str) -> Result<Vec<Attachment>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/issues/{key}/attachments"),
                &format!("issue {key} attachments"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::attachment).collect())
            .unwrap_or_default())
    }

    /// Download an attachment's bytes.
    ///
    /// The download URL comes out of the payload, which means it is supplied by
    /// the server rather than chosen by us. It is checked against the configured
    /// API host before being followed: a crafted `content` URL must not be able
    /// to send this client, carrying its OAuth header, to somewhere else.
    pub async fn download(&self, url: &str) -> Result<Vec<u8>, ApiError> {
        let expected = host_of(&self.base_url);
        if host_of(url) != expected {
            return Err(ApiError::Rejected {
                status: reqwest::StatusCode::BAD_REQUEST,
                message: format!(
                    "attachment points at `{}`, which is not the configured Tracker host `{}`",
                    host_of(url).unwrap_or_default(),
                    expected.unwrap_or_default(),
                ),
            });
        }

        let response = self.http.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 => ApiError::Unauthorized,
                403 => ApiError::Forbidden,
                404 => ApiError::NotFound("attachment".to_owned()),
                _ => ApiError::Rejected {
                    status,
                    message: String::new(),
                },
            });
        }

        Ok(response.bytes().await?.to_vec())
    }

    /// Upload a file to an issue.
    pub async fn upload(
        &self,
        key: &str,
        filename: &str,
        bytes: Vec<u8>,
    ) -> Result<Attachment, ApiError> {
        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename.to_owned());
        let form = reqwest::multipart::Form::new().part("file", part);

        let url = format!("{}/v3/issues/{key}/attachments/", self.base_url);
        let response = self.http.post(&url).multipart(form).send().await?;
        let text = classify(response, &format!("issue {key}")).await?;

        let value: Value = serde_json::from_str(&text).map_err(ApiError::Decode)?;
        parse::attachment(&value)
            .ok_or_else(|| ApiError::NotFound("uploaded attachment".to_owned()))
    }

    /// Queues visible to the active profile.
    ///
    /// Tracker paginates this endpoint; the ceiling is deliberately generous
    /// because "how many queues can I see" is a question with a small answer,
    /// and a second page here would be surprising.
    pub async fn queues(&self) -> Result<Vec<Queue>, ApiError> {
        let raw = self.get_value("/v3/queues?perPage=1000", "queues").await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(Queue::parse).collect())
            .unwrap_or_default())
    }

    /// Worklog entries across the whole organisation.
    ///
    /// `createdBy` takes a login or a uid and **not** `me`: Tracker reads it as
    /// a login and answers 422 saying no such user exists. Resolving `me` is
    /// the caller's job, with one extra request to `myself`.
    pub async fn worklog_search(
        &self,
        who: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
        per_page: u32,
    ) -> Result<Vec<Worklog>, ApiError> {
        use std::fmt::Write as _;

        let mut query = format!("perPage={per_page}");
        if let Some(who) = who {
            let _ = write!(query, "&createdBy={who}");
        }
        // One parameter carries both ends of the range, and Tracker accepts
        // either half on its own.
        match (since, until) {
            (Some(since), Some(until)) => {
                let _ = write!(query, "&createdAt=from:{since},to:{until}");
            }
            (Some(since), None) => {
                let _ = write!(query, "&createdAt=from:{since}");
            }
            (None, Some(until)) => {
                let _ = write!(query, "&createdAt=to:{until}");
            }
            (None, None) => {}
        }

        let raw = self
            .get_value(&format!("/v3/worklog?{query}"), "worklog")
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::worklog).collect())
            .unwrap_or_default())
    }

    /// Move an issue to another queue.
    ///
    /// The issue keeps its identity and loses its name: `PROJ-42` becomes
    /// `OTHER-17`, and there is no request that undoes it. Tracker drops fields
    /// the target queue does not define unless `moveAllFields` says otherwise,
    /// so that choice is the caller's rather than a default we picked for them.
    pub async fn move_issue(
        &self,
        key: &str,
        queue: &str,
        keep_fields: bool,
        initial_status: bool,
    ) -> Result<Issue, ApiError> {
        let path = format!(
            "/v3/issues/{key}/_move?queue={queue}&moveAllFields={keep_fields}&initialStatus={initial_status}"
        );
        let (raw, _) = self
            .send_value(
                reqwest::Method::POST,
                &path,
                Some(&serde_json::json!({})),
                &format!("move {key} to {queue}"),
            )
            .await?;

        parse::issue(&raw).ok_or_else(|| ApiError::NotFound(format!("issue {key} after the move")))
    }

    /// What changed on an issue, newest last.
    ///
    /// Tracker pages this with an opaque cursor rather than page numbers, and
    /// the cursor is only worth spending when somebody asks for more than the
    /// first page — which nobody has yet. So this asks for one page, and the
    /// caller says how big.
    pub async fn changelog(&self, key: &str, per_page: u32) -> Result<Vec<Change>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/issues/{key}/changelog?perPage={per_page}"),
                &format!("changelog of {key}"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::change).collect())
            .unwrap_or_default())
    }

    /// The versions a queue defines.
    ///
    /// This is what an issue's `fixVersions` refers to; without it that field
    /// is an id with no meaning.
    pub async fn queue_versions(&self, key: &str) -> Result<Vec<Version>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/queues/{key}/versions"),
                &format!("versions of queue {key}"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(Version::parse).collect())
            .unwrap_or_default())
    }

    /// The tags in use in a queue.
    pub async fn queue_tags(&self, key: &str) -> Result<Vec<String>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/queues/{key}/tags?perPage=1000"),
                &format!("tags of queue {key}"),
            )
            .await?;

        // Both shapes are accepted because the organisation this was written
        // against has no tags to answer with, and a listing that silently drops
        // every row is worse than one that reads a member it did not need.
        Ok(raw
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| match entry {
                        Value::String(name) => Some(name.clone()),
                        other => other
                            .get("name")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Everything that changes issues in a queue on its own.
    ///
    /// Three requests, and a refusal of one of them is an answer rather than a
    /// failure: triggers need queue-owner rights, so a member of the queue gets
    /// two sections and Tracker's own words about the third. All three failing
    /// is a different thing — a queue that is not there, or a token that is not
    /// allowed — and is reported as the error it is.
    pub async fn queue_automation(&self, key: &str) -> Result<Automation, ApiError> {
        let mut unreadable = Vec::new();
        let mut refused = None;

        let mut section = |name: &'static str, result: Result<Value, ApiError>| match result {
            Ok(value) => value.as_array().cloned().unwrap_or_default(),
            Err(error) => {
                unreadable.push(Unreadable {
                    // Tracker answers a 403 here with the queue owner's record
                    // and no message at all, so there are no words of its own
                    // to pass through. Saying which right is missing is the
                    // useful sentence, and our generic 403 — which also blames
                    // the organisation header — is not it.
                    section: name,
                    reason: match error {
                        ApiError::Forbidden => {
                            format!("{name} are readable by the queue owner only (403)")
                        }
                        ref other => other.to_string(),
                    },
                });
                refused.get_or_insert(error);
                Vec::new()
            }
        };

        let macros = section(
            "macros",
            self.get_value(
                &format!("/v3/queues/{key}/macros"),
                &format!("macros of queue {key}"),
            )
            .await,
        );
        let autoactions = section(
            "autoactions",
            self.get_value(
                &format!("/v3/queues/{key}/autoactions"),
                &format!("autoactions of queue {key}"),
            )
            .await,
        );
        let triggers = section(
            "triggers",
            self.get_value(
                &format!("/v3/queues/{key}/triggers"),
                &format!("triggers of queue {key}"),
            )
            .await,
        );

        if unreadable.len() == 3 {
            return Err(refused.unwrap_or(ApiError::NotFound(format!("queue {key}"))));
        }

        Ok(Automation {
            macros: macros.iter().filter_map(Macro::parse).collect(),
            autoactions: autoactions.iter().filter_map(AutoAction::parse).collect(),
            triggers: triggers.iter().filter_map(Trigger::parse).collect(),
            unreadable,
        })
    }

    /// The components of one queue, or of the whole organisation.
    ///
    /// Tracker filters by queue itself, so `--queue` is a different path rather
    /// than a listing narrowed here: asking for every component in order to
    /// throw most of them away is the kind of cost this tool exists to avoid.
    pub async fn components(&self, queue: Option<&str>) -> Result<Vec<Component>, ApiError> {
        let (path, what) = match queue {
            Some(queue) => (
                format!("/v3/queues/{queue}/components"),
                format!("components of queue {queue}"),
            ),
            None => ("/v3/components".to_owned(), "components".to_owned()),
        };
        let raw = self.get_value(&path, &what).await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(Component::parse).collect())
            .unwrap_or_default())
    }

    /// Every kind of link two issues can have.
    ///
    /// Small, fixed and organisation-wide — six entries in the organisation
    /// this was checked against, `cloners` among them, which no write in this
    /// tool can produce.
    pub async fn link_types(&self) -> Result<Vec<LinkType>, ApiError> {
        let raw = self.get_value("/v3/linktypes", "link types").await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(LinkType::parse).collect())
            .unwrap_or_default())
    }

    /// Who may do what in a queue.
    ///
    /// Two endpoints saying two different things. `permissions` is the rule as
    /// somebody configured it — named people, groups and *roles*; `access` is
    /// the list of people it comes out as. A role like "assignee" resolves per
    /// issue, so only the second answers "am I one of them" on its own.
    ///
    /// Both are refused together in the organisation this was checked against —
    /// one right governs the pair — but they are separate endpoints, and a
    /// section refused is still an answer while the other one stands.
    pub async fn queue_access(&self, key: &str) -> Result<QueueAccess, ApiError> {
        let mut unreadable = Vec::new();
        let mut refused = None;

        let mut section = |name: &'static str, result: Result<Value, ApiError>| match result {
            Ok(value) => Permission::parse_all(&value),
            Err(error) => {
                unreadable.push(Unreadable {
                    section: name,
                    // Tracker does say why here — "you have no right to view the
                    // queue's access rights" — but our 403 flattens that into a
                    // sentence that also blames the organisation header, which
                    // is the wrong suspect for this endpoint.
                    reason: match error {
                        ApiError::Forbidden => {
                            format!(
                                "{name} are readable only by those who may see queue rights (403)"
                            )
                        }
                        ref other => other.to_string(),
                    },
                });
                refused.get_or_insert(error);
                Vec::new()
            }
        };

        let permissions = section(
            "permissions",
            self.get_value(
                &format!("/v3/queues/{key}/permissions"),
                &format!("permissions of queue {key}"),
            )
            .await,
        );
        let access = section(
            "access",
            self.get_value(
                &format!("/v3/queues/{key}/access"),
                &format!("access of queue {key}"),
            )
            .await,
        );

        if unreadable.len() == 2 {
            return Err(match refused {
                // Both sections missing means the queue is, and saying so about
                // the queue reads better than about the first endpoint tried.
                Some(ApiError::NotFound(_)) | None => ApiError::NotFound(format!("queue {key}")),
                Some(other) => other,
            });
        }

        // Whose rights these are compared against. A failure here loses the
        // `you` column and nothing else, so it is not worth failing the command
        // that did answer.
        let you = match self.myself().await {
            Ok(user) => Some(user.id),
            Err(_) => None,
        };

        Ok(QueueAccess {
            permissions,
            access,
            you,
            unreadable,
        })
    }

    /// `POST /v3/bulkchange/_update` — change many issues in one request.
    ///
    /// Tracker requires the keys: a query is refused with
    /// `issues: Требуется параметр`, so what this touches is exactly what the
    /// caller named and the confirmation that names them is the whole story.
    /// Unknown keys are refused before anything is written, naming them — which
    /// is better than the issue-at-a-time path, where the first few have already
    /// been changed by the time a later one turns out not to exist.
    ///
    /// The answer is an operation to poll, not a result: see [`Self::bulk_change`].
    pub async fn bulk_update(
        &self,
        keys: &[String],
        values: &Value,
    ) -> Result<BulkChange, ApiError> {
        let body = serde_json::json!({ "issues": keys, "values": values });
        let (value, _) = self
            .post_value("/v3/bulkchange/_update", &body, "bulk change")
            .await?;
        BulkChange::parse(&value).ok_or_else(|| ApiError::NotFound("bulk change".to_owned()))
    }

    /// `POST /v3/bulkchange/_transition` — one workflow step, many issues.
    ///
    /// `values` carries what the transition demands — a resolution, usually —
    /// exactly as the single-issue path sends it, and is omitted when empty
    /// rather than sent as `{}`.
    pub async fn bulk_transition(
        &self,
        keys: &[String],
        transition: &str,
        values: &Value,
    ) -> Result<BulkChange, ApiError> {
        let mut body = serde_json::json!({ "issues": keys, "transition": transition });
        if !values.as_object().is_some_and(serde_json::Map::is_empty)
            && let Some(object) = body.as_object_mut()
        {
            object.insert("values".to_owned(), values.clone());
        }
        let (value, _) = self
            .post_value("/v3/bulkchange/_transition", &body, "bulk change")
            .await?;
        BulkChange::parse(&value).ok_or_else(|| ApiError::NotFound("bulk change".to_owned()))
    }

    /// `POST /v3/bulkchange/_move` — many issues into one queue.
    ///
    /// Every key in the list changes, and nothing undoes that; the gate this
    /// goes through asks for `--yes` even for a single issue for that reason.
    pub async fn bulk_move(
        &self,
        keys: &[String],
        queue: &str,
        keep_fields: bool,
        initial_status: bool,
    ) -> Result<BulkChange, ApiError> {
        let body = serde_json::json!({
            "issues": keys,
            "queue": queue,
            "moveAllFields": keep_fields,
            "initialStatus": initial_status,
        });
        let (value, _) = self
            .post_value("/v3/bulkchange/_move", &body, "bulk change")
            .await?;
        BulkChange::parse(&value).ok_or_else(|| ApiError::NotFound("bulk change".to_owned()))
    }

    /// `GET /v3/bulkchange/{id}` — how far a bulk change got.
    pub async fn bulk_change(&self, id: &str) -> Result<BulkChange, ApiError> {
        let value = self
            .get_value(
                &format!("/v3/bulkchange/{id}"),
                &format!("bulk change {id}"),
            )
            .await?;
        BulkChange::parse(&value).ok_or_else(|| ApiError::NotFound(format!("bulk change {id}")))
    }

    /// `GET /v3/bulkchange/{id}/issues` — what happened to each issue.
    ///
    /// Only worth a request when the counts do not already say everything: the
    /// point of a bulk change is one request instead of fifty, and printing a
    /// line per issue that succeeded would spend the saving on the output.
    pub async fn bulk_change_issues(&self, id: &str) -> Result<Vec<BulkOutcome>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/bulkchange/{id}/issues"),
                &format!("bulk change {id}"),
            )
            .await?;
        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(BulkOutcome::parse).collect())
            .unwrap_or_default())
    }

    /// One of the four organisation-wide dictionaries.
    ///
    /// Small and unpaged — the largest of the four is statuses, in the dozens —
    /// so this asks for the whole thing and says nothing about pages.
    pub async fn dictionary(&self, kind: Dictionary) -> Result<Vec<DictEntry>, ApiError> {
        let raw = self
            .get_value(&format!("/v3/{}", kind.path()), kind.path())
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::dict_entry).collect())
            .unwrap_or_default())
    }

    /// One page of the organisation's directory.
    ///
    /// Paged, unlike the dictionaries: an organisation has as many people in it
    /// as it has people, and the one this was written against already answers
    /// with a three-figure total.
    pub async fn users(&self, page: u32, per_page: u32) -> Result<Page<Person>, ApiError> {
        let path = format!("/v3/users?page={page}&perPage={per_page}");
        let (value, headers) = self
            .send_value(reqwest::Method::GET, &path, None, "users")
            .await?;

        let items = value
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::person).collect())
            .unwrap_or_default();

        Ok(Page {
            items,
            page,
            per_page,
            total: headers
                .get("x-total-count")
                .and_then(|count| count.to_str().ok())
                .and_then(|count| count.parse().ok()),
        })
    }

    /// One person, by login or by uid.
    ///
    /// There is no `users/me`: Tracker answers 404 for it, and `myself` is the
    /// endpoint that question belongs to.
    pub async fn user(&self, who: &str) -> Result<Person, ApiError> {
        let raw = self
            .get_value(&format!("/v3/users/{who}"), &format!("user {who}"))
            .await?;

        parse::person(&raw).ok_or_else(|| ApiError::NotFound(format!("user {who}")))
    }

    /// Boards visible to the active profile.
    ///
    /// Not paginated by the endpoint, and not by us: an organisation has boards
    /// in the dozens, not the thousands.
    pub async fn boards(&self) -> Result<Vec<Board>, ApiError> {
        let raw = self.get_value("/v3/boards", "boards").await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(Board::parse).collect())
            .unwrap_or_default())
    }

    /// One board.
    pub async fn board(&self, id: &str) -> Result<Board, ApiError> {
        let raw = self
            .get_value(&format!("/v3/boards/{id}"), &format!("board {id}"))
            .await?;

        Board::parse(&raw).ok_or_else(|| ApiError::NotFound(format!("board {id}")))
    }

    /// The sprints of a board.
    ///
    /// A board that cannot have sprints answers with a refusal rather than an
    /// empty list, and that refusal is passed through as Tracker worded it: a
    /// kanban board having no sprints is Tracker's answer to the question, not
    /// a failure of the command, and inventing an empty list here would hide
    /// which of the two happened.
    pub async fn sprints(&self, board: &str) -> Result<Vec<Sprint>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/boards/{board}/sprints"),
                &format!("board {board} sprints"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(Sprint::parse).collect())
            .unwrap_or_default())
    }

    /// Every sprint in the organisation.
    ///
    /// `board sprints ID` needs the board first, and a sprint name is a thing
    /// people say without knowing which board it belongs to. This is the same
    /// records with the board named on each.
    pub async fn all_sprints(&self) -> Result<Vec<Sprint>, ApiError> {
        let raw = self.get_value("/v3/sprints", "sprints").await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(Sprint::parse).collect())
            .unwrap_or_default())
    }

    /// The fields a queue defines itself.
    ///
    /// Not a subset of [`Self::queue_fields`], which lists everything the queue
    /// can use: a local field belongs to the queue, is invisible to the
    /// organisation-wide listing, and cannot be fetched through `/v3/fields` at
    /// all. So these carry their full definition — what they accept included —
    /// because there is no second command that could answer that for them.
    pub async fn queue_local_fields(&self, key: &str) -> Result<Vec<FieldSpec>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/queues/{key}/localFields"),
                &format!("local fields of queue {key}"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(FieldSpec::parse).collect())
            .unwrap_or_default())
    }

    /// Create a project, portfolio or goal with nothing but a name.
    ///
    /// Everything else about an entity is optional, and a command line is not
    /// where a portfolio's description gets written.
    pub async fn create_entity(&self, kind: &str, fields: &Value) -> Result<Entity, ApiError> {
        let body = serde_json::json!({ "fields": fields });
        let (value, _) = self
            .post_value(
                &format!("/v3/entities/{kind}?fields={ENTITY_FIELDS}"),
                &body,
                kind,
            )
            .await?;

        parse::entity(&value).ok_or_else(|| ApiError::NotFound(kind.to_owned()))
    }

    /// Delete a project, portfolio or goal.
    ///
    /// Entities can be deleted; issues cannot. That asymmetry is why the live
    /// suite may write entities and may not write issues without being told a
    /// queue to sacrifice.
    pub async fn delete_entity(&self, kind: &str, id: &str) -> Result<(), ApiError> {
        self.send_value(
            reqwest::Method::DELETE,
            &format!("/v3/entities/{kind}/{id}"),
            None,
            &format!("{kind} {id}"),
        )
        .await?;
        Ok(())
    }

    /// Change the fields of a project, portfolio or goal.
    ///
    /// Quotes the version for the same reason [`Self::place_entity`] does: a
    /// write without one lands on top of whatever happened in between.
    pub async fn update_entity(
        &self,
        kind: &str,
        id: &str,
        fields: &Value,
        version: Option<u64>,
    ) -> Result<Entity, ApiError> {
        let path = match version {
            Some(version) => {
                format!("/v3/entities/{kind}/{id}?version={version}&fields={ENTITY_FIELDS}")
            }
            None => format!("/v3/entities/{kind}/{id}?fields={ENTITY_FIELDS}"),
        };
        let body = serde_json::json!({ "fields": fields });

        let (value, _) = self
            .send_value(
                reqwest::Method::PATCH,
                &path,
                Some(&body),
                &format!("{kind} {id}"),
            )
            .await?;

        parse::entity(&value).ok_or_else(|| ApiError::NotFound(format!("{kind} {id}")))
    }

    /// Put an entity inside a portfolio, or take it out of one.
    ///
    /// `version` is Tracker's optimistic-concurrency counter and is quoted on
    /// purpose: without it the write lands whatever happened in between, and
    /// with it a portfolio that moved under us answers 412 instead of being
    /// silently overwritten.
    pub async fn place_entity(
        &self,
        kind: &str,
        id: &str,
        parent: Option<&str>,
        version: Option<u64>,
    ) -> Result<Entity, ApiError> {
        // The response is the entity as it now stands, but only of the fields
        // asked for — without this it comes back with an empty `fields` and the
        // command prints a blank summary after a write that worked.
        let path = match version {
            Some(version) => {
                format!("/v3/entities/{kind}/{id}?version={version}&fields={ENTITY_FIELDS}")
            }
            None => format!("/v3/entities/{kind}/{id}?fields={ENTITY_FIELDS}"),
        };
        let body = serde_json::json!({
            "fields": { "parentEntity": place_body(parent) }
        });

        let (value, _) = self
            .send_value(
                reqwest::Method::PATCH,
                &path,
                Some(&body),
                &format!("{kind} {id}"),
            )
            .await?;

        parse::entity(&value).ok_or_else(|| ApiError::NotFound(format!("{kind} {id}")))
    }

    /// One queue and its settings.
    pub async fn queue(&self, key: &str) -> Result<QueueSettings, ApiError> {
        let raw = self
            .get_value(&format!("/v3/queues/{key}"), &format!("queue {key}"))
            .await?;

        QueueSettings::parse(&raw).ok_or_else(|| ApiError::NotFound(format!("queue {key}")))
    }

    /// The parts of a queue that another queue can be built from.
    ///
    /// `issueTypesConfig` pairs each issue type with a workflow and a set of
    /// resolutions, and workflow ids are organisation-specific strings nobody
    /// has memorised. Copying them from a queue that already works is the only
    /// way to create one from a command line without asking for internals.
    pub async fn queue_blueprint(&self, key: &str) -> Result<Blueprint, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/queues/{key}?expand=all"),
                &format!("queue {key}"),
            )
            .await?;

        let named = |name: &str| {
            raw.get(name)
                .and_then(|field| field.get("key"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        };

        let types = raw
            .get("issueTypesConfig")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(serde_json::json!({
                            "issueType": entry.get("issueType")?.get("key")?.as_str()?,
                            "workflow": entry.get("workflow")?.get("id")?.as_str()?,
                            "resolutions": entry
                                .get("resolutions")
                                .and_then(Value::as_array)
                                .map(|resolutions| {
                                    resolutions
                                        .iter()
                                        .filter_map(|resolution| {
                                            resolution.get("key").and_then(Value::as_str)
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default(),
                        }))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if types.is_empty() {
            return Err(ApiError::NotFound(format!("issue types of queue {key}")));
        }

        Ok(Blueprint {
            default_type: named("defaultType"),
            default_priority: named("defaultPriority"),
            issue_types: types,
        })
    }

    /// Create a queue.
    pub async fn create_queue(&self, body: &Value) -> Result<QueueSettings, ApiError> {
        let (value, _) = self.post_value("/v3/queues", body, "queue").await?;

        QueueSettings::parse(&value)
            .ok_or_else(|| ApiError::NotFound("the created queue".to_owned()))
    }

    /// Every field defined in the organisation, not just one queue's.
    ///
    /// `queue fields` answers "what can I set on an issue here"; this answers
    /// "what exists at all", which is the question behind a field that a queue
    /// does not show.
    pub async fn fields(&self) -> Result<Vec<QueueField>, ApiError> {
        let raw = self.get_value("/v3/fields", "fields").await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(QueueField::parse).collect())
            .unwrap_or_default())
    }

    /// One field's definition, by the key `queue fields` prints.
    ///
    /// A local field defined inside one queue is not reachable here — it lives
    /// under the queue — and Tracker answers 404 for it, which is the honest
    /// answer rather than one worth papering over.
    pub async fn field(&self, key: &str) -> Result<FieldSpec, ApiError> {
        let raw = self
            .get_value(&format!("/v3/fields/{key}"), &format!("field {key}"))
            .await?;

        FieldSpec::parse(&raw).ok_or_else(|| ApiError::NotFound(format!("field {key}")))
    }

    /// Issue or comment templates.
    ///
    /// The path is `issueTemplates` and `commentTemplates`; there is no
    /// `_templates` collection, which is worth writing down because every
    /// plausible guess at one answers 400 or 404.
    pub async fn templates(&self, kind: TemplateKind) -> Result<Vec<Template>, ApiError> {
        let raw = self
            .get_value(&format!("/v3/{}", kind.path()), kind.path())
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(Template::parse).collect())
            .unwrap_or_default())
    }

    /// The comments of an issue.
    ///
    /// Fetched in one generous page: an issue with more than a hundred comments
    /// is rare enough that paginating here would cost more in complexity than it
    /// saves anyone.
    pub async fn issue_comments(&self, key: &str) -> Result<Vec<Comment>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/issues/{key}/comments?perPage=100"),
                &format!("issue {key} comments"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(parse::comment).collect())
            .unwrap_or_default())
    }

    /// The fields of a queue, including custom ones, as `(key, name, type)`.
    pub async fn queue_fields(&self, key: &str) -> Result<Vec<QueueField>, ApiError> {
        let raw = self
            .get_value(
                &format!("/v3/queues/{key}/fields"),
                &format!("queue {key} fields"),
            )
            .await?;

        Ok(raw
            .as_array()
            .map(|entries| entries.iter().filter_map(QueueField::parse).collect())
            .unwrap_or_default())
    }

    /// A POST that also hands back the response headers, which is where Tracker
    /// puts the pagination totals.
    async fn post_value(
        &self,
        path: &str,
        body: &Value,
        what: &str,
    ) -> Result<(Value, reqwest::header::HeaderMap), ApiError> {
        self.send_value(reqwest::Method::POST, path, Some(body), what)
            .await
    }

    async fn send_value(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        what: &str,
    ) -> Result<(Value, reqwest::header::HeaderMap), ApiError> {
        let url = format!("{}{path}", self.base_url);

        let send = || async {
            let mut request = self.http.request(method.clone(), &url);
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = request.send().await?;
            let headers = response.headers().clone();
            let text = classify(response, what).await?;
            Ok((text, headers))
        };

        // Only idempotent work is retried. Re-sending a create after a timeout
        // would risk a duplicate issue, which is worse than a clear failure.
        let (text, headers) = if method == reqwest::Method::GET {
            send.retry(
                ExponentialBuilder::default()
                    .with_max_times(self.retries)
                    .with_jitter(),
            )
            .when(is_retryable)
            .await?
        } else {
            send().await?
        };

        // A successful write may answer with an empty body.
        let value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).map_err(ApiError::Decode)?
        };
        Ok((value, headers))
    }

    async fn get_value(&self, path: &str, what: &str) -> Result<Value, ApiError> {
        Ok(self
            .send_value(reqwest::Method::GET, path, None, what)
            .await?
            .0)
    }
}

/// Which organisation-wide dictionary to read.
///
/// The four endpoints answer with the same shape but are not spelled the way
/// the values are: the endpoint is `issuetypes`, the field on an issue is
/// `type`, and the flag people reach for is `--type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dictionary {
    Types,
    Priorities,
    Statuses,
    Resolutions,
}

impl Dictionary {
    /// Every dictionary, in the order a listing shows them: what an issue *is*,
    /// then how urgent, then where it stands, then how it ended.
    pub const ALL: [Self; 4] = [
        Self::Types,
        Self::Priorities,
        Self::Statuses,
        Self::Resolutions,
    ];

    #[must_use]
    pub fn path(self) -> &'static str {
        match self {
            Self::Types => "issuetypes",
            Self::Priorities => "priorities",
            Self::Statuses => "statuses",
            Self::Resolutions => "resolutions",
        }
    }

    /// What to call it in output, singular-free: these are always lists.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Types => "types",
            Self::Priorities => "priorities",
            Self::Statuses => "statuses",
            Self::Resolutions => "resolutions",
        }
    }
}

/// A workflow transition available from the current status.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Transition {
    pub id: String,
    pub name: String,
    /// The status the issue lands in.
    pub to: Option<String>,
}

impl Transition {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id").and_then(Value::as_str)?.to_owned(),
            name: value
                .get("display")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            to: value
                .get("to")
                .and_then(|to| to.get("display").or_else(|| to.get("key")))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

/// A queue, reduced to what a listing shows.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Queue {
    pub key: String,
    pub name: String,
    pub lead: Option<String>,
}

impl Queue {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            key: value.get("key").and_then(Value::as_str)?.to_owned(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            lead: value
                .get("lead")
                .and_then(|lead| {
                    lead.get("login")
                        .or_else(|| lead.get("display"))
                        .or_else(|| lead.get("id"))
                })
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

/// A release a queue tracks work against.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Version {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// `released`, `archived`, or `open` when it is neither.
    pub state: &'static str,
    pub due: Option<String>,
}

impl Version {
    fn parse(value: &Value) -> Option<Self> {
        let flag = |member: &str| value.get(member).and_then(Value::as_bool).unwrap_or(false);

        Some(Self {
            id: match value.get("id")? {
                Value::String(id) => id.clone(),
                other => other.to_string(),
            },
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned),
            // Archived wins over released: an archived version is out of use
            // whether or not it ever shipped.
            state: if flag("archived") {
                "archived"
            } else if flag("released") {
                "released"
            } else {
                "open"
            },
            due: value
                .get("dueDate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

/// A board, reduced to what a listing shows.
///
/// Columns are the reason to look at a board from a command line: they are the
/// statuses the board arranges work by, in the order it arranges them.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Board {
    pub id: String,
    pub name: String,
    pub columns: Vec<String>,
    /// The field the board estimates by, when it estimates.
    pub estimate_by: Option<String>,
    pub owner: Option<String>,
}

impl Board {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: match value.get("id")? {
                Value::String(id) => id.clone(),
                other => other.to_string(),
            },
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            columns: value
                .get("columns")
                .and_then(Value::as_array)
                .map(|columns| {
                    columns
                        .iter()
                        .filter_map(|column| {
                            column
                                .get("display")
                                .or_else(|| column.get("id"))
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            estimate_by: value
                .get("estimateBy")
                .and_then(|field| field.get("id").or_else(|| field.get("display")))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            // Boards carry `createdBy`, not a lead, and a real organisation
            // showed that user has a display name and no login.
            owner: value
                .get("createdBy")
                .and_then(|user| {
                    user.get("login")
                        .or_else(|| user.get("display"))
                        .or_else(|| user.get("id"))
                })
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

/// One sprint of a board.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Sprint {
    pub id: String,
    pub name: String,
    pub status: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    /// Which board it belongs to. Absent when the sprint was read through that
    /// board, which already named it, and present when it was listed across the
    /// organisation, where it is what makes two sprints called "Sprint 1"
    /// tellable apart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
}

impl Sprint {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: match value.get("id")? {
                Value::String(id) => id.clone(),
                other => other.to_string(),
            },
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            status: value
                .get("status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            start: value
                .get("startDate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            end: value
                .get("endDate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            board: value
                .get("board")
                .and_then(|board| board.get("display").or_else(|| board.get("id")))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

/// The parts of an existing queue a new one can be built from.
#[derive(Debug, Clone)]
pub struct Blueprint {
    pub default_type: Option<String>,
    pub default_priority: Option<String>,
    /// `issueTypesConfig` as the create endpoint takes it: keys and ids, not
    /// the expanded objects the read answers with.
    pub issue_types: Vec<Value>,
}

/// What `parentEntity` is set to: a portfolio, or nothing.
///
/// Removing is `null`, not an empty object — an empty object is a change
/// Tracker accepts and ignores, which reads as success and is not.
fn place_body(parent: Option<&str>) -> Value {
    match parent {
        Some(parent) => serde_json::json!({ "primary": parent }),
        None => Value::Null,
    }
}

/// A queue with the settings that decide what an issue in it starts as.
///
/// The defaults are the point: `issue create -q PROJ` without a type or a
/// priority gets these, and nothing else says what they are.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueueSettings {
    pub key: String,
    pub name: String,
    pub lead: Option<String>,
    pub default_type: Option<String>,
    pub default_priority: Option<String>,
    pub version: Option<u64>,
}

impl QueueSettings {
    fn parse(value: &Value) -> Option<Self> {
        let named = |name: &str| {
            value
                .get(name)
                .and_then(|field| field.get("key").or_else(|| field.get("display")))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        };

        Some(Self {
            key: value.get("key").and_then(Value::as_str)?.to_owned(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            lead: value
                .get("lead")
                .and_then(|lead| {
                    lead.get("login")
                        .or_else(|| lead.get("display"))
                        .or_else(|| lead.get("id"))
                })
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            default_type: named("defaultType"),
            default_priority: named("defaultPriority"),
            version: value.get("version").and_then(Value::as_u64),
        })
    }
}

/// Which templates are being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Issue,
    Comment,
}

impl TemplateKind {
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Issue => "issueTemplates",
            Self::Comment => "commentTemplates",
        }
    }
}

/// One template, reduced to what a listing shows.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    /// The queue a template belongs to, when it belongs to one.
    pub queue: Option<String>,
    pub author: Option<String>,
}

impl Template {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: match value.get("id")? {
                Value::String(id) => id.clone(),
                other => other.to_string(),
            },
            name: value
                .get("name")
                .or_else(|| value.get("summary"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            queue: value
                .get("queue")
                .and_then(|queue| queue.get("key").or_else(|| queue.get("id")).or(Some(queue)))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            author: value
                .get("createdBy")
                .or_else(|| value.get("author"))
                .and_then(|user| {
                    user.get("login")
                        .or_else(|| user.get("display"))
                        .or_else(|| user.get("id"))
                })
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

/// One field of a queue. `queue fields` is how a caller learns the keys that
/// `--fields` and `--set` accept, so the key matters more than the name here.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueueField {
    pub key: String,
    pub name: String,
    pub field_type: String,
    /// A field Tracker ships with, as opposed to one this queue defines.
    pub system: bool,
}

impl QueueField {
    fn parse(value: &Value) -> Option<Self> {
        let id = value.get("id").and_then(Value::as_str)?;
        Some(Self {
            // Custom fields are addressed by the trailing segment of a
            // dotted id (`60...--storyPoints`), which is what the API accepts
            // back and what a caller can reasonably type.
            key: id.rsplit("--").next().unwrap_or(id).to_owned(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_owned(),
            field_type: value
                .get("schema")
                .and_then(|schema| schema.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            system: !id.contains("--"),
        })
    }
}

/// One kind of relationship two issues can have.
///
/// Deliberately not folded into [`Dictionary`]: the four dictionaries are values
/// a *field* takes and share one shape, and this has neither a key nor a name —
/// it has an id and two labels, one per direction. It is also not the vocabulary
/// a write takes, which is the whole reason it is worth printing.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LinkType {
    pub id: String,
    /// Tracker's wording for the end the link points away from.
    pub outward: Option<String>,
    /// Tracker's wording for the end it points at.
    pub inward: Option<String>,
}

impl BulkChange {
    /// Whether Tracker is done with it, one way or the other.
    #[must_use]
    pub fn finished(&self) -> bool {
        matches!(self.status.as_str(), "COMPLETE" | "FAILED")
    }

    /// Whether every issue it was given actually changed.
    ///
    /// `COMPLETE` alone does not say this: a change can finish having changed
    /// nothing, and that must not exit zero.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.status == "COMPLETE" && self.done.is_some() && self.done == self.total
    }

    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id").and_then(Value::as_str)?.to_owned(),
            status: value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            status_text: value
                .get("statusText")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            total: value.get("totalIssues").and_then(Value::as_u64),
            done: value.get("totalCompletedIssues").and_then(Value::as_u64),
        })
    }
}

impl BulkOutcome {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            key: value
                .get("issue")
                .and_then(|issue| issue.get("key"))
                .and_then(Value::as_str)?
                .to_owned(),
            status: value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            error: value.get("error").and_then(field_errors),
        })
    }
}

/// Tracker's per-field complaints, joined into one sentence.
///
/// The shape is the same envelope every rejection uses — `errors` keyed by
/// field, `errorMessages` for the rest — and both halves are passed through as
/// written. A message about somebody's field is theirs, not ours to reword.
fn field_errors(error: &Value) -> Option<String> {
    let mut parts: Vec<String> = error
        .get("errors")
        .and_then(Value::as_object)
        .map(|fields| {
            fields
                .iter()
                .filter_map(|(field, message)| {
                    message
                        .as_str()
                        .map(|message| format!("{field}: {message}"))
                })
                .collect()
        })
        .unwrap_or_default();
    parts.extend(
        error
            .get("errorMessages")
            .and_then(Value::as_array)
            .map(|messages| {
                messages
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

impl Permission {
    /// Every operation in the answer, in a fixed order.
    ///
    /// Tracker's own order is whatever the JSON object happened to have, and
    /// the order of these columns is a contract. `create` before `read` before
    /// the two kinds of `write` before `grant` runs from the least to the most
    /// a right lets somebody do; anything Tracker adds later lands after them
    /// rather than silently between them.
    fn parse_all(value: &Value) -> Vec<Self> {
        const ORDER: [&str; 5] = ["create", "read", "write", "writeNoAssign", "grant"];

        let Some(object) = value.as_object() else {
            return Vec::new();
        };

        let known = ORDER
            .iter()
            .filter_map(|name| object.get(*name).map(|entry| Self::parse(name, entry)));
        let rest = object
            .iter()
            .filter(|(name, entry)| !ORDER.contains(&name.as_str()) && entry.is_object())
            // `self` and `version` are the answer's own metadata, not
            // operations, and they are objects nowhere — but the filter above
            // is about names, so they are named here too.
            .filter(|(name, _)| !matches!(name.as_str(), "self" | "version"))
            .map(|(name, entry)| Self::parse(name, entry));

        known.chain(rest).collect()
    }

    fn parse(operation: &str, value: &Value) -> Self {
        let holders = |member: &str| {
            value
                .get(member)
                .and_then(Value::as_array)
                .map(|entries| entries.iter().filter_map(Holder::parse).collect())
                .unwrap_or_default()
        };
        Self {
            operation: operation.to_owned(),
            users: holders("users"),
            groups: holders("groups"),
            roles: holders("roles"),
        }
    }
}

impl Holder {
    fn parse(value: &Value) -> Option<Self> {
        let id = id_of(value)?;
        Some(Self {
            display: value
                .get("display")
                .and_then(Value::as_str)
                // A holder with no display is still a holder; the id is a worse
                // name than the display and a better one than nothing.
                .map_or_else(|| id.clone(), ToOwned::to_owned),
            id,
        })
    }
}

impl LinkType {
    fn parse(value: &Value) -> Option<Self> {
        let text = |member: &str| {
            value
                .get(member)
                .and_then(Value::as_str)
                .map(str::to_lowercase)
        };
        Some(Self {
            id: value.get("id").and_then(Value::as_str)?.to_owned(),
            outward: text("outward"),
            inward: text("inward"),
        })
    }
}

/// A part of the product a queue splits its work by.
///
/// `components` is a field on every issue, and until it could be listed a write
/// to it was a guess. Half of them have no lead, so that column is genuinely
/// optional rather than defensively so.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Component {
    pub id: String,
    pub name: String,
    /// The queue it belongs to. A component belongs to exactly one.
    pub queue: Option<String>,
    pub lead: Option<String>,
    /// Whether adding this component assigns the issue to its lead. It changes
    /// what a write does, which is why it is a column and not a detail.
    pub assign_auto: bool,
    pub description: Option<String>,
}

impl Component {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: id_of(value)?,
            name: named(value),
            queue: value
                .get("queue")
                .and_then(|queue| queue.get("key").or_else(|| queue.get("display")))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            lead: value
                .get("lead")
                .and_then(|lead| {
                    lead.get("login")
                        .or_else(|| lead.get("display"))
                        .or_else(|| lead.get("id"))
                })
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            assign_auto: value
                .get("assignAuto")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned),
        })
    }
}

/// What changes issues in a queue without anybody touching them.
///
/// One answer assembled from three endpoints, because they are three halves of
/// one question: an issue whose changelog says it was updated by the Tracker
/// robot was changed by one of these.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Automation {
    pub macros: Vec<Macro>,
    pub autoactions: Vec<AutoAction>,
    pub triggers: Vec<Trigger>,
    /// The parts Tracker would not show, in its own words.
    ///
    /// Triggers need queue-owner rights and answer 403 to everybody else. Two
    /// sections out of three is a useful answer, and failing the whole command
    /// because of the third would throw them away.
    pub unreadable: Vec<Unreadable>,
}

/// A change to many issues at once, which Tracker performs in the background.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BulkChange {
    pub id: String,
    /// `CREATED`, `COMPLETE`, `FAILED` are the ones this has seen. Anything else
    /// is treated as still running rather than as an outcome, because guessing
    /// which way an unknown status went is the one thing worth not doing here.
    pub status: String,
    /// Tracker's own sentence, in the organisation's language.
    pub status_text: String,
    /// How many issues the change is about, once Tracker has counted them.
    pub total: Option<u64>,
    /// How many of them it finished. The tally a bulk change ends with.
    pub done: Option<u64>,
}

/// What happened to one issue in a bulk change.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BulkOutcome {
    pub key: String,
    pub status: String,
    /// Tracker's own words about why this one did not change.
    pub error: Option<String>,
}

/// Who may do what in a queue: the rules, and the people they come out as.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QueueAccess {
    /// The rule per operation: named holders and roles.
    pub permissions: Vec<Permission>,
    /// The people per operation, with the roles already resolved.
    pub access: Vec<Permission>,
    /// The id of the user the token belongs to, when it could be read. What
    /// makes "who is allowed" into "am I allowed".
    pub you: Option<String>,
    pub unreadable: Vec<Unreadable>,
}

/// One operation, and everybody who holds it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Permission {
    /// `create`, `read`, `write`, `writeNoAssign`, `grant`.
    pub operation: String,
    pub users: Vec<Holder>,
    /// Documented, and absent from every queue this was checked against — so
    /// parsed, printed when present, and claimed about no further than that.
    pub groups: Vec<Holder>,
    /// `queue-lead`, `assignee`, `author`, `follower`, `access`. A role is not
    /// a set of people: which issue is being touched decides who is in it.
    pub roles: Vec<Holder>,
}

/// Somebody or something that holds a right.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Holder {
    pub id: String,
    /// Tracker's own wording, in the organisation's language.
    pub display: String,
}

/// One section that could not be read, and why.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Unreadable {
    pub section: &'static str,
    pub reason: String,
}

/// A canned change somebody applies by hand from the issue page.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Macro {
    pub id: String,
    pub name: String,
    /// The comment it posts, when it posts one.
    pub body: Option<String>,
    /// Which fields it writes. The keys, not the localised names, because the
    /// keys are what every other command here takes.
    pub updates: Vec<String>,
}

/// A change Tracker applies on a schedule to whatever matches a filter.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoAction {
    pub id: String,
    pub name: String,
    pub active: bool,
    /// The kinds of action it performs — `Transition`, `Update`, and the rest.
    pub actions: Vec<String>,
    /// How often it runs, in seconds.
    pub interval: Option<u64>,
}

/// A change Tracker applies the moment something happens to an issue.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Trigger {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub actions: Vec<String>,
    /// How many conditions have to hold. The conditions themselves are a tree
    /// of Tracker's own classes, and printing it would be longer than it is
    /// useful.
    pub conditions: usize,
}

/// The `id` of anything under a queue, whether Tracker sent it as a number or a
/// string.
fn id_of(value: &Value) -> Option<String> {
    Some(match value.get("id")? {
        Value::String(id) => id.clone(),
        other => other.to_string(),
    })
}

/// The `type` of each entry of an array, which is how Tracker names an action.
fn types_in(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("type").and_then(Value::as_str))
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn named(value: &Value) -> String {
    value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

impl Macro {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: id_of(value)?,
            name: named(value),
            body: value
                .get("body")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned),
            updates: value
                .get("issueUpdate")
                .and_then(Value::as_array)
                .map(|updates| {
                    updates
                        .iter()
                        .filter_map(|update| {
                            update
                                .get("field")
                                .and_then(|field| field.get("id"))
                                .and_then(Value::as_str)
                        })
                        .map(|id| id.rsplit("--").next().unwrap_or(id).to_owned())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

impl AutoAction {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: id_of(value)?,
            name: named(value),
            active: value
                .get("active")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            actions: types_in(value.get("actions")),
            // Milliseconds on the wire; seconds is what a person says out loud.
            interval: value
                .get("intervalMillis")
                .and_then(Value::as_u64)
                .map(|millis| millis / 1000),
        })
    }
}

impl Trigger {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            id: id_of(value)?,
            name: named(value),
            active: value
                .get("active")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            actions: types_in(value.get("actions")),
            conditions: value
                .get("conditions")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        })
    }
}

/// One field's definition: what it holds, whether it can be written, and what
/// values it accepts.
///
/// `queue fields` lists the keys; this answers the question that follows, which
/// is the one `--set` is otherwise guessing at.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldSpec {
    pub key: String,
    pub name: String,
    /// `string`, `float`, `user`, `datetime` — Tracker's own vocabulary, which
    /// is what its error messages quote back.
    pub field_type: String,
    /// What one element is, when the field holds several of them. `None` means
    /// the field takes a single value.
    pub items: Option<String>,
    pub required: bool,
    pub readonly: bool,
    /// Where Tracker files the field: `Системные`, `Agile`, and whatever the
    /// organisation added. In the organisation's own language.
    pub category: Option<String>,
    /// How the accepted values are decided, when they are decided at all.
    pub options: Option<FieldOptions>,
}

/// What a constrained field will accept.
///
/// Two cases, and telling them apart is the point: a fixed list carries its
/// values here, and everything else names a provider that answers from
/// somewhere else in the organisation — the directory, the queue, the board.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldOptions {
    /// Tracker's class name for the provider, passed through unchanged: an
    /// unrecognised one still says something, and inventing a friendlier word
    /// for it would only be a word we would have to keep in step.
    pub provider: String,
    pub values: Vec<String>,
}

impl FieldSpec {
    fn parse(value: &Value) -> Option<Self> {
        let id = value.get("id").and_then(Value::as_str)?;
        let schema = value.get("schema");
        let string_at = |parent: Option<&Value>, member: &str| {
            parent
                .and_then(|parent| parent.get(member))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        };

        let options = value.get("optionsProvider").map(|provider| FieldOptions {
            provider: provider
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            // Values arrive as whatever the field holds — the numbers 0 and 1
            // for a flag, strings for a list — and a caller has to type them
            // back either way.
            values: provider
                .get("values")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .map(|value| match value {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        });

        Some(Self {
            key: id.rsplit("--").next().unwrap_or(id).to_owned(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_owned(),
            field_type: string_at(schema, "type").unwrap_or_else(|| "unknown".to_owned()),
            items: string_at(schema, "items"),
            required: schema
                .and_then(|schema| schema.get("required"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            readonly: value
                .get("readonly")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            category: string_at(value.get("category"), "display"),
            options,
        })
    }
}

/// The checklist out of whatever Tracker answered a checklist write with.
///
/// It replies with the issue, not the item, so the list is under
/// `checklistItems`; a bare array is accepted too, because an endpoint that
/// changes its mind about the envelope should not empty somebody's checklist.
fn checklist_of(value: &Value) -> Vec<ChecklistItem> {
    let entries = value
        .get("checklistItems")
        .and_then(Value::as_array)
        .or_else(|| value.as_array());

    entries
        .map(|entries| entries.iter().filter_map(parse::checklist_item).collect())
        .unwrap_or_default()
}

/// Turn a response into either its body or a typed error.
///
/// `what` names the thing being fetched so a 404 can say which one, rather than
/// leaving the caller to guess between the issue and one of its subresources.
async fn classify(response: reqwest::Response, what: &str) -> Result<String, ApiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response.text().await?);
    }

    let message = response.text().await.unwrap_or_default();
    Err(match status.as_u16() {
        401 => ApiError::Unauthorized,
        403 => ApiError::Forbidden,
        404 => ApiError::NotFound(what.to_owned()),
        429 => ApiError::RateLimited,
        _ => ApiError::Rejected {
            status,
            message: complaint(&message),
        },
    })
}

/// What Tracker actually said, out of the envelope it says it in.
///
/// A rejection arrives as `{"errors": …, "errorMessages": […], "statusCode": …}`,
/// and printing the whole envelope buries the one sentence a caller can act on
/// under punctuation it cannot. The body is kept verbatim when it is not that
/// shape, since an unrecognised error is exactly when guessing is worst.
fn complaint(body: &str) -> String {
    let messages = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            let mut said: Vec<String> = value
                .get("errorMessages")
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            // `errors` is keyed by field, and a field-level complaint is the
            // most specific thing in the envelope when it is there.
            if let Some(errors) = value.get("errors").and_then(Value::as_object) {
                said.extend(
                    errors
                        .iter()
                        .filter_map(|(field, text)| Some(format!("{field}: {}", text.as_str()?))),
                );
            }
            (!said.is_empty()).then(|| said.join("; "))
        })
        .unwrap_or_else(|| body.to_owned());

    messages.chars().take(400).collect()
}

/// Retry transport hiccups and server-side backpressure; never retry a request
/// the server has already judged invalid.
fn is_retryable(error: &ApiError) -> bool {
    match error {
        ApiError::RateLimited => true,
        ApiError::Transport(err) => err.is_timeout() || err.is_connect(),
        ApiError::Rejected { status, .. } => status.is_server_error(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentence a caller can act on, not the envelope it arrived in.
    #[test]
    fn a_rejection_reads_as_what_tracker_said() {
        assert_eq!(
            complaint(
                r#"{"errors":{},"errorMessages":["A board of this type cannot have sprints."],"statusCode":400}"#
            ),
            "A board of this type cannot have sprints."
        );
    }

    /// A field-level complaint names its field: `summary` being required is a
    /// different fix from `queue` being wrong.
    #[test]
    fn a_field_complaint_keeps_its_field() {
        assert_eq!(
            complaint(r#"{"errors":{"summary":"cannot be empty"},"errorMessages":[]}"#),
            "summary: cannot be empty"
        );
    }

    /// An unrecognised body is passed through: guessing is worst precisely when
    /// the error is one we have not seen.
    #[test]
    fn an_unfamiliar_body_survives_untouched() {
        assert_eq!(
            complaint("<html>gateway timeout</html>"),
            "<html>gateway timeout</html>"
        );
        assert_eq!(complaint("{}"), "{}");
    }

    #[test]
    fn host_comparison_ignores_scheme_path_and_case() {
        assert_eq!(
            host_of("https://API.tracker.yandex.net/v3/issues/PROJ-1"),
            host_of("https://api.tracker.yandex.net")
        );
    }

    /// The download URL is server-supplied. A different host must not match, or
    /// a crafted attachment could send this client — and its OAuth header —
    /// somewhere else entirely.
    #[test]
    fn a_different_host_does_not_match() {
        assert_ne!(
            host_of("https://evil.example.com/steal"),
            host_of("https://api.tracker.yandex.net")
        );
    }

    /// Nor a host that merely starts the same way.
    #[test]
    fn a_prefix_of_the_real_host_does_not_match() {
        assert_ne!(
            host_of("https://api.tracker.yandex.net.evil.com/steal"),
            host_of("https://api.tracker.yandex.net")
        );
    }

    #[test]
    fn a_port_is_part_of_the_host() {
        assert_ne!(
            host_of("http://127.0.0.1:9999/x"),
            host_of("http://127.0.0.1:8888")
        );
    }
}
