//! Account and profile commands.
//!
//! `login` is the only command that touches a secret, and it reads it from a
//! prompt or stdin — never from an argument, because arguments are visible in
//! `ps` and in shell history. There is deliberately no command that prints a
//! stored token.

use std::fmt::Write as _;
use std::io::Write as _;

use clap::Subcommand;

use crate::cli::{Session, emit, report};
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
        AuthCommand::Login { account } => login(account),
        AuthCommand::Logout { account } => logout(account),
        AuthCommand::List => list(session),
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

/// Store a token for an account.
///
/// The token is read from a hidden prompt when someone is typing, and from
/// stdin when it is piped. It is never taken from an argument: arguments show up
/// in `ps` and in shell history, which is the same as writing it down.
fn login(account: &str) -> ExitCode {
    use std::io::IsTerminal;

    let token = if std::io::stdin().is_terminal() {
        match rpassword::prompt_password(format!("OAuth token for `{account}`: ")) {
            Ok(token) => token,
            Err(error) => return report(&error, ExitCode::Failure),
        }
    } else {
        let mut piped = String::new();
        if let Err(error) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut piped) {
            return report(&error, ExitCode::Failure);
        }
        piped
    };

    let token = token.trim();
    if token.is_empty() {
        return report(&"no token given", ExitCode::Auth);
    }

    match secrets::store(account, token) {
        Ok(()) => {
            let mut err = anstream::stderr();
            let _ = writeln!(
                err,
                "stored a token for `{account}` in the OS keychain; check it with `ytcli auth status`"
            );
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Auth),
    }
}

fn logout(account: &str) -> ExitCode {
    match secrets::forget(account) {
        Ok(()) => {
            let mut err = anstream::stderr();
            let _ = writeln!(err, "forgot the token for `{account}`");
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Auth),
    }
}

/// Accounts and the profiles pointing at them.
///
/// Whether a token exists is shown; the token never is.
fn list(session: &Session) -> ExitCode {
    let mut out = String::with_capacity(256);
    let active = session
        .resolved
        .as_ref()
        .map(|resolved| resolved.name.clone());

    for (name, account) in &session.config.accounts {
        let _ = writeln!(
            out,
            "account {name}  token: {}  {}",
            if secrets::is_stored(name) {
                "stored"
            } else {
                "missing"
            },
            account.description.as_deref().unwrap_or(""),
        );
    }

    for (name, profile) in &session.config.profiles {
        let marks = [
            (session.config.default_profile.as_deref() == Some(name.as_str())).then_some("default"),
            (active.as_deref() == Some(name.as_str())).then_some("active"),
        ];
        let marks: Vec<&str> = marks.into_iter().flatten().collect();
        let suffix = if marks.is_empty() {
            String::new()
        } else {
            format!("  [{}]", marks.join(", "))
        };

        let _ = writeln!(
            out,
            "profile {name}  account: {}  org: {} ({:?}){suffix}",
            profile.account, profile.org_id, profile.org_kind,
        );
    }

    if out.is_empty() {
        return report(
            &"no accounts or profiles configured yet; see `ytcli auth login --help`",
            ExitCode::Auth,
        );
    }

    emit(&out);
    ExitCode::Success
}
