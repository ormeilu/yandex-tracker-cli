//! Queue listings and field tables.

use std::fmt::Write as _;

use crate::api::{Automation, Component, FieldSpec, Queue, QueueField, QueueSettings, Template};
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

/// How many values a constrained field lists before the rest are counted
/// rather than printed.
///
/// A field with two hundred options is a real thing, and printing all of them
/// by default makes a command nobody runs twice.
const VALUES_SHOWN: usize = 20;

/// One field: what it holds, whether it can be written, and what it accepts.
///
/// The last of those is the reason the command exists. `--set` is otherwise
/// written blind and judged by Tracker, which answers with the field's name in
/// the organisation's language and no hint of what it wanted instead.
#[must_use]
pub fn field_spec(field: &FieldSpec, all: bool, ctx: &Context) -> String {
    let mut out = String::with_capacity(240);
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());
    let yes_no = |flag: bool| if flag { "yes" } else { "no" };

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&field.key, Palette::key()),
        field.name
    );

    // `type: array` on its own says nothing a caller can act on; what the
    // elements are is the part that decides whether `--set` takes one value or
    // a list.
    let kind = match &field.items {
        Some(items) => format!("{} of {items}", field.field_type),
        None => field.field_type.clone(),
    };
    let _ = writeln!(
        out,
        "{} {kind}   {} {}   {} {}",
        label("type:"),
        label("required:"),
        yes_no(field.required),
        label("readonly:"),
        yes_no(field.readonly),
    );
    if let Some(category) = &field.category {
        let _ = writeln!(out, "{} {category}", label("category:"));
    }

    out.push_str(&values(field, all, ctx));
    out
}

/// The values half of `field get`.
fn values(field: &FieldSpec, all: bool, ctx: &Context) -> String {
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    let Some(options) = &field.options else {
        return format!("{} anything of that type\n", label("values:"));
    };

    if options.values.is_empty() {
        // A provider with no list is not an empty field: the values exist, they
        // are just kept somewhere this endpoint does not reach. Naming the
        // command that does reach them is the whole use of knowing the
        // provider's name.
        return match provider_source(&options.provider) {
            Some((what, command)) => format!("{} {what} — {command}\n", label("values:")),
            None => format!(
                "{} decided by {} — not listed by this endpoint\n",
                label("values:"),
                options.provider
            ),
        };
    }

    let mut out = String::with_capacity(64 + options.values.len() * 12);
    let shown = if all {
        options.values.len()
    } else {
        options.values.len().min(VALUES_SHOWN)
    };
    let _ = writeln!(
        out,
        "{} {}",
        label("values:"),
        options.values[..shown].join(", ")
    );
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &if shown < options.values.len() {
                format!(
                    "shown {shown} of {} values; --all for the rest",
                    options.values.len()
                )
            } else {
                format!("shown {shown} of {shown} values")
            },
            Palette::label()
        )
    );
    out
}

/// Which command answers what a provider will accept, and what it holds.
///
/// Tracker names a class; a caller wants a command. Only the providers whose
/// answer this tool can actually fetch are mapped — an unrecognised one is
/// passed through by name rather than guessed at, because a wrong command here
/// costs a request and reads like a bug.
fn provider_source(provider: &str) -> Option<(&'static str, &'static str)> {
    Some(match provider {
        "TeamOptionsProvider" => ("people in the organisation", "ytcli user list"),
        "QueueOptionsProvider" => ("queue keys", "ytcli queue list"),
        "IssueTypeOptionsProvider" => ("issue types", "ytcli dict list --kind types"),
        "PriorityOptionsProvider" => ("priorities", "ytcli dict list --kind priorities"),
        "StatusOptionsProvider" => ("statuses", "ytcli dict list --kind statuses"),
        "ResolutionOptionsProvider" => ("resolutions", "ytcli dict list --kind resolutions"),
        "VersionOptionsProvider" => ("versions of the queue", "ytcli queue versions PROJ"),
        "TagOptionsProvider" => ("tags in use in the queue", "ytcli queue tags PROJ"),
        "SprintOptionsProvider" => ("sprints", "ytcli sprint list"),
        "BoardOptionsProvider" => ("boards", "ytcli board list"),
        "ProjectOptionsProvider" => ("projects", "ytcli project list"),
        "MetaEntityOptionsProvider" => ("goals", "ytcli goal list"),
        _ => return None,
    })
}

/// The fields a queue defines itself.
///
/// Carries what each accepts, which the organisation-wide `field get` cannot
/// answer for these: a local field is not reachable through `/v3/fields`, so if
/// this listing does not say, nothing does.
#[must_use]
pub fn local_fields(queue: &str, fields: &[FieldSpec], ctx: &Context) -> String {
    let columns = [
        Column::whole("KEY", 24, Palette::key()),
        Column::whole("TYPE", 10, anstyle::Style::new()),
        Column::new("NAME", 24, anstyle::Style::new()),
        Column::new("ACCEPTS", 28, Palette::label()),
    ];
    let rows: Vec<Vec<String>> = fields
        .iter()
        .map(|field| {
            vec![
                field.key.clone(),
                match &field.items {
                    Some(items) => format!("[{items}]"),
                    None => field.field_type.clone(),
                },
                field.name.clone(),
                accepts(field),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&counted(queue, fields.len(), ctx));
    out
}

/// One cell's worth of what a field accepts.
fn accepts(field: &FieldSpec) -> String {
    match &field.options {
        None => "anything of that type".to_owned(),
        // The command, not the sentence: a cell has no room for prose, and the
        // command is the half a caller can run.
        Some(options) if options.values.is_empty() => provider_source(&options.provider)
            .map_or_else(
                || options.provider.clone(),
                |(_, command)| command.to_owned(),
            ),
        Some(options) => options.values.join(", "),
    }
}

/// The components a queue splits its work by.
///
/// The name leads, because the name is what `--set components=…` takes; the id
/// is beside it for the payloads that want one. `AUTO` is there because a
/// component that assigns automatically changes what a write does, which is not
/// something to discover afterwards.
#[must_use]
pub fn components(components: &[Component], scope: Option<&str>, ctx: &Context) -> String {
    let columns = [
        Column::new("NAME", 28, Palette::key()),
        Column::whole("ID", 8, anstyle::Style::new()),
        Column::whole("QUEUE", 12, anstyle::Style::new()),
        Column::new("LEAD", 20, anstyle::Style::new()),
        Column::by_value("AUTO", 6, |value| {
            if value == "yes" {
                Palette::warn()
            } else {
                Palette::label()
            }
        }),
    ];

    let rows: Vec<Vec<String>> = components
        .iter()
        .map(|component| {
            vec![
                component.name.clone(),
                component.id.clone(),
                component.queue.clone().unwrap_or_else(|| "-".to_owned()),
                component.lead.clone().unwrap_or_else(|| "-".to_owned()),
                if component.assign_auto { "yes" } else { "no" }.to_owned(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&match scope {
        Some(queue) => counted(queue, components.len(), ctx),
        None => tally(components.len(), Some(components.len() as u64), None, ctx),
    });
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

/// What changes issues in a queue without anybody touching them.
///
/// Three sections under their own headings, and the sections Tracker refused
/// named with its reason. A queue member cannot read triggers, and printing
/// nothing for them would say "no triggers", which is a different and wrong
/// answer.
#[must_use]
pub fn automation(queue: &str, automation: &Automation, ctx: &Context) -> String {
    let mut out = String::with_capacity(320);
    out.push_str(&macros_section(queue, automation, ctx));
    out.push('\n');
    out.push_str(&autoactions_section(queue, automation, ctx));
    out.push('\n');
    out.push_str(&triggers_section(queue, automation, ctx));
    out
}

fn heading(text: &str, ctx: &Context) -> String {
    format!("{}\n", ctx.painter().paint(text, Palette::heading()))
}

/// Canned changes somebody applies by hand.
fn macros_section(queue: &str, automation: &Automation, ctx: &Context) -> String {
    let rows: Vec<Vec<String>> = automation
        .macros
        .iter()
        .map(|entry| {
            vec![
                entry.id.clone(),
                entry.name.clone(),
                actions(&entry.updates),
                // The body is a comment somebody else wrote, so it is counted
                // rather than printed: this is a listing, and the text belongs
                // in the interface that runs it.
                if entry.body.is_some() { "yes" } else { "no" }.to_owned(),
            ]
        })
        .collect();

    let mut out = heading("macros", ctx);
    out.push_str(&render(
        &[
            Column::whole("ID", 8, Palette::key()),
            Column::new("NAME", 30, anstyle::Style::new()),
            Column::new("SETS", 24, anstyle::Style::new()),
            Column::whole("COMMENTS", 8, Palette::label()),
        ],
        &rows,
        ctx,
    ));
    out.push_str(&closing(queue, "macros", rows.len(), automation, ctx));
    out
}

/// Changes Tracker applies on a schedule to whatever matches a filter.
fn autoactions_section(queue: &str, automation: &Automation, ctx: &Context) -> String {
    let rows: Vec<Vec<String>> = automation
        .autoactions
        .iter()
        .map(|entry| {
            vec![
                entry.id.clone(),
                entry.name.clone(),
                active(entry.active).to_owned(),
                entry
                    .interval
                    .map_or_else(|| "-".to_owned(), |seconds| format!("{seconds}s")),
                actions(&entry.actions),
            ]
        })
        .collect();

    let mut out = heading("autoactions", ctx);
    out.push_str(&render(
        &[
            Column::whole("ID", 8, Palette::key()),
            Column::new("NAME", 28, anstyle::Style::new()),
            Column::by_value("ACTIVE", 8, state_colour),
            Column::whole("EVERY", 10, anstyle::Style::new()),
            Column::new("DOES", 22, anstyle::Style::new()),
        ],
        &rows,
        ctx,
    ));
    out.push_str(&closing(queue, "autoactions", rows.len(), automation, ctx));
    out
}

/// Changes Tracker applies the moment something happens to an issue.
fn triggers_section(queue: &str, automation: &Automation, ctx: &Context) -> String {
    let rows: Vec<Vec<String>> = automation
        .triggers
        .iter()
        .map(|entry| {
            vec![
                entry.id.clone(),
                entry.name.clone(),
                active(entry.active).to_owned(),
                entry.conditions.to_string(),
                actions(&entry.actions),
            ]
        })
        .collect();

    let mut out = heading("triggers", ctx);
    out.push_str(&render(
        &[
            Column::whole("ID", 8, Palette::key()),
            Column::new("NAME", 28, anstyle::Style::new()),
            Column::by_value("ACTIVE", 8, state_colour),
            Column::whole("WHEN", 10, Palette::label()),
            Column::new("DOES", 22, anstyle::Style::new()),
        ],
        &rows,
        ctx,
    ));
    out.push_str(&closing(queue, "triggers", rows.len(), automation, ctx));
    out
}

/// The last line of a section: a tally, or why there is nothing to tally.
///
/// A refused section must not end with `shown 0 of 0`. That reads as "there are
/// none", which is a different answer from "you may not see them" and the one
/// mistake this command exists to avoid.
fn closing(
    queue: &str,
    section: &str,
    count: usize,
    automation: &Automation,
    ctx: &Context,
) -> String {
    match automation
        .unreadable
        .iter()
        .find(|refusal| refusal.section == section)
    {
        Some(refusal) => {
            let paint = ctx.painter();
            format!(
                "{}\n",
                paint.paint(
                    &format!("not readable — {}", refusal.reason),
                    Palette::warn()
                )
            )
        }
        None => counted(queue, count, ctx),
    }
}

fn active(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

fn state_colour(state: &str) -> anstyle::Style {
    if state == "on" {
        Palette::ok()
    } else {
        Palette::label()
    }
}

fn actions(actions: &[String]) -> String {
    if actions.is_empty() {
        "-".to_owned()
    } else {
        actions.join(", ")
    }
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
