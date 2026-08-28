//! Queue listings and field tables.

use std::fmt::Write as _;

use crate::api::{Queue, QueueField, QueueSettings, Template};
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

/// The versions a queue defines.
#[must_use]
pub fn versions(queue: &str, versions: &[crate::api::Version], ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 10, Palette::key()),
        Column::new("NAME", 28, anstyle::Style::new()),
        // A released version and an archived one are both "not open", and
        // which of the two decides whether new work may still point at it.
        Column::by_value("STATE", 10, |state| match state {
            "open" => Palette::ok(),
            _ => Palette::label(),
        }),
        Column::whole("DUE", 12, anstyle::Style::new()),
    ];

    let rows: Vec<Vec<String>> = versions
        .iter()
        .map(|version| {
            vec![
                version.id.clone(),
                version.name.clone(),
                version.state.to_owned(),
                version.due.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&counted(queue, versions.len(), ctx));
    out
}

/// The tags in use in a queue.
#[must_use]
pub fn tags(queue: &str, tags: &[String], ctx: &Context) -> String {
    let columns = [Column::whole("TAG", 30, Palette::key())];
    let rows: Vec<Vec<String>> = tags.iter().map(|tag| vec![tag.clone()]).collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&counted(queue, tags.len(), ctx));
    out
}

/// `shown N of N for QUEUE` — neither endpoint pages, so both numbers are the
/// same one, and saying so is still better than leaving the reader to wonder.
fn counted(queue: &str, count: usize, ctx: &Context) -> String {
    let paint = ctx.painter();
    format!(
        "{}\n",
        paint.paint(
            &format!("shown {count} of {count} for {queue}"),
            Palette::label()
        )
    )
}

/// One queue and the defaults an issue created in it starts with.
#[must_use]
pub fn settings(queue: &QueueSettings, ctx: &Context) -> String {
    let mut out = String::with_capacity(200);
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&queue.key, Palette::key()),
        queue.name
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}   {} {}",
        label("lead:"),
        queue.lead.as_deref().unwrap_or("-"),
        label("default type:"),
        queue.default_type.as_deref().unwrap_or("-"),
        label("default priority:"),
        queue.default_priority.as_deref().unwrap_or("-"),
    );

    out
}

/// Issue or comment templates.
///
/// The id leads because it is what a caller passes on; the queue follows,
/// because a template that belongs to a queue only applies there.
#[must_use]
pub fn templates(templates: &[Template], ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 12, Palette::key()),
        Column::new("NAME", 36, anstyle::Style::new()),
        Column::whole("QUEUE", 12, anstyle::Style::new()),
        Column::new("AUTHOR", 18, Palette::label()),
    ];
    let rows: Vec<Vec<String>> = templates
        .iter()
        .map(|template| {
            vec![
                template.id.clone(),
                template.name.clone(),
                template.queue.as_deref().unwrap_or("-").to_owned(),
                template.author.as_deref().unwrap_or("-").to_owned(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&tally(
        templates.len(),
        Some(templates.len() as u64),
        None,
        ctx,
    ));
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
            images: false,
            inline: crate::render::image::Inline::default(),
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
