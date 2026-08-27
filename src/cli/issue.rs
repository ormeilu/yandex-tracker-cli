//! Issue commands.
//!
//! `find` takes both shapes on purpose: flags for the queries people actually
//! run, and `--yql` for everything else. YQL is a read-only search language —
//! the escape hatch widens what can be *read*, never what can be changed
//! (`docs/adr/0001-security-model.md`).

use clap::{Args, Subcommand};

use crate::cli::{Session, emit, not_implemented, report};
use crate::exit::ExitCode;
use crate::render::{Format, machine, text};

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

pub async fn run(command: &IssueCommand, session: &Session) -> ExitCode {
    match command {
        IssueCommand::Get { key, fields } => get(key, fields, session).await,
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

/// Fetch one issue and render it at whichever rung of the ladder was asked for.
async fn get(key: &str, fields: &[String], session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let (mut issue, raw) = match client.issue(key).await {
        Ok(pair) => pair,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    // Links live on their own endpoint. A failure to fetch them must not hide
    // the issue itself: the caller asked for the issue, and a missing links
    // section is a smaller loss than no output at all.
    match client.issue_links(key).await {
        Ok(links) => issue.links = links,
        Err(error) => {
            tracing::warn!(%error, "could not fetch links");
        }
    }

    let rendered = match session.render.format {
        Format::Text if !fields.is_empty() => Ok(text::issue_selected(&issue, fields)),
        Format::Text => Ok(text::issue(&issue, &session.render)),
        Format::JsonRaw => machine(&raw, Format::JsonRaw),
        other => machine(&issue, other),
    };

    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}
