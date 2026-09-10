//! Wiki clones: accepted at once, finished later, and waited for.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

const GRID: &str = "8f1e2d3c-4b5a-4c6d-8e7f-9a0b1c2d3e4f";

async fn page_is_4521(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v1/pages"))
        .and(query_param("slug", "users/ilubenets/runbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_page.json")))
        .mount(&harness.server)
        .await;
}

fn started(id: &str, kind: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "operation": {"id": id, "type": kind},
        "status_url": format!("https://api.wiki.yandex.net/v1/operations/{kind}/{id}"),
        "dry_run": false
    }))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "callers build the result inline; a reference would only add ampersands"
)]
fn status(state: &str, percentage: f64, result: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "status": state,
        "progress": {"percentage": percentage, "details": ""},
        "result": result
    }))
}

async fn page_clone_accepted(harness: &Harness) {
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/clone"))
        .and(body_json(serde_json::json!({
            "target": "users/ilubenets/runbook-copy"
        })))
        .respond_with(started("op1", "clone"))
        .mount(&harness.server)
        .await;
}

/// The command waits through a running operation to the finished one, and
/// prints where the copy landed.
#[tokio::test]
async fn a_page_clone_is_waited_for() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    page_clone_accepted(&harness).await;
    Mock::given(method("GET"))
        .and(path("/v1/operations/clone/op1"))
        .respond_with(status("in_progress", 40.0, serde_json::Value::Null))
        .up_to_n_times(1)
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/operations/clone/op1"))
        .respond_with(status(
            "success",
            100.0,
            serde_json::json!({"page": {"id": 4600, "slug": "users/ilubenets/runbook-copy"}}),
        ))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "clone",
            "users/ilubenets/runbook",
            "users/ilubenets/runbook-copy",
        ])
        .assert()
        .success()
        .stdout("cloned users/ilubenets/runbook to users/ilubenets/runbook-copy\n")
        .stderr(predicate::str::contains("profile="));
}

/// `--no-wait` hands back the operation and the command that follows it,
/// without asking about it.
#[tokio::test]
async fn no_wait_returns_the_operation_at_once() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    page_clone_accepted(&harness).await;

    harness
        .run(&[
            "wiki",
            "clone",
            "users/ilubenets/runbook",
            "users/ilubenets/runbook-copy",
            "--no-wait",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("ytcli wiki operation clone op1"));

    let requests = harness.server.received_requests().await.expect("recorded");
    assert!(
        requests
            .iter()
            .all(|request| !request.url.path().starts_with("/v1/operations")),
        "{requests:?}"
    );
}

/// A documented refusal is named in words, not passed on as an envelope.
#[tokio::test]
async fn an_occupied_target_is_named() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/clone"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "debug_message": "", "details": null, "error_code": "SLUG_OCCUPIED"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "clone",
            "users/ilubenets/runbook",
            "users/ilubenets/runbook-copy",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "a page already exists at the target (SLUG_OCCUPIED)",
        ));
}

#[tokio::test]
async fn a_failed_operation_is_a_failure() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    page_clone_accepted(&harness).await;
    Mock::given(method("GET"))
        .and(path("/v1/operations/clone/op1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "failed",
            "progress": {"percentage": 10.0, "details": "target is locked"}
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "clone",
            "users/ilubenets/runbook",
            "users/ilubenets/runbook-copy",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the clone failed: target is locked",
        ));
}

/// A grid clone carries `with_data` and ends with the new grid's id.
#[tokio::test]
async fn a_grid_clone_with_data_names_the_new_grid() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/grids/{GRID}/clone")))
        .and(body_json(serde_json::json!({
            "target": "users/ilubenets/other", "with_data": true
        })))
        .respond_with(started("op2", "clone_inline_grid"))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/operations/clone_inline_grid/op2"))
        .respond_with(status(
            "success",
            100.0,
            serde_json::json!({
                "grid_id": "4c1d2e3f-0000-4000-8000-000000000002",
                "page": {"id": 4700, "slug": "users/ilubenets/other"}
            }),
        ))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "clone-grid",
            GRID,
            "users/ilubenets/other",
            "--with-data",
        ])
        .assert()
        .success()
        .stdout(format!(
            "cloned grid {GRID} to users/ilubenets/other: grid 4c1d2e3f-0000-4000-8000-000000000002\n"
        ));
}

#[tokio::test]
async fn a_dry_run_clone_sends_nothing() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "wiki",
            "clone",
            "users/ilubenets/runbook",
            "users/ilubenets/runbook-copy",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("dry run: would clone wiki page"));
    let requests = harness.server.received_requests().await.expect("recorded");
    assert!(requests.is_empty(), "{requests:?}");
}

/// Asking later reads the same operation, and says how far it has got.
#[tokio::test]
async fn an_operation_can_be_asked_about_later() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v1/operations/clone/op1"))
        .respond_with(status("in_progress", 40.0, serde_json::Value::Null))
        .mount(&harness.server)
        .await;

    harness
        .run(&["wiki", "operation", "clone", "op1"])
        .assert()
        .success()
        .stdout("operation clone op1: in_progress 40%\n");
}
