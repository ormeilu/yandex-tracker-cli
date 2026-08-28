//! The organisation's dictionaries: issue types, priorities, statuses,
//! resolutions.
//!
//! The column order carries the point of the command. `KEY` comes first because
//! it is the value a write has to quote, and `NAME` comes second because it is
//! the value a person recognises — in a Russian organisation those two read as
//! `bug` and `Ошибка`, and printing only the second would answer the question
//! nobody asked.

use std::fmt::Write as _;

use crate::api::models::DictEntry;
use crate::api::{Dictionary, LinkType};
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, render, tally};

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

/// The kinds of link, one row per direction you could write.
///
/// The point of the command is that there are **two** vocabularies here and
/// they are not the same list. `WRITE` is what `issue link add` takes; `TYPE` is
/// the id Tracker files the link under and answers reads with. Writing a type
/// id — `depends` instead of `depends on` — is refused, and it is the mistake
/// this tool's own help shipped for several releases.
///
/// A direction with no write name is printed with a dash rather than dropped:
/// `cloners` is a real type, links of it come back from reads, and no
/// relationship in the write vocabulary produces one. Hiding the row would say
/// the type does not exist.
#[must_use]
pub fn link_types(types: &[LinkType], ctx: &Context) -> String {
    let columns = [
        Column::new("WRITE", 20, Palette::key()),
        Column::new("MEANS", 26, anstyle::Style::new()),
        Column::whole("TYPE", 12, Palette::label()),
    ];

    let mut rows = Vec::with_capacity(types.len() * 2);
    for kind in types {
        for (outward, label) in [(true, &kind.outward), (false, &kind.inward)] {
            // A type whose two directions read the same — `relates` — is one
            // relationship, not two rows saying the same thing twice.
            if !outward && kind.inward == kind.outward {
                continue;
            }
            rows.push(vec![
                relationship(&kind.id, outward).unwrap_or("-").to_owned(),
                label.clone().unwrap_or_else(|| "-".to_owned()),
                kind.id.clone(),
            ]);
        }
    }

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&tally(rows.len(), Some(rows.len() as u64), None, ctx));
    out
}

/// What `issue link add` takes for one end of one type.
///
/// Verified by writing each link into a real Tracker and reading it back: the
/// pairing is not derivable from the labels, and getting it from the wording
/// alone is how the direction of `depends` came to be inverted here for months.
/// The `epic` pair is the one that could not be checked — Tracker refuses the
/// link unless the issue really is an epic — and is taken from its labels.
fn relationship(type_id: &str, outward: bool) -> Option<&'static str> {
    Some(match (type_id, outward) {
        ("relates", _) => "relates",
        ("depends", true) => "depends on",
        ("depends", false) => "is dependent by",
        ("subtask", true) => "is parent task for",
        ("subtask", false) => "is subtask for",
        ("duplicates", true) => "is duplicated by",
        ("duplicates", false) => "duplicates",
        ("epic", true) => "has epic",
        ("epic", false) => "is epic of",
        _ => return None,
    })
}
