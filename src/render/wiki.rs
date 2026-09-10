//! Yandex Wiki pages.

use std::fmt::Write as _;

use crate::api::wiki::{CursorPage, WikiPage, WikiPageRef};
use crate::render::Context;
use crate::render::style::Palette;
use crate::render::table::{Column, cursor_tally, render};

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
