//! Worklogs across issues.
//!
//! `issue worklogs PROJ-1` answers "what went into this issue"; this answers
//! "where did my week go", which used to cost one request per issue and
//! knowing which issues to ask about in the first place.
//!
//! A group of its own rather than a verb under `issue`, because `issue worklog`
//! is the writing group and a host allowlists by prefix. `ytcli worklog find`
//! shares no prefix with anything that writes.

use std::io::Write as _;

use clap::Subcommand;

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, machine, text};

#[derive(Debug, Subcommand)]
pub enum WorklogCommand {
    /// Find worklog entries across every issue.
    #[command(long_about = crate::cli::help::md(crate::cli::help::WORKLOG_FIND))]
    Find {
        /// Whose time: a login, or `me`.
        #[arg(long, short = 'b')]
        by: Option<String>,
        /// From this date, or a span back from today: `7d`, `2w`, `2026-08-01`.
        #[arg(long)]
        since: Option<String>,
        /// Up to this date. Same forms as --since.
        #[arg(long)]
        until: Option<String>,
        /// How many entries to fetch.
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
}

pub async fn run(command: &WorklogCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let WorklogCommand::Find {
        by,
        since,
        until,
        limit,
    } = command;

    // `me` is not a login Tracker knows: `createdBy=me` answers 422 saying no
    // such user exists. Resolving it here costs one request, and is the reason
    // the convenience can exist at all.
    let who = match by.as_deref() {
        Some("me") => match client.myself().await {
            Ok(user) => match user.login.or(Some(user.id)).filter(|id| !id.is_empty()) {
                Some(login) => Some(login),
                None => return report(&"this token has no login to search by", ExitCode::Auth),
            },
            Err(error) => {
                let code = error.exit_code();
                return report(&error, code);
            }
        },
        other => other.map(ToOwned::to_owned),
    };

    let since = match since.as_deref().map(as_date) {
        Some(Ok(date)) => Some(date),
        Some(Err(error)) => return report(&error, ExitCode::ConfirmationRequired),
        None => None,
    };
    let until = match until.as_deref().map(as_date) {
        Some(Ok(date)) => Some(date),
        Some(Err(error)) => return report(&error, ExitCode::ConfirmationRequired),
        None => None,
    };

    match client
        .worklog_search(who.as_deref(), since.as_deref(), until.as_deref(), *limit)
        .await
    {
        Ok(entries) => {
            let rendered = match session.render.format {
                Format::Text => Ok(text::worklog_search(&entries, &session.render)),
                Format::JsonRaw => machine(&entries, Format::Json),
                other => machine(&entries, other),
            };
            match rendered {
                Ok(text) => {
                    emit(&text);
                    // A page is a page here too, and this endpoint reports no
                    // total to compare against — so the ceiling is named rather
                    // than left to look like the whole answer.
                    if u32::try_from(entries.len()).is_ok_and(|count| count >= *limit) {
                        let mut err = anstream::stderr();
                        let _ =
                            writeln!(err, "stopped at --limit {limit}; there may be more entries");
                    }
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

/// A date, from a date or from a span back from today.
///
/// `7d` is what somebody asking about their week types, and turning it into a
/// date here keeps the API parameter to the one form Tracker documents.
fn as_date(value: &str) -> Result<String, String> {
    let Some((count, unit)) = value.split_at_checked(value.len().saturating_sub(1)) else {
        return Err(format!("cannot read `{value}` as a date or a span"));
    };

    let span = match (count.parse::<i64>(), unit) {
        (Ok(count), "d") => jiff::Span::new().try_days(count),
        (Ok(count), "w") => jiff::Span::new().try_weeks(count),
        (Ok(count), "m") => jiff::Span::new().try_months(count),
        // Not a span: a date, passed through for Tracker to accept or refuse.
        // Guessing at date formats here would only add a second opinion.
        _ => return Ok(value.to_owned()),
    }
    .map_err(|_| format!("`{value}` is too large a span"))?;

    jiff::Zoned::now()
        .checked_sub(span)
        .map(|then| then.strftime("%Y-%m-%d").to_string())
        .map_err(|_| format!("`{value}` lands outside the range of dates"))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_span_becomes_a_date_in_the_past() {
        let today = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
        let week = as_date("7d").expect("a week");

        assert_eq!(week.len(), today.len());
        assert!(week < today, "{week} is not before {today}");
    }

    /// Anything that is not a span is Tracker's to judge; a second opinion here
    /// would only be a second thing to be wrong.
    #[test]
    fn a_date_passes_through_untouched() {
        assert_eq!(as_date("2026-08-01").as_deref(), Ok("2026-08-01"));
    }
}
