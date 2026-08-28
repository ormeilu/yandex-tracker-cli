//! `worklog find`, across issues.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::Harness;

fn entries() -> serde_json::Value {
    serde_json::json!([
        {
            "id": 1,
            "issue": {"key": "PROJ-1", "display": "Attachments are lost on move"},
            "duration": "PT1H30M",
            "start": "2026-08-24T09:00:00.000+0300",
            "createdBy": {"id": "1", "login": "ilubenets", "display": "Ilya Lubenets"},
            "comment": "pairing"
        },
        {
            "id": 2,
            "issue": {"key": "INFRA-7", "display": "Rotate the certificates"},
            "duration": "PT2H",
            "start": "2026-08-25T10:00:00.000+0300",
            "createdBy": {"id": "1", "login": "ilubenets", "display": "Ilya Lubenets"}
        }
    ])
}

async fn worklog_answers(harness: &Harness, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/v3/worklog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;
}

/// The issue is the column that carries the answer: without it the rows say how
/// long something took but not what.
#[tokio::test]
async fn every_row_names_the_issue_the_time_went_to() {
    let harness = Harness::new().await;
    worklog_answers(&harness, entries()).await;

    let output = harness
        .run(&["worklog", "find", "--by", "ilubenets"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("PROJ-1"), "{stdout}");
    assert!(stdout.contains("INFRA-7"), "{stdout}");
    assert!(
        stdout.ends_with("shown 2 of 2 — 3h 30m total\n"),
        "{stdout}"
    );
}

/// Tracker reads `createdBy` as a login and answers 422 for `me`, so `me` is
/// resolved first — and the search that follows carries the real login.
#[tokio::test]
async fn me_is_resolved_to_a_login_before_the_search() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uid": 1, "login": "ilubenets", "display": "Ilya Lubenets"
        })))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/worklog"))
        .and(query_param("createdBy", "ilubenets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(entries()))
        .mount(&harness.server)
        .await;

    harness
        .run(&["worklog", "find", "--by", "me"])
        .assert()
        .success();
}

/// A span is turned into a date here so the request carries the one form
/// Tracker documents.
#[tokio::test]
async fn a_span_reaches_tracker_as_a_date() {
    let harness = Harness::new().await;
    worklog_answers(&harness, entries()).await;

    harness
        .run(&["worklog", "find", "--since", "7d"])
        .assert()
        .success();

    let requests = harness.server.received_requests().await.unwrap();
    let query = requests[0].url.query().expect("a query").to_owned();
    assert!(query.contains("createdAt=from:20"), "{query}");
    assert!(!query.contains("7d"), "{query}");
}

/// This endpoint reports no total, so a full page is indistinguishable from a
/// complete answer unless the ceiling says so.
#[tokio::test]
async fn a_result_that_fills_the_limit_says_it_stopped() {
    let harness = Harness::new().await;
    worklog_answers(&harness, entries()).await;

    harness
        .run(&["worklog", "find", "--limit", "2"])
        .assert()
        .success()
        .stderr(predicate::str::contains("stopped at --limit 2"));
}

/// Nobody having logged anything is an answer, not a failure.
#[tokio::test]
async fn an_empty_result_is_a_success() {
    let harness = Harness::new().await;
    worklog_answers(&harness, serde_json::json!([])).await;

    harness
        .run(&["worklog", "find", "--by", "nobody"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shown 0 of 0"));
}
