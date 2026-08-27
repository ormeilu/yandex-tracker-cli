//! Projects, goals and attachments.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::Harness;

fn project_page() -> serde_json::Value {
    serde_json::json!({
        "hits": 2,
        "pages": 1,
        "values": [
            {
                "id": "6511a4c2f0a1b2c3d4e5f6a7",
                "shortId": 12,
                "version": 3,
                "entityType": "project",
                "fields": {
                    "summary": "Storage rework",
                    "entityStatus": "in_progress",
                    "lead": {"id": "1120000000000219", "login": "ilubenets", "display": "Ilya Lubenets"},
                    "start": "2026-07-01",
                    "end": "2026-10-01"
                }
            },
            {
                "id": "6511a4c2f0a1b2c3d4e5f6b8",
                "shortId": 13,
                "entityType": "project",
                "fields": {"summary": "Billing", "entityStatus": "draft"}
            }
        ]
    })
}

/// An issue's `project` field refers to the short id, while `project get` takes
/// the long one. Printing only one of them guarantees somebody uses the wrong.
#[tokio::test]
async fn project_list_shows_both_identifiers() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/entities/project/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(project_page()))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["project", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("12"));
    assert!(stdout.contains("6511a4c2f0a1b2c3d4e5f6a7"));
    assert!(stdout.contains("Storage rework"));
    assert!(stdout.trim_end().ends_with("shown 2 of 2"));
}

#[tokio::test]
async fn goal_list_uses_the_goal_entity_type() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/entities/goal/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "hits": 0, "pages": 0, "values": []
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["goal", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shown 0 of 0"));
}

#[tokio::test]
async fn project_get_fences_the_description() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/entities/project/6511a4c2f0a1b2c3d4e5f6a7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "6511a4c2f0a1b2c3d4e5f6a7",
            "shortId": 12,
            "entityType": "project",
            "fields": {
                "summary": "Storage rework",
                "description": "one\ntwo\nthree\nfour\nfive",
                "entityStatus": "in_progress"
            }
        })))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["project", "get", "6511a4c2f0a1b2c3d4e5f6a7"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("short id: 12"));
    assert!(stdout.contains("<untrusted src=\"6511a4c2f0a1b2c3d4e5f6a7/description\""));
    assert!(stdout.contains("(+2 more lines: --full)"));
}

fn attachment_list(server: &str) -> serde_json::Value {
    serde_json::json!([
        {
            "self": format!("{server}/v3/issues/PROJ-1/attachments/301"),
            "id": "301",
            "name": "trace.log",
            "content": format!("{server}/v3/issues/PROJ-1/attachments/301/trace.log"),
            "createdBy": {"id": "1120000000000218", "login": "reporter", "display": "Sam"},
            "createdAt": "2026-08-21T09:00:00.000+0300",
            "mimetype": "text/plain",
            "size": 2048
        }
    ])
}

#[tokio::test]
async fn attachment_list_shows_size_and_type() {
    let harness = Harness::new().await;
    let body = attachment_list(&harness.server.uri());
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["attachment", "list", "PROJ-1"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("301"));
    assert!(stdout.contains("2.0 KB"));
    assert!(stdout.contains("text/plain"));
    assert!(stdout.contains("trace.log"));
}

#[tokio::test]
async fn attachment_download_writes_the_file_into_the_named_directory() {
    let harness = Harness::new().await;
    let body = attachment_list(&harness.server.uri());
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments/301/trace.log"))
        .respond_with(ResponseTemplate::new(200).set_body_string("log contents"))
        .mount(&harness.server)
        .await;

    let out = tempfile::tempdir().expect("temp dir");
    harness
        .run(&[
            "attachment",
            "download",
            "PROJ-1",
            "301",
            "-o",
            out.path().to_str().expect("utf-8 path"),
        ])
        .assert()
        .success();

    let written = std::fs::read_to_string(out.path().join("trace.log")).expect("file written");
    assert_eq!(written, "log contents");
}

/// The download URL is server-supplied. A crafted one must not be able to send
/// this client, carrying its OAuth header, to another host.
#[tokio::test]
async fn a_download_url_on_another_host_is_refused() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "id": "302",
                "name": "innocent.txt",
                "content": "https://evil.example.com/collect",
                "size": 10
            }])),
        )
        .mount(&harness.server)
        .await;

    let out = tempfile::tempdir().expect("temp dir");
    harness
        .run(&[
            "attachment",
            "download",
            "PROJ-1",
            "302",
            "-o",
            out.path().to_str().expect("utf-8 path"),
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("not the configured Tracker host"));
}

/// A filename chosen by an uploader must not choose a directory.
#[tokio::test]
async fn a_traversing_attachment_name_stays_inside_the_destination() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "id": "303",
                "name": "../../escaped.txt",
                "content": format!("{}/v3/issues/PROJ-1/attachments/303/x", harness.server.uri()),
                "size": 3
            }])),
        )
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments/303/x"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hi"))
        .mount(&harness.server)
        .await;

    let out = tempfile::tempdir().expect("temp dir");
    harness
        .run(&[
            "attachment",
            "download",
            "PROJ-1",
            "303",
            "-o",
            out.path().to_str().expect("utf-8 path"),
        ])
        .assert()
        .success();

    assert!(out.path().join("escaped.txt").exists());
    assert!(
        !out.path()
            .parent()
            .expect("parent")
            .join("escaped.txt")
            .exists()
    );
}

#[tokio::test]
async fn upload_sends_the_file_and_reports_the_new_id() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/attachments/"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "304",
            "name": "notes.txt",
            "size": 5
        })))
        .mount(&harness.server)
        .await;

    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "hello").expect("write file");

    harness
        .run(&[
            "attachment",
            "upload",
            "PROJ-1",
            file.to_str().expect("utf-8 path"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1 attachment 304"))
        .stderr(predicate::str::contains("→ profile=test"));
}

/// A test process has no terminal, which is the case that matters most: the
/// caller must still be told what the file is and how to open it.
#[tokio::test]
async fn show_without_a_graphics_terminal_names_the_download_command() {
    let harness = Harness::new().await;
    let body = attachment_list(&harness.server.uri());
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["attachment", "show", "PROJ-1", "301"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("trace.log (2.0 KB)"));
    assert!(stdout.contains("ytcli attachment download PROJ-1 301 -o ."));
    // Not one byte of the file, and no escape codes.
    assert!(!stdout.contains('\x1b'));
    // The bytes were never even fetched: there is nothing to draw them with.
    let requests = harness.server.received_requests().await.expect("recorded");
    assert_eq!(requests.len(), 1, "the file was downloaded for nothing");
}

/// `--format json` describes the attachment. It never emits pixels.
#[tokio::test]
async fn show_as_json_describes_the_attachment() {
    let harness = Harness::new().await;
    let body = attachment_list(&harness.server.uri());
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["attachment", "show", "PROJ-1", "301", "--format", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("json");

    assert_eq!(value["id"], "301");
    assert_eq!(value["name"], "trace.log");
    assert!(!stdout.contains('\x1b'));
}

#[tokio::test]
async fn show_of_an_unknown_attachment_exits_four() {
    let harness = Harness::new().await;
    let body = attachment_list(&harness.server.uri());
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/attachments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;

    harness
        .run(&["attachment", "show", "PROJ-1", "999"])
        .assert()
        .code(4);
}
