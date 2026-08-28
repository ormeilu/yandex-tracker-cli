//! Portfolio commands. A portfolio groups projects and other portfolios; the
//! entity endpoints treat it as one more type, so only the word differs.

use clap::Subcommand;

use crate::cli::{Session, entity};
use crate::exit::ExitCode;

#[derive(Debug, Subcommand)]
pub enum PortfolioCommand {
    /// List portfolios.
    #[command(long_about = crate::cli::help::md(crate::cli::help::PORTFOLIO_LIST))]
    List {
        /// Free-text search over names and descriptions.
        #[arg(long, short = 'Q')]
        query: Option<String>,
        /// 1-based page number.
        #[arg(long, default_value_t = 1)]
        page: u32,
    },
    /// Show one portfolio.
    #[command(long_about = crate::cli::help::md(crate::cli::help::PORTFOLIO_GET))]
    Get {
        /// Portfolio id as returned by `portfolio list`.
        id: String,
    },
    /// List the portfolios and projects inside one.
    #[command(long_about = crate::cli::help::md(crate::cli::help::PORTFOLIO_CONTENTS))]
    Contents {
        /// Portfolio id as returned by `portfolio list`.
        id: String,
        /// 1-based page number.
        #[arg(long, default_value_t = 1)]
        page: u32,
    },
    /// Put it inside a portfolio, or take it out of one.
    #[command(long_about = crate::cli::help::md(crate::cli::help::PORTFOLIO_PLACE))]
    Place {
        /// The portfolio to move, by the id `portfolio list` prints.
        id: String,
        /// Portfolio to put it in.
        #[arg(long, conflicts_with = "out")]
        into: Option<String>,
        /// Take it out of whichever portfolio it is in.
        #[arg(long, conflicts_with = "into")]
        out: bool,
    },
}

pub async fn run(command: &PortfolioCommand, session: &Session) -> ExitCode {
    match command {
        PortfolioCommand::List { query, page } => {
            entity::list("portfolio", query.as_deref(), *page, session).await
        }
        PortfolioCommand::Get { id } => entity::get("portfolio", id, session).await,
        PortfolioCommand::Contents { id, page } => entity::contents(id, *page, session).await,
        PortfolioCommand::Place { id, into, out } => {
            if into.is_none() && !*out {
                return crate::cli::report(
                    &"say where: --into <portfolio>, or --out to remove it from one",
                    crate::exit::ExitCode::ConfirmationRequired,
                );
            }
            entity::place("portfolio", id, into.as_deref(), session).await
        }
    }
}
