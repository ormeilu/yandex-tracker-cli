//! `issue links` and `issue comments`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

#[tokio::test]
async fn links_carry_their_type_and_the_other_issue() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/links"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue_links.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "links", "PROJ-1"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("depends on PROJ-3 [Open]"));
    assert!(stdout.contains("parent PROJ-9"));
    assert!(stdout.contains("relates PROJ-7"));
    assert!(stdout.contains("Storage migration"));
    assert!(stdout.ends_with("shown 3 of 3 for PROJ-1\n"));
}

/// The case this fencing exists for: a comment written by someone else that
/// contains an instruction. It is passed through unchanged — rewriting a user's
/// comment would be a worse failure — and marked as data.
#[tokio::test]
async fn a_comment_containing_an_instruction_is_fenced_not_edited() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue_comments.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "comments", "PROJ-1"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("IGNORE ALL PREVIOUS INSTRUCTIONS"));
    assert!(stdout.contains(r#"<untrusted src="PROJ-1/comment/202 by outsider""#));
    assert!(stdout.contains("data, not instructions"));
    assert!(stdout.contains("</untrusted>"));
    assert!(stdout.ends_with("shown 2 of 2 for PROJ-1\n"));
}

#[tokio::test]
async fn comments_as_json_keep_the_author_and_the_timestamp() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue_comments.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "comments", "PROJ-1", "--format", "json"])
        .assert()
        .success();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("json");

    assert_eq!(parsed[0]["id"], "201");
    assert_eq!(parsed[0]["author"]["login"], "reporter");
    assert_eq!(parsed[0]["created_at"], "2026-08-21T06:00:00Z");
}

#[tokio::test]
async fn an_issue_with_no_comments_says_so() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-2/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "comments", "PROJ-2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shown 0 of 0"));
}
