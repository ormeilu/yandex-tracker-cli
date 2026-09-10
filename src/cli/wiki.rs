//! Wiki commands: the pages of the Yandex Wiki next to the organisation's
//! Tracker, read through the same profile.
//!
//! Read verbs only (`docs/adr/0007-yandex-wiki.md`). All of them are declared,
//! so help and completions say what the group will hold; the ones not built yet
//! say so when run.

use clap::Subcommand;

use crate::api::wiki::slug_of;
use crate::cli::{Session, emit, not_implemented, report};
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
    },
    /// Show a page's comments.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_COMMENTS))]
    Comments {
        /// The page's slug, or its address.
        page: String,
    },
    /// List a page's attachments.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WIKI_ATTACHMENTS))]
    Attachments {
        /// The page's slug, or its address.
        page: String,
    },
}

pub async fn run(command: &WikiCommand, session: &Session) -> ExitCode {
    match command {
        WikiCommand::Get { page } => get(page, session).await,
        WikiCommand::List { page, cursor } => list(page, cursor.as_deref(), session).await,
        WikiCommand::Find { .. } => not_implemented("wiki find"),
        WikiCommand::Comments { .. } => not_implemented("wiki comments"),
        WikiCommand::Attachments { .. } => not_implemented("wiki attachments"),
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

    // The profile's list length, within the 1..100 the Wiki accepts.
    let page_size = u32::try_from(session.display().limit.clamp(1, 100)).unwrap_or(100);

    match client.wiki_descendants(&slug, cursor, page_size).await {
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
