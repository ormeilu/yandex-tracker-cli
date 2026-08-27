//! Projects, goals and attachments.

use std::fmt::Write as _;

use crate::api::models::{Attachment, Entity, Page};
use crate::render::Context;
use crate::render::style::Palette;

/// A listing of projects or goals.
///
/// The short id leads, because that is the number an issue's `project` field
/// refers to; the long id follows, because that is what `project get` takes.
/// Printing only one of them guarantees somebody uses the wrong one.
#[must_use]
pub fn entities(page: &Page<Entity>, ctx: &Context) -> String {
    let mut out = String::with_capacity(page.items.len() * 72 + 32);
    let paint = ctx.painter();

    if ctx.is_human() && !page.items.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            paint.paint(
                &format!("{:<8} {:<26} {:<14} {}", "SHORT", "ID", "STATUS", "SUMMARY"),
                Palette::label()
            )
        );
    }

    for entity in &page.items {
        let _ = writeln!(
            out,
            "{} {} {:<14} {}",
            paint.paint_padded(
                &entity
                    .short_id
                    .map_or_else(|| "-".to_owned(), |id| id.to_string()),
                8,
                Palette::key()
            ),
            paint.paint_padded(&truncate(&entity.id, 26), 26, Palette::label()),
            truncate(entity.status.as_deref().unwrap_or("-"), 14),
            truncate(&entity.summary, 50),
        );
    }

    let shown = page.items.len();
    let tally = match page.total {
        Some(total) => format!("shown {shown} of {total}"),
        None => format!("shown {shown} of unknown total"),
    };
    let _ = write!(out, "{}", paint.paint(&tally, Palette::label()));

    if page.has_more() {
        let _ = write!(
            out,
            "{}",
            paint.paint(
                &format!(" — next: --page {}", page.page + 1),
                Palette::warn()
            )
        );
    }
    out.push('\n');
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
        let _ = writeln!(out, "{}", label("---"));
        let _ = writeln!(
            out,
            "{}",
            paint.paint(
                &crate::render::untrusted::fence(&format!("{}/description", entity.id), &body),
                Palette::untrusted()
            )
        );
        if withheld > 0 {
            let _ = writeln!(
                out,
                "{}",
                label(&format!("(+{withheld} more lines: --full)"))
            );
        }
    }

    out
}

/// Attachments of an issue.
///
/// The filename was chosen by whoever uploaded it, so it is fenced: a name can
/// carry as much text as a comment can.
#[must_use]
pub fn attachments(key: &str, attachments: &[Attachment], ctx: &Context) -> String {
    let mut out = String::with_capacity(attachments.len() * 72 + 32);
    let paint = ctx.painter();

    if ctx.is_human() && !attachments.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            paint.paint(
                &format!("{:<14} {:<10} {:<18} {}", "ID", "SIZE", "TYPE", "NAME"),
                Palette::label()
            )
        );
    }

    for attachment in attachments {
        let _ = writeln!(
            out,
            "{} {:<10} {:<18} {}",
            paint.paint_padded(&attachment.id, 14, Palette::key()),
            attachment.size.map_or_else(|| "-".to_owned(), human_size),
            truncate(attachment.mimetype.as_deref().unwrap_or("-"), 18),
            // The filename was chosen by whoever uploaded it, so it does not get
            // the styling our own output uses.
            paint.paint(&attachment.name, Palette::untrusted()),
        );
    }

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

fn human_size(bytes: u64) -> String {
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

fn truncate(value: &str, width: usize) -> String {
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

    #[test]
    fn sizes_are_readable_without_losing_small_ones() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }
}
