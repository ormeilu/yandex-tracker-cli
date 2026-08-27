//! Goal commands.

use clap::Subcommand;

use crate::cli::{Session, not_implemented};
use crate::exit::ExitCode;

#[derive(Debug, Subcommand)]
pub enum GoalCommand {
    /// List goals.
    List,
    /// Show one goal.
    Get { id: String },
}

// The dispatcher is async because the implementations landing behind it are;
// the placeholders simply do not await anything yet.
#[allow(clippy::unused_async)]
pub async fn run(command: &GoalCommand, _session: &Session) -> ExitCode {
    match command {
        GoalCommand::List => not_implemented("goal list"),
        GoalCommand::Get { .. } => not_implemented("goal get"),
    }
}
