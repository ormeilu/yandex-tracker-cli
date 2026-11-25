//! Searching for duplicates and reusing the direction of a link.
#![allow(clippy::unwrap_used)]

use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

#[tokio::test]
async fn text_terms_match_any_issue_text_field_without_dropping_other_filters() {
    let h = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/_search"))
        .and(body_json(json!({"query": r#"Queue: "PROJ" AND Status: "open" AND Type: "bug" AND (Summary: "MTU" OR Description: "MTU" OR Comment: "MTU" OR Summary: "ICMP \"echo\"\\path" OR Description: "ICMP \"echo\"\\path" OR Comment: "ICMP \"echo\"\\path")"#})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])).append_header("X-Total-Count", "0"))
        .expect(1)
        .mount(&h.server)
        .await;
    h.run(&[
        "issue",
        "find",
        "-q",
        "PROJ",
        "-s",
        "open",
        "--type",
        "bug",
        "--text",
        "MTU",
        "--text",
        r#"ICMP "echo"\path"#,
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("shown 0 of 0"));
}

#[tokio::test]
async fn empty_text_and_raw_query_conflicts_never_reach_tracker() {
    let h = Harness::new().await;
    for args in [
        vec!["--text", "  "],
        vec!["--text", "MTU", "--yql", "Queue: PROJ"],
    ] {
        let mut command = vec!["issue", "find"];
        command.extend(args);
        h.run(&command).assert().code(2);
    }
    assert!(h.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn relationship_view_shows_the_direction_to_use_when_copying_a_link() {
    let h = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/links"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue_links.json")))
        .mount(&h.server)
        .await;
    h.run(&["issue", "links", "PROJ-1", "--relationships"])
        .assert()
        .success()
        .stdout(predicate::str::contains("102  is subtask for PROJ-9"));
}

#[tokio::test]
async fn hierarchy_shortcuts_send_the_correct_relationship_in_each_direction() {
    let h = Harness::new().await;
    for (shortcut, phrase) in [
        ("subtask-of", "is subtask for"),
        ("parent-of", "is parent task for"),
    ] {
        Mock::given(method("POST"))
            .and(path("/v3/issues/PROJ-1/links"))
            .and(body_json(
                json!({"relationship": phrase, "issue": "PROJ-2"}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 5})))
            .expect(1)
            .mount(&h.server)
            .await;
        h.run(&[
            "issue",
            "link",
            "add",
            "PROJ-1",
            shortcut,
            "PROJ-2",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(phrase));
        h.run(&["issue", "link", "add", "PROJ-1", shortcut, "PROJ-2"])
            .assert()
            .success();
    }
}
