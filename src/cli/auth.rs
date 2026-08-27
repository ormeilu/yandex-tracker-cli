//! Account and profile commands.
//!
//! `login` is the only command that touches a secret, and it reads it from a
//! prompt or stdin — never from an argument, because arguments are visible in
//! `ps` and in shell history. There is deliberately no command that prints a
//! stored token.

use std::fmt::Write as _;
use std::io::Write as _;

use clap::{Args, Subcommand};

use crate::api::{Client, ClientConfig};
use crate::cli::{Session, emit, report};
use crate::config::{OrgKind, Profile, store};
use crate::exit::ExitCode;
use crate::secrets;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Store a token for an account, and set up a profile to use it with.
    Login(LoginArgs),
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

/// Arguments for `auth login`.
///
/// The token is deliberately absent: it is read from a prompt or from stdin,
/// never from an argument, because arguments are visible in `ps` and land in
/// shell history.
#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Account name to store the token under.
    #[arg(long, short = 'a')]
    pub account: String,

    /// Organisation id. Given this, login also writes a profile.
    #[arg(long)]
    pub org_id: Option<String>,

    /// Which header carries the organisation id. Detected when omitted.
    #[arg(long, value_enum)]
    pub org_kind: Option<OrgKind>,

    /// Profile name to create; defaults to the account name.
    #[arg(long, short = 'p')]
    pub profile: Option<String>,

    /// Queue this profile assumes when a command needs one.
    #[arg(long, short = 'q')]
    pub queue: Option<String>,

    /// Make this the default profile even if another one already is.
    #[arg(long)]
    pub default: bool,

    /// Skip the check that the token and organisation actually work.
    #[arg(long)]
    pub no_verify: bool,
}

/// Run an auth subcommand.
pub async fn run(command: &AuthCommand, session: &Session) -> ExitCode {
    match command {
        AuthCommand::Status => status(session).await,
        AuthCommand::Login(args) => login(args, session).await,
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

/// Read the token, check it, store it, and write the config to use it.
///
/// Before this, logging in left someone with a token in the keychain and no
/// configuration to use it with — the account and profile still had to be
/// hand-written. One command now covers the whole path.
async fn login(args: &LoginArgs, session: &Session) -> ExitCode {
    let mut err = anstream::stderr();

    let token = match read_token(&args.account) {
        Ok(token) => token,
        Err(code) => return code,
    };

    // Verify before storing. A mistyped token that lands in the keychain fails
    // later, somewhere else, looking like a permissions problem.
    let verified = match (&args.org_id, args.no_verify) {
        (Some(org_id), false) => match verify(&token, org_id, args.org_kind).await {
            Ok((kind, who)) => {
                let _ = writeln!(err, "verified as {who} in org {org_id} ({kind:?})");
                Some(kind)
            }
            Err(code) => return code,
        },
        (Some(_), true) => Some(args.org_kind.unwrap_or(OrgKind::Cloud)),
        (None, _) => None,
    };

    // Login writes in two places — the keychain and the config file — so it
    // honours --dry-run like every other write: verify, say what would happen,
    // touch nothing.
    if session.global.dry_run {
        let _ = writeln!(
            err,
            "dry run: would store a token for `{}` in the OS keychain",
            args.account
        );
    } else {
        if let Err(error) = secrets::store(&args.account, &token) {
            return report(&error, ExitCode::Auth);
        }
        let _ = writeln!(
            err,
            "stored a token for `{}` in the OS keychain",
            args.account
        );
    }

    let Some(org_id) = args.org_id.clone() else {
        let _ = writeln!(
            err,
            "no --org-id given, so no profile was written; rerun with --org-id to finish setting up"
        );
        return ExitCode::Success;
    };

    let profile_name = args.profile.clone().unwrap_or_else(|| args.account.clone());
    let profile = Profile {
        account: args.account.clone(),
        org_id,
        org_kind: verified.unwrap_or(OrgKind::Cloud),
        default_queue: args.queue.clone(),
        display: crate::config::Display::default(),
    };

    let make_default = args.default || session.config.default_profile.is_none();

    if session.global.dry_run {
        let _ = writeln!(
            err,
            "dry run: would write profile `{profile_name}` (account={}, org={}, {:?}{}) to {}",
            profile.account,
            profile.org_id,
            profile.org_kind,
            if make_default { ", default" } else { "" },
            session.config_file.display(),
        );
        return ExitCode::Success;
    }

    match store::upsert(
        &session.config_file,
        &args.account,
        None,
        Some((&profile_name, &profile)),
        make_default,
    ) {
        Ok(_) => {
            let _ = writeln!(
                err,
                "wrote profile `{profile_name}` to {}{}",
                session.config_file.display(),
                if make_default { " (default)" } else { "" },
            );
            emit(&format!("{profile_name}\n"));
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}

/// Read the token from a hidden prompt when someone is typing, from stdin when
/// it is piped.
fn read_token(account: &str) -> Result<String, ExitCode> {
    use std::io::IsTerminal;

    let raw = if std::io::stdin().is_terminal() {
        rpassword::prompt_password(format!("OAuth token for `{account}`: "))
            .map_err(|error| report(&error, ExitCode::Failure))?
    } else {
        let mut piped = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut piped)
            .map_err(|error| report(&error, ExitCode::Failure))?;
        piped
    };

    let token = raw.trim().to_owned();
    if token.is_empty() {
        return Err(report(&"no token given", ExitCode::Auth));
    }
    Ok(token)
}

/// Check the token against the API, working out which organisation header it
/// needs if that was not said.
///
/// The two header forms are not interchangeable and the wrong one answers 403,
/// which reads like a permissions problem rather than a configuration mistake.
/// Trying both here is one extra request, once, against an afternoon of
/// confusion later.
async fn verify(
    token: &str,
    org_id: &str,
    kind: Option<OrgKind>,
) -> Result<(OrgKind, String), ExitCode> {
    let candidates: Vec<OrgKind> = match kind {
        Some(kind) => vec![kind],
        None => vec![OrgKind::Cloud, OrgKind::Yandex360],
    };

    let mut last: Option<crate::api::error::ApiError> = None;

    for candidate in candidates {
        let mut config = ClientConfig::new(token.to_owned(), org_id.to_owned(), candidate);
        if let Ok(base) = std::env::var("YTCLI_BASE_URL") {
            config.base_url = base;
        }

        let client = match Client::new(&config) {
            Ok(client) => client,
            Err(error) => {
                let code = error.exit_code();
                return Err(report(&error, code));
            }
        };

        match client.myself().await {
            Ok(user) => {
                let who = user.login.or(user.display).unwrap_or(user.id);
                return Ok((candidate, who));
            }
            // A rejected token is rejected under either header; only an
            // organisation mismatch is worth retrying the other way.
            Err(error @ crate::api::error::ApiError::Unauthorized) => {
                let code = error.exit_code();
                return Err(report(&error, code));
            }
            Err(error) => last = Some(error),
        }
    }

    let error = last.unwrap_or(crate::api::error::ApiError::Forbidden);
    let code = error.exit_code();
    Err(report(
        &format!("{error} — checked both organisation header forms"),
        code,
    ))
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
