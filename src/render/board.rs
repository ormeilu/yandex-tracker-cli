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
