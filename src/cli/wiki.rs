//! Wiki commands: the pages of the Yandex Wiki next to the organisation's
//! Tracker, read through the same profile.
//!
//! Read verbs only (`docs/adr/0007-yandex-wiki.md`).

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::api::wiki::{CommentScope, LAST_SEARCH_PAGE, slug_of};
use crate::cli::attachment::safe_filename;
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
