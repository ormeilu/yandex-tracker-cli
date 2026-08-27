//! Account and profile commands.
//!
//! `login` is the only command that touches a secret, and it reads it from a
//! prompt or stdin — never from an argument, because arguments are visible in
//! `ps` and in shell history. There is deliberately no command that prints a
//! stored token.

use std::io::Write;

use clap::Subcommand;

use crate::cli::{Session, not_implemented};
use crate::exit::ExitCode;
use crate::secrets;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Store a token for an account.
    Login {
        /// Account name to store the token under.
        #[arg(long, short = 'a')]
        account: String,
    },
    /// Remove a stored token.
    Logout {
        #[arg(long, short = 'a')]
        account: String,
    },
    /// List configured accounts and profiles.
    List,
    /// Show the active profile, where it came from, and who the token belongs to.
    Status,
}

/// Run an auth subcommand.
pub async fn run(command: &AuthCommand, session: &Session) -> ExitCode {
    match command {
        AuthCommand::Status => status(session).await,
        AuthCommand::Login { .. } => not_implemented("auth login"),
        AuthCommand::Logout { .. } => not_implemented("auth logout"),
        AuthCommand::List => not_implemented("auth list"),
    }
}

/// The vertical slice: resolve a profile, find its token, and prove the whole
/// chain works against the API.
async fn status(session: &Session) -> ExitCode {
    let mut out = anstream::stdout();
    let mut err = anstream::stderr();

    let resolved = match session.resolved() {
        Ok(resolved) => resolved,
        Err(error) => {
            let _ = writeln!(err, "error: {error}");
            return ExitCode::Auth;
        }
    };

    // The provenance line comes first and always: a change applied to the wrong
    // organisation is not something the user should have to reconstruct later.
    let _ = writeln!(out, "profile: {} (from {})", resolved.name, resolved.source);
    let _ = writeln!(
        out,
        "account: {}   org: {} ({:?})",
        resolved.profile.account, resolved.profile.org_id, resolved.profile.org_kind
    );
    let _ = writeln!(out, "queue: {}", resolved.queue.as_deref().unwrap_or("-"));

    let token = match secrets::token(&resolved.profile.account) {
        Ok(token) => token,
        Err(error) => {
            let _ = writeln!(out, "token: missing");
            let _ = writeln!(err, "error: {error}");
            return ExitCode::Auth;
        }
    };

    let config = crate::api::ClientConfig::new(
        token,
        resolved.profile.org_id.clone(),
        resolved.profile.org_kind,
    );
    let client = match crate::api::Client::new(&config) {
        Ok(client) => client,
        Err(error) => {
            let _ = writeln!(err, "error: {error}");
            return error.exit_code();
        }
    };

    match client.myself().await {
        Ok(user) => {
            let _ = writeln!(
                out,
                "token: ok   user: {}",
                user.login.or(user.display).unwrap_or(user.id)
            );
            ExitCode::Success
        }
        Err(error) => {
            let _ = writeln!(out, "token: rejected");
            let _ = writeln!(err, "error: {error}");
            error.exit_code()
        }
    }
}
