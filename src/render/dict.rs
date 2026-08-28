//! The organisation's dictionaries: issue types, priorities, statuses,
//! resolutions.
//!
//! The column order carries the point of the command. `KEY` comes first because
//! it is the value a write has to quote, and `NAME` comes second because it is
//! the value a person recognises — in a Russian organisation those two read as
//! `bug` and `Ошибка`, and printing only the second would answer the question
//! nobody asked.

use std::fmt::Write as _;

use crate::api::Dictionary;
use crate::api::models::DictEntry;
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, render};

/// One dictionary, under a heading naming which.
#[must_use]
pub fn one(kind: Dictionary, entries: &[DictEntry], ctx: &Context) -> String {
    let paint = ctx.painter();
    let mut out = String::with_capacity(64 + entries.len() * 48);

    let _ = writeln!(out, "{}", paint.paint(kind.label(), Palette::heading()));
    out.push_str(&rows(kind, entries, ctx));
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!("shown {} of {}", entries.len(), entries.len()),
            Palette::label()
        )
    );
    out
}

/// Several dictionaries in one answer, each under its own heading.
///
/// Separated by a blank line so the sections stay tellable apart in a pipe,
/// where there is no colour to do it.
#[must_use]
pub fn many(sections: &[(Dictionary, Vec<DictEntry>)], ctx: &Context) -> String {
    let mut out = String::new();
    for (index, (kind, entries)) in sections.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&one(*kind, entries, ctx));
    }
    out
}

fn rows(kind: Dictionary, entries: &[DictEntry], ctx: &Context) -> String {
    // Only statuses have a category, and a column of dashes on the other three
    // would be three quarters noise.
    let categorised = kind == Dictionary::Statuses;

    let columns: Vec<Column> = if categorised {
        vec![
            Column::whole("KEY", 20, Palette::key()),
            Column::new("NAME", 32, anstyle::Style::new()),
            Column::new("CATEGORY", 12, Palette::label()),
        ]
    } else {
        vec![
            Column::whole("KEY", 20, Palette::key()),
            Column::new("NAME", 32, anstyle::Style::new()),
        ]
    };

    let rows: Vec<Vec<String>> = entries
        .iter()
        .map(|entry| {
            let mut row = vec![entry.key.clone(), entry.name.clone()];
            if categorised {
                row.push(entry.category.clone().unwrap_or_else(|| "-".to_owned()));
            }
            row
        })
        .collect();

    render(&columns, &rows, ctx)
}
