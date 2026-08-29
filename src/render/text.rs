//! The compact text view.
//!
//! Target shape for `issue get`, roughly fifteen lines instead of a five-kilobyte
//! payload. Links are always present: "what blocks this" is the question that
//! follows "what is this", and making the caller run a second command for it
//! costs more than the four lines it saves.

use std::fmt::Write as _;

use crate::api::models::{
    Change, ChecklistItem, Comment, Issue, Link, Page, RemoteLink, User, Worklog,
};
use crate::render::style::Palette;
use crate::render::table::Column;
use crate::render::{Context, untrusted};

fn who(user: Option<&User>) -> &str {
    user.and_then(|u| u.login.as_deref().or(u.display.as_deref()))
        .unwrap_or("-")
}

fn or_dash(value: Option<&String>) -> &str {
    value.map_or("-", String::as_str)
}

/// Render one issue.
#[must_use]
pub fn issue(issue: &Issue, ctx: &Context) -> String {
    let mut out = String::with_capacity(512);
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&issue.key, Palette::key()),
        issue.summary
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}   {} {}",
        label("status:"),
        status_painted(issue.status.as_deref(), issue.status_key.as_deref(), ctx),
        label("type:"),
        or_dash(issue.issue_type.as_ref()),
        label("prio:"),
        priority_painted(
            issue.priority.as_deref(),
            issue.priority_key.as_deref(),
            ctx
        ),
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}   {} {}",
        label("assignee:"),
        who(issue.assignee.as_ref()),
        label("author:"),
        who(issue.author.as_ref()),
        label("queue:"),
        paint.paint(or_dash(issue.queue.as_ref()), Palette::key()),
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}",
        label("updated:"),
        issue
            .updated_at
            .map_or_else(|| "-".to_owned(), |ts| ts.to_string()),
        label("comments:"),
        issue
            .comment_count
            .map_or_else(|| "-".to_owned(), |n| n.to_string()),
    );

    custom_fields(&mut out, issue, ctx);

    links_section(&mut out, issue, ctx);
    description_section(&mut out, issue, ctx);

    out
}

/// Render the links of an issue on their own.
///
/// Same one-line-per-link shape as the compact view, so a caller that has seen
/// one has seen both.
#[must_use]
pub fn links(key: &str, links: &[Link]) -> String {
    let mut out = String::with_capacity(links.len() * 40 + 32);

    for link in links {
        // The id leads, as it does in every other listing whose rows can be
        // deleted: `issue link delete` takes it, and this is where the help
        // says to find it.
        let _ = writeln!(
            out,
            "{}  {} {}{}{}",
            link.id,
            relation_of(link),
            link.key,
            link.status
                .as_ref()
                .map_or_else(String::new, |status| format!(" [{status}]")),
            link.summary
                .as_ref()
                .map_or_else(String::new, |summary| format!("  {summary}")),
        );
    }

    let _ = writeln!(out, "shown {} of {} for {key}", links.len(), links.len());
    out
}

/// Render the links that leave Tracker.
///
/// A table rather than the one-line shape `links` uses: an issue link is
/// identified by a key the reader already understands, and one of these is
/// identified by an application they may not, so the application needs a column
/// of its own rather than a parenthesis.
#[must_use]
pub fn remote_links(key: &str, links: &[RemoteLink], ctx: &Context) -> String {
    let columns = [
        Column::new("RELATION", 16, Palette::label()),
        Column::new("APPLICATION", 20, anstyle::Style::new()),
        Column::whole("KEY", 16, Palette::key()),
        // Written elsewhere, by somebody outside this organisation's Tracker.
        Column::new("TITLE", 32, Palette::untrusted()),
    ];

    let rows: Vec<Vec<String>> = links
        .iter()
        .map(|link| {
            vec![
                link.relation.clone().unwrap_or_else(|| "-".to_owned()),
                link.application.clone().unwrap_or_else(|| "-".to_owned()),
                link.key.clone().unwrap_or_else(|| "-".to_owned()),
                link.title.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect();

    let mut out = crate::render::table::render(&columns, &rows, ctx);
    let paint = ctx.painter();
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!("shown {} of {} for {key}", links.len(), links.len()),
            Palette::label()
        )
    );
    out
}

/// Render comments, each fenced with its own source.
///
/// The fence names the comment and its author, so a reader can tell which part
/// of the output someone else wrote — the whole point of the marking (ADR 1).
#[must_use]
pub fn comments(key: &str, comments: &[Comment], ctx: &Context) -> String {
    let mut out = String::with_capacity(comments.len() * 160 + 32);
    let paint = ctx.painter();

    for comment in comments {
        let author = who(comment.author.as_ref());
        let when = comment
            .created_at
            .map_or_else(|| "-".to_owned(), |ts| ts.to_string());
        let _ = writeln!(
            out,
            "{}",
            paint.paint(
                &format!("--- {} by {author} at {when}", comment.id),
                Palette::label()
            )
        );
        quoted_block(
            &mut out,
            &format!("{key}/comment/{} by {author}", comment.id),
            &comment.text,
            0,
            ctx,
        );
    }

    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!("shown {} of {} for {key}", comments.len(), comments.len()),
            Palette::label()
        )
    );
    out
}

/// Render an issue's worklog.
///
/// The duration is shown the way it is typed — `1h 30m`, not `PT1H30M`. The
/// ISO form is what `--format json` carries, because that is the API's own
/// vocabulary and what a script is written against.
#[must_use]
pub fn worklogs(key: &str, worklogs: &[Worklog], ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 12, Palette::key()),
        Column::whole("DURATION", 10, anstyle::Style::new()),
        Column::whole("WHEN", 12, anstyle::Style::new()),
        Column::new("WHO", 16, anstyle::Style::new()),
        Column::new("COMMENT", 40, Palette::untrusted()),
    ];

    let rows: Vec<Vec<String>> = worklogs
        .iter()
        .map(|entry| {
            vec![
                entry.id.clone(),
                crate::api::duration::human(&entry.duration),
                entry.start.map_or_else(
                    || "-".to_owned(),
                    |start| start.to_string().chars().take(10).collect(),
                ),
                who(entry.author.as_ref()).to_owned(),
                entry.comment.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect();

    let mut out = crate::render::table::render(&columns, &rows, ctx);
    let paint = ctx.painter();
    let total = crate::api::duration::human(&total_duration(worklogs));
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!(
                "shown {} of {} for {key} — {total} total",
                rows.len(),
                rows.len()
            ),
            Palette::label()
        )
    );
    out
}

/// Worklog entries from across the organisation.
///
/// Different from [`worklogs`] in the one way that matters: the issue is a
/// column, because here it is the only thing that says what the time was for.
#[must_use]
pub fn worklog_search(entries: &[Worklog], ctx: &Context) -> String {
    let columns = [
        Column::whole("ISSUE", 14, Palette::key()),
        Column::whole("WHEN", 12, anstyle::Style::new()),
        Column::whole("DURATION", 10, anstyle::Style::new()),
        Column::new("WHO", 16, anstyle::Style::new()),
        Column::new("COMMENT", 36, Palette::untrusted()),
    ];

    let rows: Vec<Vec<String>> = entries
        .iter()
        .map(|entry| {
            vec![
                entry.issue.clone().unwrap_or_else(|| "-".to_owned()),
                entry.start.map_or_else(
                    || "-".to_owned(),
                    |start| start.to_string().chars().take(10).collect(),
                ),
                crate::api::duration::human(&entry.duration),
                who(entry.author.as_ref()).to_owned(),
                entry.comment.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect();

    let mut out = crate::render::table::render(&columns, &rows, ctx);
    let paint = ctx.painter();
    let total = crate::api::duration::human(&total_duration(entries));
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!("shown {} of {} — {total} total", rows.len(), rows.len()),
            Palette::label()
        )
    );
    out
}

/// What changed on an issue, one line per field.
///
/// The unit is a field, not an event: "who set the status to Closed" is the
/// question, and an event that touched three fields answers it three times over
/// only if it is split. The event columns repeat as a result, which is the
/// price of every line being readable on its own.
///
/// An event that changed nothing a caller can see — a transport detail, a
/// re-index — still gets a line, with `-` for the field. Dropping it would make
/// the history look like it has gaps.
#[must_use]
pub fn changelog(key: &str, changes: &[Change], ctx: &Context) -> String {
    let columns = [
        Column::whole("WHEN", 16, anstyle::Style::new()),
        Column::new("WHO", 16, anstyle::Style::new()),
        Column::new("FIELD", 16, Palette::key()),
        Column::new("FROM", 20, Palette::label()),
        Column::new("TO", 20, anstyle::Style::new()),
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    for change in changes {
        let when = change.at.map_or_else(
            || "-".to_owned(),
            // Minutes, not seconds: two changes in the same second are told
            // apart by their order, never by this column.
            |at| at.to_string().chars().take(16).collect(),
        );
        let by = who(change.by.as_ref()).to_owned();

        if change.fields.is_empty() {
            rows.push(vec![
                when,
                by,
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
            ]);
            continue;
        }
        for field in &change.fields {
            rows.push(vec![
                when.clone(),
                by.clone(),
                field.field.clone(),
                field.from.clone().unwrap_or_else(|| "-".to_owned()),
                field.to.clone().unwrap_or_else(|| "-".to_owned()),
            ]);
        }
    }

    let mut out = crate::render::table::render(&columns, &rows, ctx);
    let paint = ctx.painter();
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!(
                "shown {} of {} for {key} — from {} {}",
                rows.len(),
                rows.len(),
                changes.len(),
                if changes.len() == 1 {
                    "event"
                } else {
                    "events"
                }
            ),
            Palette::label()
        )
    );
    out
}

/// The sum of a worklog, in ISO 8601.
///
/// Days and weeks are left as they came: Tracker counts a working day as eight
/// hours and a working week as five days, and quietly turning `P1D` into 24
/// hours here would produce a total nobody's timesheet agrees with.
fn total_duration(worklogs: &[Worklog]) -> String {
    let (mut weeks, mut days, mut hours, mut minutes, mut seconds) = (0u64, 0u64, 0u64, 0u64, 0u64);

    for entry in worklogs {
        let Some(rest) = entry.duration.strip_prefix('P') else {
            continue;
        };
        let mut number = String::new();
        for character in rest.chars() {
            if character.is_ascii_digit() {
                number.push(character);
                continue;
            }
            let Ok(value) = number.parse::<u64>() else {
                number.clear();
                continue;
            };
            match character {
                'W' => weeks += value,
                'D' => days += value,
                'H' => hours += value,
                'M' => minutes += value,
                'S' => seconds += value,
                _ => {}
            }
            number.clear();
        }
    }

    minutes += seconds / 60;
    seconds %= 60;
    hours += minutes / 60;
    minutes %= 60;

    let mut out = String::from("P");
    for (value, unit) in [(weeks, 'W'), (days, 'D')] {
        if value > 0 {
            let _ = write!(out, "{value}{unit}");
        }
    }
    let mut time = String::new();
    for (value, unit) in [(hours, 'H'), (minutes, 'M'), (seconds, 'S')] {
        if value > 0 {
            let _ = write!(time, "{value}{unit}");
        }
    }
    if !time.is_empty() {
        let _ = write!(out, "T{time}");
    }
    if out == "P" { "PT0M".to_owned() } else { out }
}

/// Render an issue's checklist.
#[must_use]
pub fn checklist(key: &str, items: &[ChecklistItem], ctx: &Context) -> String {
    let mut out = String::with_capacity(items.len() * 64 + 32);
    let paint = ctx.painter();

    for item in items {
        // The box is the state, and it is the first thing on the line so a
        // column of them can be read at a glance.
        let box_ = if item.checked { "[x]" } else { "[ ]" };
        let assignee = match item.assignee.as_ref() {
            Some(user) => format!(" @{}", who(Some(user))),
            None => String::new(),
        };
        let deadline = match item.deadline.as_deref() {
            Some(date) => format!(" due {date}"),
            None => String::new(),
        };
        let _ = writeln!(
            out,
            "{} {} {}{}{}",
            paint.paint(&item.id, Palette::key()),
            box_,
            paint.paint(&item.text, Palette::untrusted()),
            assignee,
            deadline
        );
    }

    let done = items.iter().filter(|item| item.checked).count();
    let _ = writeln!(
        out,
        "{}",
        paint.paint(
            &format!(
                "shown {} of {} for {key} — {done} done",
                items.len(),
                items.len()
            ),
            Palette::label()
        )
    );
    out
}

/// The pinned custom fields, then either all of the rest or a count of them.
///
/// An agent gets the count: the set differs per queue, most are empty, and
/// dumping them makes the view unstable. A terminal gets all of them — the
/// reason to hide them was never that they are uninteresting.
fn custom_fields(out: &mut String, issue: &Issue, ctx: &Context) {
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    for key in &ctx.extra_fields {
        if let Some(value) = issue.extra.get(key) {
            let _ = writeln!(
                out,
                "{} {}",
                label(&format!("{key}:")),
                compact_value(value)
            );
        }
    }

    // The sort is load-bearing, not tidiness. `serde_json::Map` is a BTreeMap
    // only until some dependency turns on `preserve_order`, at which point it
    // becomes insertion-ordered and this would start varying with whatever order
    // Tracker happened to serialise the payload in. Field order is a contract
    // (ADR 3), so it is enforced here rather than inherited.
    let mut unpinned: Vec<&String> = issue
        .extra
        .keys()
        .filter(|key| !ctx.extra_fields.contains(key))
        .collect();
    unpinned.sort();

    if unpinned.is_empty() {
        return;
    }

    if ctx.is_human() {
        for key in unpinned {
            if let Some(value) = issue.extra.get(key) {
                let _ = writeln!(
                    out,
                    "{} {}",
                    label(&format!("{key}:")),
                    compact_value(value)
                );
            }
        }
        return;
    }

    let shown: Vec<&str> = unpinned.iter().take(3).map(|k| k.as_str()).collect();
    let rest = unpinned.len().saturating_sub(shown.len());
    let suffix = if rest > 0 {
        format!(", +{rest}")
    } else {
        String::new()
    };
    let _ = writeln!(
        out,
        "{} {} set ({}{suffix}) — see --fields",
        label("custom:"),
        unpinned.len(),
        shown.join(", "),
    );
}

/// What to call a link.
///
/// A recognised type gets our own wording; anything else keeps Tracker's, which
/// is at least true. The fallback word "link" said nothing.
fn relation_of(link: &crate::api::models::Link) -> String {
    if link.kind == crate::api::models::LinkKind::Other
        && let Some(relation) = &link.relation
    {
        return relation.clone();
    }
    link.kind.label().to_owned()
}

/// Links, one per line, always with their type.
fn links_section(out: &mut String, issue: &Issue, ctx: &Context) {
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    if issue.links.is_empty() {
        let _ = writeln!(out, "{} none", label("links:"));
        return;
    }

    let _ = writeln!(out, "{}", label("links:"));
    for link in &issue.links {
        let _ = writeln!(
            out,
            "  {} {}{}",
            label(&relation_of(link)),
            paint.paint(&link.key, Palette::key()),
            link.status
                .as_ref()
                .map_or_else(String::new, |status| format!(" [{status}]")),
        );
    }
}

/// The description, marked as somebody else's text and trimmed.
fn description_section(out: &mut String, issue: &Issue, ctx: &Context) {
    let Some(description) = issue.description.as_deref().filter(|d| !d.is_empty()) else {
        return;
    };

    let (body, withheld) = untrusted::head(description, ctx.description_lines);
    quoted_block(
        out,
        &format!("{}/description", issue.key),
        &body,
        withheld,
        ctx,
    );
}

/// A block of text somebody else wrote, in whichever form its reader can use.
///
/// The two audiences need different things from the same guarantee. An agent
/// needs the boundary to survive being pasted into a prompt, so it gets the
/// `<untrusted>` fence and the markdown source untouched. A person needs to
/// read the thing: markdown is rendered, and the boundary becomes a margin bar
/// on every line — a tag they would have to parse by eye is not a boundary.
pub(crate) fn quoted_block(
    out: &mut String,
    source: &str,
    body: &str,
    withheld: usize,
    ctx: &Context,
) {
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    if ctx.is_human() {
        let _ = writeln!(
            out,
            "{}",
            label(&format!("--- {source} (written by Tracker users)"))
        );
        out.push_str(&crate::render::markdown::quoted(
            body,
            ctx.width,
            paint,
            &ctx.inline,
        ));
    } else {
        let _ = writeln!(out, "{}", label("---"));
        // The fence is dimmed and nothing more. Giving someone else's text the
        // same styling as our own output would let it impersonate the tool.
        let _ = writeln!(
            out,
            "{}",
            paint.paint(&untrusted::fence(source, body), Palette::untrusted())
        );
    }

    if withheld > 0 {
        let _ = writeln!(
            out,
            "{}",
            label(&format!("(+{withheld} more lines: --full)"))
        );
    }
}

/// Render the transitions available from the current status.
#[must_use]
pub fn transitions(key: &str, transitions: &[crate::api::Transition]) -> String {
    let mut out = String::with_capacity(transitions.len() * 40 + 32);

    for transition in transitions {
        let _ = writeln!(
            out,
            "{:<20} {:<24} → {}",
            transition.id,
            transition.name,
            transition.to.as_deref().unwrap_or("-"),
        );
    }

    let _ = writeln!(
        out,
        "shown {} of {} for {key}",
        transitions.len(),
        transitions.len()
    );
    out
}

/// Render only the requested fields, on one line.
///
/// The cheapest rung of the ladder: a caller that needs a status does not need
/// the other fourteen lines. Fields come back in the order they were asked for,
/// and an unknown or unset field renders as `-` rather than vanishing — a
/// missing column would silently shift everything after it.
#[must_use]
pub fn issue_selected(issue: &Issue, fields: &[String]) -> String {
    let mut out = String::with_capacity(64 + fields.len() * 24);
    out.push_str(&issue.key);

    for field in fields {
        let _ = write!(out, "  {field}={}", field_value(issue, field));
    }

    out.push('\n');
    out
}

fn field_value(issue: &Issue, field: &str) -> String {
    match field {
        "key" => issue.key.clone(),
        "summary" => issue.summary.clone(),
        "status" => or_dash(issue.status.as_ref()).to_owned(),
        "type" => or_dash(issue.issue_type.as_ref()).to_owned(),
        "priority" => or_dash(issue.priority.as_ref()).to_owned(),
        "queue" => or_dash(issue.queue.as_ref()).to_owned(),
        "assignee" => who(issue.assignee.as_ref()).to_owned(),
        "author" => who(issue.author.as_ref()).to_owned(),
        "created" => issue
            .created_at
            .map_or_else(|| "-".to_owned(), |ts| ts.to_string()),
        "updated" => issue
            .updated_at
            .map_or_else(|| "-".to_owned(), |ts| ts.to_string()),
        "comments" => issue
            .comment_count
            .map_or_else(|| "-".to_owned(), |n| n.to_string()),
        "links" => {
            if issue.links.is_empty() {
                "none".to_owned()
            } else {
                issue
                    .links
                    .iter()
                    .map(|link| format!("{} {}", relation_of(link), link.key))
                    .collect::<Vec<_>>()
                    .join("; ")
            }
        }
        custom => issue
            .extra
            .get(custom)
            .map_or_else(|| "-".to_owned(), compact_value),
    }
}

/// Render a page of issues.
///
/// The tally that follows is not decoration. Without it a caller that receives
/// 25 rows cannot tell a complete answer from a truncated one, and "there are no
/// open issues" is a worse failure than a few wasted tokens.
#[must_use]
pub fn issue_page(page: &Page<Issue>, ctx: &Context) -> String {
    // The fifth value in each row is the status key. It is never printed —
    // neither format shows more cells than there are columns — and exists so
    // the colour can be decided by what a status means rather than by the
    // language the organisation shows it in.
    const STATUS_KEY: usize = 4;
    let columns = [
        Column::whole("KEY", 12, Palette::key()),
        Column::by_other("STATUS", 14, STATUS_KEY, status_style),
        Column::new("ASSIGNEE", 14, anstyle::Style::new()),
        Column::new("SUMMARY", 60, anstyle::Style::new()),
    ];
    let rows: Vec<Vec<String>> = page
        .items
        .iter()
        .map(|issue| {
            vec![
                issue.key.clone(),
                or_dash(issue.status.as_ref()).to_owned(),
                who(issue.assignee.as_ref()).to_owned(),
                issue.summary.clone(),
                issue.status_key.clone().unwrap_or_default(),
            ]
        })
        .collect();

    let mut out = crate::render::table::render(&columns, &rows, ctx);
    out.push_str(&crate::render::table::tally(
        page.items.len(),
        page.total,
        page.has_more().then_some(page.page + 1),
        ctx,
    ));
    out
}

/// Colour a status by what it means, not by the words it is shown in.
///
/// This takes the key, not the display name. A queue may invent statuses, and
/// every organisation shows them in its own language — a Russian one answers
/// `Закрыт` — so matching on the displayed text worked in English and nowhere
/// else, silently. Anything not on the well-known list stays unpainted rather
/// than guessed at.
fn status_style(key: &str) -> anstyle::Style {
    match key {
        "closed" | "resolved" | "done" | "released" | "rejected" => Palette::ok(),
        "inProgress" | "readyForReview" | "inReview" | "testing" | "needInfo" => Palette::warn(),
        _ => anstyle::Style::new(),
    }
}

fn status_painted(status: Option<&str>, key: Option<&str>, ctx: &Context) -> String {
    let Some(status) = status else {
        return "-".to_owned();
    };
    let style = key.map_or_else(anstyle::Style::new, status_style);
    ctx.painter().paint(status, style)
}

/// Critical and blocker are worth noticing; the rest are not worth a colour.
fn priority_painted(priority: Option<&str>, key: Option<&str>, ctx: &Context) -> String {
    let paint = ctx.painter();
    let Some(priority) = priority else {
        return "-".to_owned();
    };

    if matches!(key, Some("critical" | "blocker")) {
        paint.paint(priority, Palette::bad())
    } else {
        priority.to_owned()
    }
}

fn compact_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(compact_value)
            .collect::<Vec<_>>()
            .join(", "),
        serde_json::Value::Object(fields) => fields
            .get("display")
            .or_else(|| fields.get("name"))
            .or_else(|| fields.get("key"))
            .or_else(|| fields.get("id"))
            .map_or_else(|| value.to_string(), compact_value),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::api::models::{Link, LinkKind};
    use crate::render::{Audience, Format};

    fn ctx() -> Context {
        Context {
            format: Format::Text,
            audience: Audience::Machine,
            description_lines: Some(2),
            extra_fields: vec!["storyPoints".to_owned()],
            width: 80,
            images: false,
            inline: crate::render::image::Inline::default(),
        }
    }

    fn sample() -> Issue {
        let mut extra = serde_json::Map::new();
        extra.insert("storyPoints".to_owned(), serde_json::json!(3));
        extra.insert("sprint".to_owned(), serde_json::json!("S-12"));
        extra.insert("component".to_owned(), serde_json::json!("api"));
        extra.insert("team".to_owned(), serde_json::json!("core"));
        extra.insert("risk".to_owned(), serde_json::json!("low"));

        Issue {
            key: "PROJ-1".to_owned(),
            summary: "Attachments are lost on move".to_owned(),
            status: Some("In Progress".to_owned()),
            status_key: Some("inProgress".to_owned()),
            issue_type: Some("Bug".to_owned()),
            priority: Some("Critical".to_owned()),
            priority_key: Some("critical".to_owned()),
            queue: Some("PROJ".to_owned()),
            assignee: Some(User {
                id: "1".to_owned(),
                login: Some("ilubenets".to_owned()),
                display: None,
            }),
            author: Some(User {
                id: "2".to_owned(),
                login: Some("reporter".to_owned()),
                display: None,
            }),
            created_at: None,
            updated_at: Some(
                "2026-08-27T10:00:00Z"
                    .parse::<jiff::Timestamp>()
                    .expect("timestamp"),
            ),
            description: Some("line one\nline two\nline three\nline four".to_owned()),
            links: vec![
                Link {
                    id: "101".to_owned(),
                    kind: LinkKind::IsBlockedBy,
                    relation: None,
                    key: "PROJ-3".to_owned(),
                    summary: None,
                    status: Some("Open".to_owned()),
                },
                Link {
                    id: "102".to_owned(),
                    kind: LinkKind::Parent,
                    relation: None,
                    key: "PROJ-9".to_owned(),
                    summary: None,
                    status: None,
                },
            ],
            comment_count: Some(3),
            extra,
        }
    }

    /// The compact view is the contract with every caller: a reordered or
    /// silently widened field list breaks agents' prompt caches and users'
    /// scripts alike, so it is pinned here.
    #[test]
    fn issue_compact_view_is_stable() {
        insta::assert_snapshot!(issue(&sample(), &ctx()));
    }

    fn human_ctx() -> Context {
        Context {
            audience: Audience::Human,
            ..ctx()
        }
    }

    /// The terminal form is pinned too, escape codes and all: colour is part of
    /// what people see, and a change to it should be as visible in review as a
    /// change to the words.
    #[test]
    fn issue_terminal_view_is_stable() {
        insta::assert_snapshot!(issue(&sample(), &human_ctx()));
    }

    /// The rule that makes styling safe: same words, same order, same data —
    /// only escape codes differ. Compared at the same detail level, since a
    /// terminal is also given more of the issue.
    #[test]
    fn styling_changes_nothing_but_the_escape_codes() {
        let coloured_machine = Context {
            audience: Audience::Human,
            ..ctx()
        };
        let plain = issue(
            &sample(),
            &Context {
                audience: Audience::Machine,
                ..coloured_machine.clone()
            },
        );
        let coloured = issue(&sample(), &coloured_machine);

        // Human output lists custom fields instead of counting them, so compare
        // the shared prefix: everything up to that line.
        let cut = |text: &str| {
            text.lines()
                .take_while(|line| !line.contains("custom:") && !line.starts_with("component:"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(cut(&strip_ansi(&coloured)), cut(&plain));
    }

    /// A person reading their own terminal is not paying for context, so the
    /// description is not cut short there.
    #[test]
    fn a_terminal_gets_the_whole_description_and_every_custom_field() {
        let human = Context {
            audience: Audience::Human,
            description_lines: None,
            ..ctx()
        };
        let rendered = strip_ansi(&issue(&sample(), &human));

        assert!(rendered.contains("line four"));
        assert!(!rendered.contains("more lines: --full"));
        assert!(rendered.contains("component: api"));
        assert!(rendered.contains("team: core"));
        assert!(!rendered.contains("custom: "));
    }

    fn strip_ansi(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\u{1b}' {
                out.push(c);
                continue;
            }
            // Skip until the terminating letter of the CSI sequence.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        out
    }

    /// A reference field is one readable word wrapped in sixty of plumbing.
    /// Both audiences want the word; the plumbing is still in `--format json`.
    #[test]
    fn a_reference_field_renders_as_its_display_name() {
        let value = serde_json::json!([
            {"display": "Platform: backend", "id": "6", "self": "https://api/6"},
            {"display": "Platform: frontend", "id": "7", "self": "https://api/7"},
        ]);
        assert_eq!(
            compact_value(&value),
            "Platform: backend, Platform: frontend"
        );
    }

    /// An object with nothing to display is printed rather than dropped: a field
    /// that silently renders as nothing is worse than an ugly one.
    #[test]
    fn an_unrecognised_object_still_shows_its_contents() {
        let value = serde_json::json!({"weird": 1});
        assert_eq!(compact_value(&value), "{\"weird\":1}");
    }

    /// The fence is what an agent uses to tell content from instruction, and it
    /// has to survive being pasted into a prompt. Rendering is for the terminal.
    #[test]
    fn machine_output_keeps_the_fence_and_the_markdown_source() {
        let mut issue_md = sample();
        issue_md.description = Some("# Title\n\n**loud**".to_owned());
        let rendered = issue(
            &issue_md,
            &Context {
                description_lines: None,
                ..ctx()
            },
        );
        assert!(rendered.contains("<untrusted src=\"PROJ-1/description\""));
        assert!(rendered.contains("# Title"));
        assert!(rendered.contains("**loud**"));
    }

    /// A person gets the text, not its syntax — but never without the margin
    /// that says who wrote it.
    #[test]
    fn a_terminal_gets_rendered_markdown_behind_a_margin() {
        let mut issue_md = sample();
        issue_md.description = Some("# Title\n\n**loud**".to_owned());
        let rendered = issue(
            &issue_md,
            &Context {
                audience: Audience::Human,
                description_lines: None,
                ..ctx()
            },
        );
        let plain = strip_ansi(&rendered);

        assert!(!plain.contains("<untrusted"));
        assert!(!plain.contains("**"));
        assert!(plain.contains("(written by Tracker users)"));
        assert!(plain.contains("Title"));
        for line in plain
            .lines()
            .skip_while(|l| !l.contains("written by"))
            .skip(1)
        {
            assert!(line.starts_with('\u{258f}'), "unmarked line: {line}");
        }
    }

    #[test]
    fn description_is_fenced_and_trimmed() {
        let rendered = issue(&sample(), &ctx());
        assert!(rendered.contains("<untrusted src=\"PROJ-1/description\""));
        assert!(rendered.contains("(+2 more lines: --full)"));
        assert!(!rendered.contains("line three"));
    }

    #[test]
    fn links_always_carry_their_type() {
        let rendered = issue(&sample(), &ctx());
        assert!(rendered.contains("is blocked by PROJ-3 [Open]"));
        assert!(rendered.contains("parent PROJ-9"));
    }

    #[test]
    fn issue_without_links_says_so_rather_than_omitting_the_line() {
        let mut issue_without = sample();
        issue_without.links.clear();
        assert!(issue(&issue_without, &ctx()).contains("links: none"));
    }

    /// A truncated page that does not say it is truncated is worse than a
    /// verbose one: the caller concludes the result set is complete.
    #[test]
    fn page_reports_totals_and_the_next_page() {
        let page = Page {
            items: vec![sample()],
            page: 1,
            per_page: 1,
            total: Some(340),
        };
        insta::assert_snapshot!(issue_page(&page, &ctx()));
    }

    /// Enabling an unrelated feature must not reorder the view. `serde_json`'s
    /// map type changes behaviour when any dependency turns on `preserve_order`,
    /// which is exactly the kind of silent drift a fixed field order forbids.
    #[test]
    fn custom_field_summary_does_not_depend_on_payload_order() {
        let mut shuffled = sample();
        let entries: Vec<(String, serde_json::Value)> = vec![
            ("team".to_owned(), serde_json::json!("core")),
            ("component".to_owned(), serde_json::json!("api")),
            ("risk".to_owned(), serde_json::json!("low")),
            ("sprint".to_owned(), serde_json::json!("S-12")),
            ("storyPoints".to_owned(), serde_json::json!(3)),
        ];
        shuffled.extra = entries.into_iter().collect();

        assert_eq!(issue(&sample(), &ctx()), issue(&shuffled, &ctx()));
    }

    #[test]
    fn selected_fields_keep_the_order_they_were_asked_for() {
        let fields = vec![
            "status".to_owned(),
            "storyPoints".to_owned(),
            "assignee".to_owned(),
        ];
        assert_eq!(
            issue_selected(&sample(), &fields),
            "PROJ-1  status=In Progress  storyPoints=3  assignee=ilubenets\n"
        );
    }

    /// A field that is absent must still occupy its place: silently dropping it
    /// shifts every column after it for anything parsing the line.
    #[test]
    fn an_unknown_field_renders_as_a_dash() {
        let fields = vec!["nonsense".to_owned(), "status".to_owned()];
        assert_eq!(
            issue_selected(&sample(), &fields),
            "PROJ-1  nonsense=-  status=In Progress\n"
        );
    }

    #[test]
    fn complete_page_does_not_offer_a_next_one() {
        let page = Page {
            items: vec![sample()],
            page: 1,
            per_page: 25,
            total: Some(1),
        };
        let rendered = issue_page(&page, &ctx());
        assert!(rendered.contains("shown 1 of 1"));
        assert!(!rendered.contains("next:"));
    }
}
