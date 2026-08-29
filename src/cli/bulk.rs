//! Reading a bulk change back.
//!
//! A group of its own, holding one read and nothing else. The write that starts
//! a bulk change is `issue update`, which is where a caller already looks to
//! change issues; this is the handle on work Tracker is still doing, and it has
//! to be allowlistable without allowing anything to be written.

use clap::Subcommand;

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, bulk, machine};

#[derive(Debug, Subcommand)]
pub enum BulkCommand {
    /// Show how far a bulk change got.
    #[command(long_about = crate::cli::help::md(crate::cli::help::BULK_STATUS))]
    Status {
        /// The id `issue update` printed.
        id: String,
    },
}

pub async fn run(command: &BulkCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let BulkCommand::Status { id } = command;
    let change = match client.bulk_change(id).await {
        Ok(change) => change,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    if session.render.format != Format::Text {
        let format = match session.render.format {
            Format::JsonRaw => Format::Json,
            other => other,
        };
        return match machine(&change, format) {
            Ok(text) => {
                emit(&text);
                ExitCode::Success
            }
            Err(error) => report(&error, ExitCode::Failure),
        };
    }

    emit(&bulk::change(&change, &session.render));

    // A change still running has nothing per issue to say yet, and one that
    // worked has said it in the tally. Only a finished change that did not do
    // everything is worth the second request.
    if change.finished() && !change.succeeded() {
        match client.bulk_change_issues(id).await {
            Ok(outcomes) => emit(&bulk::failures(&outcomes, &session.render)),
            Err(error) => {
                let code = error.exit_code();
                return report(&error, code);
            }
        }
    }

    // Reading is not judging: this says what happened, and exits zero for having
    // answered. `issue update` is where a failed change decides an exit code.
    ExitCode::Success
}
