//! Sprints across the organisation.
//!
//! `board sprints ID` needs the board first. A sprint name is a thing people
//! say without knowing which board it belongs to, and this is the listing that
//! answers that — read-only, like every other view of a board.

use clap::Subcommand;

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, board as render, machine};

#[derive(Debug, Subcommand)]
pub enum SprintCommand {
    /// List every sprint in the organisation.
    #[command(long_about = crate::cli::help::md(crate::cli::help::SPRINT_LIST))]
    List,
}

pub async fn run(command: &SprintCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let SprintCommand::List = command;
    match client.all_sprints().await {
        Ok(sprints) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::all_sprints(&sprints, &session.render)),
                Format::JsonRaw => machine(&sprints, Format::Json),
                other => machine(&sprints, other),
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
