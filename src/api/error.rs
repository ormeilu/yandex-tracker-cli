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
            Self::Unauthorized => ExitCode::Auth,
            Self::NotFound(_) => ExitCode::NotFound,
            Self::Forbidden | Self::RateLimited | Self::Rejected { .. } => ExitCode::ApiRejected,
            Self::Transport(_) | Self::Decode(_) => ExitCode::Failure,
        }
    }
}
