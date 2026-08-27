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
use crate::cli::{Session, emit, guidance, report, wizard};
use crate::config::{OrgKind, Profile, store};
use crate::exit::ExitCode;
use crate::render::style::{Painter, Palette, prose};
use crate::secrets;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Store a token for an account, and set up a profile to use it with.
    #[command(long_about = crate::cli::guidance::LOGIN_HELP)]
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
    /// Account name to store the token under. Asked for when omitted.
    #[arg(long, short = 'a')]
    pub account: Option<String>,

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

/// Colour for stderr, which is where guidance and progress go.
///
/// Judged separately from stdout: one of the two is often a pipe while the other
/// is still a terminal.
fn stderr_painter() -> Painter {
    use std::io::IsTerminal;
    Painter::for_stream(std::io::stderr().is_terminal())
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
    let paint = session.render.painter();

    if session.config.profiles.is_empty() {
        let _ = writeln!(err, "no profiles configured yet.\n");
        let _ = writeln!(err, "{}", prose(&guidance::full(), stderr_painter()));
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
    // Which profiles can see each queue key, so the ambiguity can be reported.
    let mut queues_seen: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

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

        let _ = writeln!(
            out,
            "{} {}{}{}",
            paint.paint("profile", Palette::label()),
            paint.paint(name, Palette::key()),
            paint.paint(&source, Palette::label()),
            paint.paint(marks, Palette::ok()),
        );
        let _ = writeln!(
            out,
            "  {} {}   {} {} ({:?})   {} {}",
            paint.paint("account:", Palette::label()),
            profile.account,
            paint.paint("org:", Palette::label()),
            profile.org_id,
            profile.org_kind,
            paint.paint("queue:", Palette::label()),
            profile.default_queue.as_deref().unwrap_or("-"),
        );

        let code = report_profile(
            profile,
            brief,
            paint,
            name,
            &mut queues_seen,
            &mut out,
            &mut err,
        )
        .await;
        if code == ExitCode::Success {
            any_success = true;
        } else {
            last_failure = Some(code);
            if is_active {
                active_failure = Some(code);
            }
        }
    }

    // A queue key is unique inside an organisation, not across them, so the same
    // key in two profiles means a bare `LMS-12` is ambiguous. Better to hear that
    // here than to discover it by commenting on the wrong issue.
    let shared: Vec<(&String, &Vec<String>)> = queues_seen
        .iter()
        .filter(|(_, profiles)| profiles.len() > 1)
        .collect();
    if !shared.is_empty() {
        let _ = writeln!(err);
        for (key, profiles) in shared {
            let _ = writeln!(
                err,
                "{} queue {key} is visible in {} — write {}/{key}-1 to say which",
                paint.paint("warning:", Palette::warn()),
                profiles.join(" and "),
                profiles.first().map_or("profile", String::as_str),
            );
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
    paint: Painter,
    profile_name: &str,
    queues_seen: &mut std::collections::BTreeMap<String, Vec<String>>,
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> ExitCode {
    let token = match secrets::token(&profile.account) {
        Ok(token) => token,
        Err(error) => {
            let _ = writeln!(
                out,
                "  {} {}",
                paint.paint("token:", Palette::label()),
                paint.paint("missing", Palette::bad())
            );
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
                "  {} {}   {} {}{}",
                paint.paint("token:", Palette::label()),
                paint.paint("ok", Palette::ok()),
                paint.paint("user:", Palette::label()),
                user.login.as_deref().unwrap_or(&user.id),
                user.display
                    .as_deref()
                    .map_or_else(String::new, |display| format!(" ({display})")),
            );
        }
        Err(error) => {
            let _ = writeln!(
                out,
                "  {} {}",
                paint.paint("token:", Palette::label()),
                paint.paint("rejected", Palette::bad())
            );
            let _ = writeln!(err, "  {error}");
            if matches!(error, crate::api::error::ApiError::Unauthorized) {
                let _ = writeln!(err, "\n{}", prose(guidance::TOKEN, stderr_painter()));
            }
            return error.exit_code();
        }
    }

    if brief {
        return ExitCode::Success;
    }

    reach(&client, paint, profile_name, queues_seen, out).await;
    ExitCode::Success
}

/// What this profile can actually see.
///
/// Every lookup is best-effort: a profile without access to projects should
/// still report its queues rather than losing the whole line.
async fn reach(
    client: &Client,
    paint: Painter,
    profile_name: &str,
    queues_seen: &mut std::collections::BTreeMap<String, Vec<String>>,
    out: &mut impl std::io::Write,
) {
    let queues = client.queues().await.ok();
    let projects = client.entities("project", None, 1, 5).await.ok();
    let goals = client.entities("goal", None, 1, 1).await.ok();
    let mine = client
        .count("Assignee: me() AND Resolution: empty()")
        .await
        .ok();

    let _ = writeln!(
        out,
        "  {} {}   {} {}   {} {}   {} {}",
        paint.paint("queues:", Palette::label()),
        queues
            .as_ref()
            .map_or_else(|| "-".to_owned(), |queues| queues.len().to_string()),
        paint.paint("projects:", Palette::label()),
        projects.as_ref().map_or_else(|| "-".to_owned(), count_of),
        paint.paint("goals:", Palette::label()),
        goals.as_ref().map_or_else(|| "-".to_owned(), count_of),
        paint.paint("my open issues:", Palette::label()),
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
        let _ = writeln!(
            out,
            "  {} {}{suffix}",
            paint.paint("projects:", Palette::label()),
            names.join(", ")
        );
    }

    if let Some(queues) = queues.filter(|queues| !queues.is_empty()) {
        for queue in &queues {
            queues_seen
                .entry(queue.key.clone())
                .or_default()
                .push(profile_name.to_owned());
        }

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
        let _ = writeln!(
            out,
            "  {} {}{suffix}",
            paint.paint("queues:", Palette::label()),
            keys.join(", ")
        );
    }
}

fn count_of<T>(page: &crate::api::models::Page<T>) -> String {
    page.total
        .map_or_else(|| page.items.len().to_string(), |total| total.to_string())
}

/// Read the token, check it, store it, and write the config to use it.
///
/// Flags and prompts are the same path: whatever was passed is taken as given,
/// and anything missing is asked for — but only when someone is there to answer.
/// Outside a terminal the flags are all there is, and a gap is an error rather
/// than a prompt nobody will ever see.
async fn login(args: &LoginArgs, session: &Session) -> ExitCode {
    let interactive = wizard::is_interactive();
    let mut err = anstream::stderr();

    let Identity {
        account,
        token,
        org_id,
        org_kind: verified,
    } = match identity(args, session, interactive).await {
        Ok(identity) => identity,
        Err(code) => return code,
    };

    if session.global.dry_run {
        let _ = writeln!(
            err,
            "dry run: would store a token for `{account}` in the OS keychain"
        );
    } else {
        if let Err(error) = secrets::store(&account, &token) {
            return report(&error, ExitCode::Auth);
        }
        let _ = writeln!(err, "stored a token for `{account}` in the OS keychain");
    }

    let Some(org_id) = org_id else {
        let _ = writeln!(
            err,
            "no --org-id given, so no profile was written and nothing can be queried yet.\n"
        );
        let _ = writeln!(err, "{}", prose(guidance::ORG, stderr_painter()));
        let _ = writeln!(
            err,
            "\nThen: ytcli auth login --account {account} --org-id <id> [--queue <QUEUE>]"
        );
        return ExitCode::Success;
    };

    let org_kind = verified.unwrap_or(OrgKind::Cloud);

    let shape = Shape {
        account: &account,
        token: &token,
        org_id: &org_id,
        org_kind,
        interactive,
    };
    let (profile_name, profile, make_default) = match shape_profile(args, session, &shape).await {
        Ok(shaped) => shaped,
        Err(code) => return code,
    };

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
        &account,
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
            let _ = writeln!(err, "try it: ytcli auth status --active-only");
            emit(&format!("{profile_name}\n"));
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}

/// Who is logging in, where, and with what — everything settled before anything
/// is written.
struct Identity {
    account: String,
    token: String,
    org_id: Option<String>,
    /// The organisation flavour that answered, once verified.
    org_kind: Option<OrgKind>,
}

/// Collect and check the credentials.
///
/// Flags win; a terminal fills the gaps; outside one, a gap is an error rather
/// than a prompt nobody will see.
async fn identity(
    args: &LoginArgs,
    session: &Session,
    interactive: bool,
) -> Result<Identity, ExitCode> {
    let mut err = anstream::stderr();

    let account = match args.account.clone() {
        Some(account) => account,
        None if interactive => {
            let existing: Vec<String> = session.config.accounts.keys().cloned().collect();
            wizard::account(&existing).map_err(|error| report(&error, error.exit_code()))?
        }
        None => {
            return Err(report(
                &"--account is required when not running in a terminal",
                ExitCode::ConfirmationRequired,
            ));
        }
    };

    let token = read_token(&account, interactive)?;

    // The organisation decides whether a profile can be written at all, so it is
    // asked for rather than skipped when someone is there to answer.
    let (org_id, org_kind) = match (&args.org_id, interactive) {
        (Some(org_id), _) => (Some(org_id.clone()), args.org_kind),
        (None, true) => wizard::organisation()
            .map(|(id, kind)| (Some(id), kind))
            .map_err(|error| report(&error, error.exit_code()))?,
        (None, false) => (None, None),
    };

    let verified = match (&org_id, args.no_verify) {
        (Some(org_id), false) => {
            let (kind, who) = verify(&token, org_id, org_kind).await?;
            let _ = writeln!(err, "verified as {who} in org {org_id} ({kind:?})");
            Some(kind)
        }
        (Some(_), true) => Some(org_kind.unwrap_or(OrgKind::Cloud)),
        (None, _) => None,
    };

    Ok(Identity {
        account,
        token,
        org_id,
        org_kind: verified,
    })
}

/// What the profile is being built from, once identity is settled.
struct Shape<'a> {
    account: &'a str,
    token: &'a str,
    org_id: &'a str,
    org_kind: OrgKind,
    interactive: bool,
}

/// Decide the profile's name, its queue and whether it becomes the default.
///
/// Split out so each half of login stays readable: this one asks questions and
/// touches nothing.
async fn shape_profile(
    args: &LoginArgs,
    session: &Session,
    shape: &Shape<'_>,
) -> Result<(String, Profile, bool), ExitCode> {
    let profile_name = match args.profile.clone() {
        Some(name) => name,
        None if shape.interactive => {
            wizard::profile(shape.account).map_err(|error| report(&error, error.exit_code()))?
        }
        None => shape.account.to_owned(),
    };

    // Offer the queues this token can actually see. Verifying first is what makes
    // that possible, and turns a spelling test into a choice.
    let queue = match args.queue.clone() {
        Some(queue) => Some(queue),
        None if shape.interactive => {
            let available = queue_keys(shape.token, shape.org_id, shape.org_kind).await;
            wizard::queue(&available).map_err(|error| report(&error, error.exit_code()))?
        }
        None => None,
    };

    let current_default = session.config.default_profile.as_deref();
    let make_default = if args.default || current_default.is_none() {
        true
    } else if shape.interactive {
        wizard::make_default(&profile_name, current_default)
            .map_err(|error| report(&error, error.exit_code()))?
    } else {
        false
    };

    Ok((
        profile_name,
        Profile {
            account: shape.account.to_owned(),
            org_id: shape.org_id.to_owned(),
            org_kind: shape.org_kind,
            default_queue: queue,
            display: crate::config::Display::default(),
        },
        make_default,
    ))
}

/// Queue keys this token can see, for the picker. Best-effort: failing to list
/// them costs a dropdown, not the login.
async fn queue_keys(token: &str, org_id: &str, kind: OrgKind) -> Vec<String> {
    let mut config = ClientConfig::new(token.to_owned(), org_id.to_owned(), kind);
    if let Ok(base) = std::env::var("YTCLI_BASE_URL") {
        config.base_url = base;
    }

    let Ok(client) = Client::new(&config) else {
        return Vec::new();
    };

    client.queues().await.map_or_else(
        |_| Vec::new(),
        |queues| queues.into_iter().map(|queue| queue.key).collect(),
    )
}

/// Read the token: a hidden prompt when someone is typing, stdin when piped.
fn read_token(account: &str, interactive: bool) -> Result<String, ExitCode> {
    if interactive {
        return wizard::token(account).map_err(|error| report(&error, error.exit_code()));
    }

    let mut piped = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut piped)
        .map_err(|error| report(&error, ExitCode::Failure))?;

    let token = piped.trim().to_owned();
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
                let _ = writeln!(err, "\n{}", prose(guidance::TOKEN, stderr_painter()));
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
    let _ = writeln!(err, "\n{}", prose(guidance::ORG, stderr_painter()));
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
