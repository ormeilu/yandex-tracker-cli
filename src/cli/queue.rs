//! Queue commands. Read-only in v1: queue administration is deliberately out of
//! scope (`docs/TODO.md`).

use clap::Subcommand;

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, machine, queue};

#[derive(Debug, Subcommand)]
pub enum QueueCommand {
    /// List queues visible to this profile.
    List,
    /// Show a queue's fields, including custom ones and their keys.
    Fields {
        /// Queue key, e.g. PROJ.
        key: String,
    },
}

pub async fn run(command: &QueueCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match command {
        QueueCommand::List => match client.queues().await {
            Ok(queues) => render(&queues, session, queue::queues),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
        QueueCommand::Fields { key } => match client.queue_fields(key).await {
            Ok(fields) => render(&fields, session, queue::fields),
            Err(error) => {
                let code = error.exit_code();
                report(&error, code)
            }
        },
    }
}

/// Render either as text, through the entity's own renderer, or in whichever
/// machine format was asked for.
fn render<T: serde::Serialize>(
    value: &[T],
    session: &Session,
    as_text: impl Fn(&[T]) -> String,
) -> ExitCode {
    let rendered = match session.render.format {
        Format::Text => Ok(as_text(value)),
        // There is no upstream payload worth preserving separately here: these
        // listings are already flat, so raw and normalised would be the same
        // shape with uglier names.
        Format::JsonRaw => machine(&value, Format::Json),
        other => machine(&value, other),
    };

    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}
