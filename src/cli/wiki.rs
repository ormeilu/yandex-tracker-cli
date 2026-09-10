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
use crate::render::{Format, machine, wiki as render};

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
        WikiCommand::List { .. } => not_implemented("wiki list"),
        WikiCommand::Find { .. } => not_implemented("wiki find"),
        WikiCommand::Comments { .. } => not_implemented("wiki comments"),
        WikiCommand::Attachments { .. } => not_implemented("wiki attachments"),
    }
}

/// Show one page.
async fn get(page: &str, session: &Session) -> ExitCode {
    let slug = slug_of(page);
    if slug.is_empty() {
        return report(
            &format!(
                "`{page}` names no page: pass a slug such as users/me/notes, or the page's address"
            ),
            ExitCode::ConfirmationRequired,
        );
    }

    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match client.wiki_page(&slug).await {
        Ok(found) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::page(&found, &session.render)),
                Format::JsonRaw => machine(&found, Format::Json),
                other => machine(&found, other),
            };
            match rendered {
                Ok(text) => {
                    emit(&text);
                    ExitCode::Success
                }
                Err(error) => report(&error, ExitCode::Failure),
            }
        }
        Err(error) => {
            let code = error.exit_code();
            report(&error, code)
        }
    }
}
