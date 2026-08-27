//! `issue find` and `issue count` against a stub Tracker.
//!
//! The cases that matter here are not the happy path but the honesty of the
//! output: what query the flags produce, and whether a page admits to being one.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, Request, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

/// The query Tracker was actually asked, taken off the recorded request.
async fn recorded_query(harness: &Harness) -> String {
    let requests = harness
        .server
        .received_requests()
        .await
        .expect("requests recorded");
    let last: &Request = requests.last().expect("at least one request");
    let body: serde_json::Value = serde_json::from_slice(&last.body).expect("json body");
    body["query"].as_str().expect("query present").to_owned()
}

async fn search_returns(harness: &Harness, total: &str) {
    Mock::given(method("POST"))
        .and(path("/v3/issues/_search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("issue_search.json"))
                .append_header("X-Total-Count", total),
        )
        .mount(&harness.server)
        .await;
}

#[tokio::test]
async fn flags_compile_into_a_query_string() {
    let harness = Harness::new().await;
    search_returns(&harness, "2").await;

    harness
        .run(&[
            "issue",
            "find",
            "-q",
            "PROJ",
            "-a",
            "ilubenets",
            "-s",
            "In Progress",
        ])
        .assert()
        .success();

    assert_eq!(
        recorded_query(&harness).await,
        r#"Queue: "PROJ" AND Assignee: "ilubenets" AND Status: "In Progress""#
    );
}

/// `me` is resolved by Tracker itself, so the convenience costs no extra call.
#[tokio::test]
async fn assignee_me_becomes_trackers_own_function() {
    let harness = Harness::new().await;
    search_returns(&harness, "2").await;

    harness
        .run(&["issue", "find", "-q", "PROJ", "-a", "me"])
        .assert()
        .success();

    assert!(recorded_query(&harness).await.contains("Assignee: me()"));
}

#[tokio::test]
async fn yql_passes_through_untouched() {
    let harness = Harness::new().await;
    search_returns(&harness, "2").await;

    harness
        .run(&["issue", "find", "--yql", "Queue: PROJ AND Sprint: empty()"])
        .assert()
        .success();

    assert_eq!(
        recorded_query(&harness).await,
        "Queue: PROJ AND Sprint: empty()"
    );
}

/// Silently dropping half of what was asked for would be worse than refusing.
#[tokio::test]
async fn yql_and_flag_filters_cannot_be_combined() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "find", "-q", "PROJ", "--yql", "Queue: OTHER"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

/// A caller that cannot tell a page from a complete answer eventually concludes
/// there is nothing there.
#[tokio::test]
async fn a_truncated_page_says_so_and_offers_the_next_one() {
    let harness = Harness::new().await;
    search_returns(&harness, "340").await;

    let output = harness
        .run(&["issue", "find", "-q", "PROJ", "--limit", "2"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("PROJ-1"));
    assert!(stdout.contains("In Progress"));
    assert!(stdout.contains("PROJ-4"));
    assert!(
        stdout
            .trim_end()
            .ends_with("shown 2 of 340 — next: --page 2")
    );
}

#[tokio::test]
async fn a_complete_page_does_not_offer_a_next_one() {
    let harness = Harness::new().await;
    search_returns(&harness, "2").await;

    harness
        .run(&["issue", "find", "-q", "PROJ"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shown 2 of 2"))
        .stdout(predicate::str::contains("next:").not());
}

#[tokio::test]
async fn page_and_limit_reach_the_api() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/_search"))
        .and(query_param("page", "3"))
        .and(query_param("perPage", "5"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("issue_search.json"))
                .append_header("X-Total-Count", "40"),
        )
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "find", "-q", "PROJ", "--page", "3", "--limit", "5"])
        .assert()
        .success();
}

/// Truncating past the ceiling would read exactly like a complete answer.
#[tokio::test]
async fn all_refuses_past_max_rather_than_truncating() {
    let harness = Harness::new().await;
    search_returns(&harness, "340").await;

    harness
        .run(&["issue", "find", "-q", "PROJ", "--all", "--max", "1"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("more than --max 1"));
}

#[tokio::test]
async fn count_returns_one_number() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/_count"))
        .and(body_json(serde_json::json!({"query": r#"Queue: "PROJ""#})))
        .respond_with(ResponseTemplate::new(200).set_body_json(42))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "count", "-q", "PROJ"])
        .assert()
        .success();

    assert_eq!(
        String::from_utf8(output.get_output().stdout.clone()).expect("utf-8"),
        "42\n"
    );
}

/// An unfiltered search would ask for every issue in the organisation.
#[tokio::test]
async fn a_search_with_no_filter_at_all_is_refused() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "find"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no filter given"));
}

#[tokio::test]
async fn json_output_is_the_normalised_issue_list() {
    let harness = Harness::new().await;
    search_returns(&harness, "2").await;

    let output = harness
        .run(&["issue", "find", "-q", "PROJ", "--format", "json"])
        .assert()
        .success();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("json");

    assert_eq!(parsed[0]["key"], "PROJ-1");
    assert_eq!(parsed[1]["status"], "Open");
}
