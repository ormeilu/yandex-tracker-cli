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
pub mod issue;
pub mod project;
pub mod queue;
pub mod write;

use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::{Config, Resolved};
use crate::exit::ExitCode;
use crate::render::{Audience, Context, Format};

/// Token-efficient Yandex Tracker CLI for humans and AI agents.
#[derive(Debug, Parser)]
#[command(name = "ytcli", version, about, long_about = None)]
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
    /// Profile to act as. Overrides `YTCLI_PROFILE` and any `.tracker.toml`.
    ///
    /// The environment variable is read separately rather than through clap's
    /// `env`, so that `auth status` can honestly report which of the two the
    /// value came from.
    #[arg(long, short = 'p', global = true)]
    pub profile: Option<String>,

    /// Output format: text, json, json-raw, toon.
    #[arg(long, short = 'f', global = true, value_name = "FORMAT")]
    pub format: Option<Format>,

    /// Show full description instead of the first lines.
    #[arg(long, global = true)]
    pub full: bool,

    /// Confirm a change that affects more than one issue.
    #[arg(long, global = true)]
    pub yes: bool,

    /// Show what would change without sending anything.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Logging verbosity; repeat for more. Logs go to stderr, stdout stays clean.
    #[arg(long, short = 'v', global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Config file to use instead of the per-user default.
    #[arg(long, global = true, value_name = "PATH")]
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
    Cheatsheet(cheatsheet::CheatsheetArgs),
    /// Generate a shell completion script.
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

    /// Build an API client for the active profile.
    ///
    /// Every failure on the way here — no profile, no stored token, a token the
    /// keychain will not release — is an auth problem from the caller's point of
    /// view, and reports as one.
    pub fn client(&self) -> Result<crate::api::Client, ExitCode> {
        let resolved = self
            .resolved()
            .map_err(|error| report(&error, ExitCode::Auth))?;

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
    Context {
        format: global
            .format
            .or_else(|| display.map(|d| d.format))
            .unwrap_or_default(),
        audience: Audience::detect(),
        description_lines: if global.full {
            None
        } else {
            Some(display.map_or(10, |d| d.description_lines))
        },
        extra_fields: display.map(|d| d.extra_fields.clone()).unwrap_or_default(),
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
