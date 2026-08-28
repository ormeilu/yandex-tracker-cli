//! People.
//!
//! `LOGIN` leads because it is the value every other command takes and returns;
//! the display name is what makes the row recognisable, not what makes it
//! usable.

use std::fmt::Write as _;

use crate::api::models::Person;
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, render, tally};

/// A page of the directory.
#[must_use]
pub fn users(
    people: &[Person],
    total: Option<u64>,
    next_page: Option<u32>,
    ctx: &Context,
) -> String {
    let columns = [
        Column::whole("LOGIN", 28, Palette::key()),
        Column::new("NAME", 30, anstyle::Style::new()),
        Column::new("EMAIL", 30, Palette::label()),
        // A dismissed account still owns everything it was ever assigned, so it
        // has to be listed — but assigning new work to one is a mistake, and
        // this column is the only warning of it there is.
        Column::by_value("STATE", 10, |state| match state {
            "active" => Palette::label(),
            _ => Palette::warn(),
        }),
    ];

    let rows: Vec<Vec<String>> = people
        .iter()
        .map(|person| {
            vec![
                person.login.clone(),
                person.display.clone(),
                person.email.clone().unwrap_or_else(|| "-".to_owned()),
                state(person).to_owned(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&tally(people.len(), total, next_page, ctx));
    out
}

/// One person.
#[must_use]
pub fn user(person: &Person, ctx: &Context) -> String {
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());
    let mut out = String::with_capacity(200);

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&person.login, Palette::key()),
        person.display
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}",
        label("email:"),
        person.email.as_deref().unwrap_or("-"),
        label("uid:"),
        person.uid,
    );
    let _ = writeln!(out, "{} {}", label("state:"), state(person));

    out
}

/// What to say about an account in one word.
///
/// Three states rather than two flags: an external contributor and a departed
/// colleague are different answers to "can I assign this to them", and a row
/// carrying two booleans makes the reader do that reasoning.
fn state(person: &Person) -> &'static str {
    if person.dismissed {
        "dismissed"
    } else if person.external {
        "external"
    } else {
        "active"
    }
}
