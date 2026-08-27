//! Issue commands.
//!
//! `find` takes both shapes on purpose: flags for the queries people actually
//! run, and `--yql` for everything else. YQL is a read-only search language —
//! the escape hatch widens what can be *read*, never what can be changed
//! (`docs/adr/0001-security-model.md`).

use clap::{Args, Subcommand};

use crate::cli::{Session, not_implemented};
use crate::exit::ExitCode;

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// Show one issue: summary, fields, links, first lines of the description.
    Get {
        /// Issue key, e.g. PROJ-42.
        key: String,
        /// Comma-separated field list; accepts custom field keys.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
    },
    /// Search for issues.
    Find(FindArgs),
    /// Count matching issues without fetching them. The cheapest question here.
    Count(FindArgs),
    /// Show the links of an issue.
    Links { key: String },
    /// Show the comments of an issue.
    Comments { key: String },
    /// Create an issue.
    Create {
        #[arg(long, short = 'q')]
        queue: Option<String>,
        #[arg(long, short = 's')]
        summary: String,
        #[arg(long, short = 'd')]
        description: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// Change fields of an issue.
    Update {
        key: String,
        #[arg(long, short = 's')]
        summary: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        /// Set any field, including custom ones: --set storyPoints=3
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
    },
    /// Add a comment.
    Comment {
        key: String,
        /// Comment body; `-` reads from stdin.
        text: String,
    },
    /// Move an issue through a workflow transition.
    Transition {
        key: String,
        /// Transition id; omit to list what is available.
        transition: Option<String>,
    },
}

/// Search arguments shared by `find` and `count`.
#[derive(Debug, Args, Clone)]
pub struct FindArgs {
    #[arg(long, short = 'q')]
    pub queue: Option<String>,
    /// Login, or `me`.
    #[arg(long, short = 'a')]
    pub assignee: Option<String>,
    #[arg(long, short = 's')]
    pub status: Option<String>,
    #[arg(long, value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Raw Yandex Query Language filter. Read-only, like every other search.
    #[arg(long)]
    pub yql: Option<String>,
    /// Rows per page.
    #[arg(long)]
    pub limit: Option<usize>,
    /// 1-based page number.
    #[arg(long, default_value_t = 1)]
    pub page: u32,
    /// Walk every page, up to --max.
    #[arg(long)]
    pub all: bool,
    /// Hard ceiling for --all; refuses rather than silently truncating.
    #[arg(long)]
    pub max: Option<usize>,
}

// The dispatcher is async because the implementations landing behind it are;
// the placeholders simply do not await anything yet.
#[allow(clippy::unused_async)]
pub async fn run(command: &IssueCommand, _session: &Session) -> ExitCode {
    match command {
        IssueCommand::Get { .. } => not_implemented("issue get"),
        IssueCommand::Find(_) => not_implemented("issue find"),
        IssueCommand::Count(_) => not_implemented("issue count"),
        IssueCommand::Links { .. } => not_implemented("issue links"),
        IssueCommand::Comments { .. } => not_implemented("issue comments"),
        IssueCommand::Create { .. } => not_implemented("issue create"),
        IssueCommand::Update { .. } => not_implemented("issue update"),
        IssueCommand::Comment { .. } => not_implemented("issue comment"),
        IssueCommand::Transition { .. } => not_implemented("issue transition"),
    }
}
