//! HTTP layer against the Tracker REST API.
//!
//! We talk to the API directly instead of using the official Python-era client:
//! see `docs/adr/0004-own-http-client.md`.

pub mod error;
pub mod models;
pub mod parse;
pub mod query;

use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};

use serde_json::Value;

use crate::api::error::ApiError;
use crate::api::models::{Issue, Link, Page, User};
use crate::config::OrgKind;

/// Default API root. Overridable so tests can point at a `wiremock` server.
pub const DEFAULT_BASE_URL: &str = "https://api.tracker.yandex.net";

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
        let url = format!("{}{path}", self.base_url);

        let send = || async {
            let response = self.http.post(&url).json(body).send().await?;
            let headers = response.headers().clone();
            let text = classify(response, what).await?;
            Ok((text, headers))
        };

        let (text, headers) = send
            .retry(
                ExponentialBuilder::default()
                    .with_max_times(self.retries)
                    .with_jitter(),
            )
            .when(is_retryable)
            .await?;

        let value = serde_json::from_str(&text).map_err(ApiError::Decode)?;
        Ok((value, headers))
    }

    async fn get_value(&self, path: &str, what: &str) -> Result<Value, ApiError> {
        let url = format!("{}{path}", self.base_url);

        let send = || async {
            let response = self.http.get(&url).send().await?;
            classify(response, what).await
        };

        let body = send
            .retry(
                ExponentialBuilder::default()
                    .with_max_times(self.retries)
                    .with_jitter(),
            )
            .when(is_retryable)
            .await?;

        serde_json::from_str(&body).map_err(ApiError::Decode)
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
