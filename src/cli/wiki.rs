//! Wiki commands: the pages of the Yandex Wiki next to the organisation's
//! Tracker, read through the same profile.
//!
//! The read verbs never write (`docs/adr/0007-yandex-wiki.md`). The writes —
//! create, update, append, delete, restore — are verbs of their own and pass
//! the same gate as Tracker's: profile and organisation announced first,
//! `--dry-run` honoured, page text from a file or stdin.

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::api::wiki::{CommentScope, GridQuery, LAST_SEARCH_PAGE, slug_of};
use crate::cli::attachment::safe_filename;
use crate::cli::write::{Gate, Intent, check};
use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, RenderError, machine, wiki as render};

#[derive(Debug, Subcommand)]
pub enum WikiCommand {
    /// Show one page: its title and last change, then its text.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_GET))]
    Get {
        /// The page's slug, or its address as copied from the browser.
        page: String,
    },
    /// List the pages under one.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_LIST))]
    List {
        /// The page's slug, or its address.
        page: String,
        /// Where the previous page of this listing ended, as its tally named it.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Search pages and files.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_FIND))]
    Find {
        /// Words to look for.
        text: String,
        /// Only pages, or only attached files.
        #[arg(long = "type", value_parser = ["page", "file"])]
        kind: Option<String>,
        /// Which page of hits, from 1; the Wiki's search stops at 500.
        #[arg(long, default_value_t = 1)]
        page: u32,
    },
    /// Show a page's comments.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_COMMENTS))]
    Comments {
        /// The page's slug, or its address.
        page: String,
        /// Every post in one comment's thread, by the comment's id.
        #[arg(long, conflicts_with = "status")]
        thread: Option<u64>,
        /// Only resolved, or only unresolved, comments.
        #[arg(long, value_parser = ["resolved", "unresolved"])]
        status: Option<String>,
        /// Where the previous page of this listing ended, as its tally named it.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// List a page's attachments.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_ATTACHMENTS))]
    Attachments {
        /// The page's slug, or its address.
        page: String,
        /// Where the previous page of this listing ended, as its tally named it.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// List the grids — dynamic tables — on a page.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_GRIDS))]
    Grids {
        /// The page's slug, or its address.
        page: String,
        /// Where the previous page of this listing ended, as its tally named it.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Show one grid: its columns, then its rows.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_GRID))]
    Grid {
        /// The grid's id, as `wiki grids` lists it.
        grid: String,
        /// Only matching rows, in the Wiki's syntax: `[slug] ~ text AND [n] < 3`.
        #[arg(long)]
        filter: Option<String>,
        /// Order of the rows: `slug, -other`.
        // A descending sort starts with `-`, which is the Wiki's syntax, not a
        // flag of ours.
        #[arg(long, allow_hyphen_values = true)]
        sort: Option<String>,
        /// Only these columns, by slug, comma-separated.
        #[arg(long)]
        columns: Option<String>,
        /// Only these rows, by id, comma-separated.
        #[arg(long)]
        rows: Option<String>,
        /// The grid as it was at this revision.
        #[arg(long)]
        revision: Option<u64>,
    },
    /// List what a page holds: files and grids together.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_RESOURCES))]
    Resources {
        /// The page's slug, or its address.
        page: String,
        /// Only files, or only grids.
        #[arg(long = "type", value_parser = ["attachment", "grid"])]
        kind: Option<String>,
        /// Only those whose name or title matches.
        #[arg(long)]
        query: Option<String>,
        /// Where the previous page of this listing ended, as its tally named it.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Create a page; its parent is whatever the slug's path says.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_CREATE))]
    Create {
        /// Where the page goes: users/me/notes, or an address.
        page: String,
        #[arg(long, short = 't')]
        title: String,
        /// The page's text: a file, or `-` for stdin.
        #[arg(long, value_name = "PATH")]
        from: Option<String>,
        /// Do not notify subscribers.
        #[arg(long)]
        silent: bool,
    },
    /// Replace a page's text, or retitle it.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_UPDATE))]
    Update {
        /// The page's slug, or its address.
        page: String,
        #[arg(long, short = 't')]
        title: Option<String>,
        /// The page's new text in full: a file, or `-` for stdin.
        #[arg(long, value_name = "PATH")]
        from: Option<String>,
        /// Fold in edits made since, rather than be refused over them.
        #[arg(long)]
        merge: bool,
        /// Do not notify subscribers.
        #[arg(long)]
        silent: bool,
    },
    /// Add text to a page: at the bottom, the top, or an anchor.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_APPEND))]
    Append {
        /// The page's slug, or its address.
        page: String,
        /// The text to add: a file, or `-` for stdin.
        #[arg(long, value_name = "PATH")]
        from: String,
        /// At the top rather than the bottom.
        #[arg(long, conflicts_with = "anchor")]
        top: bool,
        /// At this anchor in the page, such as #deploy.
        #[arg(long)]
        anchor: Option<String>,
        /// Do not notify subscribers.
        #[arg(long)]
        silent: bool,
    },
    /// Delete a page, printing the token that restores it.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_DELETE))]
    Delete {
        /// The page's slug, or its address.
        page: String,
        /// Its subpages too. Needs --yes.
        #[arg(long)]
        recursive: bool,
    },
    /// Restore a deleted page by the token its deletion printed.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_RESTORE))]
    Restore { token: String },
    /// Comment on a page, or reply to a comment.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_COMMENT))]
    Comment {
        /// The page's slug, or its address.
        page: String,
        /// The comment; `-` reads it from stdin.
        text: String,
        /// Reply to this comment, by its id.
        #[arg(long, value_name = "ID")]
        reply_to: Option<u64>,
        /// The passage of the page the comment is about.
        #[arg(long)]
        quote: Option<String>,
    },
    /// Delete a comment. There is no undo, so it needs --yes.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_DELETE_COMMENT))]
    DeleteComment {
        /// The page's slug, or its address.
        page: String,
        /// The comment's id, as `wiki comments` shows it.
        comment: u64,
    },
    /// Show who can read and edit a page.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_ACCESS))]
    Access {
        /// The page's slug, or its address.
        page: String,
    },
    /// Give a user or a group a role on a page.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_GRANT))]
    #[command(group(clap::ArgGroup::new("who").required(true).multiple(false)))]
    Grant {
        /// The page's slug, or its address.
        page: String,
        /// `reader`, `editor`, `extra_editor` (may also manage access) or `author`.
        #[arg(long, value_parser = ["reader", "editor", "extra_editor", "author"])]
        role: String,
        /// A login; its uid is looked up in Tracker.
        #[arg(long, group = "who")]
        user: Option<String>,
        /// A user's uid, as the organisation's directory has it.
        #[arg(long, group = "who")]
        uid: Option<String>,
        /// A user's Yandex Cloud id.
        #[arg(long, group = "who", value_name = "ID")]
        cloud_uid: Option<String>,
        /// A group, as SOURCE:ID; the source is dir, cloud, com or staff.
        #[arg(long, group = "who", value_name = "SOURCE:ID")]
        group: Option<String>,
        /// Keep the role off the subpages.
        #[arg(long)]
        no_inherit: bool,
        /// Allow a change that could lock you yourself out of the page.
        #[arg(long)]
        allow_selflock: bool,
    },
    /// Change a grant's role, or whether subpages inherit it.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_REGRANT))]
    Regrant {
        /// The page's slug, or its address.
        page: String,
        /// The grant's id, as `wiki access` lists it.
        access: String,
        #[arg(long, value_parser = ["reader", "editor", "extra_editor", "author"])]
        role: Option<String>,
        #[arg(long, value_parser = ["inherited", "not_inherited"])]
        inheritance: Option<String>,
        /// Allow a change that could lock you yourself out of the page.
        #[arg(long)]
        allow_selflock: bool,
    },
    /// Remove a grant, or every personal grant on a page.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_REVOKE))]
    Revoke {
        /// The page's slug, or its address.
        page: String,
        /// The grant's id, as `wiki access` lists it.
        #[arg(required_unless_present = "all")]
        access: Option<String>,
        /// Every personal grant on the page. Needs --yes.
        #[arg(long, conflicts_with = "access")]
        all: bool,
        /// Allow a change that could lock you yourself out of the page.
        #[arg(long)]
        allow_selflock: bool,
    },
    /// Copy a page to a new address, and wait for the copy.
    #[command(name = "clone", long_about = crate::cli::help::md(crate::cli::help::WIKI_CLONE))]
    ClonePage {
        /// The page's slug, or its address.
        page: String,
        /// Where the copy goes; a page there already is a refusal.
        target: String,
        /// The copy's title, when it should not keep the original's.
        #[arg(long, short = 't')]
        title: Option<String>,
        /// Subscribe to the copy.
        #[arg(long)]
        subscribe: bool,
        /// Print the operation and return, rather than wait for the copy.
        #[arg(long)]
        no_wait: bool,
    },
    /// Copy a grid onto a page, and wait for the copy.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_CLONE_GRID))]
    CloneGrid {
        /// The grid's id, as `wiki grids` lists it.
        grid: String,
        /// The page the copy goes on; created if it is not there.
        target: String,
        #[arg(long, short = 't')]
        title: Option<String>,
        /// Copy the rows too, not only the columns.
        #[arg(long)]
        with_data: bool,
        /// Print the operation and return, rather than wait for the copy.
        #[arg(long)]
        no_wait: bool,
    },
    /// Show where a clone has got to.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_OPERATION))]
    Operation {
        /// `clone` for a page, `clone_inline_grid` for a grid.
        #[arg(value_parser = ["clone", "clone_inline_grid"])]
        kind: String,
        id: String,
    },
    /// Download one file attached to a page.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_DOWNLOAD))]
    Download {
        /// The page's slug or address — or the file's own, `<slug>/.files/<name>`.
        page: String,
        /// The file's id or name, as `wiki attachments` lists them; not needed
        /// when the first argument is the file's address.
        file: Option<String>,
        /// Directory to write into.
        #[arg(long, short = 'o')]
        out: PathBuf,
        /// Overwrite a file that is already there.
        #[arg(long)]
        force: bool,
    },
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per verb; splitting the dispatch would only hide the list"
)]
pub async fn run(command: &WikiCommand, session: &Session) -> ExitCode {
    match command {
        WikiCommand::Get { page } => get(page, session).await,
        WikiCommand::List { page, cursor } => list(page, cursor.as_deref(), session).await,
        WikiCommand::Find { text, kind, page } => find(text, kind.as_deref(), *page, session).await,
        WikiCommand::Comments {
            page,
            thread,
            status,
            cursor,
        } => {
            let scope = match thread {
                Some(comment) => CommentScope::Thread(*comment),
                None => CommentScope::Page {
                    status: status.as_deref(),
                },
            };
            comments(page, scope, cursor.as_deref(), session).await
        }
        WikiCommand::Attachments { page, cursor } => {
            attachments(page, cursor.as_deref(), session).await
        }
        WikiCommand::Grids { page, cursor } => grids(page, cursor.as_deref(), session).await,
        WikiCommand::Grid {
            grid: id,
            filter,
            sort,
            columns,
            rows,
            revision,
        } => {
            let query = GridQuery {
                filter: filter.as_deref(),
                sort: sort.as_deref(),
                columns: columns.as_deref(),
                rows: rows.as_deref(),
                revision: *revision,
            };
            grid(id, query, session).await
        }
        WikiCommand::Resources {
            page,
            kind,
            query,
            cursor,
        } => {
            resources(
                page,
                kind.as_deref(),
                query.as_deref(),
                cursor.as_deref(),
                session,
            )
            .await
        }
        WikiCommand::Create {
            page,
            title,
            from,
            silent,
        } => create(page, title, from.as_deref(), *silent, session).await,
        WikiCommand::Update {
            page,
            title,
            from,
            merge,
            silent,
        } => {
            let change = Change {
                title: title.as_deref(),
                from: from.as_deref(),
                merge: *merge,
                silent: *silent,
            };
            update(page, change, session).await
        }
        WikiCommand::Append {
            page,
            from,
            top,
            anchor,
            silent,
        } => append(page, from, place(*top, anchor.as_deref()), *silent, session).await,
        WikiCommand::Delete { page, recursive } => delete(page, *recursive, session).await,
        WikiCommand::Restore { token } => restore(token, session).await,
        WikiCommand::Comment {
            page,
            text,
            reply_to,
            quote,
        } => comment(page, text, *reply_to, quote.as_deref(), session).await,
        WikiCommand::DeleteComment { page, comment } => {
            delete_comment(page, *comment, session).await
        }
        WikiCommand::Access { page } => show_access(page, session).await,
        WikiCommand::Grant {
            page,
            role,
            user,
            uid,
            cloud_uid,
            group,
            no_inherit,
            allow_selflock,
        } => {
            let who = match (user, uid, cloud_uid, group) {
                (Some(login), ..) => Grantee::Login(login),
                (_, Some(uid), ..) => Grantee::Uid(uid),
                (_, _, Some(id), _) => Grantee::CloudUid(id),
                (.., Some(group)) => Grantee::Group(group),
                _ => {
                    return report(
                        &"name who: --user, --uid, --cloud-uid or --group",
                        ExitCode::ConfirmationRequired,
                    );
                }
            };
            grant(page, role, who, *no_inherit, *allow_selflock, session).await
        }
        WikiCommand::Regrant {
            page,
            access,
            role,
            inheritance,
            allow_selflock,
        } => {
            let body = match (role, inheritance) {
                (None, None) => {
                    return report(
                        &"nothing to change: pass --role, --inheritance, or both",
                        ExitCode::ConfirmationRequired,
                    );
                }
                (role, inheritance) => {
                    let mut body = serde_json::json!({});
                    if let Some(role) = role {
                        body["role"] = serde_json::Value::String(role.clone());
                    }
                    if let Some(inheritance) = inheritance {
                        body["inheritance"] = serde_json::Value::String(inheritance.clone());
                    }
                    body
                }
            };
            regrant(page, access, &body, *allow_selflock, session).await
        }
        WikiCommand::Revoke {
            page,
            access,
            all: _,
            allow_selflock,
        } => revoke(page, access.as_deref(), *allow_selflock, session).await,
        WikiCommand::ClonePage {
            page,
            target,
            title,
            subscribe,
            no_wait,
        } => {
            let mut body = serde_json::json!({});
            if let Some(title) = title {
                body["title"] = serde_json::Value::String(title.clone());
            }
            if *subscribe {
                body["subscribe_me"] = serde_json::Value::Bool(true);
            }
            clone_page(page, target, body, *no_wait, session).await
        }
        WikiCommand::CloneGrid {
            grid,
            target,
            title,
            with_data,
            no_wait,
        } => {
            let mut body = serde_json::json!({});
            if let Some(title) = title {
                body["title"] = serde_json::Value::String(title.clone());
            }
            if *with_data {
                body["with_data"] = serde_json::Value::Bool(true);
            }
            clone_grid(grid, target, body, *no_wait, session).await
        }
        WikiCommand::Operation { kind, id } => {
            let operation = crate::api::wiki::WikiOperation {
                id: id.clone(),
                kind: kind.clone(),
            };
            show_operation(&operation, session).await
        }
        WikiCommand::Download {
            page,
            file,
            out,
            force,
        } => download(page, file.as_deref(), out, *force, session).await,
    }
}

/// Show one page.
async fn get(page: &str, session: &Session) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.wiki_page(&slug).await {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::page(&found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// The pages under one, a page of them at a time.
async fn list(page: &str, cursor: Option<&str>, session: &Session) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client
        .wiki_descendants(&slug, cursor, page_size(session))
        .await
    {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::pages(&found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// Search pages and files.
async fn find(text: &str, kind: Option<&str>, page: u32, session: &Session) -> ExitCode {
    if !(1..=LAST_SEARCH_PAGE).contains(&page) {
        return report(
            &format!("--page runs from 1 to {LAST_SEARCH_PAGE}: the Wiki's search stops there"),
            ExitCode::ConfirmationRequired,
        );
    }
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    // The profile's list length, within the 1..50 search accepts.
    let limit = u32::try_from(session.display().limit.clamp(1, 50)).unwrap_or(50);

    match client.wiki_search(text, kind, page, limit).await {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::hits(&found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// A page's comments, or one thread of them.
async fn comments(
    page: &str,
    scope: CommentScope<'_>,
    cursor: Option<&str>,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client
        .wiki_comments(&slug, scope, cursor, page_size(session))
        .await
    {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::comments(&slug, &found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// The files attached to a page.
async fn attachments(page: &str, cursor: Option<&str>, session: &Session) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client
        .wiki_attachments(&slug, cursor, page_size(session))
        .await
    {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::attachments(&found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// The grids on a page.
async fn grids(page: &str, cursor: Option<&str>, session: &Session) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.wiki_grids(&slug, cursor, page_size(session)).await {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::grids(&found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// One grid.
async fn grid(id: &str, query: GridQuery<'_>, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.wiki_grid(id.trim(), query).await {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::grid(&found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// What a page holds.
async fn resources(
    page: &str,
    kind: Option<&str>,
    query: Option<&str>,
    cursor: Option<&str>,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client
        .wiki_resources(&slug, kind, query, cursor, page_size(session))
        .await
    {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::resources(&found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}

/// A page's text for a write: from a file, or `-` for stdin — never from an
/// argument, where a runbook would have to survive shell quoting.
fn content(from: &str) -> Result<String, ExitCode> {
    if from == "-" {
        let mut text = String::new();
        return match std::io::Read::read_to_string(&mut std::io::stdin(), &mut text) {
            Ok(_) => Ok(text),
            Err(error) => Err(report(&error, ExitCode::Failure)),
        };
    }
    std::fs::read_to_string(from)
        .map_err(|error| report(&format!("cannot read {from}: {error}"), ExitCode::Failure))
}

/// Announce the write and apply `--dry-run` and `--yes`, before any request:
/// a dry run of a write under a page does not even look the page up.
fn gated(
    action: &str,
    body: &serde_json::Value,
    confirm: bool,
    session: &Session,
) -> Option<ExitCode> {
    let intent = Intent {
        action,
        targets: &[],
        body,
        always_confirm: confirm,
    };
    match check(&intent, session) {
        Gate::Proceed => None,
        Gate::Stop(code) => Some(code),
    }
}

fn failed(error: &crate::api::error::ApiError) -> ExitCode {
    report(error, error.exit_code())
}

/// Create a page.
async fn create(
    page: &str,
    title: &str,
    from: Option<&str>,
    silent: bool,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let mut body = serde_json::json!({ "slug": slug, "title": title });
    if let Some(from) = from {
        match content(from) {
            Ok(text) => body["content"] = serde_json::Value::String(text),
            Err(code) => return code,
        }
    }
    if let Some(code) = gated(&format!("create wiki page `{slug}`"), &body, false, session) {
        return code;
    }

    match client.wiki_create(&body, silent).await {
        Ok(made) => {
            emit(&format!("created {} (id {})\n", made.slug, made.id));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// What `wiki update` was asked to change.
struct Change<'a> {
    title: Option<&'a str>,
    from: Option<&'a str>,
    merge: bool,
    silent: bool,
}

/// Replace a page's text, retitle it, or both.
async fn update(page: &str, change: Change<'_>, session: &Session) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    if change.title.is_none() && change.from.is_none() {
        return report(
            &"nothing to change: pass --title, --from, or both",
            ExitCode::ConfirmationRequired,
        );
    }
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let mut body = serde_json::json!({});
    if let Some(title) = change.title {
        body["title"] = serde_json::Value::String(title.to_owned());
    }
    if let Some(from) = change.from {
        match content(from) {
            Ok(text) => body["content"] = serde_json::Value::String(text),
            Err(code) => return code,
        }
    }
    if let Some(code) = gated(&format!("update wiki page `{slug}`"), &body, false, session) {
        return code;
    }

    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    match client
        .wiki_update(id, &body, change.merge, change.silent)
        .await
    {
        Ok(page) => {
            emit(&format!("updated {} (id {})\n", page.slug, page.id));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// Where appended text goes, as the Wiki names it.
fn place(top: bool, anchor: Option<&str>) -> serde_json::Value {
    match anchor {
        Some(anchor) => serde_json::json!({ "anchor": { "name": anchor } }),
        None => serde_json::json!({
            "body": { "location": if top { "top" } else { "bottom" } }
        }),
    }
}

/// Add text to a page.
async fn append(
    page: &str,
    from: &str,
    place: serde_json::Value,
    silent: bool,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let text = match content(from) {
        Ok(text) => text,
        Err(code) => return code,
    };
    if text.is_empty() {
        return report(
            &"nothing to append: the text is empty",
            ExitCode::ConfirmationRequired,
        );
    }
    let mut body = place;
    body["content"] = serde_json::Value::String(text);
    if let Some(code) = gated(
        &format!("append to wiki page `{slug}`"),
        &body,
        false,
        session,
    ) {
        return code;
    }

    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    match client.wiki_append(id, &body, silent).await {
        Ok(page) => {
            emit(&format!("appended to {} (id {})\n", page.slug, page.id));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// Delete a page, and print the only thing that can bring it back.
///
/// The recovery token is shown once, by this command, and by nothing else
/// ever again; so it goes to stdout with the exact command that uses it.
/// Taking the subpages too is a different size of mistake, and needs `--yes`.
async fn delete(page: &str, recursive: bool, session: &Session) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let body = serde_json::json!({ "recursive": recursive });
    let action = if recursive {
        format!("delete wiki page `{slug}` and every page under it")
    } else {
        format!("delete wiki page `{slug}`")
    };
    if let Some(code) = gated(&action, &body, recursive, session) {
        return code;
    }

    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    match client.wiki_delete(id, recursive).await {
        Ok(token) => {
            emit(&format!(
                "deleted {slug} (id {id})\n\
                 recovery token {token} — shown only now; to restore:\n  \
                 ytcli wiki restore {token}\n"
            ));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// Bring a deleted page back.
async fn restore(token: &str, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let body = serde_json::json!({});
    if let Some(code) = gated(
        &format!("restore the wiki page deleted under token {token}"),
        &body,
        false,
        session,
    ) {
        return code;
    }

    match client.wiki_restore(token.trim()).await {
        Ok(restored) => {
            let pages = restored
                .pages_count
                .map_or_else(String::new, |count| format!(", {count} pages"));
            emit(&format!(
                "restored {} (id {}{pages})\n",
                restored.slug, restored.id
            ));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// Comment on a page, or reply to one of its comments.
///
/// Like `issue comment`, the text is the argument, or stdin with `-`: a
/// comment is short more often than a page is.
async fn comment(
    page: &str,
    text: &str,
    reply_to: Option<u64>,
    quote: Option<&str>,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let said = if text == "-" {
        match content("-") {
            Ok(text) => text,
            Err(code) => return code,
        }
    } else {
        text.to_owned()
    };
    if said.trim().is_empty() {
        return report(
            &"nothing to say: the comment is empty",
            ExitCode::ConfirmationRequired,
        );
    }
    let mut body = serde_json::json!({ "body": said });
    if let Some(parent) = reply_to {
        body["parent_id"] = serde_json::json!(parent);
    }
    if let Some(quote) = quote {
        body["inline_text"] = serde_json::Value::String(quote.to_owned());
    }
    let action = match reply_to {
        Some(parent) => format!("reply to comment {parent} on wiki page `{slug}`"),
        None => format!("comment on wiki page `{slug}`"),
    };
    if let Some(code) = gated(&action, &body, false, session) {
        return code;
    }

    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    match client.wiki_comment(id, &body).await {
        Ok(made) => {
            emit(&format!("commented on {slug}: comment {}\n", made.id));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// Delete a comment. The Wiki keeps nothing to restore it from.
async fn delete_comment(page: &str, comment: u64, session: &Session) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let body = serde_json::json!({ "comment": comment });
    if let Some(code) = gated(
        &format!("delete comment {comment} on wiki page `{slug}`"),
        &body,
        true,
        session,
    ) {
        return code;
    }

    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    match client.wiki_delete_comment(id, comment).await {
        Ok(left) => {
            let left = left.map_or_else(String::new, |count| format!("; {count} left"));
            emit(&format!("deleted comment {comment} on {slug}{left}\n"));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// Who can read and edit a page.
async fn show_access(page: &str, session: &Session) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.wiki_access(&slug).await {
        Ok(found) => finish(match session.render.format {
            Format::Text => Ok(render::access(&found, &session.render)),
            Format::JsonRaw => machine(&found, Format::Json),
            other => machine(&found, other),
        }),
        Err(error) => failed(&error),
    }
}

/// Whom a grant is for, as it was named.
enum Grantee<'a> {
    /// A login, whose uid Tracker knows.
    Login(&'a str),
    Uid(&'a str),
    CloudUid(&'a str),
    /// `SOURCE:ID`.
    Group(&'a str),
}

/// Give a user or a group a role on a page.
///
/// The Wiki takes a uid, and people know logins; so a login is looked up in
/// Tracker, which is in the same organisation. That lookup is a request, and
/// it happens after the gate, so a dry run shows where the uid will go
/// rather than making it.
async fn grant(
    page: &str,
    role: &str,
    who: Grantee<'_>,
    no_inherit: bool,
    allow_selflock: bool,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let mut body = serde_json::json!({ "role": role });
    let named_as = match who {
        Grantee::Login(login) => {
            body["user"] = serde_json::json!({ "uid": format!("<uid of {login}, from Tracker>") });
            login.to_owned()
        }
        Grantee::Uid(uid) => {
            body["user"] = serde_json::json!({ "uid": uid });
            format!("uid {uid}")
        }
        Grantee::CloudUid(id) => {
            body["user"] = serde_json::json!({ "cloud_uid": id });
            format!("cloud uid {id}")
        }
        Grantee::Group(spec) => {
            let Some((source, id)) = spec.split_once(':').filter(|(source, id)| {
                matches!(*source, "dir" | "cloud" | "com" | "staff") && !id.is_empty()
            }) else {
                return report(
                    &format!(
                        "--group takes SOURCE:ID, the source one of dir, cloud, com, staff; got `{spec}`"
                    ),
                    ExitCode::ConfirmationRequired,
                );
            };
            body["group"] = serde_json::json!({ "id": id, "src": source });
            format!("group {id}")
        }
    };
    if no_inherit {
        body["inheritance"] = serde_json::Value::String("not_inherited".to_owned());
    }
    if let Some(code) = gated(
        &format!("grant {role} on wiki page `{slug}` to {named_as}"),
        &body,
        false,
        session,
    ) {
        return code;
    }

    if let Grantee::Login(login) = who {
        match client.user(login).await {
            Ok(person) if !person.uid.is_empty() => {
                body["user"] = serde_json::json!({ "uid": person.uid });
            }
            Ok(_) => {
                return report(
                    &format!("Tracker knows {login} but gives no uid for them; pass --uid"),
                    ExitCode::NotFound,
                );
            }
            Err(error) => return failed(&error),
        }
    }
    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    match client.wiki_grant(id, &body, allow_selflock).await {
        Ok(entry) => {
            emit(&format!(
                "granted {role} on {slug} to {named_as} (access {})\n",
                entry.id
            ));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// Change one grant.
async fn regrant(
    page: &str,
    access: &str,
    body: &serde_json::Value,
    allow_selflock: bool,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    if let Some(code) = gated(
        &format!("change access {access} on wiki page `{slug}`"),
        body,
        false,
        session,
    ) {
        return code;
    }

    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    match client.wiki_regrant(id, access, body, allow_selflock).await {
        Ok(entry) => {
            emit(&format!(
                "changed access {access} on {slug}: {}\n",
                if entry.role.is_empty() {
                    "-"
                } else {
                    &entry.role
                }
            ));
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// Remove one grant, or — with no grant named — every personal one, which
/// is a larger mistake to make by accident and so needs `--yes`.
async fn revoke(
    page: &str,
    access: Option<&str>,
    allow_selflock: bool,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    let (action, body) = match access {
        Some(access) => (
            format!("revoke access {access} on wiki page `{slug}`"),
            serde_json::json!({ "access": access }),
        ),
        None => (
            format!("revoke every personal access on wiki page `{slug}`"),
            serde_json::json!({ "access": "all personal" }),
        ),
    };
    if let Some(code) = gated(&action, &body, access.is_none(), session) {
        return code;
    }

    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    match client.wiki_revoke(id, access, allow_selflock).await {
        Ok(()) => {
            emit(&match access {
                Some(access) => format!("revoked access {access} on {slug}\n"),
                None => format!("revoked every personal access on {slug}\n"),
            });
            ExitCode::Success
        }
        Err(error) => failed(&error),
    }
}

/// How long a clone is waited for before the command hands back its id.
const CLONE_WAIT: std::time::Duration = std::time::Duration::from_secs(600);

/// The refusals the Wiki documents for a clone, in words a caller can act on.
const CLONE_REFUSALS: [(&str, &str); 6] = [
    (
        "IS_CLOUD_PAGE",
        "the page is a cloud page, which the Wiki cannot clone",
    ),
    ("SLUG_OCCUPIED", "a page already exists at the target"),
    ("SLUG_RESERVED", "the target address is reserved"),
    (
        "FORBIDDEN",
        "this account may not create a page at the target",
    ),
    ("QUOTA_EXCEEDED", "the organisation's Wiki quota is used up"),
    (
        "CLUSTER_BLOCKED",
        "the Wiki is not taking writes here right now",
    ),
];

/// A refused clone, told by its `error_code` when the Wiki gave one.
fn clone_refused(error: &crate::api::error::ApiError) -> ExitCode {
    if let crate::api::error::ApiError::Rejected { message, .. } = error
        && let Some((code, meaning)) = CLONE_REFUSALS
            .iter()
            .find(|(code, _)| message.contains(code))
    {
        return report(
            &format!("the Wiki would not clone it: {meaning} ({code})"),
            ExitCode::ApiRejected,
        );
    }
    failed(error)
}

/// Copy a page.
async fn clone_page(
    page: &str,
    target: &str,
    mut body: serde_json::Value,
    no_wait: bool,
    session: &Session,
) -> ExitCode {
    let slug = match named(page) {
        Ok(slug) => slug,
        Err(code) => return code,
    };
    let target = match named(target) {
        Ok(target) => target,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    body["target"] = serde_json::Value::String(target.clone());
    if let Some(code) = gated(
        &format!("clone wiki page `{slug}` to `{target}`"),
        &body,
        false,
        session,
    ) {
        return code;
    }

    let id = match client.wiki_page_id(&slug).await {
        Ok(id) => id,
        Err(error) => return failed(&error),
    };
    let operation = match client.wiki_clone_page(id, &body).await {
        Ok(operation) => operation,
        Err(error) => return clone_refused(&error),
    };
    followed(&client, &operation, no_wait, |done| {
        format!("cloned {slug} to {}\n", done.page_slug().unwrap_or(&target))
    })
    .await
}

/// Copy a grid onto a page.
async fn clone_grid(
    grid: &str,
    target: &str,
    mut body: serde_json::Value,
    no_wait: bool,
    session: &Session,
) -> ExitCode {
    let target = match named(target) {
        Ok(target) => target,
        Err(code) => return code,
    };
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };
    body["target"] = serde_json::Value::String(target.clone());
    if let Some(code) = gated(
        &format!("clone wiki grid `{grid}` onto `{target}`"),
        &body,
        false,
        session,
    ) {
        return code;
    }

    let operation = match client.wiki_clone_grid(grid.trim(), &body).await {
        Ok(operation) => operation,
        Err(error) => return clone_refused(&error),
    };
    followed(&client, &operation, no_wait, |done| {
        format!(
            "cloned grid {grid} to {}: grid {}\n",
            done.page_slug().unwrap_or(&target),
            done.grid_id().as_deref().unwrap_or("-")
        )
    })
    .await
}

/// Wait for an operation to end and say what it made — or, with `--no-wait`
/// or past the deadline, hand back the command that asks again.
///
/// Progress goes to stderr, and only to a terminal; stdout carries the one
/// line of result.
async fn followed(
    client: &crate::api::Client,
    operation: &crate::api::wiki::WikiOperation,
    no_wait: bool,
    describe: impl FnOnce(&crate::api::wiki::OperationStatus) -> String,
) -> ExitCode {
    let ask_again = format!("ytcli wiki operation {} {}", operation.kind, operation.id);
    if no_wait {
        emit(&format!(
            "started operation {} {}; follow it with `{ask_again}`\n",
            operation.kind, operation.id
        ));
        return ExitCode::Success;
    }

    let walk = crate::render::progress::Walk::start("cloning");
    let deadline = std::time::Instant::now() + CLONE_WAIT;
    let status = loop {
        let status = match client.wiki_operation(operation).await {
            Ok(status) => status,
            Err(error) => {
                walk.finish();
                return failed(&error);
            }
        };
        if status.is_done() {
            break status;
        }
        walk.say(&status.percentage.map_or_else(
            || format!("cloning: {}", status.status),
            |percentage| format!("cloning: {percentage:.0}%"),
        ));
        if std::time::Instant::now() >= deadline {
            walk.finish();
            return report(
                &format!("the Wiki is still working on it; ask again with `{ask_again}`"),
                ExitCode::Failure,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    };
    walk.finish();

    if status.status == "failed" {
        return report(
            &format!(
                "the clone failed: {}",
                status
                    .details
                    .as_deref()
                    .unwrap_or("the Wiki gave no reason")
            ),
            ExitCode::ApiRejected,
        );
    }
    emit(&describe(&status));
    ExitCode::Success
}

/// Where an operation has got to.
async fn show_operation(
    operation: &crate::api::wiki::WikiOperation,
    session: &Session,
) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.wiki_operation(operation).await {
        Ok(status) => finish(match session.render.format {
            Format::Text => Ok(render::operation(operation, &status)),
            Format::JsonRaw => machine(&status, Format::Json),
            other => machine(&status, other),
        }),
        Err(error) => failed(&error),
    }
}

/// Where a download's bytes come from.
enum Source {
    Attachment { page: u64, file: u64 },
    Address(String),
}

/// Write one file into a directory the caller named.
///
/// The file keeps its own name, cleaned of anything that could steer it out of
/// that directory, exactly as Tracker's downloads do. The destination is
/// checked before a byte is fetched.
async fn download(
    page: &str,
    file: Option<&str>,
    out: &Path,
    force: bool,
    session: &Session,
) -> ExitCode {
    let target = slug_of(page);
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let (name, source) = if let Some(file) = file {
        let slug = match named(page) {
            Ok(slug) => slug,
            Err(code) => return code,
        };
        match client.wiki_attachment_named(&slug, file).await {
            Ok((page, found)) => (
                safe_filename(&found.name, &found.id.to_string()),
                Source::Attachment {
                    page,
                    file: found.id,
                },
            ),
            Err(error) => {
                let code = error.exit_code();
                return report(&error, code);
            }
        }
    } else if let Some((_, name)) = target.split_once("/.files/") {
        (
            safe_filename(name, "download"),
            Source::Address(target.clone()),
        )
    } else {
        return report(
            &format!(
                "`{page}` is a page, not a file: name the file too (`wiki attachments` lists them), \
                 or pass the file's address, <slug>/.files/<name>"
            ),
            ExitCode::ConfirmationRequired,
        );
    };

    let destination = out.join(name);
    if destination.exists() && !force {
        return report(
            &format!(
                "{} already exists; pass --force to overwrite",
                destination.display()
            ),
            ExitCode::ConfirmationRequired,
        );
    }

    let fetched = match &source {
        Source::Attachment { page, file } => client.wiki_attachment_bytes(*page, *file).await,
        Source::Address(path) => client.wiki_file_bytes(path).await,
    };
    let bytes = match fetched {
        Ok(bytes) => bytes,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    if let Err(error) = std::fs::create_dir_all(out) {
        return report(&error, ExitCode::Failure);
    }
    if let Err(error) = std::fs::write(&destination, &bytes) {
        return report(&error, ExitCode::Failure);
    }

    emit(&format!("{}\n", destination.display()));
    ExitCode::Success
}

/// The profile's list length, within the 1..100 the Wiki's listings accept.
fn page_size(session: &Session) -> u32 {
    u32::try_from(session.display().limit.clamp(1, 100)).unwrap_or(100)
}

/// The slug a command was given, or the refusal to guess one.
fn named(page: &str) -> Result<String, ExitCode> {
    let slug = slug_of(page);
    if slug.is_empty() {
        return Err(report(
            &format!(
                "`{page}` names no page: pass a slug such as users/me/notes, or the page's address"
            ),
            ExitCode::ConfirmationRequired,
        ));
    }
    Ok(slug)
}

fn finish(rendered: Result<String, RenderError>) -> ExitCode {
    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}
