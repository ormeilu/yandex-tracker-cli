//! Queue commands. Read-only in v1: queue administration is deliberately out of
//! scope (`docs/TODO.md`).

use clap::Subcommand;

use crate::cli::{Session, not_implemented};
use crate::exit::ExitCode;

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

// The dispatcher is async because the implementations landing behind it are;
// the placeholders simply do not await anything yet.
#[allow(clippy::unused_async)]
pub async fn run(command: &QueueCommand, _session: &Session) -> ExitCode {
    match command {
        QueueCommand::List => not_implemented("queue list"),
        QueueCommand::Fields { .. } => not_implemented("queue fields"),
    }
}
