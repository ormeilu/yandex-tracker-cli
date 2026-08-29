//! The answer to a change that touched more than one issue.
//!
//! A bulk change is one request that ends in a count, so the tally rule applies
//! to it directly: `changed N of M`. What it cannot say on its own is *which*
//! ones, and that is only worth asking for when the counts do not already
//! answer it — printing a line per issue that succeeded would spend the saving
//! the command exists to make.

use std::fmt::Write as _;

use crate::api::{BulkChange, BulkOutcome};
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, render};

/// The tally a change to several issues ends with, whichever way it was made.
///
/// One request or fifty, the caller is owed the same sentence: how many of the
/// issues they named actually changed.
#[must_use]
pub fn changed(done: u64, total: u64, ctx: &Context) -> String {
    let colour = if done == total {
        Palette::label()
    } else {
        Palette::warn()
    };
    format!(
        "{}\n",
        ctx.painter()
            .paint(&format!("changed {done} of {total}"), colour)
    )
}

/// The tally, and the id that outlives the command.
///
/// The id is printed on every outcome rather than only on failure: it is the
/// only handle on work Tracker is still doing, and a caller who did not keep it
/// has no way back to the answer.
#[must_use]
pub fn change(change: &BulkChange, ctx: &Context) -> String {
    let paint = ctx.painter();
    let mut out = String::with_capacity(96);

    let counted = match (change.done, change.total) {
        (Some(done), Some(total)) => format!("changed {done} of {total}"),
        // Before Tracker has counted the issues there is no tally to print, and
        // inventing one from the keys we sent would be our number, not its.
        _ => format!("{} — not counted yet", change.status.to_lowercase()),
    };

    let colour = if change.succeeded() {
        Palette::label()
    } else {
        Palette::warn()
    };
    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&counted, colour),
        paint.paint(&format!("bulkchange {}", change.id), Palette::key())
    );

    // Tracker's own sentence, kept whenever it is saying something other than
    // "fine": it is in the organisation's language and is the only wording that
    // will match what the web interface shows.
    if !change.succeeded() && !change.status_text.is_empty() {
        let _ = writeln!(out, "{}", paint.paint(&change.status_text, Palette::warn()));
    }
    out
}

/// One line per issue that did not change, and Tracker's reason for each.
///
/// Only the failures: the ones that worked are in the tally, and repeating them
/// would make the output grow with the size of the change.
#[must_use]
pub fn failures(outcomes: &[BulkOutcome], ctx: &Context) -> String {
    let rows: Vec<Vec<String>> = outcomes
        .iter()
        .filter(|outcome| outcome.status != "COMPLETE")
        .map(|outcome| {
            vec![
                outcome.key.clone(),
                outcome.status.to_lowercase(),
                outcome.error.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect();

    if rows.is_empty() {
        return String::new();
    }

    render(
        &[
            Column::whole("KEY", 14, Palette::key()),
            Column::whole("STATUS", 10, Palette::warn()),
            Column::new("WHY", 48, anstyle::Style::new()),
        ],
        &rows,
        ctx,
    )
}
