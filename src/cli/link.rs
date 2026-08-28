//! The vocabulary of links between issues.
//!
//! Read-only, and a group of its own rather than a verb under `issue`: a link
//! type is organisation-wide and about a relationship *between* issues, not a
//! value one issue's field takes — which is what `dict` is for, and why these
//! do not fit it. `issue link` is the write and shares no token with this.

use clap::Subcommand;

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, dict as render, machine};

#[derive(Debug, Subcommand)]
pub enum LinkCommand {
    /// List the kinds of link, and what a write takes for each.
    #[command(long_about = crate::cli::help::md(crate::cli::help::LINK_TYPES))]
    Types,
}

pub async fn run(command: &LinkCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let LinkCommand::Types = command;
    match client.link_types().await {
        Ok(types) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::link_types(&types, &session.render)),
                Format::JsonRaw => machine(&types, Format::Json),
                other => machine(&types, other),
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
