//! HTTP layer against the Tracker REST API.
//!
//! We talk to the API directly instead of using the official Python-era client:
//! see `docs/adr/0004-own-http-client.md`.

pub mod error;
pub mod models;

use std::time::Duration;

use backon::{ExponentialBuilder, Retryable};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};

use crate::api::error::ApiError;
use crate::api::models::User;
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
        let value: serde_json::Value = self.get_json("/v3/myself").await?;
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

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = format!("{}{path}", self.base_url);

        let send = || async {
            let response = self.http.get(&url).send().await?;
            classify(response).await
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

/// Turn a response into either its body or a typed error.
async fn classify(response: reqwest::Response) -> Result<String, ApiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response.text().await?);
    }

    let message = response.text().await.unwrap_or_default();
    Err(match status.as_u16() {
        401 => ApiError::Unauthorized,
        403 => ApiError::Forbidden,
        404 => ApiError::NotFound("resource".to_owned()),
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
