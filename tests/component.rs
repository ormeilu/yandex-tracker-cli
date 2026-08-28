//! `component list`, against a stub Tracker.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

/// The name leads because the name is what a write takes, and half of a real
/// organisation's components have no lead at all.
#[tokio::test]
async fn components_lead_with_the_name_a_write_would_use() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/components"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("components.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["component", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.starts_with("Billing"), "{stdout}");
    assert!(stdout.contains("PROJ"), "{stdout}");
    assert!(stdout.contains("ilubenets"), "{stdout}");
    // A component with no lead is normal, not an error.
    assert!(stdout.contains("Platform: backend"), "{stdout}");
    assert!(stdout.ends_with("shown 2 of 2\n"), "{stdout}");
}

/// Tracker filters by queue itself. Fetching every component in order to throw
/// most of them away is the cost this tool exists to avoid, so `--queue` is a
/// different request — and the tally says which queue it answered for.
#[tokio::test]
async fn one_queue_is_a_different_request_not_a_filter_here() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/INFRA/components"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "id": 6,
                "name": "Platform: backend",
                "queue": {"key": "INFRA"},
                "assignAuto": false
            }])),
        )
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["component", "list", "-q", "INFRA"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.ends_with("shown 1 of 1 for INFRA\n"), "{stdout}");
}

/// Setting a component that assigns automatically does two things, and the
/// second one is not obvious from the write.
#[tokio::test]
async fn a_component_that_reassigns_says_so() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/components"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("components.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["component", "list", "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("json");

    assert_eq!(parsed[0]["assign_auto"], serde_json::json!(true));
    assert_eq!(parsed[1]["assign_auto"], serde_json::json!(false));
    assert_eq!(parsed[1]["lead"], serde_json::Value::Null);
}
