//! Writing commands, and the gate in front of them.
//!
//! The interesting assertions are not that a POST happens but that it does not:
//! `--dry-run` must reach the network never, and the announcement of which
//! organisation is about to change must always be there.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

#[tokio::test]
async fn create_sends_the_queue_summary_and_returns_the_new_key() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/"))
        .and(body_json(serde_json::json!({
            "queue": {"key": "PROJ"},
            "summary": "Retry uploads on 5xx",
            "assignee": "ilubenets",
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(fixture("issue.json")))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "create",
            "-q",
            "PROJ",
            "-s",
            "Retry uploads on 5xx",
            "--assignee",
            "ilubenets",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("PROJ-1  "));
}

/// The whole point of a dry run: nothing reaches Tracker. The stub has no
/// mounted route, so any request at all would fail the test.
#[tokio::test]
async fn dry_run_sends_nothing_and_prints_the_body() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "issue",
            "create",
            "-q",
            "PROJ",
            "-s",
            "nothing to see",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "dry run: would create an issue in PROJ",
        ))
        .stderr(predicate::str::contains("\"summary\": \"nothing to see\""));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// "Which organisation did I just change" must never be a question the output
/// leaves open.
#[tokio::test]
async fn every_write_announces_the_profile_and_organisation() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/comments"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({"id": 205, "text": "done"})),
        )
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "comment", "PROJ-1", "done"])
        .assert()
        .success()
        .stderr(predicate::str::contains("→ profile=test org=12345"))
        .stdout(predicate::str::contains("PROJ-1 comment 205"));
}

#[tokio::test]
async fn update_types_values_by_reading_them_as_json() {
    let harness = Harness::new().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/issues/PROJ-1"))
        .and(body_json(serde_json::json!({
            "storyPoints": 3,
            "summary": "still a string",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue.json")))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "--set",
            "storyPoints=3",
            "--set",
            "summary=still a string",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn an_update_that_changes_nothing_is_refused() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "update", "PROJ-1"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nothing to change"));
}

#[tokio::test]
async fn a_malformed_assignment_is_refused_before_anything_is_sent() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "update", "PROJ-1", "--set", "storyPoints"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("expected key=value"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// Listing transitions is a read, and must not be gated as a write.
#[tokio::test]
async fn transition_without_an_id_lists_what_is_available() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/transitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": "close",
                "display": "Close",
                "to": {"id": "3", "key": "closed", "display": "Closed"}
            },
            {
                "id": "start_progress",
                "display": "Start progress",
                "to": {"id": "2", "key": "inProgress", "display": "In Progress"}
            }
        ])))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "transition", "PROJ-1"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("close"));
    assert!(stdout.contains("→ Closed"));
    assert!(stdout.ends_with("shown 2 of 2 for PROJ-1\n"));
}

#[tokio::test]
async fn transition_with_an_id_executes_it() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/transitions/close/_execute"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "transition", "PROJ-1", "close"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1 close"));
}

/// A create that timed out must not be retried: a duplicate issue is worse than
/// a clear failure.
#[tokio::test]
async fn a_failed_write_is_not_retried() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "create", "-q", "PROJ", "-s", "once"])
        .assert()
        .code(5);
}
