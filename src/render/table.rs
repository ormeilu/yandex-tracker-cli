//! Listings, in the two shapes their two readers need.
//!
//! A row is built once and formatted twice. That is the whole point of this
//! module: the data a terminal shows and the data a pipe shows are the same
//! values in the same order, and only the arrangement differs (ADR 3).
//!
//! **A pipe gets fixed-width columns.** Every run of a command produces the same
//! byte offsets whatever the window is and whatever the rows contain, which is
//! what makes the output safe to `cut`, to diff, and to cache.
//!
//! **A terminal gets a table sized to its contents**, because a person is not
//! parsing byte offsets and a column padded to a width nothing in it uses is
//! just wasted screen. Columns shrink to fit the window, widest first, so the
//! keys stay readable when the summaries do not fit.

use std::fmt::Write as _;

use anstyle::Style;
use tabled::builder::Builder;
use tabled::settings::peaker::Priority;
use tabled::settings::{Padding, Width};

use crate::render::Context;
use crate::render::style::{Painter, Palette};

/// How a column's cells are painted.
///
/// Some columns say something by their value rather than by their position — a
/// field that is custom rather than system is the reason to run the command that
/// lists it — so the style can depend on the cell. Only a terminal ever sees the
/// difference; a pipe is never painted at all.
#[derive(Clone, Copy)]
pub enum Paint {
    Fixed(Style),
    ByValue(fn(&str) -> Style),
}

impl std::fmt::Debug for Paint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(style) => f.debug_tuple("Fixed").field(style).finish(),
            Self::ByValue(_) => f.write_str("ByValue(..)"),
        }
    }
}

impl Paint {
    fn style(self, cell: &str) -> Style {
        match self {
            Self::Fixed(style) => style,
            Self::ByValue(pick) => pick(cell),
        }
    }
}

/// One column: how a pipe lays it out, and how it is painted.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    pub header: &'static str,
    /// Width a pipe pads or cuts this column to.
    pub width: usize,
    /// Cut a value that is too long. A key is never cut — a truncated
    /// identifier is not an identifier.
    pub truncate: bool,
    pub paint: Paint,
}

impl Column {
    #[must_use]
    pub const fn new(header: &'static str, width: usize, style: Style) -> Self {
        Self {
            header,
            width,
            truncate: true,
            paint: Paint::Fixed(style),
        }
    }

    /// A column whose values are never cut.
    #[must_use]
    pub const fn whole(header: &'static str, width: usize, style: Style) -> Self {
        Self {
            truncate: false,
            ..Self::new(header, width, style)
        }
    }

    /// A column painted from its own value.
    #[must_use]
    pub const fn by_value(header: &'static str, width: usize, pick: fn(&str) -> Style) -> Self {
        Self {
            header,
            width,
            truncate: true,
            paint: Paint::ByValue(pick),
        }
    }
}

/// Render rows as a listing, without the tally that follows them.
#[must_use]
pub fn render(columns: &[Column], rows: &[Vec<String>], ctx: &Context) -> String {
    if ctx.is_human() {
        human(columns, rows, ctx)
    } else {
        machine(columns, rows)
    }
}

/// Fixed-width columns, separated by one space.
///
/// The last column is never padded: trailing spaces are invisible until
/// something copies them.
fn machine(columns: &[Column], rows: &[Vec<String>]) -> String {
    let mut out = String::with_capacity(rows.len() * 80);

    for row in rows {
        let mut line = String::with_capacity(80);
        for (index, cell) in row.iter().enumerate() {
            let Some(column) = columns.get(index) else {
                continue;
            };
            let value = if column.truncate {
                truncate(cell, column.width)
            } else {
                cell.clone()
            };
            if index + 1 == row.len() {
                line.push_str(&value);
            } else {
                let _ = write!(
                    line,
                    "{value}{} ",
                    " ".repeat(column.width.saturating_sub(value.chars().count()))
                );
            }
        }
        let _ = writeln!(out, "{}", line.trim_end());
    }

    out
}

/// A table sized to its contents, shrunk to the window if it does not fit.
fn human(columns: &[Column], rows: &[Vec<String>], ctx: &Context) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let paint = ctx.painter();

    let mut builder = Builder::with_capacity(rows.len() + 1, columns.len());
    builder.push_record(
        columns
            .iter()
            .map(|column| paint.paint(column.header, Palette::label())),
    );
    for row in rows {
        builder.push_record(paint_row(columns, row, paint));
    }

    let mut table = builder.build();
    table
        .with(tabled::settings::Style::blank())
        .with(Padding::new(0, 2, 0, 0));

    // Shrink the widest columns first: a cut summary is still useful, a cut key
    // is not.
    table.with(
        Width::truncate(ctx.width)
            .suffix("…")
            .priority(Priority::max(true)),
    );

    // tabled pads the last column out to the table width; those spaces are
    // invisible until something copies them.
    let mut out = String::with_capacity(rows.len() * 96);
    for line in table.to_string().lines() {
        let _ = writeln!(out, "{}", line.trim_end());
    }
    out
}

fn paint_row(columns: &[Column], row: &[String], paint: Painter) -> Vec<String> {
    row.iter()
        .enumerate()
        .map(|(index, cell)| match columns.get(index) {
            Some(column) => paint.paint(cell, column.paint.style(cell)),
            None => cell.clone(),
        })
        .collect()
}

/// The `shown N of M` line every listing ends with, and the next page when one
/// exists.
///
/// Never optional. A caller that receives 25 rows and cannot tell a complete
/// answer from a truncated one will eventually conclude there is nothing to
/// find, which is a worse failure than any number of wasted tokens.
#[must_use]
pub fn tally(shown: usize, total: Option<u64>, next_page: Option<u32>, ctx: &Context) -> String {
    let paint = ctx.painter();
    let counted = match total {
        Some(total) => format!("shown {shown} of {total}"),
        None => format!("shown {shown} of unknown total"),
    };

    let mut out = paint.paint(&counted, Palette::label());
    if let Some(page) = next_page {
        out.push_str(&paint.paint(&format!(" — next: --page {page}"), Palette::warn()));
    }
    out.push('\n');
    out
}

pub(crate) fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let mut kept: String = value.chars().take(width.saturating_sub(1)).collect();
    kept.push('…');
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Audience, Format};

    fn ctx(audience: Audience) -> Context {
        Context {
            format: Format::Text,
            audience,
            description_lines: None,
            extra_fields: Vec::new(),
            width: 80,
        }
    }

    fn columns() -> Vec<Column> {
        vec![
            Column::whole("KEY", 12, Palette::key()),
            Column::new("SUMMARY", 40, Style::new()),
        ]
    }

    fn rows() -> Vec<Vec<String>> {
        vec![
            vec!["PROJ-1".to_owned(), "short".to_owned()],
            vec!["PROJ-22".to_owned(), "a longer summary".to_owned()],
        ]
    }

    /// The promise of the fixed-width form: a column starts at the same offset
    /// on every row, whatever the row contains.
    #[test]
    fn a_pipe_puts_every_column_at_a_fixed_offset() {
        let out = machine(&columns(), &rows());
        for (line, row) in out.lines().zip(rows()) {
            let summary: String = line.chars().skip(13).collect();
            assert_eq!(summary, row[1], "the second column moved");
        }
    }

    #[test]
    fn a_pipe_gets_no_trailing_padding() {
        let out = machine(&columns(), &rows());
        assert!(out.lines().all(|line| !line.ends_with(' ')));
    }

    /// The rule that makes two renderings of one row safe: same values, same
    /// order, whatever the decoration.
    #[test]
    fn both_forms_carry_the_same_values() {
        let piped = machine(&columns(), &rows());
        let terminal = human(&columns(), &rows(), &ctx(Audience::Human));

        for row in rows() {
            for cell in row {
                assert!(piped.contains(&cell), "{cell} missing from the pipe form");
                assert!(
                    terminal.contains(&cell),
                    "{cell} missing from the terminal form"
                );
            }
        }
    }

    /// A key is an identifier a caller types back. Cutting one produces
    /// something that looks like a key and is not.
    #[test]
    fn a_key_is_never_cut() {
        let long = vec![vec!["PROJECT-1234567890".to_owned(), "summary".to_owned()]];
        assert!(machine(&columns(), &long).contains("PROJECT-1234567890"));
    }

    #[test]
    fn an_over_long_value_is_cut_with_an_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }

    #[test]
    fn a_terminal_table_stays_inside_the_window() {
        let wide = vec![vec!["PROJ-1".to_owned(), "x".repeat(400)]];
        let narrow = Context {
            width: 40,
            ..ctx(Audience::Human)
        };
        let out = human(&columns(), &wide, &narrow);
        assert!(
            out.lines()
                .all(|line| strip_ansi(line).chars().count() <= 40),
            "a line ran past the window"
        );
    }

    fn strip_ansi(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\u{1b}' {
                out.push(c);
                continue;
            }
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        out
    }

    #[test]
    fn the_tally_names_the_next_page_when_there_is_one() {
        let ctx = ctx(Audience::Machine);
        assert_eq!(
            tally(25, Some(340), Some(2), &ctx),
            "shown 25 of 340 — next: --page 2\n"
        );
        assert_eq!(tally(1, Some(1), None, &ctx), "shown 1 of 1\n");
        assert_eq!(tally(1, None, None, &ctx), "shown 1 of unknown total\n");
    }
}
