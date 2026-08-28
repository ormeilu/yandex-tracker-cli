//! Goal commands.

use clap::Subcommand;

use crate::cli::{Session, entity};
use crate::exit::ExitCode;

#[derive(Debug, Subcommand)]
pub enum GoalCommand {
    /// List goals.
    #[command(long_about = crate::cli::help::md(crate::cli::help::GOAL_LIST))]
    List {
        /// Free-text search over names and descriptions.
        #[arg(long, short = 'Q')]
        query: Option<String>,
        /// 1-based page number.
        #[arg(long, default_value_t = 1)]
        page: u32,
    },
    /// Show one goal.
    #[command(long_about = crate::cli::help::md(crate::cli::help::GOAL_GET))]
    Get { id: String },
}

pub async fn run(command: &GoalCommand, session: &Session) -> ExitCode {
    match command {
        GoalCommand::List { query, page } => {
            entity::list("goal", query.as_deref(), *page, session).await
        }
        GoalCommand::Get { id } => entity::get("goal", id, session).await,
    }
}
