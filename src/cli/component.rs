//! Components: the parts a queue splits its work by.
//!
//! Read-only. Components are configured once per queue by whoever owns it, and
//! the question a command line has is what they are called — `--set
//! components=…` takes the name, and without a listing that is a guess.

use clap::Subcommand;

use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, machine, queue as render};

#[derive(Debug, Subcommand)]
pub enum ComponentCommand {
    /// List components, in one queue or in the whole organisation.
    #[command(long_about = crate::cli::help::md(crate::cli::help::COMPONENT_LIST))]
    List {
        /// Only this queue's components, e.g. PROJ.
        #[arg(long, short = 'q')]
        queue: Option<String>,
    },
}

pub async fn run(command: &ComponentCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    let ComponentCommand::List { queue } = command;
    match client.components(queue.as_deref()).await {
        Ok(components) => {
            let rendered = match session.render.format {
                Format::Text => Ok(render::components(
                    &components,
                    queue.as_deref(),
                    &session.render,
                )),
                Format::JsonRaw => machine(&components, Format::Json),
                other => machine(&components, other),
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
