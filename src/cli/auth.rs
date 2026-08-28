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
use crate::render::style::{Painter, Palette};
use crate::secrets;

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Store a token for an account, and set up a profile to use it with.
    #[command(long_about = crate::cli::guidance::login_help())]
    Login(LoginArgs),
    /// Remove a stored token.
    #[command(long_about = crate::cli::help::md(crate::cli::help::AUTH_LOGOUT))]
    Logout {
        #[arg(long, short = 'a')]
        account: String,
    },
    /// List configured accounts and profiles.
    #[command(long_about = crate::cli::help::md(crate::cli::help::AUTH_LIST))]
    List,
    /// Make a profile the default one.
    #[command(long_about = crate::cli::help::md(crate::cli::help::AUTH_USE))]
    Use {
        /// Profile name, as `auth list` prints it.
        profile: String,
    },
    /// Check every profile: who the token belongs to, and what it can see.
    #[command(long_about = crate::cli::help::md(crate::cli::help::AUTH_STATUS))]
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
        AuthCommand::Use { profile } => use_profile(session, profile),
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
    let paint = session.render.painter();

    if session.config.profiles.is_empty() {
        let _ = writeln!(err, "no profiles configured yet.\n");
        let _ = writeln!(err, "{}", guidance::full());
        let _ = writeln!(
            err,
            "Then: ytcli auth login --account <name> --org-id <id> [--queue <QUEUE>]"
        );
        return ExitCode::Auth;
    }

    report_sources(session, paint, &mut out);

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

    remember_queues(session, brief, active_only, active.as_deref(), &queues_seen);
    warn_about_collisions(session, paint, &queues_seen);

    // A shell that exports YTCLI_TOKEN on entering a directory — the oh-my-zsh
    // `dotenv` plugin does exactly this — makes every profile authenticate as
    // one person, and the rows then agree with each other for a reason that has
    // nothing to do with the configuration being read.
    if secrets::overridden() && session.config.profiles.len() > 1 {
        let _ = writeln!(
            err,
            "{} YTCLI_TOKEN is set, so every profile above was read through that one token, whatever account it names",
            paint.paint("warning:", Palette::warn()),
        );
    }

    // The command someone runs to find out which profile is in play is the
    // command that should say how to change it.
    if session.config.profiles.len() > 1 {
        let _ = writeln!(
            err,
            "{}",
            paint.paint(
                "change the default with: ytcli auth use <profile>",
                Palette::label()
            )
        );
    }

    // The active profile decides the outcome — a broken profile nobody is using
    // should not make a script think the tool is unusable. But if *nothing*
    // worked, saying so beats reporting success for a run that found none.
    active_failure
        .or_else(|| (!any_success).then_some(last_failure).flatten())
        .unwrap_or(ExitCode::Success)
}

/// Persist the queue map, so a later bare key can be judged without a request.
fn remember_queues(
    session: &Session,
    brief: bool,
    active_only: bool,
    active: Option<&str>,
    queues_seen: &std::collections::BTreeMap<String, Vec<String>>,
) {
    if brief {
        return;
    }

    let cache_path = crate::config::cache::path_for(&session.config_file);
    let mut cache = crate::config::cache::Cache::load(&cache_path);

    for name in session
        .config
        .profiles
        .keys()
        .filter(|name| !active_only || active == Some(name.as_str()))
    {
        let keys: Vec<String> = queues_seen
            .iter()
            .filter(|(_, profiles)| profiles.iter().any(|profile| profile == name))
            .map(|(key, _)| key.clone())
            .collect();
        cache.record(name, &keys);
    }

    cache.save(&cache_path);
}

/// Where the configuration itself came from, before anything about profiles.
///
/// Two questions get asked whenever this command surprises somebody: which file
/// was read, and what in the environment is overriding it. Both are cheap to
/// answer and neither is guessable from the rows below — a token from the
/// environment and a token from the keychain produce identical-looking output
/// until one of them is named.
///
/// Variable **names** only. One of them holds a token, and a diagnostic that
/// prints credentials is a diagnostic nobody can paste into a bug report.
fn report_sources(session: &Session, paint: Painter, out: &mut impl std::io::Write) {
    let from = match std::env::var("YTCLI_CONFIG") {
        Ok(path) if session.config_file == std::path::Path::new(&path) => "from YTCLI_CONFIG",
        _ if session.global.config.is_some() => "from --config",
        _ => "default location",
    };

    let _ = writeln!(
        out,
        "{} {} ({})",
        paint.paint("config:", Palette::label()),
        session.config_file.display(),
        paint.paint(from, Palette::label()),
    );

    // Everything `YTCLI_`-prefixed: figment merges these over the file, so a
    // value in the config that does not match what the tool is doing is usually
    // one of these.
    let mut overriding: Vec<String> = std::env::vars()
        .map(|(name, _)| name)
        .filter(|name| name.starts_with("YTCLI_") && !name.is_empty())
        .collect();
    overriding.sort();

    if !overriding.is_empty() {
        let _ = writeln!(
            out,
            "{} {}",
            paint.paint("environment:", Palette::label()),
            overriding.join(", "),
        );
    }
}

/// Say which queue keys mean two different things.
///
/// Two profiles seeing one queue key is only a problem when they are looking at
/// two different organisations: then `FINANSY-1` names two issues and the tool
/// refuses to choose. Inside one organisation it names one issue seen through
/// two logins, either of which fetches it — warning about that would be telling
/// the reader their setup is broken when it is working as designed.
///
/// Better heard here than discovered by commenting on the wrong issue.
fn warn_about_collisions(
    session: &Session,
    paint: Painter,
    queues_seen: &std::collections::BTreeMap<String, Vec<String>>,
) {
    let mut err = anstream::stderr();

    let organisation = |name: &str| {
        session
            .config
            .profiles
            .get(name)
            .map(|profile| profile.org_id.clone())
    };

    let ambiguous: Vec<(&String, &Vec<String>)> = queues_seen
        .iter()
        .filter(|(_, profiles)| {
            profiles.len() > 1
                && profiles
                    .iter()
                    .filter_map(|name| organisation(name))
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    > 1
        })
        .collect();
    if ambiguous.is_empty() {
        return;
    }

    let _ = writeln!(err);
    for (key, profiles) in ambiguous {
        let _ = writeln!(
            err,
            "{} queue {key} is visible in {} — in different organisations, so a bare {key}-1 will be refused; write {}/{key}-1",
            paint.paint("warning:", Palette::warn()),
            profiles.join(" and "),
            profiles.first().map_or("profile", String::as_str),
        );
    }
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
    let (token, origin) = match secrets::token_from(&profile.account) {
        Ok(pair) => pair,
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

    // Which token answered, when it is not the one this profile's account
    // holds. Without this the reader has no way to tell that every profile is
    // being read through one identity.
    let via = match origin {
        secrets::Origin::Environment => " (from YTCLI_TOKEN)",
        // Named rather than left blank: "where did this credential come from"
        // is the question, and an unlabelled answer is only obvious to whoever
        // wrote the tool.
        secrets::Origin::Keychain => " (from keychain)",
    };

    match client.myself().await {
        Ok(user) => {
            let _ = writeln!(
                out,
                "  {} {}{via}   {} {}{}",
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
                let _ = writeln!(err, "\n{}", guidance::block(guidance::TOKEN));
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

    // Interactive login always asks for the token — there is no flag to pass one
    // in, on purpose — so there is always something the procedure is needed for.
    if interactive {
        wizard::introduce();
    }

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
        let _ = writeln!(err, "{}", guidance::block(guidance::ORG));
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

            // Listing them anyway makes recording them free, and a collision
            // with an existing profile can then be caught on the next command
            // rather than after acting on the wrong issue.
            if !session.global.dry_run {
                let cache_path = crate::config::cache::path_for(&session.config_file);
                let mut cache = crate::config::cache::Cache::load(&cache_path);
                cache.record(&profile_name, &available);
                cache.save(&cache_path);
            }

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
                let _ = writeln!(err, "\n{}", guidance::block(guidance::TOKEN));
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
    let _ = writeln!(err, "\n{}", guidance::block(guidance::ORG));
    Err(reported)
}

/// Point `default_profile` at another profile.
///
/// A local edit and nothing else: no token is read, no request is made. The
/// profile has to exist, because a default naming a profile that does not is a
/// config every later command fails on with a worse message than this one.
fn use_profile(session: &Session, profile: &str) -> ExitCode {
    let mut err = anstream::stderr();

    if !session.config.profiles.contains_key(profile) {
        let known: Vec<&str> = session.config.profiles.keys().map(String::as_str).collect();
        return report(
            &format!(
                "no profile called `{profile}`; configured: {}",
                if known.is_empty() {
                    "none — run `ytcli auth login`".to_owned()
                } else {
                    known.join(", ")
                }
            ),
            ExitCode::NotFound,
        );
    }

    let previous = session.config.default_profile.clone();
    if previous.as_deref() == Some(profile) {
        let _ = writeln!(err, "`{profile}` is already the default profile");
        return ExitCode::Success;
    }

    if session.global.dry_run {
        let _ = writeln!(
            err,
            "dry run: would make `{profile}` the default profile in {}",
            session.config_file.display()
        );
        return ExitCode::Success;
    }

    match store::set_default(&session.config_file, profile) {
        Ok(_) => {
            let _ = writeln!(
                err,
                "default profile: {} → {profile}",
                previous.as_deref().unwrap_or("none"),
            );
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
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
