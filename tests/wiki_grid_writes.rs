//! Wiki grid writes: each made against a revision, announced and gated.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

const GRID: &str = "8f1e2d3c-4b5a-4c6d-8e7f-9a0b1c2d3e4f";

/// The read a write without --revision starts with; the fixture is at 12.
async fn grid_is_at_revision_12(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/grids/{GRID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_grid.json")))
        .mount(&harness.server)
        .await;
}

async fn nothing_was_sent(harness: &Harness) {
    let requests = harness.server.received_requests().await.expect("recorded");
    assert!(requests.is_empty(), "{requests:?}");
}

fn revision(to: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({ "revision": to }))
}

/// Rows from stdin go with the revision just read, and the output names the
/// rows made and the revision left.
#[tokio::test]
async fn rows_are_added_against_the_current_revision() {
    let harness = Harness::new().await;
    grid_is_at_revision_12(&harness).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/grids/{GRID}/rows")))
        .and(body_json(serde_json::json!({
            "rows": [{"version": "1.4.0"}],
            "after_row_id": "2",
            "revision": "12"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "revision": "13",
            "results": [{"id": "3", "row": ["1.4.0", [], null, null, false]}]
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["wiki", "rows-add", GRID, "--from", "-", "--after", "2"])
        .write_stdin(r#"[{"version": "1.4.0"}]"#)
        .assert()
        .success()
        .stdout(format!(
            "added 1 row to grid {GRID} (rows 3); revision 13\n"
        ))
        .stderr(predicate::str::contains("profile="));
}

/// A revision the caller read is the one sent, and the grid is not read again:
/// a change made since is the Wiki's to refuse.
#[tokio::test]
async fn a_given_revision_is_sent_as_it_is() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/grids/{GRID}/cells")))
        .and(body_json(serde_json::json!({
            "cells": [
                {"row_id": 1, "column_slug": "done", "value": true},
                {"row_id": 2, "column_slug": "version", "value": "1.3.1"}
            ],
            "revision": "11"
        })))
        .respond_with(revision("12"))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "cells-set",
            GRID,
            "--set",
            "1:done=true",
            "--set",
            "2:version=1.3.1",
            "--revision",
            "11",
        ])
        .assert()
        .success()
        .stdout(format!("set 2 cells in grid {GRID}; revision 12\n"));

    let requests = harness.server.received_requests().await.expect("recorded");
    assert!(
        requests
            .iter()
            .all(|request| request.method.as_str() != "GET"),
        "{requests:?}"
    );
}

/// A refused revision is the Wiki's words, and a failure.
#[tokio::test]
async fn a_stale_revision_is_refused_not_overwritten() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/grids/{GRID}/cells")))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "debug_message": "revision mismatch", "details": null, "error_code": "CONFLICT"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "cells-set",
            GRID,
            "--set",
            "1:done=true",
            "--revision",
            "3",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("revision mismatch"));
}

#[tokio::test]
async fn a_malformed_cell_is_refused_before_any_request() {
    let harness = Harness::new().await;

    harness
        .run(&["wiki", "cells-set", GRID, "--set", "done=true"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ROW:SLUG=VALUE"));
    nothing_was_sent(&harness).await;
}

/// A dry run says where the revision will come from, and sends nothing — not
/// even the read for it.
#[tokio::test]
async fn a_dry_run_grid_write_sends_nothing() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "wiki",
            "rows-move",
            GRID,
            "4",
            "--position",
            "0",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("<current, read first>"));
    nothing_was_sent(&harness).await;
}

/// Deleting rows, columns or the grid has no undo: --yes, and nothing sent
/// without it.
#[tokio::test]
async fn deletes_need_yes() {
    let harness = Harness::new().await;
    for args in [
        &["wiki", "rows-delete", GRID, "3"][..],
        &["wiki", "columns-delete", GRID, "notes"],
        &["wiki", "grid-delete", GRID],
    ] {
        harness
            .run(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("--yes"));
    }
    nothing_was_sent(&harness).await;

    grid_is_at_revision_12(&harness).await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/grids/{GRID}/rows")))
        .and(body_json(
            serde_json::json!({ "row_ids": ["3", "4"], "revision": "12" }),
        ))
        .respond_with(revision("13"))
        .mount(&harness.server)
        .await;

    harness
        .run(&["wiki", "rows-delete", GRID, "3", "4", "--yes"])
        .assert()
        .success()
        .stdout(format!("deleted 2 rows from grid {GRID}; revision 13\n"));
}

#[tokio::test]
async fn a_grid_is_created_on_a_page() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/grids"))
        .and(body_json(serde_json::json!({
            "page": {"slug": "users/ilubenets/runbook"}, "title": "Releases"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_grid.json")))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "grid-create",
            "users/ilubenets/runbook",
            "--title",
            "Releases",
        ])
        .assert()
        .success()
        .stdout(format!(
            "created grid {GRID} on users/ilubenets/runbook; revision 12\n"
        ));
}

/// The default order is the Wiki's map of slug to direction.
#[tokio::test]
async fn a_grid_order_is_sent_as_a_map() {
    let harness = Harness::new().await;
    grid_is_at_revision_12(&harness).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/grids/{GRID}")))
        .and(body_json(serde_json::json!({
            "default_sort": {"version": "desc"}, "revision": "12"
        })))
        .respond_with(revision("13"))
        .mount(&harness.server)
        .await;

    harness
        .run(&["wiki", "grid-update", GRID, "--sort", "version:desc"])
        .assert()
        .success()
        .stdout(format!("changed grid {GRID}; revision 13\n"));
}

/// Columns come from JSON, and a file that is not a list of them is refused.
#[tokio::test]
async fn columns_come_from_a_json_list() {
    let harness = Harness::new().await;

    harness
        .run(&["wiki", "columns-add", GRID, "--from", "-"])
        .write_stdin(r#"{"slug": "notes"}"#)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("expected a JSON array"));
    nothing_was_sent(&harness).await;

    grid_is_at_revision_12(&harness).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/grids/{GRID}/columns")))
        .and(body_json(serde_json::json!({
            "columns": [{"slug": "notes", "title": "Notes", "type": "string", "required": false}],
            "position": 1,
            "revision": "12"
        })))
        .respond_with(revision("13"))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "columns-add",
            GRID,
            "--from",
            "-",
            "--position",
            "1",
        ])
        .write_stdin(
            r#"[{"slug": "notes", "title": "Notes", "type": "string", "required": false}]"#,
        )
        .assert()
        .success()
        .stdout(format!("added 1 column to grid {GRID}; revision 13\n"));
}
