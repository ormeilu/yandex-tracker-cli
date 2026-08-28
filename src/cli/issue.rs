//! Issue commands.
//!
//! `find` takes both shapes on purpose: flags for the queries people actually
//! run, and `--yql` for everything else. YQL is a read-only search language —
//! the escape hatch widens what can be *read*, never what can be changed
//! (`docs/adr/0001-security-model.md`).

use clap::{Args, Subcommand};

use crate::api::query::Filter;
use crate::cli::write::{Gate, Intent, check, parse_assignment};
use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, image, machine, text};

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// Show one issue: summary, fields, links, first lines of the description.
    #[command(long_about = crate::cli::help::ISSUE_GET)]
    Get {
        /// Issue key, e.g. PROJ-42. Prefix it with a profile — `work/PROJ-42` —
        /// when two profiles can both see a queue with that key.
        key: String,
        /// Comma-separated field list; accepts custom field keys.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
    },
    /// Search for issues.
    #[command(long_about = crate::cli::help::ISSUE_FIND)]
    Find(FindArgs),
    /// Count matching issues without fetching them. The cheapest question here.
    #[command(long_about = crate::cli::help::ISSUE_COUNT)]
    Count(FindArgs),
    /// Show the links of an issue.
    #[command(long_about = crate::cli::help::ISSUE_LINKS)]
    Links { key: String },
    /// Show the comments of an issue.
    #[command(long_about = crate::cli::help::ISSUE_COMMENTS)]
    Comments { key: String },
    /// Create an issue.
    #[command(long_about = crate::cli::help::ISSUE_CREATE)]
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
    #[command(long_about = crate::cli::help::ISSUE_UPDATE)]
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
    },
    /// Add a comment.
    #[command(long_about = crate::cli::help::ISSUE_COMMENT)]
    Comment {
        key: String,
        /// Comment body; `-` reads from stdin.
        text: String,
    },
    /// Show the worklog of an issue.
    #[command(long_about = crate::cli::help::ISSUE_WORKLOGS)]
    Worklogs { key: String },
    /// Show the checklist of an issue.
    #[command(long_about = crate::cli::help::ISSUE_CHECKLIST)]
    Checklist { key: String },
    /// Record or remove time spent. Every verb here writes.
    #[command(subcommand, long_about = crate::cli::help::ISSUE_WORKLOG)]
    Worklog(WorklogCommand),
    /// Change an issue's checklist. Every verb here writes.
    #[command(subcommand, long_about = crate::cli::help::ISSUE_CHECK)]
    Check(CheckCommand),
    /// Link or unlink issues. Every verb here writes.
    #[command(subcommand, long_about = crate::cli::help::ISSUE_LINK)]
    Link(LinkCommand),
    /// Move an issue through a workflow transition.
    #[command(long_about = crate::cli::help::ISSUE_TRANSITION)]
    Transition {
        key: String,
        /// Transition id; omit to list what is available.
        transition: Option<String>,
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
    #[command(long_about = crate::cli::help::WORKLOG_ADD)]
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
    /// Remove one worklog entry.
    #[command(long_about = crate::cli::help::WORKLOG_DELETE)]
    Delete { key: String, id: String },
}

/// Writing to a checklist. Reading it is `issue checklist`.
#[derive(Debug, Subcommand)]
pub enum CheckCommand {
    /// Add a line to the checklist.
    #[command(long_about = crate::cli::help::CHECK_ADD)]
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
    #[command(long_about = crate::cli::help::CHECK_TICK)]
    Tick { key: String, id: String },
    /// Put a ticked line back.
    #[command(long_about = crate::cli::help::CHECK_UNTICK)]
    Untick { key: String, id: String },
    /// Remove a line.
    #[command(long_about = crate::cli::help::CHECK_DELETE)]
    Delete { key: String, id: String },
}

/// Writing links. Reading them is `issue links`.
#[derive(Debug, Subcommand)]
pub enum LinkCommand {
    /// Link two issues.
    #[command(long_about = crate::cli::help::LINK_ADD)]
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
    #[command(long_about = crate::cli::help::LINK_DELETE)]
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
        IssueCommand::Comments { key } => comments(key, session).await,
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
        } => update(keys, summary.as_deref(), assignee.as_deref(), set, session).await,
        IssueCommand::Comment { key, text } => comment(key, text, session).await,
        IssueCommand::Transition { key, transition } => {
            transition_cmd(key, transition.as_deref(), session).await
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
    let (client, key) = match session.client_for(target) {
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
    let (client, key) = match session.client_for(target) {
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
            let (client, key) = match session.client_for(key) {
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
        WorklogCommand::Delete { key, id } => {
            delete_with_gate(key, id, session, "worklog", |client, key, id| {
                Box::pin(async move { client.delete_worklog(key, id).await })
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
            let (client, key) = match session.client_for(key) {
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
    let (client, key) = match session.client_for(target) {
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

async fn link_write(command: &LinkCommand, session: &Session) -> ExitCode {
    match command {
        LinkCommand::Add {
            key,
            relation,
            other,
        } => {
            let (client, key) = match session.client_for(key) {
                Ok(pair) => pair,
                Err(code) => return code,
            };

            let body = serde_json::json!({ "relationship": relation, "issue": other });
            let targets = [key.clone()];
            let intent = Intent {
                action: &format!("link {key} {relation} {other}"),
                targets: &targets,
                body: &body,
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
    let (client, key) = match session.client_for(target) {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    let body = serde_json::json!({ "delete": id });
    let targets = [key.clone()];
    let intent = Intent {
        action: &format!("delete {what} {id} of {key}"),
        targets: &targets,
        body: &body,
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
    let (client, key) = match session.client_for(target) {
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
    let (client, key) = match session.client_for(target) {
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

async fn comments(target: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target) {
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
    session: &Session,
) -> ExitCode {
    let mut resolved = Vec::with_capacity(targets.len());
    for target in targets {
        match session.client_for(target) {
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
    let intent = Intent {
        action: &format!("update {}", keys.join(", ")),
        targets: &keys,
        body: &body,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    // One at a time, and stopping at the first failure. Tracker has no bulk
    // endpoint and rate-limits, and a run that carried on would leave the caller
    // to work out which issues it got to before it stopped.
    for (client, key) in &resolved {
        match client.update_issue(key, &body).await {
            Ok(issue) => emit(&text::issue_selected(
                &issue,
                &["status".to_owned(), "assignee".to_owned()],
            )),
            Err(error) => {
                let code = error.exit_code();
                return report(&error, code);
            }
        }
    }

    ExitCode::Success
}

async fn comment(target: &str, raw: &str, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target) {
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

/// With no transition named, list what is available.
///
/// That is the common case for a caller who does not know the workflow, and it
/// is a read: listing must not require the write gate.
async fn transition_cmd(target: &str, transition: Option<&str>, session: &Session) -> ExitCode {
    let (client, key) = match session.client_for(target) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let key = key.as_str();

    let Some(transition) = transition else {
        return match client.transitions(key).await {
            Ok(transitions) => {
                let rendered = match session.render.format {
                    Format::Text => Ok(text::transitions(key, &transitions)),
                    Format::JsonRaw => machine(&transitions, Format::Json),
                    other => machine(&transitions, other),
                };
                finish(rendered)
            }
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        };
    };

    let body = serde_json::json!({});
    let targets = [key.to_owned()];
    let intent = Intent {
        action: &format!("move {key} through `{transition}`"),
        targets: &targets,
        body: &body,
    };
    if let Gate::Stop(code) = check(&intent, session) {
        return code;
    }

    match client.execute_transition(key, transition, &body).await {
        Ok(()) => {
            emit(&format!("{key} {transition}\n"));
            ExitCode::Success
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
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
