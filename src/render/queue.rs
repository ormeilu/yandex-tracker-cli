//! Queue listings and field tables.

use std::fmt::Write as _;

use crate::api::{Queue, QueueField};
use crate::render::Context;
use crate::render::style::Palette;

/// One line per queue: key first, because the key is what every other command
/// takes.
#[must_use]
pub fn queues(queues: &[Queue], ctx: &Context) -> String {
    let mut out = String::with_capacity(queues.len() * 48 + 32);
    let paint = ctx.painter();

    if ctx.is_human() && !queues.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            paint.paint(
                &format!("{:<12} {:<28} {}", "KEY", "NAME", "LEAD"),
                Palette::label()
            )
        );
    }

    for queue in queues {
        let _ = writeln!(
            out,
            "{} {:<28} {}",
            paint.paint_padded(&queue.key, 12, Palette::key()),
            queue.name,
            queue.lead.as_deref().unwrap_or("-"),
        );
    }

    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!("shown {} of {}", queues.len(), queues.len()),
            Palette::label()
        )
    );
    out
}

/// Field keys, types and names.
///
/// The key column comes first and is the point of the command: it is what
/// `--fields` and `--set` accept, and without it a caller is guessing. Custom
/// fields are marked, since those are the ones that differ per queue and are
/// therefore the ones worth pinning in a profile.
#[must_use]
pub fn fields(fields: &[QueueField], ctx: &Context) -> String {
    let mut out = String::with_capacity(fields.len() * 56 + 32);
    let paint = ctx.painter();

    if ctx.is_human() && !fields.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            paint.paint(
                &format!("{:<28} {:<12} {:<8} {}", "KEY", "TYPE", "ORIGIN", "NAME"),
                Palette::label()
            )
        );
    }

    for field in fields {
        // Custom fields are the reason to run this command, so they are the ones
        // that stand out.
        let origin = if field.system {
            paint.paint_padded("system", 8, Palette::label())
        } else {
            paint.paint_padded("custom", 8, Palette::warn())
        };
        let _ = writeln!(
            out,
            "{} {:<12} {} {}",
            paint.paint_padded(&field.key, 28, Palette::key()),
            field.field_type,
            origin,
            field.name,
        );
    }

    let custom = fields.iter().filter(|field| !field.system).count();
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!(
                "shown {} of {} ({custom} custom)",
                fields.len(),
                fields.len()
            ),
            Palette::label()
        )
    );
    out
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            format: crate::render::Format::Text,
            audience: crate::render::Audience::Machine,
            description_lines: Some(10),
            extra_fields: Vec::new(),
            width: 80,
        }
    }

    #[test]
    fn field_listing_marks_custom_fields_and_counts_them() {
        let listing = fields(
            &[
                QueueField {
                    key: "summary".to_owned(),
                    name: "Summary".to_owned(),
                    field_type: "string".to_owned(),
                    system: true,
                },
                QueueField {
                    key: "storyPoints".to_owned(),
                    name: "Story points".to_owned(),
                    field_type: "number".to_owned(),
                    system: false,
                },
            ],
            &ctx(),
        );

        assert!(listing.contains("summary"));
        assert!(listing.contains("system"));
        assert!(listing.contains("storyPoints"));
        assert!(listing.ends_with("shown 2 of 2 (1 custom)\n"));
    }
}
