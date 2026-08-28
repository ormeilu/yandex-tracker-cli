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
    /// Create a goal.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ENTITY_CREATE))]
    Create {
        #[command(flatten)]
        fields: entity::Fields,
    },
    /// Change the fields of one.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ENTITY_UPDATE))]
    Update {
        /// The id `goal list` prints.
        id: String,
        #[command(flatten)]
        fields: entity::Fields,
    },
    /// Delete one. Needs --yes, and nothing puts it back.
    #[command(long_about = crate::cli::help::md(crate::cli::help::ENTITY_DELETE))]
    Delete {
        /// The id `goal list` prints.
        id: String,
    },
}

pub async fn run(command: &GoalCommand, session: &Session) -> ExitCode {
    match command {
        GoalCommand::List { query, page } => {
            entity::list("goal", query.as_deref(), *page, session).await
        }
        GoalCommand::Get { id } => entity::get("goal", id, session).await,
        GoalCommand::Create { fields } => entity::create("goal", fields, session).await,
        GoalCommand::Update { id, fields } => entity::update("goal", id, fields, session).await,
        GoalCommand::Delete { id } => entity::remove("goal", id, session).await,
    }
}
