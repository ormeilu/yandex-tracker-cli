//! `me` on a write.
//!
//! A read hands `me` to YQL and Tracker answers it; a write sends it as a login
//! and Tracker answers 422, "no such user". The promises here: the login goes
//! out in its place, a dry run already shows the login, only fields that hold
//! people are touched, and nothing is asked of Tracker when nobody said `me`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

/// `GET /v3/myself`, answered as `ilubenets` and expected exactly once: two
/// mentions of `me` in one command are one question.
async fn mount_myself(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("user.json")))
        .expect(1)
        .mount(&harness.server)
        .await;
}

async fn mount_field(harness: &Harness, key: &str, schema: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/v3/fields/{key}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": key, "key": key, "name": key, "schema": schema,
        })))
        .mount(&harness.server)
        .await;
}

async fn requested(harness: &Harness, wanted: &str) -> bool {
    harness
        .server
        .received_requests()
        .await
        .expect("recorded")
        .iter()
        .any(|request| request.url.path() == wanted)
}

/// The case the issue was filed over: what a dry run shows is what would be
/// sent, and that is a login.
#[tokio::test]
async fn a_dry_run_of_create_shows_the_login_where_me_was_written() {
    let harness = Harness::new().await;
    mount_myself(&harness).await;

    harness
        .run(&[
            "issue",
            "create",
            "-q",
            "PROJ",
            "-s",
            "t",
            "--assignee",
            "me",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("\"assignee\": \"ilubenets\""))
        .stderr(predicate::str::contains("\"me\"").not());

    assert!(
        !requested(&harness, "/v3/issues/").await,
        "a dry run must not write"
    );
}

#[tokio::test]
async fn update_sends_the_login_where_me_was_written() {
    let harness = Harness::new().await;
    mount_myself(&harness).await;
    mount_field(
        &harness,
        "followers",
        serde_json::json!({"type": "array", "items": "user"}),
    )
    .await;
    Mock::given(method("PATCH"))
        .and(path("/v3/issues/PROJ-1"))
        .and(body_json(serde_json::json!({
            "assignee": "ilubenets",
            "followers": ["ilubenets", "ivan"],
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue.json")))
        .expect(1)
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "--assignee",
            "me",
            "--set",
            r#"followers=["me","ivan"]"#,
        ])
        .assert()
        .success();
}

/// A comma-separated list stays a string; only the entry that says `me`
/// changes.
#[tokio::test]
async fn me_in_a_comma_separated_list_is_replaced_entry_by_entry() {
    let harness = Harness::new().await;
    mount_myself(&harness).await;
    mount_field(
        &harness,
        "followers",
        serde_json::json!({"type": "array", "items": "user"}),
    )
    .await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "--set",
            "followers=me,ivan",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "\"followers\": \"ilubenets,ivan\"",
        ));
}

/// `summary=me` is a summary. The registry says what a field holds; its name
/// does not, and the login is not even asked for.
#[tokio::test]
async fn me_in_a_field_that_does_not_hold_people_is_left_alone() {
    let harness = Harness::new().await;
    mount_field(&harness, "summary", serde_json::json!({"type": "string"})).await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "--set",
            "summary=me",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("\"summary\": \"me\""));

    assert!(!requested(&harness, "/v3/myself").await);
}

/// A queue's own field is not in the organisation's registry, which answers
/// 404 for it; the queue's local fields say what it holds.
#[tokio::test]
async fn a_local_field_that_holds_people_is_found_in_its_queue() {
    let harness = Harness::new().await;
    mount_myself(&harness).await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/localFields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queue_local_fields.json")))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "create",
            "-q",
            "PROJ",
            "-s",
            "t",
            "--set",
            "reviewers=me",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("\"reviewers\": \"ilubenets\""));
}

/// Not knowing what a field holds is a reason to stop, not to send `me` and
/// let Tracker answer 422 for it.
#[tokio::test]
async fn a_field_whose_type_cannot_be_read_stops_the_write() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/fields/qaEngineer"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "update", "PROJ-1", "--set", "qaEngineer=me"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains(
            "could not tell whether `qaEngineer` holds people",
        ));

    assert!(!requested(&harness, "/v3/issues/PROJ-1").await);
}

/// No `me`, no extra request: the convenience costs nothing to those who do
/// not use it.
#[tokio::test]
async fn a_write_without_me_asks_tracker_nothing_first() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "--assignee",
            "ivan",
            "--set",
            "followers=ivan,petr",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("\"assignee\": \"ivan\""));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

#[tokio::test]
async fn a_transition_field_that_holds_people_takes_me() {
    let harness = Harness::new().await;
    mount_myself(&harness).await;
    mount_field(&harness, "assignee", serde_json::json!({"type": "user"})).await;

    harness
        .run(&[
            "issue",
            "transition",
            "PROJ-1",
            "start",
            "--set",
            "assignee=me",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("\"assignee\": \"ilubenets\""));
}

#[tokio::test]
async fn a_checklist_line_can_be_assigned_to_me() {
    let harness = Harness::new().await;
    mount_myself(&harness).await;

    harness
        .run(&[
            "issue",
            "check",
            "add",
            "PROJ-1",
            "write the tests",
            "--assignee",
            "me",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("\"assignee\": \"ilubenets\""));
}
