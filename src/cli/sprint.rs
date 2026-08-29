//! Sprints across the organisation.
//!
//! `board sprints ID` needs the board first. A sprint name is a thing people
//! say without knowing which board it belongs to, and this is the listing that
//! answers that — read-only, like every other view of a board.

use clap::Subcommand;

use crate::api::Sprint;
use crate::cli::{Session, emit, report};
use crate::exit::ExitCode;
use crate::render::{Format, board as render, machine};

#[derive(Debug, Subcommand)]
pub enum SprintCommand {
    /// List every sprint in the organisation.
    #[command(long_about = crate::cli::help::md(crate::cli::help::SPRINT_LIST))]
    List {
        /// Show only the sprint to plan into: the nearest draft, or the running
        /// one when there is no draft.
        #[arg(long)]
        planning: bool,
    },
    /// Show one sprint: its dates, and how far through it is.
    #[command(long_about = crate::cli::help::md(crate::cli::help::SPRINT_GET))]
    Get {
        id: String,
        /// Skip the two counts that say how many of its issues are resolved.
        #[arg(long)]
        no_issues: bool,
    },
}

pub async fn run(command: &SprintCommand, session: &Session) -> ExitCode {
    let client = match session.client() {
        Ok(client) => client,
        Err(code) => return code,
    };

    match command {
        SprintCommand::List { planning } => list(&client, *planning, session).await,
        SprintCommand::Get { id, no_issues } => get(&client, id, *no_issues, session).await,
    }
}

async fn list(client: &crate::api::Client, planning: bool, session: &Session) -> ExitCode {
    let sprints = match client.all_sprints().await {
        Ok(sprints) => sprints,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    // A filter, not a different answer: the tally still counts what was found,
    // so `shown 1 of 9` says plainly that eight were left out.
    let shown: Vec<Sprint> = if planning {
        planning_sprint(&sprints).cloned().into_iter().collect()
    } else {
        sprints.clone()
    };

    let rendered = match session.render.format {
        Format::Text => Ok(render::all_sprints(&shown, &session.render)),
        Format::JsonRaw => machine(&shown, Format::Json),
        other => machine(&shown, other),
    };
    match rendered {
        Ok(text) => {
            emit(&text);
            ExitCode::Success
        }
        Err(error) => report(&error, ExitCode::Failure),
    }
}

/// The sprint to put new work into.
///
/// Not the running one: work planned now belongs to the next sprint, and the
/// running one is what people are already doing. So the nearest draft wins, and
/// the running sprint is the answer only when no draft exists — which is the
/// case where "plan into the current one" is genuinely what was meant.
///
/// Two spellings for a sprint that has not started — Tracker has answered with
/// both `draft` and `planned` — and both are accepted rather than one being
/// picked as the true one.
fn planning_sprint(sprints: &[Sprint]) -> Option<&Sprint> {
    let live = |sprint: &&Sprint| {
        !matches!(
            sprint.status.as_deref(),
            Some("archived" | "closed" | "completed")
        )
    };
    let by_start = |sprint: &&Sprint| sprint.start.clone().unwrap_or_else(|| "9999".to_owned());

    sprints
        .iter()
        .filter(live)
        .filter(|sprint| matches!(sprint.status.as_deref(), Some("draft" | "planned")))
        .min_by_key(by_start)
        .or_else(|| {
            sprints
                .iter()
                .filter(live)
                .filter(|sprint| sprint.status.as_deref() == Some("in_progress"))
                .min_by_key(by_start)
        })
}

async fn get(
    client: &crate::api::Client,
    id: &str,
    no_issues: bool,
    session: &Session,
) -> ExitCode {
    let sprint = match client.sprint(id).await {
        Ok(sprint) => sprint,
        Err(error) => {
            let code = error.exit_code();
            return report(&error, code);
        }
    };

    if session.render.format != Format::Text {
        let format = match session.render.format {
            Format::JsonRaw => Format::Json,
            other => other,
        };
        return match machine(&sprint, format) {
            Ok(text) => {
                emit(&text);
                ExitCode::Success
            }
            Err(error) => report(&error, ExitCode::Failure),
        };
    }

    // Two counts, and only when they were asked for. A sprint that Tracker
    // cannot count issues for is still a sprint worth printing: the dates are
    // the part that was read successfully, and losing them to report a failed
    // count would answer less than was already known.
    let counts = if no_issues {
        None
    } else {
        issue_counts(client, id).await
    };

    let today = jiff::Zoned::now().strftime("%Y-%m-%d").to_string();
    emit(&render::sprint(&sprint, counts, &today, &session.render));
    ExitCode::Success
}

/// `(resolved, total)` for a sprint, or nothing if either count failed.
async fn issue_counts(client: &crate::api::Client, id: &str) -> Option<(u64, u64)> {
    let total = client.count(&format!("Sprint: {id}")).await.ok()?;
    let open = client
        .count(&format!("Sprint: {id} AND Resolution: empty()"))
        .await
        .ok()?;
    Some((total.saturating_sub(open), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprint(id: &str, status: &str, start: &str) -> Sprint {
        Sprint {
            id: id.to_owned(),
            name: format!("Sprint {id}"),
            status: Some(status.to_owned()),
            start: Some(start.to_owned()),
            end: None,
            board: None,
        }
    }

    /// The whole point of the flag: new work goes into the next sprint, not the
    /// one people are in the middle of.
    #[test]
    fn a_draft_beats_the_running_sprint() {
        let sprints = [
            sprint("1", "in_progress", "2026-08-01"),
            sprint("2", "planned", "2026-08-15"),
            sprint("3", "draft", "2026-09-01"),
        ];

        assert_eq!(
            planning_sprint(&sprints).map(|sprint| sprint.id.as_str()),
            Some("2"),
            "the nearest draft, not the furthest"
        );
    }

    /// With nothing planned, "plan into the current one" is what was meant.
    #[test]
    fn without_a_draft_the_running_sprint_is_the_answer() {
        let sprints = [
            sprint("1", "archived", "2026-07-01"),
            sprint("2", "in_progress", "2026-08-01"),
        ];

        assert_eq!(
            planning_sprint(&sprints).map(|sprint| sprint.id.as_str()),
            Some("2")
        );
    }

    #[test]
    fn a_board_with_nothing_live_has_no_answer() {
        let sprints = [sprint("1", "archived", "2026-07-01")];
        assert!(planning_sprint(&sprints).is_none());
    }
}
