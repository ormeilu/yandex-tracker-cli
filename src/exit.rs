//! Process exit codes.
//!
//! Codes are part of the public contract: scripts and agents branch on them.
//! Note what is deliberately *not* encoded here — "the result set was truncated"
//! and "the result set was empty" are both success. Pagination state travels in
//! the output text (see `docs/adr/0003-output-ladder.md`), never in the exit code.

/// Exit code returned to the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    /// Command completed.
    Success = 0,
    /// Anything unexpected: I/O, transport, unhandled API error.
    Failure = 1,
    /// The command needs confirmation that was not given (`--yes` missing).
    ConfirmationRequired = 2,
    /// No credentials, expired token, or the profile could not be resolved.
    Auth = 3,
    /// The addressed entity does not exist or is not visible to this profile.
    NotFound = 4,
    /// The API refused the request (permissions, validation, rate limit).
    ApiRejected = 5,
    /// Recognised command that this build does not implement yet.
    NotImplemented = 64,
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        Self::from(code as u8)
    }
}
