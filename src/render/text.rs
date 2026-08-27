//! The compact text view.
//!
//! Target shape for `issue get`, roughly fifteen lines instead of a five-kilobyte
//! payload. Links are always present: "what blocks this" is the question that
//! follows "what is this", and making the caller run a second command for it
//! costs more than the four lines it saves.

use std::fmt::Write as _;

use crate::api::models::{Comment, Issue, Link, Page, User};
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

    let _ = writeln!(out, "{}  {}", issue.key, issue.summary);
    let _ = writeln!(
        out,
        "status: {}   type: {}   prio: {}",
        or_dash(issue.status.as_ref()),
        or_dash(issue.issue_type.as_ref()),
        or_dash(issue.priority.as_ref()),
    );
    let _ = writeln!(
        out,
        "assignee: {}   author: {}   queue: {}",
        who(issue.assignee.as_ref()),
        who(issue.author.as_ref()),
        or_dash(issue.queue.as_ref()),
    );
    let _ = writeln!(
        out,
        "updated: {}   comments: {}",
        issue
            .updated_at
            .map_or_else(|| "-".to_owned(), |ts| ts.to_string()),
        issue
            .comment_count
            .map_or_else(|| "-".to_owned(), |n| n.to_string()),
    );

    for key in &ctx.extra_fields {
        if let Some(value) = issue.extra.get(key) {
            let _ = writeln!(out, "{key}: {}", compact_value(value));
        }
    }

    // Custom fields are summarised rather than dumped: they differ per queue, so
    // printing them all makes the view unstable and mostly empty.
    //
    // The sort is load-bearing, not tidiness. `serde_json::Map` is a BTreeMap
    // only until some dependency turns on `preserve_order`, at which point it
    // becomes insertion-ordered and this line would start varying with whatever
    // order Tracker happened to serialise the payload in. Field order is a
    // contract (ADR 3), so it is enforced here rather than inherited.
    let mut unpinned: Vec<&String> = issue
        .extra
        .keys()
        .filter(|key| !ctx.extra_fields.contains(key))
        .collect();
    unpinned.sort();
    if !unpinned.is_empty() {
        let shown: Vec<&str> = unpinned.iter().take(3).map(|k| k.as_str()).collect();
        let rest = unpinned.len().saturating_sub(shown.len());
        let suffix = if rest > 0 {
            format!(", +{rest}")
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "custom: {} set ({}{suffix}) — see --fields",
            unpinned.len(),
            shown.join(", "),
        );
    }

    if issue.links.is_empty() {
        let _ = writeln!(out, "links: none");
    } else {
        let _ = writeln!(out, "links:");
        for link in &issue.links {
            let _ = writeln!(
                out,
                "  {} {}{}",
                link.kind.label(),
                link.key,
                link.status
                    .as_ref()
                    .map_or_else(String::new, |status| format!(" [{status}]")),
            );
        }
    }

    if let Some(description) = issue.description.as_deref().filter(|d| !d.is_empty()) {
        let (body, withheld) = untrusted::head(description, ctx.description_lines);
        let _ = writeln!(out, "---");
        let _ = writeln!(
            out,
            "{}",
            untrusted::fence(&format!("{}/description", issue.key), &body)
        );
        if withheld > 0 {
            let _ = writeln!(out, "(+{withheld} more lines: --full)");
        }
    }

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
        let _ = writeln!(
            out,
            "{} {}{}{}",
            link.kind.label(),
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

/// Render comments, each fenced with its own source.
///
/// The fence names the comment and its author, so a reader can tell which part
/// of the output someone else wrote — the whole point of the marking (ADR 1).
#[must_use]
pub fn comments(key: &str, comments: &[Comment]) -> String {
    let mut out = String::with_capacity(comments.len() * 160 + 32);

    for comment in comments {
        let author = who(comment.author.as_ref());
        let when = comment
            .created_at
            .map_or_else(|| "-".to_owned(), |ts| ts.to_string());
        let _ = writeln!(out, "--- {} by {author} at {when}", comment.id);
        let _ = writeln!(
            out,
            "{}",
            untrusted::fence(
                &format!("{key}/comment/{} by {author}", comment.id),
                &comment.text
            )
        );
    }

    let _ = writeln!(
        out,
        "shown {} of {} for {key}",
        comments.len(),
        comments.len()
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
                    .map(|link| format!("{} {}", link.kind.label(), link.key))
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

/// Render a page of issues as fixed-width columns plus an explicit tally.
///
/// The tally is not decoration. Without it a caller that receives 25 rows cannot
/// tell a complete answer from a truncated one, and "there are no open issues"
/// is a worse failure than a few wasted tokens.
#[must_use]
pub fn issue_page(page: &Page<Issue>, _ctx: &Context) -> String {
    let mut out = String::with_capacity(page.items.len() * 80 + 64);

    for issue in &page.items {
        let _ = writeln!(
            out,
            "{:<12} {:<14} {:<14} {}",
            issue.key,
            truncate(or_dash(issue.status.as_ref()), 14),
            truncate(who(issue.assignee.as_ref()), 14),
            truncate(&issue.summary, 60),
        );
    }

    let shown = page.items.len();
    match page.total {
        Some(total) => {
            let _ = write!(out, "shown {shown} of {total}");
        }
        None => {
            let _ = write!(out, "shown {shown} of unknown total");
        }
    }
    if page.has_more() {
        let _ = write!(out, " — next: --page {}", page.page + 1);
    }
    out.push('\n');

    out
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let mut kept: String = value.chars().take(width.saturating_sub(1)).collect();
    kept.push('…');
    kept
}

fn compact_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
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
            issue_type: Some("Bug".to_owned()),
            priority: Some("Critical".to_owned()),
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
                    kind: LinkKind::IsBlockedBy,
                    key: "PROJ-3".to_owned(),
                    summary: None,
                    status: Some("Open".to_owned()),
                },
                Link {
                    kind: LinkKind::Parent,
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
