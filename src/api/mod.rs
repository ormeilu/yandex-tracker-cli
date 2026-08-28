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
    Attachment, ChecklistItem, Comment, Entity, Issue, Link, Page, User, Worklog,
};
use crate::config::OrgKind;

/// Default API root. Overridable so tests can point at a `wiremock` server.
pub const DEFAULT_BASE_URL: &str = "https://api.tracker.yandex.net";

/// Entity fields we ask for. Requesting an explicit set keeps the response small
/// and its shape predictable; the endpoints return only identity otherwise.
const ENTITY_FIELDS: &str =
    "summary,description,entityStatus,start,end,lead,author,parentEntity,entityType";

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
            message: message.chars().take(400).collect(),
        },
    })
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
