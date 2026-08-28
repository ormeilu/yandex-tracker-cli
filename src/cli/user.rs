//! People.
//!
//! Every issue answer carries logins — assignee, author, whoever logged the
//! time — and without this group they stay opaque strings. It is also the only
//! way to get `--assignee` right on the first attempt rather than the second:
//! Tracker validates a login by refusing the write.
//!
//! Read-only. Nothing in this group changes anything about anybody.

use std::io::Write as _;

use clap::{Args, Subcommand};

use crate::api::models::{Page, Person};
use crate::cli::{Session, emit as write_out, report};
use crate::exit::ExitCode;
use crate::render::{Format, machine, user as render};

#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// List the people in the organisation.
    #[command(long_about = crate::cli::help::md(crate::cli::help::USER_LIST))]
    List(PageArgs),
    /// Show one person, by login or uid.
    #[command(long_about = crate::cli::help::md(crate::cli::help::USER_GET))]
    Get {
        /// Login, or the numeric uid. Not `me` — `auth status` answers that.
        who: String,
    },
    /// Find people whose login, name or email contains some text.
    #[command(long_about = crate::cli::help::md(crate::cli::help::USER_FIND))]
    Find {
        /// Matched case-insensitively against login, display name and email.
        text: String,
        /// How many people to read through before giving up.
        #[arg(long, default_value_t = 1000)]
        scan: usize,
    },
}

#[derive(Debug, Args, Clone)]
pub struct PageArgs {
    /// Rows per page.
    #[arg(long)]
    pub limit: Option<usize>,
    /// 1-based page number.
    #[arg(long, default_value_t = 1)]
    pub page: u32,
}

pub async fn run(command: &UserCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match command {
        UserCommand::List(args) => {
            let per_page = args.limit.unwrap_or(session.display().limit);
            let Ok(per_page) = u32::try_from(per_page.max(1)) else {
                return report(&"--limit is too large", ExitCode::ConfirmationRequired);
            };

            match client.users(args.page.max(1), per_page).await {
                Ok(page) => {
                    let next = page.has_more().then(|| page.page + 1);
                    emit(&page, next, session)
                }
                Err(error) => {
                    let code = error.exit_code();
                    report(&error, code)
                }
            }
        }
        UserCommand::Get { who } => match client.user(who).await {
            Ok(person) => {
                let rendered = match session.render.format {
                    Format::Text => Ok(render::user(&person, &session.render)),
                    Format::JsonRaw => machine(&person, Format::Json),
                    other => machine(&person, other),
                };
                match rendered {
                    Ok(text) => {
                        write_out(&text);
                        ExitCode::Success
                    }
                    Err(error) => report(&error, ExitCode::Failure),
                }
            }
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        UserCommand::Find { text, scan } => find(&client, text, *scan, session).await,
    }
}

/// Search, done here rather than by Tracker.
///
/// There is no user search endpoint — `/v3/users/_search` is a 404, and the
/// name is read as a login — so matching means reading the directory and
/// filtering it. That is honest but not free, which is why `--scan` is a
/// visible ceiling rather than a hidden one, and why the tally says how many
/// people were actually read.
async fn find(client: &crate::api::Client, text: &str, scan: usize, session: &Session) -> ExitCode {
    let needle = text.to_lowercase();
    let mut matched: Vec<Person> = Vec::new();
    let mut read = 0usize;
    let mut page_number = 1;
    let mut total = None;
    let walk = crate::render::progress::Walk::start("reading the directory");

    loop {
        let page = match client.users(page_number, 100).await {
            Ok(page) => page,
            Err(error) => {
                walk.finish();
                let code = error.exit_code();
                return report(&error, code);
            }
        };
        total = page.total.or(total);
        read += page.items.len();

        let more = page.has_more();
        matched.extend(
            page.items
                .into_iter()
                .filter(|person| matches(person, &needle)),
        );
        walk.page(page_number, matched.len(), total);

        if !more || read >= scan {
            break;
        }
        page_number += 1;
    }
    walk.finish();

    // The tally counts what was read, not what the organisation has: a match
    // count against a total nobody searched would claim a completeness this
    // command cannot offer.
    let incomplete = total.is_some_and(|total| read < usize::try_from(total).unwrap_or(usize::MAX));
    let Ok(count) = u32::try_from(matched.len()) else {
        return report(&"too many results to render", ExitCode::Failure);
    };
    let page = Page {
        items: matched,
        page: 1,
        per_page: count.max(1),
        total: Some(read as u64),
    };

    // No `next: --page 2` here: a filtered answer has no second page to ask
    // for, and offering one would send the caller back to the unfiltered
    // listing.
    let code = emit(&page, None, session);
    if incomplete && session.render.format == Format::Text {
        let mut err = anstream::stderr();
        let _ = writeln!(
            err,
            "searched {read} of {} people; raise --scan to look further",
            total.map_or_else(|| "unknown".to_owned(), |total| total.to_string())
        );
    }
    code
}

fn matches(person: &Person, needle: &str) -> bool {
    person.login.to_lowercase().contains(needle)
        || person.display.to_lowercase().contains(needle)
        || person
            .email
            .as_deref()
            .is_some_and(|email| email.to_lowercase().contains(needle))
}

fn emit(page: &Page<Person>, next: Option<u32>, session: &Session) -> ExitCode {
    let rendered = match session.render.format {
        Format::Text => Ok(render::users(
            &page.items,
            page.total,
            next,
            &session.render,
        )),
        Format::JsonRaw => machine(&page.items, Format::Json),
        other => machine(&page.items, other),
    };

    match rendered {
        Ok(text) => {
            write_out(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}
