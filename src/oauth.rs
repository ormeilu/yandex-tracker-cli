//! Signing in through Yandex OAuth, so nobody has to paste a token.
//!
//! The device-code flow: ask for a short code, show it, and poll until the
//! person has confirmed it in a browser — on this machine or any other, which is
//! what makes it work over SSH and inside an agent's sandbox too. The grant comes
//! with a refresh token, which is what `auth refresh` spends.
//!
//! Exchanging a code for a token needs an application's id and secret. The
//! shared ytcli application's pair is compiled in from the environment of the
//! build, so it never lives in the tree; the same variables at run time win, for
//! anyone who would rather sign in through an application of their own. Why a
//! secret shipped inside a binary is acceptable: `docs/adr/0008-device-sign-in.md`.

use std::time::Duration;

/// Where Yandex OAuth lives. Overridable so tests can point at a stub.
pub const DEFAULT_OAUTH_URL: &str = "https://oauth.yandex.ru";

const CLIENT_ID_ENV: &str = "YTCLI_OAUTH_CLIENT_ID";
const CLIENT_SECRET_ENV: &str = "YTCLI_OAUTH_CLIENT_SECRET";
const URL_ENV: &str = "YTCLI_OAUTH_URL";

/// What `--read-only` asks for instead of everything the application may grant:
/// reading Tracker and reading the Wiki, and nothing that writes to either.
pub const READ_ONLY_SCOPE: &str = "tracker:read wiki:read";

/// Used when Yandex does not say how long to wait between polls.
const DEFAULT_INTERVAL: u64 = 5;
/// Yandex's own addition to the interval when it answers `slow_down`.
const SLOW_DOWN_STEP: u64 = 5;

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error(
        "this build has no OAuth application to sign in with; paste a token instead, \
         or set {CLIENT_ID_ENV} and {CLIENT_SECRET_ENV} to an application of your own"
    )]
    NotConfigured,
    #[error("transport error talking to Yandex OAuth")]
    Transport(#[from] reqwest::Error),
    #[error("the sign-in was declined in the browser")]
    Denied,
    #[error("the code expired before it was confirmed; run the command again")]
    Expired,
    #[error("Yandex OAuth refused the request: {0}")]
    Rejected(String),
    #[error("could not decode the Yandex OAuth response")]
    Decode(#[source] serde_json::Error),
}

impl OAuthError {
    #[must_use]
    pub fn exit_code(&self) -> crate::exit::ExitCode {
        match self {
            Self::Transport(_) | Self::Decode(_) => crate::exit::ExitCode::Failure,
            Self::NotConfigured | Self::Denied | Self::Expired | Self::Rejected(_) => {
                crate::exit::ExitCode::Auth
            }
        }
    }
}

/// A code waiting to be confirmed.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DeviceCode {
    /// What the polls quote back; never shown.
    #[serde(rename = "device_code")]
    secret: String,
    /// What the person types into the page at `verification_url`.
    pub user_code: String,
    pub verification_url: String,
    #[serde(default)]
    interval: Option<u64>,
    /// Seconds until the code stops being accepted.
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// A token, and what renews it.
#[derive(Clone, serde::Deserialize)]
pub struct Grant {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

// Derived Debug would print both tokens into any log that formats a grant.
impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("refresh_token", &self.refresh_token.is_some())
            .finish_non_exhaustive()
    }
}

/// One answer to a poll.
enum Poll {
    Pending,
    SlowDown,
    Granted(Grant),
}

#[derive(serde::Deserialize)]
struct Failure {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// An OAuth application to sign in through.
#[derive(Clone)]
pub struct App {
    http: reqwest::Client,
    base_url: String,
    client_id: String,
    client_secret: String,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("base_url", &self.base_url)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// The application's id and secret: the run-time environment's pair if it has
/// one, otherwise the pair this binary was built with.
///
/// Taken as a pair, never mixed: an id from one source and a secret from the
/// other is an application that does not exist, and Yandex's answer to it —
/// `invalid_client` — says nothing about why.
fn credentials() -> Option<(String, String)> {
    let runtime = std::env::var(CLIENT_ID_ENV)
        .ok()
        .filter(|id| !id.is_empty())
        .map(|id| (id, std::env::var(CLIENT_SECRET_ENV).unwrap_or_default()));
    let built = option_env!("YTCLI_OAUTH_CLIENT_ID")
        .zip(option_env!("YTCLI_OAUTH_CLIENT_SECRET"))
        .map(|(id, secret)| (id.to_owned(), secret.to_owned()));

    runtime
        .or(built)
        .filter(|(id, secret)| !id.is_empty() && !secret.is_empty())
}

impl App {
    /// The application this binary signs in through, if it has one.
    pub fn from_environment() -> Result<Self, OAuthError> {
        let (client_id, client_secret) = credentials().ok_or(OAuthError::NotConfigured)?;
        let base_url = std::env::var(URL_ENV)
            .ok()
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| DEFAULT_OAUTH_URL.to_owned());
        Self::new(&base_url, client_id, client_secret)
    }

    /// An application given explicitly, at an explicit address.
    pub fn new(
        base_url: &str,
        client_id: String,
        client_secret: String,
    ) -> Result<Self, OAuthError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("ytcli/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            client_id,
            client_secret,
        })
    }

    /// Whether signing in through the browser is possible at all.
    #[must_use]
    pub fn is_configured() -> bool {
        credentials().is_some()
    }

    /// `POST /device/code` — a code for the person to confirm.
    ///
    /// Without a scope the token gets everything the application was registered
    /// with; with one, only that.
    pub async fn request_code(&self, scope: Option<&str>) -> Result<DeviceCode, OAuthError> {
        let mut fields = vec![
            ("client_id", self.client_id.as_str()),
            ("device_name", "ytcli"),
        ];
        if let Some(scope) = scope {
            fields.push(("scope", scope));
        }

        let response = self.post("/device/code", &fields).await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(failure(status, &body));
        }
        serde_json::from_str(&body).map_err(OAuthError::Decode)
    }

    /// Ask once: the grant if the code has been confirmed already, `None` if
    /// not yet.
    pub async fn try_grant(&self, code: &DeviceCode) -> Result<Option<Grant>, OAuthError> {
        match self.poll(code).await? {
            Poll::Granted(grant) => Ok(Some(grant)),
            Poll::Pending | Poll::SlowDown => Ok(None),
        }
    }

    /// Poll until the code is confirmed, declined, or runs out.
    pub async fn await_grant(&self, code: &DeviceCode) -> Result<Grant, OAuthError> {
        let mut interval = code.interval.unwrap_or(DEFAULT_INTERVAL);
        let deadline = code
            .expires_in
            .map(|seconds| std::time::Instant::now() + Duration::from_secs(seconds));

        loop {
            match self.poll(code).await? {
                Poll::Granted(grant) => return Ok(grant),
                Poll::SlowDown => interval += SLOW_DOWN_STEP,
                Poll::Pending => {}
            }

            if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                return Err(OAuthError::Expired);
            }
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    }

    async fn poll(&self, code: &DeviceCode) -> Result<Poll, OAuthError> {
        self.exchange(&[
            ("grant_type", "device_code"),
            ("code", code.secret.as_str()),
        ])
        .await
    }

    /// `POST /token` with `grant_type=refresh_token`.
    pub async fn refresh(&self, refresh_token: &str) -> Result<Grant, OAuthError> {
        match self
            .exchange(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .await?
        {
            Poll::Granted(grant) => Ok(grant),
            // Neither belongs to a refresh; seeing one means Yandex answered a
            // question that was not asked.
            Poll::Pending | Poll::SlowDown => Err(OAuthError::Rejected(
                "an unexpected pending answer to a refresh".to_owned(),
            )),
        }
    }

    async fn exchange(&self, grant: &[(&str, &str)]) -> Result<Poll, OAuthError> {
        let mut fields = grant.to_vec();
        fields.push(("client_id", self.client_id.as_str()));
        fields.push(("client_secret", self.client_secret.as_str()));

        let response = self.post("/token", &fields).await?;
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            return serde_json::from_str(&body)
                .map(Poll::Granted)
                .map_err(OAuthError::Decode);
        }

        match serde_json::from_str::<Failure>(&body) {
            Ok(failure) if failure.error == "authorization_pending" => Ok(Poll::Pending),
            Ok(failure) if failure.error == "slow_down" => Ok(Poll::SlowDown),
            _ => Err(failure(status, &body)),
        }
    }

    async fn post(
        &self,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Result<reqwest::Response, OAuthError> {
        Ok(self
            .http
            .post(format!("{}{path}", self.base_url))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form(fields))
            .send()
            .await?)
    }
}

/// Turn an error answer into the error that says what to do about it.
fn failure(status: reqwest::StatusCode, body: &str) -> OAuthError {
    match serde_json::from_str::<Failure>(body) {
        Ok(failure) => match failure.error.as_str() {
            "access_denied" => OAuthError::Denied,
            "expired_token" => OAuthError::Expired,
            // Yandex checks the secret only once the code is confirmed, so this
            // arrives after the person has done everything right, and has to
            // say it was not their doing.
            "invalid_client" => OAuthError::Rejected(format!(
                "invalid_client — {}. The application's id or secret is wrong: \
                 check {CLIENT_ID_ENV} and {CLIENT_SECRET_ENV}, or the build that set them",
                failure
                    .error_description
                    .as_deref()
                    .unwrap_or("unknown client")
            )),
            _ => OAuthError::Rejected(failure.error_description.map_or_else(
                || failure.error.clone(),
                |description| format!("{} — {description}", failure.error),
            )),
        },
        Err(_) => OAuthError::Rejected(status.to_string()),
    }
}

/// `application/x-www-form-urlencoded`, by hand: reqwest's encoder sits behind a
/// feature, and five fields do not justify turning it on.
fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode(text: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scope list is two words joined by a space, each with a colon in it;
    /// sent unencoded, Yandex reads the second word as a stray parameter.
    #[test]
    fn a_scope_list_survives_the_form_encoding() {
        assert_eq!(
            form(&[("scope", "tracker:read wiki:read")]),
            "scope=tracker%3Aread%20wiki%3Aread"
        );
    }
}
