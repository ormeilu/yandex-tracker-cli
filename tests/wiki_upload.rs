//! Wiki uploads: four requests a file, aborted when one fails.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, header, method, path, query_param};
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

fn session(status: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "session_id": "s1", "file_name": "notes.txt", "file_size": 5,
        "status": status, "created_at": "2026-09-11T10:00:00Z",
        "user": {"id": 11, "username": "ilubenets", "display_name": "Ilya Lubenets",
                 "is_dismissed": false, "affiliation": "staff"}
    }))
}

async fn session_opens(harness: &Harness) {
    Mock::given(method("POST"))
        .and(path("/v1/upload_sessions"))
        .and(body_json(
            serde_json::json!({ "file_name": "notes.txt", "file_size": 5 }),
        ))
        .respond_with(session("not_started"))
        .mount(&harness.server)
        .await;
}

fn notes(dir: &tempfile::TempDir) -> String {
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, b"hello").unwrap();
    file.to_str().unwrap().to_owned()
}

fn paths(requests: &[wiremock::Request]) -> Vec<String> {
    requests
        .iter()
        .map(|request| format!("{} {}", request.method, request.url.path()))
        .collect()
}

/// Opened, sent as raw bytes, finished, attached — and the output is the
/// attachment it became.
#[tokio::test]
async fn a_file_goes_through_an_upload_session_to_the_page() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    session_opens(&harness).await;
    Mock::given(method("PUT"))
        .and(path("/v1/upload_sessions/s1/upload_part"))
        .and(query_param("part_number", "1"))
        .and(header("content-type", "application/octet-stream"))
        .respond_with(session("in_progress"))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/upload_sessions/s1/finish"))
        .respond_with(session("finished"))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/attachments"))
        .and(body_json(serde_json::json!({ "upload_sessions": ["s1"] })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "results": [{
                "id": 903, "name": "notes.txt", "download_url": "users/ilubenets/runbook/.files/notes.txt",
                "size": "0.00", "description": "", "created_at": "2026-09-11T10:00:01Z",
                "mimetype": "text/plain", "has_preview": false
            }]
        })))
        .mount(&harness.server)
        .await;
    let dir = tempfile::tempdir().expect("temp dir");

    harness
        .run(&["wiki", "upload", "users/ilubenets/runbook", &notes(&dir)])
        .assert()
        .success()
        .stdout("uploaded notes.txt to users/ilubenets/runbook: attachment 903\n")
        .stderr(predicate::str::contains("profile="));

    let requests = harness.server.received_requests().await.expect("recorded");
    let part = requests
        .iter()
        .find(|request| request.method.as_str() == "PUT")
        .expect("a part was sent");
    assert_eq!(part.body, b"hello");
}

/// A part that fails leaves no session behind holding quota.
#[tokio::test]
async fn a_failed_part_aborts_the_session() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    session_opens(&harness).await;
    Mock::given(method("PUT"))
        .and(path("/v1/upload_sessions/s1/upload_part"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/upload_sessions/s1/abort"))
        .respond_with(session("aborted"))
        .mount(&harness.server)
        .await;
    let dir = tempfile::tempdir().expect("temp dir");

    harness
        .run(&["wiki", "upload", "users/ilubenets/runbook", &notes(&dir)])
        .assert()
        .failure();

    let sent = paths(&harness.server.received_requests().await.expect("recorded"));
    assert!(
        sent.contains(&"POST /v1/upload_sessions/s1/abort".to_owned()),
        "{sent:?}"
    );
    assert!(
        !sent.iter().any(|line| line.ends_with("/attachments")),
        "{sent:?}"
    );
}

/// Every file is read first: a missing one stops the command before a byte
/// is sent.
#[tokio::test]
async fn a_missing_file_stops_the_upload_before_anything_is_sent() {
    let harness = Harness::new().await;
    let dir = tempfile::tempdir().expect("temp dir");

    harness
        .run(&[
            "wiki",
            "upload",
            "users/ilubenets/runbook",
            &notes(&dir),
            "no-such-file.pdf",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no-such-file.pdf"));
    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

#[tokio::test]
async fn a_dry_run_upload_sends_nothing() {
    let harness = Harness::new().await;
    let dir = tempfile::tempdir().expect("temp dir");

    harness
        .run(&[
            "wiki",
            "upload",
            "users/ilubenets/runbook",
            &notes(&dir),
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("dry run: would upload 1 file"));
    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// No undo, so --yes; and the file is found by name in the listing.
#[tokio::test]
async fn deleting_an_attachment_needs_yes_and_names_the_file() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "wiki",
            "delete-attachment",
            "users/ilubenets/runbook",
            "rollback.pdf",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));
    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );

    page_is_4521(&harness).await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/4521/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_attachments.json")))
        .mount(&harness.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/pages/4521/attachments/901"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "delete-attachment",
            "users/ilubenets/runbook",
            "rollback.pdf",
            "--yes",
        ])
        .assert()
        .success()
        .stdout("deleted attachment rollback.pdf (901) from users/ilubenets/runbook\n");
}
