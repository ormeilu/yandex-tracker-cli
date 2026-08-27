//! Issue commands.
//!
//! `find` takes both shapes on purpose: flags for the queries people actually
//! run, and `--yql` for everything else. YQL is a read-only search language —
//! the escape hatch widens what can be *read*, never what can be changed
//! (`docs/adr/0001-security-model.md`).

use clap::{Args, Subcommand};

use crate::api::query::Filter;
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
    ///
    /// Conflicts with the flag filters on purpose: combining them would have to
    /// either silently drop half of what was asked for or invent an AND the
    /// caller did not write.
    #[arg(long, conflicts_with_all = ["queue", "assignee", "status", "tags"])]
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
        IssueCommand::Find(args) => find(args, session).await,
        IssueCommand::Count(args) => count(args, session).await,
        IssueCommand::Links { key } => links(key, session).await,
        IssueCommand::Comments { key } => comments(key, session).await,
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

/// Turn the search flags into a query, falling back to the profile's queue.
///
/// A search with nothing to narrow it would ask Tracker for every issue in the
/// organisation, so the pinned queue steps in — that is what pinning is for.
fn query_for(args: &FindArgs, session: &Session) -> Result<String, ExitCode> {
    if let Some(yql) = &args.yql {
        return Ok(yql.clone());
    }

    let mut filter = Filter {
        queue: args.queue.clone(),
        assignee: args.assignee.clone(),
        status: args.status.clone(),
        tags: args.tags.clone(),
    };

    if filter.queue.is_none() {
        filter.queue = session.default_queue().map(ToOwned::to_owned);
    }

    if filter.is_empty() {
        return Err(report(
            &"no filter given: pass --queue, --assignee, --status, --tags or --yql, \
              or pin a queue in .tracker.toml",
            ExitCode::ConfirmationRequired,
        ));
    }

    Ok(filter.to_query())
}

async fn find(args: &FindArgs, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let query = match query_for(args, session) {
        Ok(query) => query,
        Err(code) => return code,
    };

    let display = session.display();
    let per_page = args.limit.unwrap_or(display.limit);
    let Ok(per_page) = u32::try_from(per_page.max(1)) else {
        return report(&"--limit is too large", ExitCode::ConfirmationRequired);
    };

    if args.all {
        return find_all(&client, &query, per_page, args, session).await;
    }

    match client.search(&query, args.page.max(1), per_page).await {
        Ok(page) => emit_page(&page, session),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Walk every page up to the ceiling.
///
/// Refusing past `--max` rather than truncating is the whole point: a silently
/// short answer reads exactly like a complete one.
async fn find_all(
    client: &crate::api::Client,
    query: &str,
    per_page: u32,
    args: &FindArgs,
    session: &Session,
) -> ExitCode {
    let max = args.max.unwrap_or(session.display().max);
    let mut collected: Vec<crate::api::models::Issue> = Vec::new();
    let mut page_number = 1;
    let mut total = None;

    loop {
        let page = match client.search(query, page_number, per_page).await {
            Ok(page) => page,
            Err(error) => {
                let code = error.exit_code();
                return report(&error, code);
            }
        };
        total = page.total.or(total);

        if collected.len() + page.items.len() > max {
            return report(
                &format!(
                    "more than --max {max} issues match ({}); narrow the filter or raise --max",
                    total.map_or_else(|| "unknown total".to_owned(), |t| t.to_string()),
                ),
                ExitCode::ConfirmationRequired,
            );
        }

        let more = page.has_more();
        collected.extend(page.items);
        if !more {
            break;
        }
        page_number += 1;
    }

    let Ok(count) = u32::try_from(collected.len()) else {
        return report(&"too many results to render", ExitCode::Failure);
    };
    let page = crate::api::models::Page {
        items: collected,
        page: 1,
        per_page: count.max(1),
        total: total.or(Some(u64::from(count))),
    };
    emit_page(&page, session)
}

fn emit_page(
    page: &crate::api::models::Page<crate::api::models::Issue>,
    session: &Session,
) -> ExitCode {
    let rendered = match session.render.format {
        Format::Text => Ok(text::issue_page(page, &session.render)),
        // The upstream payload for a search is an array of issues; raw and
        // normalised differ only in field names, so raw maps onto our schema.
        Format::JsonRaw => machine(&page.items, Format::Json),
        other => machine(&page.items, other),
    };

    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}

/// The cheapest question in the tool: one number, no issue bodies.
async fn count(args: &FindArgs, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let query = match query_for(args, session) {
        Ok(query) => query,
        Err(code) => return code,
    };

    match client.count(&query).await {
        Ok(count) => {
            emit(&format!("{count}\n"));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

async fn links(key: &str, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.issue_links(key).await {
        Ok(links) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::links(key, &links)),
                Format::JsonRaw => machine(&links, Format::Json),
                other => machine(&links, other),
            };
            finish(rendered)
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

async fn comments(key: &str, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.issue_comments(key).await {
        Ok(comments) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::comments(key, &comments)),
                Format::JsonRaw => machine(&comments, Format::Json),
                other => machine(&comments, other),
            };
            finish(rendered)
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

fn finish(rendered: Result<String, crate::render::RenderError>) -> ExitCode {
    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}
