//! Wiki comment writes: comment, reply, delete — announced and gated.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

async fn page_is_4521(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v1/pages"))
        .and(query_param("slug", "users/ilubenets/runbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_page.json")))
        .mount(&harness.server)
        .await;
}

async fn nothing_was_sent(harness: &Harness) {
    let requests = harness.server.received_requests().await.expect("recorded");
    assert!(requests.is_empty(), "{requests:?}");
}

fn comment_7004() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "id": 7004,
        "body": "Agreed.\n",
        "author": {"id": 11, "username": "ilubenets", "display_name": "Ilya Lubenets",
                   "is_dismissed": false, "affiliation": "staff"},
        "created_at": "2026-09-11T10:00:00Z",
        "is_deleted": false,
        "resolve_status": "unresolved",
        "reactions": [],
        "parent_id": 7001,
        "thread_id": 7001
    }))
}

/// A reply read from stdin goes to the page's id with its parent named, after
/// the write is announced; the output is the new comment's id.
#[tokio::test]
async fn a_reply_from_stdin_names_its_parent() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/comments"))
        .and(body_json(
            serde_json::json!({ "body": "Agreed.\n", "parent_id": 7001 }),
        ))
        .respond_with(comment_7004())
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "comment",
            "users/ilubenets/runbook",
            "-",
            "--reply-to",
            "7001",
        ])
        .write_stdin("Agreed.\n")
        .assert()
        .success()
        .stdout("commented on users/ilubenets/runbook: comment 7004\n")
        .stderr(predicate::str::contains("profile="));
}

/// The passage a comment is about reaches the Wiki as `inline_text`.
#[tokio::test]
async fn a_quoted_passage_is_sent_as_inline_text() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/comments"))
        .and(body_json(serde_json::json!({
            "body": "Out of date", "inline_text": "Watch the pipeline."
        })))
        .respond_with(comment_7004())
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "comment",
            "users/ilubenets/runbook",
            "Out of date",
            "--quote",
            "Watch the pipeline.",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn a_dry_run_comment_sends_nothing() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "wiki",
            "comment",
            "users/ilubenets/runbook",
            "hello",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "dry run: would comment on wiki page `users/ilubenets/runbook`",
        ));
    nothing_was_sent(&harness).await;
}

/// No undo, so no delete without --yes — and nothing is sent to find out.
#[tokio::test]
async fn deleting_a_comment_needs_yes_and_reports_what_is_left() {
    let harness = Harness::new().await;

    harness
        .run(&["wiki", "delete-comment", "users/ilubenets/runbook", "7001"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));
    nothing_was_sent(&harness).await;

    page_is_4521(&harness).await;
    Mock::given(method("DELETE"))
        .and(path("/v1/pages/4521/comments/7001"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "comments_count": 1 })),
        )
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "delete-comment",
            "users/ilubenets/runbook",
            "7001",
            "--yes",
        ])
        .assert()
        .success()
        .stdout("deleted comment 7001 on users/ilubenets/runbook; 1 left\n");
}
