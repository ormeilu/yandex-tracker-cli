//! Projects, goals and attachments.

use std::fmt::Write as _;

use crate::api::models::{Attachment, Entity, Page};
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, render, tally};

/// A listing of projects or goals.
///
/// The short id leads, because that is the number an issue's `project` field
/// refers to; the long id follows, because that is what `project get` takes.
/// Printing only one of them guarantees somebody uses the wrong one.
#[must_use]
pub fn entities(page: &Page<Entity>, ctx: &Context) -> String {
    let columns = [
        Column::whole("SHORT", 8, Palette::key()),
        Column::new("ID", 26, Palette::label()),
        Column::new("STATUS", 14, anstyle::Style::new()),
        Column::new("SUMMARY", 50, anstyle::Style::new()),
    ];
    let rows: Vec<Vec<String>> = page
        .items
        .iter()
        .map(|entity| {
            vec![
                entity
                    .short_id
                    .map_or_else(|| "-".to_owned(), |id| id.to_string()),
                entity.id.clone(),
                entity.status.as_deref().unwrap_or("-").to_owned(),
                entity.summary.clone(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&tally(
        page.items.len(),
        page.total,
        page.has_more().then_some(page.page + 1),
        ctx,
    ));
    out
}

/// One project or goal.
#[must_use]
pub fn entity(entity: &Entity, ctx: &Context) -> String {
    let mut out = String::with_capacity(320);
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&entity.id, Palette::key()),
        entity.summary
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}   {} {}",
        label("short id:"),
        paint.paint(
            &entity
                .short_id
                .map_or_else(|| "-".to_owned(), |id| id.to_string()),
            Palette::key()
        ),
        label("status:"),
        entity.status.as_deref().unwrap_or("-"),
        label("lead:"),
        entity
            .lead
            .as_ref()
            .and_then(|lead| lead.login.as_deref().or(lead.display.as_deref()))
            .unwrap_or("-"),
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}",
        label("start:"),
        entity.start.as_deref().unwrap_or("-"),
        label("end:"),
        entity.end.as_deref().unwrap_or("-"),
    );

    if let Some(description) = entity.description.as_deref().filter(|d| !d.is_empty()) {
        let (body, withheld) = crate::render::untrusted::head(description, ctx.description_lines);
        crate::render::text::quoted_block(
            &mut out,
            &format!("{}/description", entity.id),
            &body,
            withheld,
            ctx,
        );
    }

    out
}

/// Attachments of an issue.
///
/// The filename was chosen by whoever uploaded it, so it does not get the
/// styling our own output uses: a name carries as much text as a comment can.
#[must_use]
pub fn attachments(key: &str, attachments: &[Attachment], ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 14, Palette::key()),
        Column::whole("SIZE", 10, anstyle::Style::new()),
        Column::new("TYPE", 18, anstyle::Style::new()),
        Column::whole("NAME", 40, Palette::untrusted()),
    ];
    let rows: Vec<Vec<String>> = attachments
        .iter()
        .map(|attachment| {
            vec![
                attachment.id.clone(),
                attachment.size.map_or_else(|| "-".to_owned(), human_size),
                attachment.mimetype.as_deref().unwrap_or("-").to_owned(),
                attachment.name.clone(),
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
                "shown {} of {} for {key}",
                attachments.len(),
                attachments.len()
            ),
            Palette::label()
        )
    );
    out
}

/// A byte count in the units a person reads.
#[must_use]
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    // Precision beyond 2^52 bytes is not a concern for a file size, and the
    // value is only ever rendered to one decimal place.
    #[allow(clippy::cast_precision_loss)]
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_readable_without_losing_small_ones() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }
}
