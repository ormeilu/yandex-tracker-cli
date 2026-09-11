//! Yandex Wiki pages.

use std::fmt::Write as _;

use crate::api::wiki::{
    CursorPage, WikiAttachment, WikiComment, WikiGrid, WikiGridRef, WikiHits, WikiPage,
    WikiPageRef, WikiResource,
};
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, cursor_tally, open_page_tally, render};

/// Search hits: where each one is, what kind, when it last changed, and its
/// title — dimmed, because somebody else wrote it.
///
/// The excerpt the Wiki sends with each hit is left to `--format json`: it is
/// somebody else's words, cut mid-sentence, and `wiki get` reads the real
/// thing for one request.
#[must_use]
pub fn hits(found: &WikiHits, ctx: &Context) -> String {
    let columns = [
        Column::whole("SLUG", 44, Palette::key()),
        Column::new("TYPE", 5, anstyle::Style::new()),
        Column::new("MODIFIED", 10, Palette::label()),
        Column::new("TITLE", 50, Palette::untrusted()),
    ];
    let rows: Vec<Vec<String>> = found
        .results
        .iter()
        .map(|hit| {
            vec![
                hit.slug.clone(),
                hit.kind.clone(),
                // The date alone: the time of a last change is noise in a list.
                hit.modified_at
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), |at| at.chars().take(10).collect()),
                hit.title.clone(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&open_page_tally(found.results.len(), found.next_page, ctx));
    out
}

/// A page's comments, each fenced as somebody else's words.
///
/// The header line is ours: who, when, whether it is resolved, and — when a
/// thread holds more than the one post — the flag that reads the rest. A
/// deleted comment keeps its header, so the thread around it still makes
/// sense, and loses its fence, since there is nothing left to quote.
#[must_use]
pub fn comments(slug: &str, list: &CursorPage<WikiComment>, ctx: &Context) -> String {
    let mut out = String::with_capacity(list.results.len() * 200 + 64);
    let paint = ctx.painter();

    for comment in &list.results {
        let author = comment.author.as_deref().unwrap_or("-");
        let mut header = format!(
            "--- {} by {author} at {}",
            comment.id,
            comment.created_at.as_deref().unwrap_or("-")
        );
        if comment.resolved {
            header.push_str(" (resolved)");
        }
        if comment.deleted {
            header.push_str(" (deleted)");
        }
        if let Some(posts) = comment.thread_posts.filter(|posts| *posts > 1) {
            let _ = write!(header, " — {posts} in thread: --thread {}", comment.id);
        }
        let _ = writeln!(out, "{}", paint.paint(&header, Palette::label()));

        if comment.deleted || comment.body.is_empty() {
            continue;
        }
        crate::render::text::quoted_block(
            &mut out,
            &format!("wiki:{slug}/comment/{} by {author}", comment.id),
            crate::render::untrusted::Author::Wiki,
            &comment.body,
            0,
            ctx,
        );
    }

    out.push_str(&cursor_tally(
        list.results.len(),
        list.next_cursor.as_deref(),
        ctx,
    ));
    out
}

/// A page's attachments.
///
/// The name was chosen by whoever uploaded it, so it gets the styling of
/// somebody else's text, as Tracker's attachment names do. The size is shown as
/// the Wiki sends it: it is a string in units the Wiki does not name, and
/// guessing them would print a wrong number with confidence.
#[must_use]
pub fn attachments(list: &CursorPage<WikiAttachment>, ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 10, Palette::key()),
        Column::whole("SIZE", 10, anstyle::Style::new()),
        Column::new("TYPE", 18, anstyle::Style::new()),
        Column::new("CREATED", 10, Palette::label()),
        Column::whole("NAME", 40, Palette::untrusted()),
    ];
    let rows: Vec<Vec<String>> = list
        .results
        .iter()
        .map(|file| {
            vec![
                file.id.to_string(),
                file.size.clone(),
                file.mimetype.as_deref().unwrap_or("-").to_owned(),
                file.created_at
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), |at| at.chars().take(10).collect()),
                file.name.clone(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&cursor_tally(
        list.results.len(),
        list.next_cursor.as_deref(),
        ctx,
    ));
    out
}

/// One grid: what it is and its columns, then its rows as somebody else's text.
///
/// The rows go out tab-separated inside the fence: every cell is somebody
/// else's words, a tab-separated block is what a pipe and an agent read with
/// the least overhead, and the fence has to hold the whole table rather than a
/// cell at a time. A tab, a newline or a backslash inside a cell is written
/// `\t`, `\n`, `\\`, so every row stays one line and the escape can be undone.
/// A person gets the same block kept verbatim instead of reflowed as prose.
/// Rows are cut like a description, and `--full` shows them all.
#[must_use]
pub fn grid(grid: &WikiGrid, ctx: &Context) -> String {
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());
    let mut out = String::with_capacity(256 + grid.rows.len() * 64);

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&grid.id, Palette::key()),
        grid.title
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}",
        label("page:"),
        grid.page.as_ref().map_or("-", |page| page.slug.as_str()),
        label("revision:"),
        if grid.revision.is_empty() {
            "-"
        } else {
            &grid.revision
        },
    );
    let columns: Vec<String> = grid
        .columns
        .iter()
        .map(|column| format!("{}:{}", column.slug, column.kind))
        .collect();
    let _ = writeln!(out, "{} {}", label("columns:"), columns.join(" "));

    let mut table = String::with_capacity(grid.rows.len() * 64);
    let titles: Vec<String> = grid
        .columns
        .iter()
        .map(|column| tsv_cell(&column.title))
        .collect();
    let _ = writeln!(table, "{}", titles.join("\t"));
    for row in &grid.rows {
        let cells: Vec<String> = row
            .cells
            .iter()
            .map(|value| tsv_cell(&cell_text(value)))
            .collect();
        let _ = writeln!(table, "{}", cells.join("\t"));
    }

    let (body, withheld) = crate::render::untrusted::head(&table, ctx.description_lines);
    let body = if ctx.is_human() {
        format!("```\n{}\n```", body.trim_end())
    } else {
        body
    };
    crate::render::text::quoted_block(
        &mut out,
        &format!("wiki:grid/{}", grid.id),
        crate::render::untrusted::Author::Wiki,
        &body,
        withheld,
        ctx,
    );

    let shown = grid.rows.len().saturating_sub(withheld);
    let _ = writeln!(
        out,
        "{}",
        label(&format!("shown {shown} of {}", grid.rows.len()))
    );
    out
}

/// A cell as a person reads it: a user by login, a ticket by key, a Tracker
/// field by what it displays, a list joined. The typed value is in JSON.
fn cell_text(value: &serde_json::Value) -> String {
    use serde_json::Value;

    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(cell_text).collect::<Vec<_>>().join(", "),
        Value::Object(fields) => ["username", "display", "key"]
            .iter()
            .find_map(|name| fields.get(*name).and_then(Value::as_str))
            .map_or_else(|| value.to_string(), str::to_owned),
        other => other.to_string(),
    }
}

/// Keep a cell on its line: the escape is reversible, so nothing is lost.
fn tsv_cell(text: &str) -> String {
    let mut cell = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => cell.push_str("\\\\"),
            '\t' => cell.push_str("\\t"),
            '\n' => cell.push_str("\\n"),
            '\r' => cell.push_str("\\r"),
            other => cell.push(other),
        }
    }
    cell
}

/// The grids on a page: the id `wiki grid` takes, then when and what.
#[must_use]
pub fn grids(list: &CursorPage<WikiGridRef>, ctx: &Context) -> String {
    let columns = [
        Column::whole("ID", 36, Palette::key()),
        Column::new("CREATED", 10, Palette::label()),
        Column::new("TITLE", 50, Palette::untrusted()),
    ];
    let rows: Vec<Vec<String>> = list
        .results
        .iter()
        .map(|grid| {
            vec![
                grid.id.clone(),
                day(grid.created_at.as_deref()),
                grid.title.clone(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&cursor_tally(
        list.results.len(),
        list.next_cursor.as_deref(),
        ctx,
    ));
    out
}

/// What a page holds, files and grids in one list.
#[must_use]
pub fn resources(list: &CursorPage<WikiResource>, ctx: &Context) -> String {
    let columns = [
        Column::new("TYPE", 10, anstyle::Style::new()),
        Column::whole("ID", 36, Palette::key()),
        Column::new("CREATED", 10, Palette::label()),
        Column::whole("NAME", 40, Palette::untrusted()),
    ];
    let rows: Vec<Vec<String>> = list
        .results
        .iter()
        .map(|resource| {
            vec![
                resource.kind.clone(),
                resource.id.clone(),
                day(resource.created_at.as_deref()),
                resource.name.clone(),
            ]
        })
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&cursor_tally(
        list.results.len(),
        list.next_cursor.as_deref(),
        ctx,
    ));
    out
}

/// The date alone: the time of day is noise in a list.
fn day(at: Option<&str>) -> String {
    at.map_or_else(|| "-".to_owned(), |at| at.chars().take(10).collect())
}

/// Who can read and edit a page: the policy, then every grant.
///
/// Each grant says which of the Wiki's three lists it came from, because
/// "why can this person see it" is answered by the list, not the role. A
/// group's name was chosen by somebody else and is styled as such.
#[must_use]
pub fn access(access: &crate::api::wiki::WikiAccess, ctx: &Context) -> String {
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());
    let mut out = String::with_capacity(128 + access.entries.len() * 80);

    let policy = access.policy.as_deref().unwrap_or("-");
    let inherited = access
        .inherited_policy
        .as_deref()
        .filter(|_| policy == "inherited")
        .map_or_else(String::new, |inherited| format!(" ({inherited})"));
    let _ = writeln!(
        out,
        "{}  {} {policy}{inherited}   {} {}",
        paint.paint(&access.slug, Palette::key()),
        label("access:"),
        label("all staff:"),
        access.all_staff_role.as_deref().unwrap_or("-"),
    );

    let columns = [
        Column::whole("ID", 24, Palette::key()),
        Column::new("ROLE", 12, anstyle::Style::new()),
        Column::new("KIND", 5, anstyle::Style::new()),
        Column::new("VIA", 9, Palette::label()),
        Column::whole("WHO", 30, Palette::untrusted()),
    ];
    let rows: Vec<Vec<String>> = access
        .entries
        .iter()
        .map(|entry| {
            vec![
                entry.id.clone(),
                entry.role.clone(),
                entry.kind.clone(),
                entry.via.clone(),
                entry.who.clone(),
            ]
        })
        .collect();
    out.push_str(&render(&columns, &rows, ctx));
    out.push_str(&cursor_tally(access.entries.len(), None, ctx));
    out
}

/// Where an operation has got to, in one line.
#[must_use]
pub fn operation(
    operation: &crate::api::wiki::WikiOperation,
    status: &crate::api::wiki::OperationStatus,
) -> String {
    let mut line = format!(
        "operation {} {}: {}",
        operation.kind, operation.id, status.status
    );
    if let Some(percentage) = status.percentage.filter(|_| !status.is_done()) {
        let _ = write!(line, " {percentage:.0}%");
    }
    if let Some(slug) = status.page_slug() {
        let _ = write!(line, " — page {slug}");
    }
    if let Some(grid) = status.grid_id() {
        let _ = write!(line, ", grid {grid}");
    }
    if let Some(details) = &status.details {
        let _ = write!(line, " ({details})");
    }
    line.push('\n');
    line
}

/// The pages under one.
///
/// The slug leads, because it is what `wiki get` takes; the id follows, because
/// it is what the Wiki's own addresses use. There are no titles: the Wiki does
/// not send them in a listing, and asking for each would cost a request a row.
#[must_use]
pub fn pages(list: &CursorPage<WikiPageRef>, ctx: &Context) -> String {
    let columns = [
        Column::whole("SLUG", 60, Palette::key()),
        Column::whole("ID", 10, Palette::label()),
    ];
    let rows: Vec<Vec<String>> = list
        .results
        .iter()
        .map(|page| vec![page.slug.clone(), page.id.to_string()])
        .collect();

    let mut out = render(&columns, &rows, ctx);
    out.push_str(&cursor_tally(
        list.results.len(),
        list.next_cursor.as_deref(),
        ctx,
    ));
    out
}

/// One page: where it is and what it is called, then its text.
///
/// The slug leads, because it is what `wiki get` takes back. The text is
/// fenced and shortened exactly like a description: somebody else wrote it,
/// and a runbook is as long as a description is short.
#[must_use]
pub fn page(page: &WikiPage, ctx: &Context) -> String {
    let mut out = String::with_capacity(256 + page.content.as_ref().map_or(0, String::len));
    let paint = ctx.painter();
    let label = |text: &str| paint.paint(text, Palette::label());

    let _ = writeln!(
        out,
        "{}  {}",
        paint.paint(&page.slug, Palette::key()),
        page.title
    );
    let _ = writeln!(
        out,
        "{} {}   {} {}   {} {}",
        label("id:"),
        page.id,
        label("type:"),
        page.page_type.as_deref().unwrap_or("-"),
        label("modified:"),
        page.modified_at.as_deref().unwrap_or("-"),
    );

    if let Some(content) = page.content.as_deref().filter(|text| !text.is_empty()) {
        // A draw.io diagram or a pasted image carries its picture inline as
        // base64: kilobytes nobody can read, in every read of the page. Outside
        // `--full` the payload is folded to its size; `--full` stays
        // byte-for-byte, so nothing of the text is ever lost for good.
        let folded;
        let content = if ctx.description_lines.is_some() {
            folded = fold_data_uris(content);
            folded.as_str()
        } else {
            content
        };
        let (body, withheld) = crate::render::untrusted::head(content, ctx.description_lines);
        crate::render::text::quoted_block(
            &mut out,
            &format!("wiki:{}", page.slug),
            crate::render::untrusted::Author::Wiki,
            &body,
            withheld,
            ctx,
        );
    }

    out
}

/// Payloads shorter than this are left alone: folding a tiny icon would cost
/// more words than it saves.
const FOLD_OVER: usize = 256;

/// `data:<type>;base64,<payload>` with every long payload replaced by
/// `…(N KB, --full shows it)`. Everything else is left exactly as written.
fn fold_data_uris(text: &str) -> String {
    const MARKER: &str = ";base64,";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("data:") {
        let after = &rest[at..];
        let Some(marker) = after.find(MARKER).filter(|&end| {
            after[5..end]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-./".contains(c))
        }) else {
            out.push_str(&rest[..at + 5]);
            rest = &rest[at + 5..];
            continue;
        };
        let start = at + marker + MARKER.len();
        let length = rest[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || "+/=".contains(c)))
            .unwrap_or(rest.len() - start);
        out.push_str(&rest[..start]);
        if length > FOLD_OVER {
            // Bytes the payload decodes to, which is what a person means by a
            // picture's size, in tenths of a kilobyte, rounded.
            let tenths = (length * 3 / 4 * 10 + 512) / 1024;
            let _ = write!(
                out,
                "…({}.{} KB, --full shows it)",
                tenths / 10,
                tenths % 10
            );
        } else {
            out.push_str(&rest[start..start + length]);
        }
        rest = &rest[start + length..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Audience, Format};

    #[test]
    fn a_long_inline_picture_is_folded_to_its_size() {
        let payload = "A".repeat(4096);
        let text = format!(
            "before {{% drawio data=\"data:image/svg+xml;base64,{payload}\" width=\"600\" %}} after"
        );
        assert_eq!(
            fold_data_uris(&text),
            "before {% drawio data=\"data:image/svg+xml;base64,…(3.0 KB, --full shows it)\" width=\"600\" %} after"
        );
    }

    #[test]
    fn a_short_payload_and_plain_text_are_left_alone() {
        let text = "icon ![x](data:image/png;base64,iVBORw0KGgo=) and the word data: here";
        assert_eq!(fold_data_uris(text), text);
    }

    #[test]
    fn full_shows_an_inline_picture_byte_for_byte() {
        let content = format!("x data:image/png;base64,{}", "B".repeat(1000));
        let mut shown = sample();
        shown.content = Some(content.clone());
        let mut full = ctx();
        full.description_lines = None;
        assert!(page(&shown, &full).contains(&content));
        assert!(page(&shown, &ctx()).contains("…(0.7 KB, --full shows it)"));
    }

    fn ctx() -> Context {
        Context {
            format: Format::Text,
            audience: Audience::Machine,
            description_lines: Some(2),
            extra_fields: Vec::new(),
            width: 80,
            images: false,
            inline: crate::render::image::Inline::default(),
        }
    }

    fn sample() -> WikiPage {
        WikiPage {
            id: 4521,
            slug: "users/ilubenets/runbook".to_owned(),
            title: "Deploy runbook".to_owned(),
            page_type: Some("wysiwyg".to_owned()),
            modified_at: Some("2026-09-01T10:15:00Z".to_owned()),
            content: Some("# Deploy\n\n1. Tag the release.\n2. Watch the pipeline.".to_owned()),
        }
    }

    /// A listing with more to come: every row, then a tally that does not
    /// pretend to know the total.
    #[test]
    fn pages_view_is_stable() {
        let list = CursorPage {
            results: vec![
                WikiPageRef {
                    id: 4521,
                    slug: "users/ilubenets/runbook".to_owned(),
                },
                WikiPageRef {
                    id: 4522,
                    slug: "users/ilubenets/runbook/rollback".to_owned(),
                },
            ],
            next_cursor: Some("eyJpZCI6NDUyMn0=".to_owned()),
        };
        insta::assert_snapshot!(pages(&list, &ctx()));
    }

    /// Hits with another page to come; titles are data, the tally names the
    /// next page because search is numbered.
    #[test]
    fn hits_view_is_stable() {
        let found = WikiHits {
            results: vec![
                crate::api::wiki::WikiHit {
                    slug: "users/ilubenets/runbook".to_owned(),
                    title: "Deploy runbook".to_owned(),
                    kind: "page".to_owned(),
                    modified_at: Some("2026-09-01T10:15:00Z".to_owned()),
                    url: None,
                    snippet: Some("tag the release".to_owned()),
                },
                crate::api::wiki::WikiHit {
                    slug: "users/ilubenets/runbook/.files/rollback.pdf".to_owned(),
                    title: "rollback.pdf".to_owned(),
                    kind: "file".to_owned(),
                    modified_at: None,
                    url: None,
                    snippet: None,
                },
            ],
            next_page: Some(2),
        };
        insta::assert_snapshot!(hits(&found, &ctx()));
    }

    /// A thread worth opening, a resolved comment, and a deleted one that
    /// keeps its place but has nothing to fence.
    #[test]
    fn comments_view_is_stable() {
        let comment = |id: u64, body: &str| WikiComment {
            id,
            author: Some("ilubenets".to_owned()),
            created_at: Some("2026-09-02T08:00:00Z".to_owned()),
            body: body.to_owned(),
            resolved: false,
            deleted: false,
            quote: None,
            thread_posts: Some(1),
        };
        let list = CursorPage {
            results: vec![
                WikiComment {
                    thread_posts: Some(3),
                    ..comment(7001, "Step 2 needs the canary first.")
                },
                WikiComment {
                    resolved: true,
                    ..comment(7002, "Typo in the title.")
                },
                WikiComment {
                    deleted: true,
                    ..comment(7003, "")
                },
            ],
            next_cursor: None,
        };
        insta::assert_snapshot!(comments("users/ilubenets/runbook", &list, &ctx()));
    }

    /// Files with more to come; a missing type and date fall back to a dash.
    #[test]
    fn attachments_view_is_stable() {
        let list = CursorPage {
            results: vec![
                WikiAttachment {
                    id: 901,
                    name: "rollback.pdf".to_owned(),
                    size: "0.25".to_owned(),
                    mimetype: Some("application/pdf".to_owned()),
                    created_at: Some("2026-09-01T11:00:00Z".to_owned()),
                    author: Some("ilubenets".to_owned()),
                    download_url: None,
                },
                WikiAttachment {
                    id: 902,
                    name: "notes.txt".to_owned(),
                    size: "-".to_owned(),
                    mimetype: None,
                    created_at: None,
                    author: None,
                    download_url: None,
                },
            ],
            next_cursor: Some("eyJpZCI6OTAyfQ==".to_owned()),
        };
        insta::assert_snapshot!(attachments(&list, &ctx()));
    }

    fn sample_grid() -> WikiGrid {
        use crate::api::wiki::{GridColumn, GridRow};
        let column = |slug: &str, title: &str, kind: &str| GridColumn {
            slug: slug.to_owned(),
            title: title.to_owned(),
            kind: kind.to_owned(),
        };
        WikiGrid {
            id: "8f1e2d3c-4b5a-4c6d-8e7f-9a0b1c2d3e4f".to_owned(),
            title: "Releases".to_owned(),
            page: Some(WikiPageRef {
                id: 4521,
                slug: "users/ilubenets/runbook".to_owned(),
            }),
            revision: "12".to_owned(),
            columns: vec![
                column("version", "Version", "string"),
                column("owner", "Owner", "staff"),
                column("ticket", "Ticket", "ticket"),
                column("status", "Status", "ticket_field"),
                column("done", "Done", "checkbox"),
            ],
            rows: vec![
                GridRow {
                    id: "1".to_owned(),
                    cells: vec![
                        serde_json::json!("1.2.0"),
                        serde_json::json!([{"username": "ilubenets", "display_name": "Ilya"}]),
                        serde_json::json!({"key": "PROJ-1", "resolved": false}),
                        serde_json::json!({"key": "inProgress", "display": "В работе"}),
                        serde_json::json!(false),
                    ],
                },
                GridRow {
                    id: "2".to_owned(),
                    cells: vec![
                        serde_json::json!("1.3.0\tbeta\nsecond line"),
                        serde_json::json!([]),
                        serde_json::Value::Null,
                        serde_json::Value::Null,
                        serde_json::json!(true),
                    ],
                },
            ],
        }
    }

    /// Cells flattened to what a person reads, tabs and newlines escaped so
    /// a row stays a line, and the whole table inside one fence.
    #[test]
    fn grid_view_is_stable() {
        let full = Context {
            description_lines: None,
            ..ctx()
        };
        insta::assert_snapshot!(grid(&sample_grid(), &full));
    }

    /// Cut like a description: the tally counts the rows actually shown.
    #[test]
    fn a_long_grid_is_cut_and_says_so() {
        let text = grid(&sample_grid(), &ctx());
        assert!(text.contains("(+1 more lines: --full)"), "{text}");
        assert!(text.trim_end().ends_with("shown 1 of 2"), "{text}");
    }

    #[test]
    fn grids_view_is_stable() {
        let list = CursorPage {
            results: vec![WikiGridRef {
                id: "8f1e2d3c-4b5a-4c6d-8e7f-9a0b1c2d3e4f".to_owned(),
                title: "Releases".to_owned(),
                created_at: Some("2026-08-01T09:00:00Z".to_owned()),
            }],
            next_cursor: None,
        };
        insta::assert_snapshot!(grids(&list, &ctx()));
    }

    #[test]
    fn resources_view_is_stable() {
        let list = CursorPage {
            results: vec![
                WikiResource {
                    kind: "attachment".to_owned(),
                    id: "901".to_owned(),
                    name: "rollback.pdf".to_owned(),
                    created_at: Some("2026-09-01T11:00:00Z".to_owned()),
                },
                WikiResource {
                    kind: "grid".to_owned(),
                    id: "8f1e2d3c-4b5a-4c6d-8e7f-9a0b1c2d3e4f".to_owned(),
                    name: "Releases".to_owned(),
                    created_at: None,
                },
            ],
            next_cursor: Some("eyJpZCI6OTAyfQ==".to_owned()),
        };
        insta::assert_snapshot!(resources(&list, &ctx()));
    }

    /// A custom policy with a direct grant to a user and an inherited one to
    /// a group; the list each came from is part of the answer.
    #[test]
    fn access_view_is_stable() {
        use crate::api::wiki::{AccessEntry, WikiAccess};
        let entry = |id: &str, role: &str, kind: &str, who: &str, via: &str| AccessEntry {
            id: id.to_owned(),
            role: role.to_owned(),
            kind: kind.to_owned(),
            who: who.to_owned(),
            via: via.to_owned(),
            inheritance: None,
        };
        let page_access = WikiAccess {
            slug: "users/ilubenets/runbook".to_owned(),
            policy: Some("custom".to_owned()),
            inherited_policy: None,
            all_staff_role: None,
            entries: vec![
                entry("a1", "author", "user", "ilubenets", "direct"),
                entry("g7", "reader", "group", "Backend team", "inherited"),
            ],
        };
        insta::assert_snapshot!(access(&page_access, &ctx()));
    }

    /// Running, then finished: the percentage while it means something, and
    /// what the clone made once it is done.
    #[test]
    fn operation_view_is_stable() {
        use crate::api::wiki::{OperationStatus, WikiOperation};
        let grid = WikiOperation {
            id: "op2".to_owned(),
            kind: "clone_inline_grid".to_owned(),
        };
        let running = OperationStatus {
            status: "in_progress".to_owned(),
            percentage: Some(40.0),
            details: None,
            result: None,
        };
        let done = OperationStatus {
            status: "success".to_owned(),
            percentage: Some(100.0),
            details: None,
            result: Some(serde_json::json!({
                "grid_id": "4c1d2e3f-0000-4000-8000-000000000002",
                "page": {"id": 4700, "slug": "users/ilubenets/other"}
            })),
        };
        insta::assert_snapshot!(operation(&grid, &running) + &operation(&grid, &done));
    }

    /// The compact view is the contract with every caller, so it is pinned.
    #[test]
    fn page_compact_view_is_stable() {
        insta::assert_snapshot!(page(&sample(), &ctx()));
    }

    /// The terminal form too, escape codes and all.
    #[test]
    fn page_terminal_view_is_stable() {
        let human = Context {
            audience: Audience::Human,
            ..ctx()
        };
        insta::assert_snapshot!(page(&sample(), &human));
    }
}
