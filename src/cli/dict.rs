//! The values a write is allowed to take.
//!
//! Everything else in this tool answers what *is*; this answers what *may be*.
//! `issue create --type` and `issue update --set priority=…` are otherwise
//! written on faith and judged by Tracker, which refuses with a message about a
//! field the caller was never shown a list for.
//!
//! Read-only, and organisation-wide: a queue narrows these — `queue get` says
//! which type and priority its issues start with — but the dictionary itself is
//! defined once for the whole organisation.

use clap::{Subcommand, ValueEnum};

use crate::api::Dictionary;
use crate::api::models::DictEntry;
use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, dict as render, machine};

#[derive(Debug, Subcommand)]
pub enum DictCommand {
    /// List the values issues can take.
    #[command(long_about = crate::cli::help::md(crate::cli::help::DICT_LIST))]
    List {
        /// One dictionary instead of all four.
        #[arg(long, value_enum)]
        kind: Option<Kind>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Kind {
    Types,
    Priorities,
    Statuses,
    Resolutions,
}

impl From<Kind> for Dictionary {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Types => Self::Types,
            Kind::Priorities => Self::Priorities,
            Kind::Statuses => Self::Statuses,
            Kind::Resolutions => Self::Resolutions,
        }
    }
}

pub async fn run(command: &DictCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let DictCommand::List { kind } = command;

    // All four by default, and sequentially. The whole point of the command is
    // to be the one call an agent makes before writing anything, and four small
    // responses in one answer beat four round trips to discover the same thing.
    let wanted: Vec<Dictionary> = match kind {
        Some(kind) => vec![(*kind).into()],
        None => Dictionary::ALL.to_vec(),
    };

    let mut sections: Vec<(Dictionary, Vec<DictEntry>)> = Vec::with_capacity(wanted.len());
    for kind in wanted {
        match client.dictionary(kind).await {
            Ok(entries) => sections.push((kind, entries)),
            Err(error) => {
                let code = error.exit_code();
                return report(&error, code);
            }
        }
    }

    let rendered = match session.render.format {
        Format::Text => Ok(render::many(&sections, &session.render)),
        other => {
            // Machine output is keyed by dictionary rather than concatenated:
            // `bug` the issue type and `bug` the anything-else are only telling
            // apart by which list they came from.
            let keyed: serde_json::Map<String, serde_json::Value> = sections
                .iter()
                .map(|(kind, entries)| {
                    (
                        kind.label().to_owned(),
                        serde_json::to_value(entries).unwrap_or(serde_json::Value::Null),
                    )
                })
                .collect();
            machine(
                &keyed,
                if other == Format::JsonRaw {
                    Format::Json
                } else {
                    other
                },
            )
        }
    };

    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}
