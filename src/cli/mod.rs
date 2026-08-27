//! The command shell.
//!
//! The verb is the risk class. Read verbs (`get`, `find`, `list`, `status`) can
//! never write, and there is no generic pass-through that would let one smuggle a
//! change past that rule. That is what makes a permission allowlist like
//! `ytcli issue get:*` meaningful for an agent host (`docs/adr/0001-security-model.md`).

pub mod attachment;
pub mod auth;
pub mod cheatsheet;
pub mod entity;
pub mod goal;
pub mod guidance;
pub mod help;
pub mod issue;
pub mod project;
pub mod queue;
pub mod wizard;
pub mod write;

use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::{Config, Resolved};
use crate::exit::ExitCode;
use crate::render::{Audience, Context, Format};

/// Token-efficient Yandex Tracker CLI for humans and AI agents.
#[derive(Debug, Parser)]
#[command(name = "ytcli", version, about, long_about = help::ROOT)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub global: GlobalArgs,
}

/// Flags every command accepts.
#[derive(Debug, Args, Clone)]
pub struct GlobalArgs {
    // `YTCLI_PROFILE` is read separately rather than through clap's `env`, so
    // that `auth status` can report which of the two the value came from. That
    // is a fact about the implementation, so it stays out of the help text,
    // which is reprinted under every command.
    /// Act as this profile; overrides `YTCLI_PROFILE` and `.tracker.toml`.
    #[arg(long, short = 'p', global = true)]
    pub profile: Option<String>,

    /// text (compact, default), json (our schema), json-raw, toon.
    #[arg(long, short = 'f', global = true, value_name = "FORMAT")]
    pub format: Option<Format>,

    /// Print the whole description, however long.
    #[arg(long, global = true)]
    pub full: bool,

    /// Confirm a change that touches more than one issue.
    #[arg(long, global = true)]
    pub yes: bool,

    /// Print the request that would be sent, and send nothing.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Log to stderr; repeat for more. stdout stays pipeable.
    #[arg(long, short = 'v', global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Config file to use instead of the per-user one.
    ///
    /// Also read from `YTCLI_CONFIG`, which is how a test or a container points
    /// the tool at a config without rewriting every documented command line.
    #[arg(long, global = true, env = "YTCLI_CONFIG", value_name = "PATH")]
    pub config: Option<PathBuf>,
}

/// Top-level command groups, one per entity.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Accounts, organisations and who you currently are.
    #[command(subcommand)]
    Auth(auth::AuthCommand),
    /// Issues: read, search and change.
    #[command(subcommand)]
    Issue(issue::IssueCommand),
    /// Queues and their fields.
    #[command(subcommand)]
    Queue(queue::QueueCommand),
    /// Projects.
    #[command(subcommand)]
    Project(project::ProjectCommand),
    /// Goals.
    #[command(subcommand)]
    Goal(goal::GoalCommand),
    /// Issue attachments.
    #[command(subcommand)]
    Attachment(attachment::AttachmentCommand),
    /// Print a compact reference of the whole CLI, for agents.
    #[command(long_about = help::CHEATSHEET)]
    Cheatsheet(cheatsheet::CheatsheetArgs),
    /// Generate a shell completion script.
    #[command(long_about = help::COMPLETIONS)]
    Completions {
        /// Shell to generate for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

/// Everything a command implementation needs, assembled once.
#[derive(Debug)]
pub struct Session {
    pub config: Config,
    /// Where `config` came from, so `auth login` can write back to it.
    pub config_file: PathBuf,
    pub resolved: Option<Resolved>,
    pub render: Context,
    pub global: GlobalArgs,
}

impl Session {
    /// The active profile, or an auth error explaining how to get one.
    pub fn resolved(&self) -> Result<&Resolved, crate::config::ConfigError> {
        self.resolved
            .as_ref()
            .ok_or(crate::config::ConfigError::NoProfile)
    }

    /// Split a possibly profile-qualified target and build the client for it.
    ///
    /// Queue keys are only unique **inside** an organisation: two profiles can
    /// both see a `LMS`, and `LMS-12` then names two different issues. Rather
    /// than guess, a caller can say which one — `work/LMS-12` — and the rest of
    /// the command runs against that profile.
    pub fn client_for(&self, target: &str) -> Result<(crate::api::Client, String), ExitCode> {
        let Some((profile, key)) = target.split_once('/') else {
            self.refuse_if_ambiguous(target)?;
            return Ok((self.client()?, target.to_owned()));
        };

        // A slash with nothing useful around it is a typo, not a qualifier.
        if profile.is_empty() || key.is_empty() {
            return Err(report(
                &format!("`{target}` is not a valid key; write it as PROJ-1 or profile/PROJ-1"),
                ExitCode::ConfirmationRequired,
            ));
        }

        let resolved = self
            .config
            .resolve(Some(profile), None, std::path::Path::new("."))
            .map_err(|error| report(&error, ExitCode::Auth))?;

        let mut err = anstream::stderr();
        let _ = writeln!(
            err,
            "→ profile={} org={} (from the key `{target}`)",
            resolved.name, resolved.profile.org_id
        );

        Ok((self.client_with(&resolved)?, key.to_owned()))
    }

    /// Stop a bare key that could mean two different issues.
    ///
    /// Only a *known* collision refuses — one this tool has actually seen, from
    /// a previous `auth status` or `auth login`. Anything unknown proceeds:
    /// blocking on a guess would make the common case worse to protect against
    /// a situation most people never have.
    fn refuse_if_ambiguous(&self, key: &str) -> Result<(), ExitCode> {
        let Some(queue) = crate::config::cache::queue_of(key) else {
            return Ok(());
        };

        let configured: Vec<String> = self.config.profiles.keys().cloned().collect();
        let cache =
            crate::config::cache::Cache::load(&crate::config::cache::path_for(&self.config_file));
        let owners = cache.profiles_for(queue, &configured);

        if owners.len() < 2 {
            return Ok(());
        }

        let qualified = owners
            .iter()
            .map(|profile| format!("{profile}/{key}"))
            .collect::<Vec<_>>()
            .join(" or ");

        Err(report(
            &format!(
                "`{key}` is ambiguous: queue {queue} is visible in {} — write {qualified}",
                owners.join(" and "),
            ),
            ExitCode::ConfirmationRequired,
        ))
    }

    /// Build an API client for the active profile.
    ///
    /// Every failure on the way here — no profile, no stored token, a token the
    /// keychain will not release — is an auth problem from the caller's point of
    /// view, and reports as one.
    pub fn client(&self) -> Result<crate::api::Client, ExitCode> {
        let resolved = self
            .resolved()
            .map_err(|error| report(&error, ExitCode::Auth))?;
        self.client_with(resolved)
    }

    /// A client for a specific profile.
    pub fn client_with(&self, resolved: &Resolved) -> Result<crate::api::Client, ExitCode> {
        let token = crate::secrets::token(&resolved.profile.account)
            .map_err(|error| report(&error, ExitCode::Auth))?;

        let mut config = crate::api::ClientConfig::new(
            token,
            resolved.profile.org_id.clone(),
            resolved.profile.org_kind,
        );
        // Pointing the client at a stub server is how the CLI is tested end to
        // end; nothing else should be setting this.
        if let Ok(base) = std::env::var("YTCLI_BASE_URL") {
            config.base_url = base;
        }

        crate::api::Client::new(&config).map_err(|error| {
            let code = error.exit_code();
            report(&error, code)
        })
    }

    /// Display defaults for the active profile, or the built-in ones.
    #[must_use]
    pub fn display(&self) -> crate::config::Display {
        self.resolved
            .as_ref()
            .map(|r| r.profile.display.clone())
            .unwrap_or_default()
    }

    /// The queue to act on when the command did not name one.
    #[must_use]
    pub fn default_queue(&self) -> Option<&str> {
        self.resolved.as_ref().and_then(|r| r.queue.as_deref())
    }
}

/// Print an error to stderr and hand back the exit code to return.
pub fn report(error: &dyn std::fmt::Display, code: ExitCode) -> ExitCode {
    let mut err = anstream::stderr();
    let _ = writeln!(err, "error: {error}");
    code
}

/// Write rendered output to stdout.
pub fn emit(text: &str) {
    let mut out = anstream::stdout();
    let _ = write!(out, "{text}");
}

/// Build the rendering context from flags, profile defaults and the terminal.
#[must_use]
pub fn render_context(global: &GlobalArgs, resolved: Option<&Resolved>) -> Context {
    let display = resolved.map(|r| &r.profile.display);
    let audience = Audience::detect();

    // Truncation exists to save an agent's context, and a person reading their
    // own terminal has none of that problem — being handed two thirds of a
    // description and a note about the rest is just an extra command to type.
    // A terminal therefore gets everything unless the profile says otherwise.
    let description_lines = if global.full {
        None
    } else {
        match (audience, display) {
            (Audience::Human, None) => None,
            (Audience::Human, Some(display)) => display.description_lines_human,
            (Audience::Machine, display) => Some(display.map_or(10, |d| d.description_lines)),
        }
    };

    Context {
        format: global
            .format
            .or_else(|| display.map(|d| d.format))
            .unwrap_or_default(),
        audience,
        description_lines,
        extra_fields: display.map(|d| d.extra_fields.clone()).unwrap_or_default(),
        width: match audience {
            // Prose is wrapped to the window, within reason: a full-width
            // paragraph on an ultrawide monitor is unreadable, and a very narrow
            // terminal cannot be helped.
            Audience::Human => terminal_width().clamp(40, 110),
            // A pipe gets one width forever. Making output depend on the window
            // it was produced in would mean two runs of the same command
            // disagree, which is the kind of drift a fixed shape forbids.
            Audience::Machine => 100,
        },
    }
}

/// The terminal width, or a sane guess.
///
/// A pseudo-terminal with no size set reports zero columns rather than failing,
/// and wrapping prose to that would be worse than not asking at all — anything
/// implausibly narrow is treated as "unknown", not as the answer.
fn terminal_width() -> usize {
    const UNKNOWN: usize = 100;
    match termimad::crossterm::terminal::size() {
        Ok((cols, _)) if cols >= 20 => cols as usize,
        _ => UNKNOWN,
    }
}

/// Placeholder for a command that is declared but not built yet.
///
/// It exists so the command tree, its help text and the shell completions are
/// real from the first commit; the implementations land behind them.
#[must_use]
pub fn not_implemented(what: &str) -> ExitCode {
    let mut err = anstream::stderr();
    let _ = writeln!(
        err,
        "`{what}` is not implemented in this build yet — see docs/TODO.md"
    );
    ExitCode::NotImplemented
}
