//! Project commands. Projects group issues across queues; portfolios sit above
//! them and are out of scope for v1.

use clap::Subcommand;

use crate::cli::{Session, not_implemented};
use crate::exit::ExitCode;

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List projects.
    List,
    /// Show one project.
    Get {
        /// Project id as returned by `project list`, not an issue key.
        id: String,
    },
}

// The dispatcher is async because the implementations landing behind it are;
// the placeholders simply do not await anything yet.
#[allow(clippy::unused_async)]
pub async fn run(command: &ProjectCommand, _session: &Session) -> ExitCode {
    match command {
        ProjectCommand::List => not_implemented("project list"),
        ProjectCommand::Get { .. } => not_implemented("project get"),
    }
}
