//! `wiki download`: one file off a page, into a directory the caller named.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

async fn page_with_files(harness: &Harness, files: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/v1/pages"))
        .and(query_param("slug", "users/ilubenets/runbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_page.json")))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/4521/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(files))
        .mount(&harness.server)
        .await;
}

async fn file_901_is(harness: &Harness, body: &'static [u8]) {
    Mock::given(method("GET"))
        .and(path("/v1/pages/4521/attachments/901/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&harness.server)
        .await;
}

fn downloads(requests: &[wiremock::Request]) -> usize {
    requests
        .iter()
        .filter(|request| request.url.path().ends_with("/download"))
        .count()
}

/// A file named as `wiki attachments` lists it lands under its own name, and
/// the path it went to is the output.
#[tokio::test]
async fn a_file_named_by_its_name_is_written_under_it() {
    let harness = Harness::new().await;
    page_with_files(&harness, fixture("wiki_attachments.json")).await;
    file_901_is(&harness, b"%PDF-1.7").await;
    let out = tempfile::tempdir().expect("temp dir");

    harness
        .run(&[
            "wiki",
            "download",
            "users/ilubenets/runbook",
            "rollback.pdf",
            "-o",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::ends_with("rollback.pdf\n"));

    assert_eq!(
        std::fs::read(out.path().join("rollback.pdf")).unwrap(),
        b"%PDF-1.7"
    );
}

/// The id works as well as the name.
#[tokio::test]
async fn a_file_named_by_its_id_is_found_too() {
    let harness = Harness::new().await;
    page_with_files(&harness, fixture("wiki_attachments.json")).await;
    file_901_is(&harness, b"%PDF-1.7").await;
    let out = tempfile::tempdir().expect("temp dir");

    harness
        .run(&[
            "wiki",
            "download",
            "users/ilubenets/runbook",
            "901",
            "-o",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(out.path().join("rollback.pdf").exists());
}

/// An existing file is kept, and nothing is fetched to find that out.
#[tokio::test]
async fn an_existing_file_is_not_overwritten_without_force() {
    let harness = Harness::new().await;
    page_with_files(&harness, fixture("wiki_attachments.json")).await;
    file_901_is(&harness, b"new").await;
    let out = tempfile::tempdir().expect("temp dir");
    std::fs::write(out.path().join("rollback.pdf"), b"old").unwrap();
    let dir = out.path().to_str().unwrap();

    harness
        .run(&[
            "wiki",
            "download",
            "users/ilubenets/runbook",
            "rollback.pdf",
            "-o",
            dir,
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--force"));
    let requests = harness.server.received_requests().await.expect("recorded");
    assert_eq!(downloads(&requests), 0);
    assert_eq!(
        std::fs::read(out.path().join("rollback.pdf")).unwrap(),
        b"old"
    );

    harness
        .run(&[
            "wiki",
            "download",
            "users/ilubenets/runbook",
            "rollback.pdf",
            "-o",
            dir,
            "--force",
        ])
        .assert()
        .success();
    assert_eq!(
        std::fs::read(out.path().join("rollback.pdf")).unwrap(),
        b"new"
    );
}

/// Whoever uploaded the file chose its name; it decides the name inside the
/// directory and nothing about which directory.
#[tokio::test]
async fn a_crafted_name_cannot_leave_the_directory() {
    let harness = Harness::new().await;
    page_with_files(
        &harness,
        serde_json::json!({
            "results": [{
                "id": 901, "name": "../../evil.sh", "download_url": "x",
                "size": "0.01", "description": "", "created_at": "2026-09-01T11:00:00Z",
                "mimetype": "text/x-sh", "has_preview": false
            }],
            "next_cursor": null
        }),
    )
    .await;
    file_901_is(&harness, b"echo hi").await;
    let parent = tempfile::tempdir().expect("temp dir");
    let out = parent.path().join("inside");

    harness
        .run(&[
            "wiki",
            "download",
            "users/ilubenets/runbook",
            "901",
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(out.join("evil.sh").exists());
    assert!(!parent.path().join("evil.sh").exists());
}

/// A file's own address needs no page lookup: the Wiki resolves it, moved
/// pages included.
#[tokio::test]
async fn a_file_address_is_downloaded_by_it() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/attachments/download_by_url"))
        .and(query_param(
            "url",
            "users/ilubenets/runbook/.files/rollback.pdf",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(&b"%PDF-1.7"[..]))
        .mount(&harness.server)
        .await;
    let out = tempfile::tempdir().expect("temp dir");

    harness
        .run(&[
            "wiki",
            "download",
            "https://wiki.yandex.ru/users/ilubenets/runbook/.files/rollback.pdf",
            "-o",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(
        std::fs::read(out.path().join("rollback.pdf")).unwrap(),
        b"%PDF-1.7"
    );
}

#[tokio::test]
async fn a_file_the_page_does_not_have_is_not_found() {
    let harness = Harness::new().await;
    page_with_files(&harness, fixture("wiki_attachments.json")).await;
    let out = tempfile::tempdir().expect("temp dir");

    harness
        .run(&[
            "wiki",
            "download",
            "users/ilubenets/runbook",
            "nope.pdf",
            "-o",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .code(4)
        .stderr(predicate::str::contains(
            "attachment `nope.pdf` on wiki page `users/ilubenets/runbook` not found",
        ));
}

/// A page with no file named is a question, not a guess.
#[tokio::test]
async fn a_page_without_a_file_is_refused() {
    let harness = Harness::new().await;
    let out = tempfile::tempdir().expect("temp dir");

    harness
        .run(&[
            "wiki",
            "download",
            "users/ilubenets/runbook",
            "-o",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("name the file"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}
