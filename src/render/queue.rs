//! Queue listings and field tables.

use std::fmt::Write as _;

use crate::api::{Queue, QueueField};
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, render, tally};

/// One line per queue: key first, because the key is what every other command
/// takes.
#[must_use]
pub fn queues(queues: &[Queue], ctx: &Context) -> String {
    let columns = [
        Column::whole("KEY", 12, Palette::key()),
        Column::whole("NAME", 28, anstyle::Style::new()),
        Column::whole("LEAD", 20, anstyle::Style::new()),
    ];
    let rows: Vec<Vec<String>> = queues
        .iter()
        .map(|queue| {
            vec![
                queue.key.clone(),
                queue.name.clone(),
                queue.lead.as_deref().unwrap_or("-").to_owned(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&tally(queues.len(), Some(queues.len() as u64), None, ctx));
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
    let columns = [
        Column::whole("KEY", 28, Palette::key()),
        Column::whole("TYPE", 12, anstyle::Style::new()),
        // Custom fields are the reason to run this command, so they are the ones
        // that stand out.
        Column::by_value("ORIGIN", 8, |origin| {
            if origin == "custom" {
                Palette::warn()
            } else {
                Palette::label()
            }
        }),
        Column::whole("NAME", 30, anstyle::Style::new()),
    ];
    let rows: Vec<Vec<String>> = fields
        .iter()
        .map(|field| {
            vec![
                field.key.clone(),
                field.field_type.clone(),
                if field.system { "system" } else { "custom" }.to_owned(),
                field.name.clone(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);

    let custom = fields.iter().filter(|field| !field.system).count();
    let paint = ctx.painter();
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
