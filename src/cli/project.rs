//! Project commands. Projects group issues across queues; portfolios sit above
//! them and are out of scope for v1.

use clap::Subcommand;

use crate::cli::{Session, entity};
use crate::exit::ExitCode;

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List projects.
    #[command(long_about = crate::cli::help::md(crate::cli::help::PROJECT_LIST))]
    List {
        /// Free-text search over names and descriptions.
        #[arg(long, short = 'Q')]
        query: Option<String>,
        /// 1-based page number.
        #[arg(long, default_value_t = 1)]
        page: u32,
    },
    /// Show one project.
    #[command(long_about = crate::cli::help::md(crate::cli::help::PROJECT_GET))]
    Get {
        /// Project id as returned by `project list`, not an issue key.
        id: String,
    },
    /// Put it inside a portfolio, or take it out of one.
    #[command(long_about = crate::cli::help::md(crate::cli::help::PROJECT_PLACE))]
    Place {
        /// The project to move, by the id `project list` prints.
        id: String,
        /// Portfolio to put it in.
        #[arg(long, conflicts_with = "out")]
        into: Option<String>,
        /// Take it out of whichever portfolio it is in.
        #[arg(long, conflicts_with = "into")]
        out: bool,
    },
}

pub async fn run(command: &ProjectCommand, session: &Session) -> ExitCode {
    match command {
        ProjectCommand::List { query, page } => {
            entity::list("project", query.as_deref(), *page, session).await
        }
        ProjectCommand::Get { id } => entity::get("project", id, session).await,
        ProjectCommand::Place { id, into, out } => {
            if into.is_none() && !*out {
                return crate::cli::report(
                    &"say where: --into <portfolio>, or --out to remove it from one",
                    crate::exit::ExitCode::ConfirmationRequired,
                );
            }
            entity::place("project", id, into.as_deref(), session).await
        }
    }
}
