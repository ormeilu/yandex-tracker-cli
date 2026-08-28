//! Board commands. Read-only: a board is a view of work, and moving work about
//! from a command line is what `issue update` is for.

use clap::Subcommand;

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, board as render, machine};

#[derive(Debug, Subcommand)]
pub enum BoardCommand {
    /// List boards.
    #[command(long_about = crate::cli::help::BOARD_LIST)]
    List,
    /// Show one board and its columns.
    #[command(long_about = crate::cli::help::BOARD_GET)]
    Get {
        /// Board id as returned by `board list`.
        id: String,
    },
    /// List the sprints of a board.
    #[command(long_about = crate::cli::help::BOARD_SPRINTS)]
    Sprints {
        /// Board id as returned by `board list`.
        id: String,
    },
}

pub async fn run(command: &BoardCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let rendered = match command {
        BoardCommand::List => match client.boards().await {
            Ok(boards) => match session.render.format {
                Format::Text => Ok(render::boards(&boards, &session.render)),
                Format::JsonRaw => machine(&boards, Format::Json),
                other => machine(&boards, other),
            },
            Err(error) => return failed(&error),
        },
        BoardCommand::Get { id } => match client.board(id).await {
            Ok(board) => match session.render.format {
                Format::Text => Ok(render::board(&board, &session.render)),
                Format::JsonRaw => machine(&board, Format::Json),
                other => machine(&board, other),
            },
            Err(error) => return failed(&error),
        },
        BoardCommand::Sprints { id } => match client.sprints(id).await {
            Ok(sprints) => match session.render.format {
                Format::Text => Ok(render::sprints(id, &sprints, &session.render)),
                Format::JsonRaw => machine(&sprints, Format::Json),
                other => machine(&sprints, other),
            },
            Err(error) => return failed(&error),
        },
    };

    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}

fn failed(error: &crate::api::error::ApiError) -> ExitCode {
    let code = error.exit_code();
    report(error, code)
}
