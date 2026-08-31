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
    /// Change an existing profile: its name, its note, the organisation it points at.
    #[command(long_about = crate::cli::help::md(crate::cli::help::AUTH_EDIT))]
    Edit(EditArgs),
    /// Delete a profile from the config file. The account and its token stay.
    #[command(long_about = crate::cli::help::md(crate::cli::help::AUTH_REMOVE))]
    Remove {
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

    /// Note saying which organisation this profile is; shown wherever the
    /// profile is named. Asked for in a terminal, and left as it was when a
    /// re-login omits it.
    #[arg(long)]
    pub description: Option<String>,

    /// Make this the default profile even if another one already is.
    #[arg(long)]
    pub default: bool,

    /// Skip the check that the token and organisation actually work.
    #[arg(long)]
    pub no_verify: bool,
}

/// Arguments for `auth edit`.
///
/// Everything is optional except the profile, and anything not passed is left
/// exactly as it was: this command exists to change one thing without having to
/// restate the rest of a profile that already works.
#[derive(Debug, Args)]
pub struct EditArgs {
    /// Profile to change, as `auth list` prints it.
    pub profile: String,

    /// Rename it. `default_profile` follows; a committed `.tracker.toml` does not.
    #[arg(long)]
    pub name: Option<String>,

    /// Note saying which organisation this is.
    #[arg(long)]
    pub description: Option<String>,

    /// Remove the note.
    #[arg(long, conflicts_with = "description")]
    pub clear_description: bool,

    /// Account whose credential this profile uses.
    #[arg(long, short = 'a')]
    pub account: Option<String>,

    /// Organisation id.
    #[arg(long)]
    pub org_id: Option<String>,

    /// Which header carries the organisation id.
    #[arg(long, value_enum)]
    pub org_kind: Option<OrgKind>,

    /// Queue assumed when a command needs one and none was given.
    #[arg(long, short = 'q')]
    pub queue: Option<String>,

    /// Stop assuming a queue.
    #[arg(long, conflicts_with = "queue")]
    pub clear_queue: bool,
}

/// Run an auth subcommand.
pub async fn run(command: &AuthCommand, session: &Session) -> ExitCode {
    match command {
        AuthCommand::Status { brief, active_only } => status(session, *brief, *active_only).await,
        AuthCommand::Login(args) => login(args, session).await,
        AuthCommand::Logout { account } => logout(account),
        AuthCommand::List => list(session),
        AuthCommand::Use { profile } => use_profile(session, profile),
        AuthCommand::Edit(args) => edit(args, session),
        AuthCommand::Remove { profile } => remove(session, profile),
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
        describe_profile(profile, paint, &mut out);

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

/// The two lines under a profile heading: its note, then what it points at.
fn describe_profile(
    profile: &crate::config::Profile,
    paint: Painter,
    out: &mut impl std::io::Write,
) {
    if let Some(description) = profile.description.as_deref() {
        let _ = writeln!(
            out,
            "  {} {description}",
            paint.paint("note:", Palette::label()),
        );
    }

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
            "dry run: would write profile `{profile_name}` (account={}, org={}, {:?}{}{}) to {}",
            profile.account,
            profile.org_id,
            profile.org_kind,
            profile
                .description
                .as_deref()
                .map_or_else(String::new, |note| format!(", {note}")),
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

    // Kept when a re-login does not mention it: the note is about the
    // organisation, which has not changed just because the token was renewed.
    let existing = session
        .config
        .profiles
        .get(&profile_name)
        .and_then(|profile| profile.description.clone());
    let description = match (args.description.clone(), shape.interactive) {
        (Some(text), _) => Some(text),
        (None, true) => wizard::description(existing.as_deref())
            .map_err(|error| report(&error, error.exit_code()))?
            .or(existing),
        (None, false) => existing,
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
            description,
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

/// "That name is not in the config, and here are the ones that are."
///
/// The list matters more than the refusal: the usual cause is a typo or a
/// profile from another machine, and both are answered by seeing the names.
fn unknown<'a>(what: &str, name: &str, configured: impl Iterator<Item = &'a String>) -> ExitCode {
    let known: Vec<&str> = configured.map(String::as_str).collect();
    report(
        &format!(
            "no {what} called `{name}`; configured: {}",
            if known.is_empty() {
                "none — run `ytcli auth login`".to_owned()
            } else {
                known.join(", ")
            }
        ),
        ExitCode::NotFound,
    )
}

/// Point `default_profile` at another profile.
///
/// A local edit and nothing else: no token is read, no request is made. The
/// profile has to exist, because a default naming a profile that does not is a
/// config every later command fails on with a worse message than this one.
fn use_profile(session: &Session, profile: &str) -> ExitCode {
    let mut err = anstream::stderr();

    if !session.config.profiles.contains_key(profile) {
        return unknown("profile", profile, session.config.profiles.keys());
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

/// Change an existing profile.
///
/// Like `auth use`, a local edit: no token is read and no request is made, so a
/// profile can be corrected whether or not its credentials currently work. What
/// is not passed is not touched — the point of the command is changing one
/// thing without restating a profile that already works.
fn edit(args: &EditArgs, session: &Session) -> ExitCode {
    let mut err = anstream::stderr();

    if !session.config.profiles.contains_key(&args.profile) {
        return unknown("profile", &args.profile, session.config.profiles.keys());
    }

    // An account nobody has logged into is a profile that fails on every later
    // command, with a message about the account rather than about this edit.
    if let Some(account) = args
        .account
        .as_deref()
        .filter(|account| !session.config.accounts.contains_key(*account))
    {
        return unknown("account", account, session.config.accounts.keys());
    }

    // An empty string is how a shell says "nothing", so it means the same as
    // --clear-description rather than writing a note nobody can read.
    let description = if args.clear_description {
        Some(None)
    } else {
        args.description
            .as_deref()
            .map(str::trim)
            .map(|text| (!text.is_empty()).then_some(text))
    };

    let edits = store::Edits {
        name: args.name.as_deref(),
        account: args.account.as_deref(),
        org_id: args.org_id.as_deref(),
        org_kind: args.org_kind,
        description,
        default_queue: if args.clear_queue {
            Some(None)
        } else {
            args.queue.as_deref().map(Some)
        },
    };

    if edits.is_empty() {
        return report(
            &format!(
                "nothing to change; pass --name, --description, --account, --org-id, --org-kind or --queue (see `ytcli auth edit --help`)\ncurrently: {}",
                describe_current(session, &args.profile)
            ),
            ExitCode::ConfirmationRequired,
        );
    }

    if session.global.dry_run {
        let _ = writeln!(
            err,
            "dry run: would change profile `{}` in {}",
            args.profile,
            session.config_file.display()
        );
        return ExitCode::Success;
    }

    match store::edit(&session.config_file, &args.profile, &edits) {
        Ok(_) => {
            let name = args.name.as_deref().unwrap_or(&args.profile);
            if let Some(new_name) = args.name.as_deref().filter(|name| *name != args.profile) {
                rename_side_effects(session, &args.profile, new_name, &mut err);
            }
            let _ = writeln!(
                err,
                "profile `{name}`: {}",
                describe_after(session, &args.profile, &edits)
            );
            if args.org_id.is_some() || args.org_kind.is_some() || args.account.is_some() {
                let _ = writeln!(
                    err,
                    "check it: ytcli auth status --profile {name} --active-only"
                );
            }
            emit(&format!("{name}\n"));
            ExitCode::Success
        }
        Err(error) => {
            let code = match error {
                store::EditError::Unknown(_) => ExitCode::NotFound,
                // Neither is ApiRejected: nothing was sent. A name already in
                // use, and a file that will not parse, are both plain failures
                // of this local edit.
                store::EditError::NameTaken(_) | store::EditError::Store(_) => ExitCode::Failure,
            };
            report(&error, code)
        }
    }
}

/// Delete a profile.
///
/// The counterpart to `auth login`, and deliberately not the counterpart to
/// `auth logout`: logout forgets a credential, this forgets an organisation
/// someone was reaching through one. The token stays in the keychain, because
/// one account usually backs several profiles.
///
/// `--yes` is required even for one profile. Nothing here is sent anywhere, but
/// the `[profiles.x]` table carries display settings and pinned custom fields
/// that only exist in this file, and re-logging in does not bring them back.
fn remove(session: &Session, profile: &str) -> ExitCode {
    let mut err = anstream::stderr();

    let Some(current) = session.config.profiles.get(profile) else {
        return unknown("profile", profile, session.config.profiles.keys());
    };

    // The same promise every write makes: say which organisation this is about
    // before touching it. Here it matters more than usual — profile names are
    // short and similar, and organisation ids are what actually differ.
    let about = format!(
        "account={} org={} ({:?})",
        current.account, current.org_id, current.org_kind
    );

    if session.global.dry_run {
        let _ = writeln!(
            err,
            "dry run: would remove profile `{profile}` ({about}) from {}",
            session.config_file.display()
        );
        return ExitCode::Success;
    }

    if !session.global.yes {
        let _ = writeln!(
            err,
            "refusing to remove profile `{profile}` ({about}) without --yes: \
             its display settings and pinned fields live only in {}",
            session.config_file.display()
        );
        return ExitCode::ConfirmationRequired;
    }

    let account = current.account.clone();

    match store::remove(&session.config_file, profile) {
        Ok(removed) => {
            let _ = writeln!(err, "removed profile `{profile}` ({about})");
            removal_side_effects(
                session,
                profile,
                &account,
                removed.cleared_default,
                &mut err,
            );
            emit(&format!("{profile}\n"));
            ExitCode::Success
        }
        Err(error) => {
            let code = match error {
                store::EditError::Unknown(_) => ExitCode::NotFound,
                store::EditError::NameTaken(_) | store::EditError::Store(_) => ExitCode::Failure,
            };
            report(&error, code)
        }
    }
}

/// Everything outside the profile table that a removal leaves dangling.
///
/// Each of these is something the user would otherwise meet later, as a failure
/// with a worse message than this one.
fn removal_side_effects(
    session: &Session,
    profile: &str,
    account: &str,
    cleared_default: bool,
    err: &mut impl std::io::Write,
) {
    let cache_path = crate::config::cache::path_for(&session.config_file);
    let mut cache = crate::config::cache::Cache::load(&cache_path);
    if cache.forget(profile) {
        cache.save(&cache_path);
    }

    if cleared_default {
        let remaining: Vec<&str> = session
            .config
            .profiles
            .keys()
            .map(String::as_str)
            .filter(|name| *name != profile)
            .collect();
        let _ = writeln!(err, "default profile: {profile} → none");
        match remaining.as_slice() {
            [] => {
                let _ = writeln!(err, "no profiles left; `ytcli auth login` makes another");
            }
            [only] => {
                let _ = writeln!(err, "pick the next one: ytcli auth use {only}");
            }
            names => {
                let _ = writeln!(
                    err,
                    "pick the next one: ytcli auth use <{}>",
                    names.join("|")
                );
            }
        }
    }

    // The credential outlives the profile on purpose; saying so is what keeps
    // "I deleted it" from meaning two different things.
    let still_used = session
        .config
        .profiles
        .iter()
        .any(|(name, other)| name != profile && other.account == account);
    if !still_used && secrets::is_stored(account) {
        let _ = writeln!(
            err,
            "note: account `{account}` still holds a token; ytcli auth logout --account {account} forgets it"
        );
    }

    // Committed and shared with other checkouts, so it is reported rather than
    // rewritten — the same rule a rename follows.
    if let Some((path, _)) =
        crate::config::paths::find_project_pin(&std::env::current_dir().unwrap_or_default())
            .filter(|(_, pin)| pin.profile.as_deref() == Some(profile))
    {
        let _ = writeln!(
            err,
            "note: {} still names `{profile}`; update it by hand",
            path.display()
        );
    }
}

/// Carry a rename through the things outside the profile table that name it,
/// and say what a local edit cannot reach.
fn rename_side_effects(session: &Session, from: &str, to: &str, err: &mut impl std::io::Write) {
    let cache_path = crate::config::cache::path_for(&session.config_file);
    let mut cache = crate::config::cache::Cache::load(&cache_path);
    if cache.rename(from, to) {
        cache.save(&cache_path);
    }

    let _ = writeln!(err, "renamed profile `{from}` → `{to}`");

    // A committed `.tracker.toml` is shared with other people and other
    // checkouts; rewriting it from here would change what a colleague's next
    // command does, so it is reported instead.
    if let Some((path, _)) =
        crate::config::paths::find_project_pin(&std::env::current_dir().unwrap_or_default())
            .filter(|(_, pin)| pin.profile.as_deref() == Some(from))
    {
        let _ = writeln!(
            err,
            "note: {} still names `{from}`; update it by hand",
            path.display()
        );
    }

    if session.config.default_profile.as_deref() == Some(from) {
        let _ = writeln!(err, "default profile: {from} → {to}");
    }
}

/// The profile as it stands, for the message that says nothing was asked for.
fn describe_current(session: &Session, profile: &str) -> String {
    session
        .config
        .profiles
        .get(profile)
        .map_or_else(String::new, |current| {
            format!(
                "account={} org={} ({:?}) queue={} description={}",
                current.account,
                current.org_id,
                current.org_kind,
                current.default_queue.as_deref().unwrap_or("-"),
                current.description.as_deref().unwrap_or("-"),
            )
        })
}

/// What this edit changed, named key by key so the line is about the change and
/// not about the profile.
fn describe_after(session: &Session, profile: &str, edits: &store::Edits<'_>) -> String {
    let current = session.config.profiles.get(profile);
    let mut parts: Vec<String> = Vec::new();

    if let Some(account) = edits.account {
        parts.push(format!("account={account}"));
    }
    if let Some(org_id) = edits.org_id {
        parts.push(format!("org={org_id}"));
    }
    if let Some(org_kind) = edits.org_kind {
        parts.push(format!("org_kind={org_kind:?}"));
    }
    match edits.default_queue {
        Some(Some(queue)) => parts.push(format!("queue={queue}")),
        Some(None) => parts.push("queue removed".to_owned()),
        None => {}
    }
    match edits.description {
        Some(Some(text)) => parts.push(format!("description=\"{text}\"")),
        Some(None) => parts.push("description removed".to_owned()),
        None => {}
    }

    if parts.is_empty() {
        // A rename on its own: say what the profile is now, since its identity
        // is exactly what just changed.
        return current.map_or_else(String::new, |current| {
            format!("account={} org={}", current.account, current.org_id)
        });
    }

    parts.join(" ")
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

        let note = profile
            .description
            .as_deref()
            .map_or_else(String::new, |description| format!("  {description}"));

        let _ = writeln!(
            out,
            "profile {name}  account: {}  org: {} ({:?}){suffix}{note}",
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
