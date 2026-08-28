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
    Attachment, Change, ChecklistItem, Comment, DictEntry, Entity, Issue, Link, Page, Person, User,
    Worklog,
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
        })
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

    /// Create a project, portfolio or goal with nothing but a name.
    ///
    /// Everything else about an entity is optional, and a command line is not
    /// where a portfolio's description gets written.
    pub async fn create_entity(&self, kind: &str, summary: &str) -> Result<Entity, ApiError> {
        let body = serde_json::json!({ "fields": { "summary": summary } });
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
