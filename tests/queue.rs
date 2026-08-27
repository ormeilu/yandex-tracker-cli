//! `queue list` and `queue fields` against a stub Tracker.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

#[tokio::test]
async fn queue_list_shows_key_name_and_lead() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queues.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("PROJ"));
    assert!(stdout.contains("Product"));
    assert!(stdout.contains("ilubenets"));
    // A queue without a lead still occupies its column.
    assert!(stdout.contains("INFRA"));
    assert!(stdout.contains(" -\n"));
    assert!(stdout.ends_with("shown 2 of 2\n"));
}

/// The point of the command: custom field keys are what `--fields` and `--set`
/// accept, and Tracker prefixes them with an opaque id that nobody can type.
#[tokio::test]
async fn queue_fields_exposes_typeable_custom_field_keys() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/fields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queue_fields.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "fields", "PROJ"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("storyPoints"));
    assert!(!stdout.contains("603bd9b6cdc7ba0d2f4b1a55--"));
    assert!(stdout.contains("integer"));
    assert!(stdout.contains("custom"));
    assert!(stdout.contains("summary"));
    assert!(stdout.contains("system"));
    assert!(stdout.ends_with("shown 4 of 4 (2 custom)\n"));
}

#[tokio::test]
async fn queue_fields_as_json_carries_the_same_keys() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/fields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queue_fields.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["queue", "fields", "PROJ", "--format", "json"])
        .assert()
        .success();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("json");

    assert_eq!(parsed[2]["key"], "storyPoints");
    assert_eq!(parsed[2]["field_type"], "integer");
    assert_eq!(parsed[2]["system"], false);
}

#[tokio::test]
async fn an_unknown_queue_exits_four() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/NOPE/fields"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&harness.server)
        .await;

    harness
        .run(&["queue", "fields", "NOPE"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("queue NOPE fields not found"));
}
