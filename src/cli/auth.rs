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
use crate::cli::{Session, emit, guidance, report};
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
    /// Check every profile: who the token belongs to, and what it can see.
    Status {
        /// Identity only — skip the counts, and the requests behind them.
        #[arg(long)]
        brief: bool,
        /// Check only the active profile instead of all of them.
        #[arg(long)]
        active_only: bool,
    },
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
        AuthCommand::Status { brief, active_only } => status(session, *brief, *active_only).await,
        AuthCommand::Login(args) => login(args, session).await,
        AuthCommand::Logout { account } => logout(account),
        AuthCommand::List => list(session),
    }
}

/// Report on the configured profiles.
///
/// This is the command someone runs when something is wrong, so it answers the
/// questions that actually get asked: which profile is in play and where that
/// choice came from, whether the token works, who it belongs to, and what it can
/// reach. Checking every profile rather than only the active one is deliberate —
/// "it works with my other login" is the usual next question.
///
/// The counts cost a handful of requests per profile. That is fine for a
/// diagnostic and wrong for a hot path, which is what `--brief` is for.
async fn status(session: &Session, brief: bool, active_only: bool) -> ExitCode {
    let mut out = anstream::stdout();
    let mut err = anstream::stderr();

    if session.config.profiles.is_empty() {
        let _ = writeln!(err, "no profiles configured yet.\n");
        let _ = writeln!(err, "{}", guidance::full());
        let _ = writeln!(
            err,
            "Then: ytcli auth login --account <name> --org-id <id> [--queue <QUEUE>]"
        );
        return ExitCode::Auth;
    }

    let active = session
        .resolved
        .as_ref()
        .map(|resolved| resolved.name.clone());
    let mut active_failure = None;
    let mut any_success = false;
    let mut last_failure = None;

    for (name, profile) in &session.config.profiles {
        let is_active = active.as_deref() == Some(name.as_str());
        if active_only && !is_active {
            continue;
        }

        let source = if is_active {
            session
                .resolved
                .as_ref()
                .map_or_else(String::new, |resolved| {
                    format!(" (from {})", resolved.source)
                })
        } else {
            String::new()
        };
        let marks = if is_active { "  [active]" } else { "" };

        let _ = writeln!(out, "profile {name}{source}{marks}");
        let _ = writeln!(
            out,
            "  account: {}   org: {} ({:?})   queue: {}",
            profile.account,
            profile.org_id,
            profile.org_kind,
            profile.default_queue.as_deref().unwrap_or("-"),
        );

        let code = report_profile(profile, brief, &mut out, &mut err).await;
        if code == ExitCode::Success {
            any_success = true;
        } else {
            last_failure = Some(code);
            if is_active {
                active_failure = Some(code);
            }
        }
    }

    // The active profile decides the outcome — a broken profile nobody is using
    // should not make a script think the tool is unusable. But if *nothing*
    // worked, saying so beats reporting success for a run that found none.
    active_failure
        .or_else(|| (!any_success).then_some(last_failure).flatten())
        .unwrap_or(ExitCode::Success)
}

/// Everything that needs the network, for one profile.
async fn report_profile(
    profile: &crate::config::Profile,
    brief: bool,
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> ExitCode {
    let token = match secrets::token(&profile.account) {
        Ok(token) => token,
        Err(error) => {
            let _ = writeln!(out, "  token: missing");
            let _ = writeln!(err, "  {error}");
            return ExitCode::Auth;
        }
    };

    let mut config = ClientConfig::new(token, profile.org_id.clone(), profile.org_kind);
    if let Ok(base) = std::env::var("YTCLI_BASE_URL") {
        config.base_url = base;
    }
    let client = match Client::new(&config) {
        Ok(client) => client,
        Err(error) => {
            let _ = writeln!(err, "  {error}");
            return error.exit_code();
        }
    };

    match client.myself().await {
        Ok(user) => {
            let _ = writeln!(
                out,
                "  token: ok   user: {}{}",
                user.login.as_deref().unwrap_or(&user.id),
                user.display
                    .as_deref()
                    .map_or_else(String::new, |display| format!(" ({display})")),
            );
        }
        Err(error) => {
            let _ = writeln!(out, "  token: rejected");
            let _ = writeln!(err, "  {error}");
            if matches!(error, crate::api::error::ApiError::Unauthorized) {
                let _ = writeln!(err, "\n{}", guidance::TOKEN);
            }
            return error.exit_code();
        }
    }

    if brief {
        return ExitCode::Success;
    }

    // Each of these is best-effort: a profile without access to projects should
    // still report its queues rather than failing the whole line.
    let queues = client.queues().await.ok();
    let projects = client.entities("project", None, 1, 5).await.ok();
    let goals = client.entities("goal", None, 1, 1).await.ok();
    let mine = client
        .count("Assignee: me() AND Resolution: empty()")
        .await
        .ok();

    let _ = writeln!(
        out,
        "  queues: {}   projects: {}   goals: {}   my open issues: {}",
        queues
            .as_ref()
            .map_or_else(|| "-".to_owned(), |queues| queues.len().to_string()),
        projects.as_ref().map_or_else(|| "-".to_owned(), count_of),
        goals.as_ref().map_or_else(|| "-".to_owned(), count_of),
        mine.map_or_else(|| "-".to_owned(), |count| count.to_string()),
    );

    if let Some(projects) = projects.filter(|page| !page.items.is_empty()) {
        let names: Vec<String> = projects
            .items
            .iter()
            .map(|project| {
                project.short_id.map_or_else(
                    || project.summary.clone(),
                    |id| format!("{} ({id})", project.summary),
                )
            })
            .collect();
        let more = projects
            .total
            .unwrap_or(names.len() as u64)
            .saturating_sub(names.len() as u64);
        let suffix = if more > 0 {
            format!(", +{more} more")
        } else {
            String::new()
        };
        let _ = writeln!(out, "  projects: {}{suffix}", names.join(", "));
    }

    if let Some(queues) = queues.filter(|queues| !queues.is_empty()) {
        let keys: Vec<&str> = queues
            .iter()
            .take(8)
            .map(|queue| queue.key.as_str())
            .collect();
        let more = queues.len().saturating_sub(keys.len());
        let suffix = if more > 0 {
            format!(", +{more} more")
        } else {
            String::new()
        };
        let _ = writeln!(out, "  queues: {}{suffix}", keys.join(", "));
    }

    ExitCode::Success
}

fn count_of<T>(page: &crate::api::models::Page<T>) -> String {
    page.total
        .map_or_else(|| page.items.len().to_string(), |total| total.to_string())
}

/// Read the token, check it, store it, and write the config to use it.
///
/// Before this, logging in left someone with a token in the keychain and no
/// configuration to use it with — the account and profile still had to be
/// hand-written. One command now covers the whole path.
async fn login(args: &LoginArgs, session: &Session) -> ExitCode {
    let mut err = anstream::stderr();

    // Guidance goes to stderr before the prompt, and only when it is likely to
    // be needed: a first login for this account. Printing it every time trains
    // people to scroll past it.
    if !secrets::is_stored(&args.account) {
        let _ = writeln!(err, "{}\n", guidance::TOKEN);
    }

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
            "no --org-id given, so no profile was written and nothing can be queried yet.\n"
        );
        let _ = writeln!(err, "{}", guidance::ORG);
        let _ = writeln!(
            err,
            "\nThen: ytcli auth login --account {} --org-id <id> [--queue <QUEUE>]",
            args.account
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
                let reported = report(&error, code);
                let mut err = anstream::stderr();
                let _ = writeln!(err, "\n{}", guidance::TOKEN);
                return Err(reported);
            }
            Err(error) => last = Some(error),
        }
    }

    let error = last.unwrap_or(crate::api::error::ApiError::Forbidden);
    let code = error.exit_code();
    let reported = report(
        &format!("{error} — checked both organisation header forms"),
        code,
    );
    let mut err = anstream::stderr();
    let _ = writeln!(err, "\n{}", guidance::ORG);
    Err(reported)
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
