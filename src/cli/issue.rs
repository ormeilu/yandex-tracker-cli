//! Issue commands.
//!
//! `find` takes both shapes on purpose: flags for the queries people actually
//! run, and `--yql` for everything else. YQL is a read-only search language —
//! the escape hatch widens what can be *read*, never what can be changed
//! (`docs/adr/0001-security-model.md`).

use std::io::Write as _;

use clap::{Args, Subcommand};

use crate::api::query::Filter;
use crate::cli::write::{Gate, Intent, check, parse_assignment};
use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{self, Format, image, machine, text};

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// Show one issue: summary, fields, links, first lines of the description.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_GET))]
    Get {
        /// Issue key, e.g. PROJ-42. Prefix it with a profile — `work/PROJ-42` —
        /// when two profiles can both see a queue with that key.
        key: String,
        /// Comma-separated field list; accepts custom field keys.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
    },
    /// Search for issues.
    ///
    /// Every other group here lists with `list` — queues, boards, fields,
    /// templates, projects. An agent that has learnt that spelling reaches for
    /// `issue list` too, and a "no such subcommand" for a verb the tool already
    /// uses everywhere else costs a round trip to discover nothing.
    #[command(visible_alias = "list", long_about = crate::cli::help::md(crate::cli::help::ISSUE_FIND))]
    Find(FindArgs),
    /// Count matching issues without fetching them. The cheapest question here.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_COUNT))]
    Count(FindArgs),
    /// Show the links of an issue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_LINKS))]
    Links { key: String },
    /// Show the links from an issue to things outside Tracker.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_REMOTELINKS))]
    Remotelinks { key: String },
    /// Show what changed on an issue, and who changed it.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_CHANGELOG))]
    Changelog {
        key: String,
        /// How many recorded events to fetch.
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Show the comments of an issue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_COMMENTS))]
    Comments { key: String },
    /// Create an issue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_CREATE))]
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
    /// Change fields of one or more issues.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_UPDATE))]
    Update {
        /// Issues to change. More than one needs --yes.
        #[arg(required = true)]
        keys: Vec<String>,
        #[arg(long, short = 's')]
        summary: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        /// Set any field, including custom ones: --set storyPoints=3
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
        /// Hand the change to Tracker and print its id without waiting for it
        /// to finish. Several issues only.
        #[arg(long)]
        no_wait: bool,
    },
    /// Add a comment, or edit and remove one. Every verb here writes.
    ///
    /// A group with a bare form: `issue comment PROJ-1 "text"` is how this was
    /// spelled before there was anything to edit, it is in every allowlist
    /// people wrote, and breaking it to gain two subcommands would be a poor
    /// trade. `add` is the same thing said explicitly.
    #[command(args_conflicts_with_subcommands = true,
              long_about = crate::cli::help::md(crate::cli::help::ISSUE_COMMENT))]
    Comment {
        #[command(subcommand)]
        command: Option<CommentCommand>,
        /// Issue to comment on.
        key: Option<String>,
        /// Comment body; `-` reads from stdin.
        text: Option<String>,
    },
    /// Show the worklog of an issue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_WORKLOGS))]
    Worklogs { key: String },
    /// Show the checklist of an issue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_CHECKLIST))]
    Checklist { key: String },
    /// Record or remove time spent. Every verb here writes.
    #[command(subcommand, long_about = crate::cli::help::md(crate::cli::help::ISSUE_WORKLOG))]
    Worklog(WorklogCommand),
    /// Change an issue's checklist. Every verb here writes.
    #[command(subcommand, long_about = crate::cli::help::md(crate::cli::help::ISSUE_CHECK))]
    Check(CheckCommand),
    /// Link or unlink issues. Every verb here writes.
    #[command(subcommand, long_about = crate::cli::help::md(crate::cli::help::ISSUE_LINK))]
    Link(LinkCommand),
    /// Move an issue to another queue. Its key changes, and nothing undoes it.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_MOVE))]
    Move {
        /// Issue keys. Several go in one request, to one queue.
        #[arg(required = true)]
        keys: Vec<String>,
        /// Queue to move them into.
        #[arg(long, short = 't')]
        to: String,
        /// Carry over fields the target queue does not define. Without this,
        /// Tracker drops them.
        #[arg(long)]
        keep_fields: bool,
        /// Start the issue at the target workflow's first status instead of
        /// keeping the one it has.
        #[arg(long)]
        initial_status: bool,
        /// Return the bulk change id instead of waiting for Tracker to finish.
        #[arg(long)]
        no_wait: bool,
    },
    /// Move an issue through a workflow transition.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_TRANSITION))]
    Transition {
        /// One issue key, or a key and a transition id; several keys need --to.
        #[arg(required = true)]
        keys: Vec<String>,
        /// Transition id. Required when naming more than one issue.
        #[arg(long, short = 't')]
        to: Option<String>,
        /// Resolution to close with. `dict list --kind resolutions` lists them.
        #[arg(long, short = 'r')]
        resolution: Option<String>,
        /// Any other field the transition needs: --set comment=text
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
        /// Return the bulk change id instead of waiting for Tracker to finish.
        #[arg(long)]
        no_wait: bool,
    },
}

/// Writing to a worklog.
///
/// Reading it is `issue worklogs`, deliberately a different word rather than a
/// `list` under here: a host allowlists by command prefix, and a group holding
/// both a read and a write cannot be allowed without allowing the writes too.
#[derive(Debug, Subcommand)]
pub enum WorklogCommand {
    /// Record time spent on an issue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WORKLOG_ADD))]
    Add {
        key: String,
        /// How long: 1h30m, 45m, 2d, or an ISO 8601 duration.
        duration: String,
        /// What the time went on.
        #[arg(long, short = 'm')]
        comment: Option<String>,
        /// When the work started, as a date or a timestamp. Defaults to now.
        #[arg(long)]
        start: Option<String>,
    },
    /// Correct an entry that is already recorded.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WORKLOG_EDIT))]
    Edit {
        key: String,
        /// Worklog id, from `issue worklogs`.
        id: String,
        /// The corrected duration.
        #[arg(long, short = 'd')]
        duration: Option<String>,
        /// The corrected comment.
        #[arg(long, short = 'm')]
        comment: Option<String>,
    },
    /// Remove one worklog entry.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WORKLOG_DELETE))]
    Delete { key: String, id: String },
}

/// Editing comments. Reading them is `issue comments`.
#[derive(Debug, Subcommand)]
pub enum CommentCommand {
    /// Add a comment.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ISSUE_COMMENT))]
    Add {
        key: String,
        /// Comment body; `-` reads from stdin.
        text: String,
    },
    /// Replace the text of a comment.
    #[command(long_about = crate::cli::help::md(crate::cli::help::COMMENT_EDIT))]
    Edit {
        key: String,
        /// Comment id, from `issue comments`.
        id: String,
        /// The new body in full; `-` reads from stdin.
        text: String,
    },
    /// Remove a comment.
    #[command(long_about = crate::cli::help::md(crate::cli::help::COMMENT_DELETE))]
    Delete { key: String, id: String },
}

/// Writing to a checklist. Reading it is `issue checklist`.
#[derive(Debug, Subcommand)]
pub enum CheckCommand {
    /// Add a line to the checklist.
    #[command(long_about = crate::cli::help::md(crate::cli::help::CHECK_ADD))]
    Add {
        key: String,
        text: String,
        #[arg(long)]
        assignee: Option<String>,
        /// Deadline, as `2026-09-01`.
        #[arg(long)]
        deadline: Option<String>,
    },
    /// Tick a line off.
    #[command(long_about = crate::cli::help::md(crate::cli::help::CHECK_TICK))]
    Tick { key: String, id: String },
    /// Put a ticked line back.
    #[command(long_about = crate::cli::help::md(crate::cli::help::CHECK_UNTICK))]
    Untick { key: String, id: String },
    /// Remove a line.
    #[command(long_about = crate::cli::help::md(crate::cli::help::CHECK_DELETE))]
    Delete { key: String, id: String },
}

/// Writing links. Reading them is `issue links`.
#[derive(Debug, Subcommand)]
pub enum LinkCommand {
    /// Link two issues.
    #[command(long_about = crate::cli::help::md(crate::cli::help::LINK_ADD))]
    Add {
        key: String,
        /// Relationship from this issue to the other: relates, depends,
        /// is-dependent-by, subtask, parent, duplicates, is-duplicated-by,
        /// epic, has-epic.
        relation: String,
        /// The other issue.
        other: String,
    },
    /// Remove a link, by the link id `issue links` prints.
    #[command(long_about = crate::cli::help::md(crate::cli::help::LINK_DELETE))]
    Delete { key: String, id: String },
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
        IssueCommand::Remotelinks { key } => remote_links(key, session).await,
        IssueCommand::Comments { key } => comments(key, session).await,
        IssueCommand::Changelog { key, limit } => changelog(key, *limit, session).await,
        IssueCommand::Move {
            keys,
            to,
            keep_fields,
            initial_status,
            no_wait,
        } => move_issues(keys, to, *keep_fields, *initial_status, *no_wait, session).await,
        IssueCommand::Create {
            queue,
            summary,
            description,
            assignee,
            tags,
        } => {
            create(
                queue.as_deref(),
                summary,
                description.as_deref(),
                assignee.as_deref(),
                tags,
                session,
            )
            .await
        }
        IssueCommand::Update {
            keys,
            summary,
            assignee,
            set,
            no_wait,
        } => {
            update(
                keys,
                summary.as_deref(),
                assignee.as_deref(),
                set,
                *no_wait,
                session,
            )
            .await
        }
        IssueCommand::Comment { command, key, text } => match (command, key, text) {
            (Some(command), _, _) => comment_write(command, session).await,
            (None, Some(key), Some(text)) => comment(key, text, session).await,
            // clap cannot require two positionals that a subcommand replaces,
            // so the bare form is checked here rather than in the parser.
            (None, ..) => report(
                &"usage: ytcli issue comment <KEY> <TEXT>, or `issue comment --help`",
                ExitCode::ConfirmationRequired,
            ),
        },
        IssueCommand::Transition {
            keys,
            to,
            resolution,
            set,
            no_wait,
        } => {
            transition_cmd(
                keys,
                to.as_deref(),
                resolution.as_deref(),
                set,
                *no_wait,
                session,
            )
            .await
        }
        IssueCommand::Worklogs { key } => worklogs(key, session).await,
        IssueCommand::Checklist { key } => checklist(key, session).await,
        IssueCommand::Worklog(command) => worklog_write(command, session).await,
        IssueCommand::Check(command) => check_write(command, session).await,
        IssueCommand::Link(command) => link_write(command, session).await,
    }
}

/// Read an issue's worklog.
async fn worklogs(target: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    match client.worklogs(&key).await {
        Ok(entries) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::worklogs(&key, &entries, &session.render)),
                other => machine(&entries, other),
            };
            finish(rendered)
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Read an issue's checklist.
async fn checklist(target: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    match client.checklist(&key).await {
        Ok(items) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::checklist(&key, &items, &session.render)),
                other => machine(&items, other),
            };
            finish(rendered)
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

async fn worklog_write(command: &WorklogCommand, session: &Session) -> ExitCode {
    match command {
        WorklogCommand::Add {
            key,
            duration,
            comment,
            start,
        } => {
            let (client, key) = match session.client_for(key).await {
                Ok(pair) => pair,
                Err(code) => return code,
            };

            let iso = match crate::api::duration::to_iso8601(duration) {
                Ok(iso) => iso,
                Err(error) => return report(&error, ExitCode::ConfirmationRequired),
            };

            let mut body = serde_json::Map::new();
            body.insert("duration".to_owned(), serde_json::json!(iso));
            // Tracker requires a start; "now" is what somebody logging time at
            // the end of the work means, and it is what they would type.
            body.insert(
                "start".to_owned(),
                serde_json::json!(start.clone().unwrap_or_else(now_for_tracker)),
            );
            if let Some(comment) = comment {
                body.insert("comment".to_owned(), serde_json::json!(comment));
            }
            let body = serde_json::Value::Object(body);

            let targets = [key.clone()];
            let intent = Intent {
                action: &format!("log {duration} against {key}"),
                targets: &targets,
                body: &body,
                always_confirm: false,
            };
            if let Gate::Stop(code) = check(&intent, session) {
                return code;
            }

            match client.add_worklog(&key, &body).await {
                Ok(entry) => {
                    emit(&format!(
                        "{key} worklog {} {}\n",
                        entry.id,
                        crate::api::duration::human(&entry.duration)
                    ));
                    ExitCode::Success
                }
                Err(error) => {
                    let code = error.exit_code();
                    report(&error, code)
                }
            }
        }
        WorklogCommand::Edit {
            key,
            id,
            duration,
            comment,
        } => worklog_edit(key, id, duration.as_deref(), comment.as_deref(), session).await,
        WorklogCommand::Delete { key, id } => {
            delete_with_gate(key, id, session, "worklog", |client, key, id| {
                Box::pin(async move { client.delete_worklog(key, id).await })
            })
            .await
        }
    }
}

/// Correct an entry that is already recorded.
async fn worklog_edit(
    key: &str,
    id: &str,
    duration: Option<&str>,
    comment: Option<&str>,
    session: &Session,
) -> ExitCode {
    // Nothing to change is a mistake worth catching before a request,
    // like an update that sets no field.
    if duration.is_none() && comment.is_none() {
        return report(
            &"nothing to change: pass --duration, --comment, or both",
            ExitCode::ConfirmationRequired,
        );
    }

    let (client, key) = match session.client_for(key).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    let mut body = serde_json::Map::new();
    if let Some(duration) = duration {
        let iso = match crate::api::duration::to_iso8601(duration) {
            Ok(iso) => iso,
            Err(error) => return report(&error, ExitCode::ConfirmationRequired),
        };
        body.insert("duration".to_owned(), serde_json::json!(iso));
    }
    if let Some(comment) = comment {
        body.insert("comment".to_owned(), serde_json::json!(comment));
    }
    let body = serde_json::Value::Object(body);

    let targets = [key.clone()];
    let intent = Intent {
        action: &format!("correct worklog {id} of {key}"),
        targets: &targets,
        body: &body,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.update_worklog(&key, id, &body).await {
        Ok(entry) => {
            emit(&format!(
                "{key} worklog {} {}\n",
                entry.id,
                crate::api::duration::human(&entry.duration)
            ));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Adding, rewriting and removing comments.
async fn comment_write(command: &CommentCommand, session: &Session) -> ExitCode {
    match command {
        CommentCommand::Add { key, text } => comment(key, text, session).await,
        CommentCommand::Edit { key, id, text } => {
            let (client, key) = match session.client_for(key).await {
                Ok(pair) => pair,
                Err(code) => return code,
            };

            let text_body = match body_text(text) {
                Ok(text) => text,
                Err(code) => return code,
            };

            let body = serde_json::json!({ "text": text_body });
            let targets = [key.clone()];
            let intent = Intent {
                action: &format!("replace the text of comment {id} on {key}"),
                targets: &targets,
                body: &body,
                always_confirm: false,
            };
            if let Gate::Stop(code) = check(&intent, session) {
                return code;
            }

            match client.update_comment(&key, id, &text_body).await {
                Ok(comment) => {
                    emit(&format!("{key} comment {} edited\n", comment.id));
                    ExitCode::Success
                }
                Err(error) => {
                    let code = error.exit_code();
                    report(&error, code)
                }
            }
        }
        CommentCommand::Delete { key, id } => {
            delete_with_gate(key, id, session, "comment", |client, key, id| {
                Box::pin(async move { client.delete_comment(key, id).await })
            })
            .await
        }
    }
}

async fn check_write(command: &CheckCommand, session: &Session) -> ExitCode {
    match command {
        CheckCommand::Add {
            key,
            text: line,
            assignee,
            deadline,
        } => {
            let (client, key) = match session.client_for(key).await {
                Ok(pair) => pair,
                Err(code) => return code,
            };

            let mut body = serde_json::Map::new();
            body.insert("text".to_owned(), serde_json::json!(line));
            if let Some(assignee) = assignee {
                body.insert("assignee".to_owned(), serde_json::json!(assignee));
            }
            if let Some(deadline) = deadline {
                body.insert(
                    "deadline".to_owned(),
                    serde_json::json!({ "date": deadline }),
                );
            }
            let body = serde_json::Value::Object(body);

            let targets = [key.clone()];
            let intent = Intent {
                action: &format!("add a checklist line to {key}"),
                targets: &targets,
                body: &body,
                always_confirm: false,
            };
            if let Gate::Stop(code) = check(&intent, session) {
                return code;
            }

            match client.add_checklist_item(&key, &body).await {
                Ok(items) => {
                    emit(&text::checklist(&key, &items, &session.render));
                    ExitCode::Success
                }
                Err(error) => {
                    let code = error.exit_code();
                    report(&error, code)
                }
            }
        }
        CheckCommand::Tick { key, id } => set_checked(key, id, true, session).await,
        CheckCommand::Untick { key, id } => set_checked(key, id, false, session).await,
        CheckCommand::Delete { key, id } => {
            delete_with_gate(key, id, session, "checklist item", |client, key, id| {
                Box::pin(async move { client.delete_checklist_item(key, id).await })
            })
            .await
        }
    }
}

async fn set_checked(target: &str, id: &str, checked: bool, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    let body = serde_json::json!({ "checked": checked });
    let targets = [key.clone()];
    let verb = if checked { "tick" } else { "untick" };
    let intent = Intent {
        action: &format!("{verb} checklist item {id} of {key}"),
        targets: &targets,
        body: &body,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.update_checklist_item(&key, id, &body).await {
        Ok(items) => {
            emit(&text::checklist(&key, &items, &session.render));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// The four link *type* ids that are not link *relationships*.
///
/// `GET /v3/linktypes` answers with `depends`, `subtask`, `epic` and the rest;
/// a write takes a directional phrase — `depends on`, `is subtask for` — and
/// Tracker refuses anything else with `Unrecognized value`. The two lists were
/// confused in this tool's own help until a live check caught it, which is
/// evidence enough that the confusion is easy.
///
/// Only these four are refused here. Tracker tolerates hyphens for the rest,
/// and `cloners` is a real type in an organisation checked against, so a closed
/// list of accepted values here would block a write that would have worked.
fn corrected(relation: &str) -> Option<&'static str> {
    let normalised = relation
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], " ");
    Some(match normalised.as_str() {
        "depends" => "depends on",
        "parent" => "is parent task for",
        "subtask" => "is subtask for",
        "epic" => "is epic of",
        _ => return None,
    })
}

async fn link_write(command: &LinkCommand, session: &Session) -> ExitCode {
    match command {
        LinkCommand::Add {
            key,
            relation,
            other,
        } => {
            // Four names look right and are not, and they are wrong in a way
            // Tracker's own refusal does not help with: it names the value it
            // did not recognise and never what it wanted instead.
            if let Some(correct) = corrected(relation) {
                return report(
                    &format!(
                        "`{relation}` is the id of a link type, not a relationship: write `{correct}`. \
                         `ytcli link types` lists both."
                    ),
                    ExitCode::ConfirmationRequired,
                );
            }

            let (client, key) = match session.client_for(key).await {
                Ok(pair) => pair,
                Err(code) => return code,
            };

            let body = serde_json::json!({ "relationship": relation, "issue": other });
            let targets = [key.clone()];
            let intent = Intent {
                action: &format!("link {key} {relation} {other}"),
                targets: &targets,
                body: &body,
                always_confirm: false,
            };
            if let Gate::Stop(code) = check(&intent, session) {
                return code;
            }

            match client.add_link(&key, relation, other).await {
                Ok(()) => {
                    emit(&format!("{key} {relation} {other}\n"));
                    ExitCode::Success
                }
                Err(error) => {
                    let code = error.exit_code();
                    report(&error, code)
                }
            }
        }
        LinkCommand::Delete { key, id } => {
            delete_with_gate(key, id, session, "link", |client, key, id| {
                Box::pin(async move { client.delete_link(key, id).await })
            })
            .await
        }
    }
}

/// The shape every deletion here shares: announce, gate, delete, say so.
///
/// Tracker has no undelete for any of these, and none of them is reported by
/// anything else afterwards, so the line printed at the end is the only record
/// the caller gets.
async fn delete_with_gate<F>(
    target: &str,
    id: &str,
    session: &Session,
    what: &str,
    delete: F,
) -> ExitCode
where
    F: for<'a> FnOnce(
        &'a crate::api::Client,
        &'a str,
        &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), crate::api::error::ApiError>> + 'a>,
    >,
{
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    let body = serde_json::json!({ "delete": id });
    let targets = [key.clone()];
    let intent = Intent {
        action: &format!("delete {what} {id} of {key}"),
        targets: &targets,
        body: &body,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match delete(&client, &key, id).await {
        Ok(()) => {
            emit(&format!("{key} {what} {id} deleted\n"));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Now, in the form Tracker takes for a worklog start.
fn now_for_tracker() -> String {
    jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3f%z")
        .to_string()
}

/// Fetch one issue and render it at whichever rung of the ladder was asked for.
async fn get(target: &str, fields: &[String], session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

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

    // Pictures are fetched before the issue is rendered, because the ones the
    // description points at are drawn where it points at them.
    let mut ctx = session.render.clone();
    let mut drawn = Vec::new();
    if fields.is_empty() {
        let (inline, used) = inline_images(&client, key, issue.description.as_deref(), &ctx).await;
        ctx.inline = inline;
        drawn = used;
    }

    let rendered = match ctx.format {
        Format::Text if !fields.is_empty() => Ok(text::issue_selected(&issue, fields)),
        Format::Text => Ok(text::issue(&issue, &ctx)),
        Format::JsonRaw => machine(&raw, Format::JsonRaw),
        other => machine(&issue, other),
    };

    match rendered {
        Ok(text) => {
            emit(&text);
            // Whatever the description did not point at is still worth seeing,
            // and it goes after the issue because nothing said where it belongs.
            draw_remaining_images(&client, key, &drawn, &ctx).await;
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}

/// How many unreferenced images are drawn after an issue before it stops and
/// names the rest.
///
/// A screenshot is worth the space; six of them are a wall between the reader
/// and the next command. The rest are one `attachment show` away.
const IMAGES_SHOWN: usize = 4;

/// Whether this command may draw at all, and how wide.
///
/// Nothing is drawn for a pipe, an agent, a `--format` other than text, a
/// terminal without a graphics protocol, or `--no-images`. In every one of
/// those cases the attachments are not requested either, so the cheap path
/// costs exactly what it did before.
fn drawing(ctx: &crate::render::Context) -> Option<image::Protocol> {
    if !ctx.images || !ctx.is_human() || ctx.format != Format::Text {
        return None;
    }
    image::protocol()
}

/// Fetch and draw the pictures the description points at.
///
/// Returns them keyed by the URL as written, plus the ids that were used, so
/// the caller knows which attachments have already been shown.
///
/// Only attachments are drawn. A description can name any URL it likes, and
/// following one would turn reading an issue into fetching whatever an issue's
/// author decided this tool should fetch — with the client's own credentials,
/// at that. The reference has to resolve to a file already attached to this
/// issue, or it stays the markdown it was.
async fn inline_images(
    client: &crate::api::Client,
    key: &str,
    description: Option<&str>,
    ctx: &crate::render::Context,
) -> (image::Inline, Vec<String>) {
    let mut inline = image::Inline::default();
    let mut used = Vec::new();

    let Some(protocol) = drawing(ctx) else {
        return (inline, used);
    };
    let Some(description) = description else {
        return (inline, used);
    };
    let references = crate::render::markdown::image_references(description);
    if references.is_empty() {
        return (inline, used);
    }

    let attachments = match client.attachments(key).await {
        Ok(attachments) => attachments,
        Err(error) => {
            tracing::warn!(%error, "could not fetch attachments");
            return (inline, used);
        }
    };

    // Two columns for the margin bar the quoted block puts on every line.
    let width = ctx.width.saturating_sub(2);

    for (alt, url) in references {
        let Some(attachment) = attachment_for(&attachments, url) else {
            tracing::debug!(url, "no attachment matches this image reference");
            continue;
        };
        let Some(picture) = fetch_picture(client, attachment, protocol, width).await else {
            continue;
        };

        used.push(attachment.id.clone());
        // The caption names the file, because the alt text is what the author
        // said and the filename is what `attachment show` and `download` take.
        let caption = if alt.is_empty() {
            attachment.name.clone()
        } else {
            format!("{alt} — {}", attachment.name)
        };
        inline.insert(url.to_owned(), image::Picture { caption, ..picture });
    }

    (inline, used)
}

/// The attachment a markdown image URL refers to.
///
/// Tracker writes these as `/ajax/v2/attachments/29?inline=true`, so the id is
/// the last path segment; a description written by hand may name the file
/// instead. Both are matched against what is actually attached to this issue,
/// and nothing else is followed.
fn attachment_for<'a>(
    attachments: &'a [crate::api::models::Attachment],
    url: &str,
) -> Option<&'a crate::api::models::Attachment> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').find(|segment| !segment.is_empty())?;

    attachments
        .iter()
        .find(|attachment| attachment.id == last || attachment.name == last)
}

/// Download one attachment and turn it into a picture, if it is one.
async fn fetch_picture(
    client: &crate::api::Client,
    attachment: &crate::api::models::Attachment,
    protocol: image::Protocol,
    width: usize,
) -> Option<image::Picture> {
    let url = attachment.content.as_deref()?;
    let bytes = match client.download(url).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(%error, id = attachment.id, "could not download attachment");
            return None;
        }
    };

    // The declared type is a claim; the bytes are the fact. Drawing on the
    // claim alone would hand a terminal whatever a file said it was.
    let kind = image::Kind::of(&bytes)?;
    if !protocol.carries(kind) {
        return None;
    }

    Some(image::Picture {
        escape: image::draw(protocol, &bytes, &attachment.name, width),
        caption: attachment.name.clone(),
    })
}

/// Draw the image attachments the description never mentioned.
///
/// Failures are logged and swallowed. The caller asked for an issue and already
/// has it; losing a picture is a smaller loss than replacing the issue with an
/// error about one.
async fn draw_remaining_images(
    client: &crate::api::Client,
    key: &str,
    already_drawn: &[String],
    ctx: &crate::render::Context,
) {
    let Some(protocol) = drawing(ctx) else {
        return;
    };

    let attachments = match client.attachments(key).await {
        Ok(attachments) => attachments,
        Err(error) => {
            tracing::warn!(%error, "could not fetch attachments");
            return;
        }
    };

    // The declared type decides what is worth downloading; the bytes decide
    // what is actually drawn. Trusting the declaration alone would fetch a
    // 40 MB video that claimed to be a PNG.
    let images: Vec<_> = attachments
        .iter()
        .filter(|attachment| !already_drawn.contains(&attachment.id))
        .filter(|attachment| {
            attachment
                .mimetype
                .as_deref()
                .is_some_and(|kind| kind.starts_with("image/"))
        })
        .collect();

    for attachment in images.iter().take(IMAGES_SHOWN) {
        if let Some(picture) = fetch_picture(client, attachment, protocol, ctx.width).await {
            emit(&picture.escape);
            emit(&format!("{}\n", picture.caption));
        }
    }

    if images.len() > IMAGES_SHOWN {
        emit(&format!(
            "{} more image(s): ytcli attachment show {key} <id>\n",
            images.len() - IMAGES_SHOWN
        ));
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
    let walk = crate::render::progress::Walk::start("searching");

    loop {
        let page = match client.search(query, page_number, per_page).await {
            Ok(page) => page,
            Err(error) => {
                walk.finish();
                let code = error.exit_code();
                return report(&error, code);
            }
        };
        total = page.total.or(total);

        if collected.len() + page.items.len() > max {
            walk.finish();
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
        walk.page(page_number, collected.len(), total);
        if !more {
            break;
        }
        page_number += 1;
    }
    walk.finish();

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

async fn links(target: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

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

async fn remote_links(target: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    match client.issue_remote_links(key).await {
        Ok(links) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::remote_links(key, &links, &session.render)),
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

async fn comments(target: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    match client.issue_comments(key).await {
        Ok(comments) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::comments(key, &comments, &session.render)),
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

/// What changed, and who changed it.
async fn changelog(target: &str, limit: u32, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    match client.changelog(key, limit.max(1)).await {
        Ok(changes) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::changelog(key, &changes, &session.render)),
                Format::JsonRaw => machine(&changes, Format::Json),
                other => machine(&changes, other),
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

/// Read a body that may have been piped in: `-` means stdin.
///
/// Agents produce long text; making them quote it into an argument invites
/// mangling, and a shell argument is visible in `ps` besides.
fn body_text(raw: &str) -> Result<String, ExitCode> {
    if raw != "-" {
        return Ok(raw.to_owned());
    }

    let mut text = String::new();
    match std::io::Read::read_to_string(&mut std::io::stdin(), &mut text) {
        Ok(_) => Ok(text),
        Err(error) => Err(report(&error, ExitCode::Failure)),
    }
}

async fn create(
    queue: Option<&str>,
    summary: &str,
    description: Option<&str>,
    assignee: Option<&str>,
    tags: &[String],
    session: &Session,
) -> ExitCode {
    let Some(queue) = queue.or_else(|| session.default_queue()) else {
        return report(
            &"no queue given: pass --queue or pin one in .tracker.toml",
            ExitCode::ConfirmationRequired,
        );
    };

    let description = match description.map(body_text).transpose() {
        Ok(description) => description,
        Err(code) => return code,
    };

    let mut body = serde_json::json!({
        "queue": { "key": queue },
        "summary": summary,
    });
    if let Some(description) = &description {
        body["description"] = serde_json::json!(description);
    }
    if let Some(assignee) = assignee {
        body["assignee"] = serde_json::json!(assignee);
    }
    if !tags.is_empty() {
        body["tags"] = serde_json::json!(tags);
    }

    let intent = Intent {
        action: &format!("create an issue in {queue}"),
        targets: &[],
        body: &body,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.create_issue(&body).await {
        Ok(issue) => {
            emit(&format!("{}  {}\n", issue.key, issue.summary));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Apply the same change to every named issue.
///
/// The body is built once, because the point of naming several issues is that
/// they get the same change; a per-issue variation would be several commands.
/// Targets are resolved before anything is sent, so a typo in the third key does
/// not leave the first two changed and the caller guessing.
async fn update(
    targets: &[String],
    summary: Option<&str>,
    assignee: Option<&str>,
    set: &[String],
    no_wait: bool,
    session: &Session,
) -> ExitCode {
    let mut resolved = Vec::with_capacity(targets.len());
    for target in targets {
        match session.client_for(target).await {
            Ok(pair) => resolved.push(pair),
            Err(code) => return code,
        }
    }

    let mut body = serde_json::Map::new();
    if let Some(summary) = summary {
        body.insert("summary".to_owned(), serde_json::json!(summary));
    }
    if let Some(assignee) = assignee {
        body.insert("assignee".to_owned(), serde_json::json!(assignee));
    }
    for assignment in set {
        match parse_assignment(assignment) {
            Ok((field, value)) => {
                body.insert(field, value);
            }
            Err(error) => return report(&error, ExitCode::ConfirmationRequired),
        }
    }

    if body.is_empty() {
        return report(
            &"nothing to change: pass --summary, --assignee or --set key=value",
            ExitCode::ConfirmationRequired,
        );
    }

    let body = serde_json::Value::Object(body);
    let keys: Vec<String> = resolved.iter().map(|(_, key)| key.clone()).collect();

    // A bulk change is one request to one organisation. Keys resolve per
    // profile, so a list can straddle two of them, and that list has to go the
    // slow way round.
    let one_org = resolved.first().is_some_and(|(first, _)| {
        resolved
            .iter()
            .all(|(client, _)| client.org() == first.org())
    });
    let bulk = keys.len() > 1 && one_org;

    // What --dry-run prints has to be what would actually be sent, and the two
    // paths do not send the same shape.
    let request = if bulk {
        serde_json::json!({ "issues": keys, "values": body })
    } else {
        body.clone()
    };
    let intent = Intent {
        action: &format!("update {}", keys.join(", ")),
        targets: &keys,
        body: &request,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    if bulk {
        let Some((client, _)) = resolved.first() else {
            return ExitCode::Success;
        };
        return bulk_update(client, &keys, &body, no_wait, session).await;
    }

    // One issue, or several that no single request can cover. Stopping at the
    // first failure: a run that carried on would leave the caller to work out
    // which issues it got to before it stopped.
    let mut done = 0_u64;
    for (client, key) in &resolved {
        match client.update_issue(key, &body).await {
            Ok(issue) => {
                done += 1;
                emit(&text::issue_selected(
                    &issue,
                    &["status".to_owned(), "assignee".to_owned()],
                ));
            }
            Err(error) => {
                let code = error.exit_code();
                // The tally first: how far it got is the part that decides what
                // the caller has to do next.
                if keys.len() > 1 {
                    emit(&render::bulk::changed(
                        done,
                        keys.len() as u64,
                        &session.render,
                    ));
                }
                return report(&error, code);
            }
        }
    }

    if keys.len() > 1 {
        emit(&render::bulk::changed(
            done,
            keys.len() as u64,
            &session.render,
        ));
    }
    ExitCode::Success
}

/// How long to wait for Tracker to finish a bulk change before saying so.
///
/// Long enough that an ordinary change is simply done when the command returns,
/// and short enough that a caller is not held indefinitely by work that is
/// Tracker's to finish either way. Past it the id is the answer.
const BULK_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Every issue in one request, and the answer polled until it is one.
///
/// Tracker validates the whole list before it writes anything: an unknown key
/// is a refusal naming it, rather than half the change applied and an error.
async fn bulk_update(
    client: &crate::api::Client,
    keys: &[String],
    values: &serde_json::Value,
    no_wait: bool,
    session: &Session,
) -> ExitCode {
    let started = match client.bulk_update(keys, values).await {
        Ok(change) => change,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    awaited(client, started, no_wait, session).await
}

/// Poll a bulk change to its end, and report it.
///
/// Shared by every command that can start one: what a caller is owed after
/// `_update`, `_transition` and `_move` is the same tally and the same id.
async fn awaited(
    client: &crate::api::Client,
    started: crate::api::BulkChange,
    no_wait: bool,
    session: &Session,
) -> ExitCode {
    if no_wait {
        // Accepted, which is what was asked for and all that is being claimed.
        emit(&render::bulk::change(&started, &session.render));
        return ExitCode::Success;
    }

    let mut change = started;
    let deadline = std::time::Instant::now() + BULK_WAIT;
    while !change.finished() {
        if std::time::Instant::now() >= deadline {
            emit(&render::bulk::change(&change, &session.render));
            return report(
                &format!(
                    "Tracker is still working on it; ask again with `ytcli bulk status {}`",
                    change.id
                ),
                ExitCode::Failure,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        match client.bulk_change(&change.id).await {
            Ok(next) => change = next,
            Err(error) => {
                let code = error.exit_code();
                return report(&error, code);
            }
        }
    }

    finished(client, &change, session).await
}

/// Report a bulk change that Tracker has finished with.
///
/// The per-issue listing costs a request, so it is only asked for when the
/// counts leave something unexplained. A change where everything worked is one
/// line, which is the whole point.
async fn finished(
    client: &crate::api::Client,
    change: &crate::api::BulkChange,
    session: &Session,
) -> ExitCode {
    emit(&render::bulk::change(change, &session.render));

    if change.succeeded() {
        return ExitCode::Success;
    }

    match client.bulk_change_issues(&change.id).await {
        Ok(outcomes) => emit(&render::bulk::failures(&outcomes, &session.render)),
        Err(error) => {
            // The change itself was reported; failing to explain it further is
            // not a reason to lose the part that was already answered.
            let mut err = anstream::stderr();
            let _ = writeln!(err, "could not read which issues failed: {error}");
        }
    }
    ExitCode::ApiRejected
}

async fn comment(target: &str, raw: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    let text_body = match body_text(raw) {
        Ok(text) => text,
        Err(code) => return code,
    };

    let body = serde_json::json!({ "text": text_body });
    let targets = [key.to_owned()];
    let intent = Intent {
        action: &format!("comment on {key}"),
        targets: &targets,
        body: &body,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.add_comment(key, &text_body).await {
        Ok(comment) => {
            emit(&format!("{key} comment {}\n", comment.id));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}
/// Resolve every target, and say whether one request could cover them all.
///
/// A bulk change is one request to one organisation, so a list that straddles
/// two profiles has to go the slow way round whatever the endpoint offers.
async fn resolve_all(
    targets: &[String],
    session: &Session,
) -> Result<(Vec<(crate::api::Client, String)>, bool), ExitCode> {
    let mut resolved = Vec::with_capacity(targets.len());
    for target in targets {
        match session.client_for(target).await {
            Ok(pair) => resolved.push(pair),
            Err(code) => return Err(code),
        }
    }

    let one_org = resolved.first().is_some_and(|(first, _)| {
        resolved
            .iter()
            .all(|(client, _)| client.org() == first.org())
    });
    Ok((resolved, one_org))
}

/// With no transition named, list what is available.
///
/// That is the common case for a caller who does not know the workflow, and it
/// is a read: listing must not require the write gate.
async fn transitions_of(target: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    match client.transitions(&key).await {
        Ok(transitions) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::transitions(&key, &transitions)),
                Format::JsonRaw => machine(&transitions, Format::Json),
                other => machine(&transitions, other),
            };
            finish(rendered)
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Move issues through one workflow transition.
///
/// Unlike a move, this is reversible in kind — a workflow that got somewhere
/// can usually get back — so a list of keys is gated the ordinary way: `--yes`
/// for more than one, nothing for a single issue.
async fn transition_cmd(
    targets: &[String],
    to: Option<&str>,
    resolution: Option<&str>,
    set: &[String],
    no_wait: bool,
    session: &Session,
) -> ExitCode {
    // One command, two shapes. `transition PROJ-1 close` reads naturally and is
    // what the documentation has always shown; a list of keys leaves no
    // unambiguous place for the transition, so it is named with --to.
    let (targets, transition) = match (to, targets) {
        (Some(to), keys) => (keys, Some(to.to_owned())),
        (None, [key]) => (std::slice::from_ref(key), None),
        (None, [key, id]) => (std::slice::from_ref(key), Some(id.clone())),
        (None, _) => {
            return report(
                &"naming several issues needs --to TRANSITION: ytcli issue transition A-1 A-2 --to close --yes",
                ExitCode::ConfirmationRequired,
            );
        }
    };

    let Some(transition) = transition else {
        return match targets.first() {
            Some(target) => transitions_of(target, session).await,
            None => ExitCode::Success,
        };
    };

    // A transition can require fields, and a workflow that closes an issue
    // almost always requires a resolution. Without these the command could not
    // reach half the statuses in an ordinary queue.
    let mut fields = serde_json::Map::new();
    if let Some(resolution) = resolution {
        fields.insert("resolution".to_owned(), serde_json::json!(resolution));
    }
    for assignment in set {
        match parse_assignment(assignment) {
            Ok((field, value)) => {
                fields.insert(field, value);
            }
            Err(error) => return report(&error, ExitCode::ConfirmationRequired),
        }
    }
    let body = serde_json::Value::Object(fields);

    let (resolved, one_org) = match resolve_all(targets, session).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let keys: Vec<String> = resolved.iter().map(|(_, key)| key.clone()).collect();
    let bulk = keys.len() > 1 && one_org;

    // What --dry-run prints has to be what would actually be sent, and the two
    // paths do not send the same shape.
    let request = if bulk {
        serde_json::json!({ "issues": keys, "transition": transition, "values": body })
    } else {
        body.clone()
    };
    let intent = Intent {
        action: &format!("move {} through `{transition}`", keys.join(", ")),
        targets: &keys,
        body: &request,
        always_confirm: false,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    if bulk {
        let Some((client, _)) = resolved.first() else {
            return ExitCode::Success;
        };
        return match client.bulk_transition(&keys, &transition, &body).await {
            Ok(started) => awaited(client, started, no_wait, session).await,
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        };
    }

    let mut done = 0_u64;
    for (client, key) in &resolved {
        match client.execute_transition(key, &transition, &body).await {
            Ok(()) => {
                done += 1;
                emit(&format!("{key} {transition}\n"));
            }
            Err(error) => {
                let code = error.exit_code();
                if keys.len() > 1 {
                    emit(&render::bulk::changed(
                        done,
                        keys.len() as u64,
                        &session.render,
                    ));
                }
                return rejected_for_fields(&error, &body, code);
            }
        }
    }

    if keys.len() > 1 {
        emit(&render::bulk::changed(
            done,
            keys.len() as u64,
            &session.render,
        ));
    }
    ExitCode::Success
}

/// Report a refused transition, and say how to supply what it wanted.
///
/// Tracker names the fields it wanted, in the organisation's own language and by
/// their display names — which are not what `--set` takes. Its sentence is
/// passed through as written, and what is added after it is the part it cannot
/// know: how to supply them here.
fn rejected_for_fields(
    error: &crate::api::error::ApiError,
    body: &serde_json::Value,
    code: ExitCode,
) -> ExitCode {
    let wants_fields = body.as_object().is_some_and(serde_json::Map::is_empty)
        && matches!(error, crate::api::error::ApiError::Rejected { .. });
    let outcome = report(error, code);

    if wants_fields {
        let mut err = anstream::stderr();
        let _ = writeln!(
            err,
            "this transition wants fields: pass them with --resolution or --set key=value \
             (`ytcli dict list --kind resolutions` names the resolutions)"
        );
    }
    outcome
}

/// Send issues to another queue.
///
/// Gated with `always_confirm` rather than the ordinary write gate: the key
/// changes, every reference to the old one is left pointing at a redirect, and
/// no request puts it back. That is irreversible in kind, like claiming a queue
/// key, not merely at scale — so a single issue asks for `--yes` too, and a list
/// of them asks once for all of them after printing every key it will change.
async fn move_issues(
    targets: &[String],
    queue: &str,
    keep_fields: bool,
    initial_status: bool,
    no_wait: bool,
    session: &Session,
) -> ExitCode {
    let (resolved, one_org) = match resolve_all(targets, session).await {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let keys: Vec<String> = resolved.iter().map(|(_, key)| key.clone()).collect();
    let bulk = keys.len() > 1 && one_org;

    let mut request = serde_json::json!({
        "queue": queue,
        "moveAllFields": keep_fields,
        "initialStatus": initial_status,
    });
    if bulk && let Some(object) = request.as_object_mut() {
        object.insert("issues".to_owned(), serde_json::json!(keys));
    }
    let intent = Intent {
        action: &format!("move {} to {queue}, changing the key", keys.join(", ")),
        targets: &keys,
        body: &request,
        always_confirm: true,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    if bulk {
        let Some((client, _)) = resolved.first() else {
            return ExitCode::Success;
        };
        return match client
            .bulk_move(&keys, queue, keep_fields, initial_status)
            .await
        {
            Ok(started) => awaited(client, started, no_wait, session).await,
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        };
    }

    let mut done = 0_u64;
    for (client, key) in &resolved {
        match client
            .move_issue(key, queue, keep_fields, initial_status)
            .await
        {
            Ok(issue) => {
                done += 1;
                // The new key is the whole result: nothing else the caller holds
                // still addresses this issue.
                emit(&format!("{key} → {}\n", issue.key));
            }
            Err(error) => {
                let code = error.exit_code();
                if keys.len() > 1 {
                    emit(&render::bulk::changed(
                        done,
                        keys.len() as u64,
                        &session.render,
                    ));
                }
                return report(&error, code);
            }
        }
    }

    if keys.len() > 1 {
        emit(&render::bulk::changed(
            done,
            keys.len() as u64,
            &session.render,
        ));
    }
    ExitCode::Success
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::Attachment;

    fn attachment(id: &str, name: &str) -> Attachment {
        Attachment {
            id: id.to_owned(),
            name: name.to_owned(),
            size: None,
            mimetype: Some("image/png".to_owned()),
            author: None,
            created_at: None,
            content: Some("https://api.tracker.yandex.net/x".to_owned()),
        }
    }

    /// The form Tracker actually writes into a description.
    #[test]
    fn an_attachment_url_resolves_by_its_last_path_segment() {
        let attachments = [attachment("29", "screenshot.png")];

        assert_eq!(
            attachment_for(&attachments, "/ajax/v2/attachments/29?inline=true").map(|a| &a.id),
            Some(&"29".to_owned())
        );
        assert_eq!(
            attachment_for(&attachments, "/ajax/v2/attachments/29/").map(|a| &a.id),
            Some(&"29".to_owned())
        );
        // A description written by hand names the file instead.
        assert_eq!(
            attachment_for(&attachments, "screenshot.png").map(|a| &a.id),
            Some(&"29".to_owned())
        );
    }

    /// A description can name any URL its author likes. Following one would
    /// turn reading an issue into fetching whatever that author chose — with
    /// this client's own credentials attached. Only files already on the issue
    /// are drawn; everything else stays the markdown it was.
    #[test]
    fn a_url_that_is_not_an_attachment_of_this_issue_is_not_followed() {
        let attachments = [attachment("29", "screenshot.png")];

        assert!(attachment_for(&attachments, "https://example.com/evil.png").is_none());
        assert!(attachment_for(&attachments, "/ajax/v2/attachments/30").is_none());
        assert!(attachment_for(&attachments, "http://169.254.169.254/latest/meta-data").is_none());
    }
}
