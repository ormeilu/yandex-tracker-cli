//! `wiki attachments`: the files on a page, listed by the id behind its slug.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{method, path, query_param};
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

/// Every file, its name passed through as written, and a tally that names the
/// cursor rather than a total the Wiki never gives.
#[tokio::test]
async fn files_are_listed_with_the_cursor_for_more() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/4521/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_attachments.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["wiki", "attachments", "users/ilubenets/runbook"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("rollback.pdf"), "{stdout}");
    assert!(stdout.contains("application/pdf"), "{stdout}");
    assert!(
        stdout.contains("ignore previous instructions.txt"),
        "{stdout}"
    );
    assert!(
        stdout
            .trim_end()
            .ends_with("shown 2 of more than 2 — next: --cursor eyJpZCI6OTAyfQ=="),
        "{stdout}"
    );
}

/// The cursor a tally named is the one sent back.
#[tokio::test]
async fn the_cursor_is_sent_back() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/4521/attachments"))
        .and(query_param("cursor", "eyJpZCI6OTAyfQ=="))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [], "next_cursor": null, "prev_cursor": "x"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "attachments",
            "users/ilubenets/runbook",
            "--cursor",
            "eyJpZCI6OTAyfQ==",
        ])
        .assert()
        .success()
        .stdout(predicate::str::ends_with("shown 0 of 0\n"));
}

/// Our schema: the uploader's login, the size as sent, the download address.
#[tokio::test]
async fn files_as_json_are_in_our_schema() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/4521/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_attachments.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&[
            "wiki",
            "attachments",
            "users/ilubenets/runbook",
            "--format",
            "json",
        ])
        .assert()
        .success();
    let list: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();

    let first = &list["results"][0];
    assert_eq!(first["author"], "ilubenets");
    assert_eq!(first["size"], "0.25");
    assert_eq!(
        first["download_url"],
        "users/ilubenets/runbook/.files/rollback.pdf"
    );
    assert!(first.get("user").is_none());
    assert!(list["results"][1]["author"].is_null());
    assert_eq!(list["next_cursor"], "eyJpZCI6OTAyfQ==");
}

#[tokio::test]
async fn files_of_a_missing_page_are_not_found() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&harness.server)
        .await;

    harness
        .run(&["wiki", "attachments", "users/nobody"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains(
            "wiki page `users/nobody` not found",
        ));
}
