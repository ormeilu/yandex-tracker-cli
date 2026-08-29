//! Boards and their sprints.

use std::fmt::Write as _;

use crate::api::{Board, Sprint};
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, render, tally};

/// One line per board.
///
/// The column count rather than the columns: a listing answers "which board",
/// and the columns themselves are what `board get` is for.
#[must_use]
pub fn boards(boards: &[Board], ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 8, Palette::key()),
        Column::new("NAME", 36, anstyle::Style::new()),
        Column::whole("COLUMNS", 8, anstyle::Style::new()),
        Column::new("ESTIMATE", 14, Palette::label()),
    ];
    let rows: Vec<Vec<String>> = boards
        .iter()
        .map(|board| {
            vec![
                board.id.clone(),
                board.name.clone(),
                board.columns.len().to_string(),
                board.estimate_by.as_deref().unwrap_or("-").to_owned(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&tally(boards.len(), Some(boards.len() as u64), None, ctx));
    out
}

/// One board, with its columns in the order it arranges work by.
#[must_use]
pub fn board(board: &Board, ctx: &Context) -> String {
    let mut out = String::with_capacity(240);
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&board.id, Palette::key()),
        board.name
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}",
        label("estimate:"),
        board.estimate_by.as_deref().unwrap_or("-"),
        label("owner:"),
        board.owner.as_deref().unwrap_or("-"),
    );
    let _ = writeln!(
        out,
        "{} {}",
        label("columns:"),
        if board.columns.is_empty() {
            "-".to_owned()
        } else {
            board.columns.join(" → ")
        }
    );

    out
}

/// The glyph for a sprint's state.
///
/// Three states and three shapes, in a terminal only: a filled circle is
/// running, a half-filled one is planned, an empty one is neither. A pipe keeps
/// the word Tracker gave, because that is what a caller filters on.
fn state(status: Option<&str>, ctx: &Context) -> String {
    let word = status.unwrap_or("-");
    if !ctx.is_human() {
        return word.to_owned();
    }

    let (glyph, style) = match status {
        Some("in_progress") => ("\u{25cf}", Palette::ok()),
        Some("draft" | "planned") => ("\u{25d0}", Palette::warn()),
        _ => ("\u{25cb}", Palette::label()),
    };
    format!("{} {word}", ctx.painter().paint(glyph, style))
}

/// One sprint, and how far through it is.
///
/// Two ratios, because a sprint that is four days from its end with half its
/// issues open is a different situation from one that has just started, and a
/// list of dates makes the reader do that arithmetic themselves.
///
/// `counts` is `(resolved, total)`, and absent when the issues were not asked
/// for: two counts are two requests, and a caller who only wanted the dates
/// should not pay for them.
#[must_use]
pub fn sprint(sprint: &Sprint, counts: Option<(u64, u64)>, today: &str, ctx: &Context) -> String {
    let mut out = String::with_capacity(240);
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&sprint.id, Palette::key()),
        sprint.name
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}",
        label("status:"),
        state(sprint.status.as_deref(), ctx),
        label("board:"),
        sprint.board.as_deref().unwrap_or("-"),
    );

    let start = sprint.start.as_deref().unwrap_or("-");
    let end = sprint.end.as_deref().unwrap_or("-");
    let _ = write!(out, "{} {start} \u{2192} {end}", label("dates:"));
    if let Some((elapsed, length)) = days(sprint.start.as_deref(), sprint.end.as_deref(), today) {
        let _ = write!(
            out,
            "   {} days",
            crate::render::bar::ratio(elapsed, length, ctx)
        );
    }
    let _ = writeln!(out);

    if let Some((resolved, total)) = counts {
        let _ = writeln!(
            out,
            "{} {} resolved",
            label("issues:"),
            crate::render::bar::ratio(resolved, total, ctx)
        );
    }

    out
}

/// Days elapsed of days planned, from the dates as Tracker writes them.
///
/// Both ends are counted, so a one-day sprint is one day long rather than zero.
/// A sprint that has run over its end date reports more elapsed than planned,
/// and the bar caps itself; pretending otherwise would hide the thing worth
/// noticing.
fn days(start: Option<&str>, end: Option<&str>, today: &str) -> Option<(u64, u64)> {
    let start: jiff::civil::Date = start?.parse().ok()?;
    let end: jiff::civil::Date = end?.parse().ok()?;
    let today: jiff::civil::Date = today.parse().ok()?;

    let length = (end - start).get_days().checked_add(1)?;
    let elapsed = (today - start).get_days().checked_add(1)?;
    u64::try_from(length)
        .ok()
        .map(|length| (u64::try_from(elapsed).unwrap_or(0), length))
}

/// Every sprint in the organisation, with the board each belongs to.
///
/// The board column is the difference from [`sprints`]: two boards each having
/// a "Sprint 1" is normal, and without it the listing would be a set of names
/// nobody could act on.
#[must_use]
pub fn all_sprints(sprints: &[Sprint], ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 8, Palette::key()),
        Column::new("NAME", 26, anstyle::Style::new()),
        Column::new("BOARD", 20, Palette::label()),
        Column::new("STATUS", 12, anstyle::Style::new()),
        Column::whole("START", 12, anstyle::Style::new()),
        Column::whole("END", 12, anstyle::Style::new()),
    ];
    let rows: Vec<Vec<String>> = sprints
        .iter()
        .map(|sprint| {
            vec![
                sprint.id.clone(),
                sprint.name.clone(),
                sprint.board.as_deref().unwrap_or("-").to_owned(),
                sprint.status.as_deref().unwrap_or("-").to_owned(),
                sprint.start.as_deref().unwrap_or("-").to_owned(),
                sprint.end.as_deref().unwrap_or("-").to_owned(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&tally(sprints.len(), Some(sprints.len() as u64), None, ctx));
    out
}

/// The sprints of a board.
#[must_use]
pub fn sprints(board: &str, sprints: &[Sprint], ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 8, Palette::key()),
        Column::new("NAME", 30, anstyle::Style::new()),
        Column::new("STATUS", 14, anstyle::Style::new()),
        Column::whole("START", 12, anstyle::Style::new()),
        Column::whole("END", 12, anstyle::Style::new()),
    ];
    let rows: Vec<Vec<String>> = sprints
        .iter()
        .map(|sprint| {
            vec![
                sprint.id.clone(),
                sprint.name.clone(),
                sprint.status.as_deref().unwrap_or("-").to_owned(),
                sprint.start.as_deref().unwrap_or("-").to_owned(),
                sprint.end.as_deref().unwrap_or("-").to_owned(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    let paint = ctx.painter();
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!(
                "shown {} of {} for board {board}",
                sprints.len(),
                sprints.len()
            ),
            Palette::label()
        )
    );
    out
}
