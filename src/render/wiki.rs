//! Yandex Wiki pages.

use std::fmt::Write as _;

use crate::api::wiki::{CursorPage, WikiComment, WikiHits, WikiPage, WikiPageRef};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Audience, Format};

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
