//! Queue workflow contracts at the CLI and HTTP boundaries.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

#[tokio::test]
async fn batch_get_keeps_order_and_continues_past_a_missing_key() {
    let harness = Harness::new().await;
    for (key, summary) in [("PROJ-1", "First"), ("PROJ-2", "Second")] {
        Mock::given(method("GET"))
            .and(path(format!("/v3/issues/{key}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"key": key, "summary": summary})),
            )
            .mount(&harness.server)
            .await;
    }
    let args = ["issue", "get", "PROJ-2", "PROJ-9", "PROJ-1", "PROJ-2"];
    let mut text = args.to_vec();
    text.extend(["--fields", "summary"]);
    let result = harness.run(&text).assert().code(4).stdout(
        "PROJ-2  summary=Second\nPROJ-9  -\nPROJ-1  summary=First\nPROJ-2  summary=Second\nshown 3 of 4\n",
    );
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert_eq!(stderr.matches("profile=").count(), 1);
    assert!(stderr.contains("PROJ-9"));
    assert_eq!(harness.server.received_requests().await.unwrap().len(), 4);

    let mut raw = args.to_vec();
    raw.extend(["--format", "json-raw"]);
    let result = harness.run(&raw).assert().code(4);
    let rows: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(
        rows,
        json!([
            {"key": "PROJ-2", "summary": "Second"}, null,
            {"key": "PROJ-1", "summary": "First"}, {"key": "PROJ-2", "summary": "Second"}
        ])
    );
}

#[tokio::test]
async fn parent_and_epic_use_trackers_documented_query_fields() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/_search"))
        .and(body_json(
            json!({"query": r#"Queue: "PROJ" AND "Parent Issue": "PROJ-7" AND Epic: "PROJ-8""#}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([]))
                .append_header("X-Total-Count", "0"),
        )
        .expect(1)
        .mount(&harness.server)
        .await;
    harness
        .run(&[
            "issue", "find", "-q", "PROJ", "--parent", "PROJ-7", "--epic", "PROJ-8",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("shown 0 of 0"));
    harness
        .run(&[
            "issue",
            "find",
            "--parent",
            "PROJ-7",
            "--yql",
            "Queue: PROJ",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[tokio::test]
async fn link_add_only_accepts_a_rejected_write_if_the_same_link_exists() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/links"))
        .respond_with(
            ResponseTemplate::new(422).set_body_json(json!({"errorMessages": ["already linked"]})),
        )
        .expect(3)
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/links"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":"1","type":{"id":"relates","outward":"relates","inward":"relates"},"direction":"outward","object":{"key":"PROJ-2"}},
            {"id":"2","type":{"id":"depends","outward":"depends on","inward":"is dependent by"},"direction":"outward","object":{"key":"PROJ-3"}}
        ])))
        .expect(3)
        .mount(&harness.server)
        .await;
    for _ in 0..2 {
        harness
            .run(&["issue", "link", "add", "PROJ-1", "relates", "PROJ-2"])
            .assert()
            .success()
            .stdout("already linked: PROJ-1 relates PROJ-2\n");
    }
    harness
        .run(&[
            "issue",
            "link",
            "add",
            "PROJ-1",
            "is dependent by",
            "PROJ-3",
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("already linked"));
}

#[tokio::test]
async fn wiki_source_prints_only_the_page_markup_byte_for_byte() {
    let harness = Harness::new().await;
    let mut page = fixture("wiki_page.json");
    let content = "# Heading\r\n\nПривет  ";
    page["content"] = json!(content);
    Mock::given(method("GET"))
        .and(path("/v1/pages"))
        .and(query_param("slug", "example/runbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(page))
        .mount(&harness.server)
        .await;
    harness
        .run(&["wiki", "get", "example/runbook", "--source"])
        .assert()
        .success()
        .stdout(content);
}
