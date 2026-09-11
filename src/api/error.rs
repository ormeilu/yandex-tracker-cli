//! Typed API failures.
//!
//! The variants exist so the shell can map them to distinct exit codes and to
//! actionable messages; a single opaque "request failed" would make both
//! impossible.

use crate::exit::ExitCode;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("transport error talking to Tracker")]
    Transport(#[from] reqwest::Error),
    #[error("not authenticated: the token was rejected (401)")]
    Unauthorized,
    #[error("forbidden (403): the account lacks rights, or the organisation header is wrong")]
    Forbidden,
    // The Wiki's refusal has a likelier cause than Tracker's: a token issued
    // before the Wiki permission was added to the application. Saying so turns
    // a rights puzzle into one command.
    #[error(
        "the Wiki refused this token: it needs the wiki:read permission — sign in again \
         with `ytcli auth login` — or this account cannot see that page"
    )]
    WikiForbidden,
    // The Wiki answers 403 with `FORCED_SYNC_REQUIRED` when it has never heard of
    // the organisation: the Wiki was not opened there yet. No sign-in fixes
    // that, so blaming the token would send people round in circles.
    #[error(
        "the Wiki is not set up in this organisation yet — open https://wiki.yandex.ru once, \
         signed in to it, and try again"
    )]
    WikiNotEnabled,
    #[error("{0} not found")]
    NotFound(String),
    #[error("rate limited by Tracker (429)")]
    RateLimited,
    #[error("Tracker rejected the request ({status}): {message}")]
    Rejected {
        status: reqwest::StatusCode,
        message: String,
    },
    #[error("could not decode the Tracker response")]
    Decode(#[source] serde_json::Error),
}

impl ApiError {
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Unauthorized | Self::WikiForbidden => ExitCode::Auth,
            Self::NotFound(_) => ExitCode::NotFound,
            Self::Forbidden | Self::WikiNotEnabled | Self::RateLimited | Self::Rejected { .. } => {
                ExitCode::ApiRejected
            }
            Self::Transport(_) | Self::Decode(_) => ExitCode::Failure,
        }
    }
}
