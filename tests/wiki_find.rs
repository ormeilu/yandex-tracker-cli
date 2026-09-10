//! `wiki find`: the Wiki's search, the one Wiki listing that pages by number.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

/// The first page of hits, and a tally that neither invents a total nor lets
/// the page pass for all of them.
#[tokio::test]
async fn hits_end_with_the_next_page_to_ask_for() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(body_partial_json(
            serde_json::json!({"query": "deploy runbook", "cursor": 1}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_search.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["wiki", "find", "deploy runbook"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("users/ilubenets/runbook"), "{stdout}");
    assert!(stdout.contains("Deploy runbook"), "{stdout}");
    assert!(
        stdout
            .trim_end()
            .ends_with("shown 2 of more than 2 — next: --page 2"),
        "{stdout}"
    );
}

/// The page number and the type filter reach the Wiki as its own fields, and a
/// page with nothing after it says it is the last.
#[tokio::test]
async fn a_later_page_of_one_type_is_asked_for_as_such() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(body_partial_json(
            serde_json::json!({"cursor": 2, "filters": {"type": "page"}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [{
                "url": "https://wiki.yandex.ru/users/ilubenets/notes/",
                "slug": "users/ilubenets/notes",
                "title": "Notes",
                "content": "",
                "type": "page",
                "modified_at": "2026-07-01T00:00:00Z"
            }],
            "next_cursor": null,
            "prev_cursor": "1"
        })))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["wiki", "find", "rollback", "--type", "page", "--page", "2"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("users/ilubenets/notes"), "{stdout}");
    assert!(stdout.trim_end().ends_with("shown 1 of 1"), "{stdout}");
}

/// The Wiki stops at page 500; asking past it is refused before a request.
#[tokio::test]
async fn a_page_past_the_last_one_search_serves_is_refused() {
    let harness = Harness::new().await;

    harness
        .run(&["wiki", "find", "anything", "--page", "501"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("stops there"));

    let requests = harness.server.received_requests().await.expect("recorded");
    assert!(requests.is_empty());
}

/// Our schema: the Wiki's `content` is an excerpt, so it is called one, and the
/// next page number is there for a script to use.
#[tokio::test]
async fn hits_as_json_carry_the_excerpt_and_the_next_page() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_search.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["wiki", "find", "deploy", "--format", "json"])
        .assert()
        .success();
    let found: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();

    assert_eq!(found["results"][0]["type"], "page");
    assert_eq!(
        found["results"][0]["snippet"],
        "…tag the release, then watch the pipeline…"
    );
    assert!(found["results"][0].get("content").is_none());
    assert_eq!(found["next_page"], 2);
}
