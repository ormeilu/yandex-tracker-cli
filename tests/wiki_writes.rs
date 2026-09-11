//! Wiki page writes: announced, gated, and fed from a file or stdin.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

const TOKEN: &str = "0b6c2a4e-1f3d-4e5a-9b7c-8d9e0f1a2b3c";

async fn page_is_4521(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v1/pages"))
        .and(query_param("slug", "users/ilubenets/runbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_page.json")))
        .mount(&harness.server)
        .await;
}

async fn nothing_was_sent(harness: &Harness) {
    let requests = harness.server.received_requests().await.expect("recorded");
    assert!(requests.is_empty(), "{requests:?}");
}

/// The text comes from a file, byte for byte; the write names the profile
/// and organisation before anything else; the output is the new page.
#[tokio::test]
async fn a_page_is_created_from_a_file_after_the_write_is_announced() {
    let harness = Harness::new().await;
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("notes.md");
    std::fs::write(&file, "# Notes\n\n- one\n").unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .and(query_param_is_missing("is_silent"))
        .and(body_json(serde_json::json!({
            "slug": "users/ilubenets/notes",
            "title": "Notes",
            "content": "# Notes\n\n- one\n"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 4600, "slug": "users/ilubenets/notes", "title": "Notes", "page_type": "wysiwyg"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "create",
            "users/ilubenets/notes",
            "--title",
            "Notes",
            "--from",
            file.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout("created users/ilubenets/notes (id 4600)\n")
        .stderr(predicate::str::contains("profile="));
}

/// A dry run says what it would send and sends nothing at all.
#[tokio::test]
async fn a_dry_run_sends_nothing() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "wiki",
            "create",
            "users/ilubenets/notes",
            "--title",
            "Notes",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "dry run: would create wiki page `users/ilubenets/notes`",
        ));
    nothing_was_sent(&harness).await;

    // Not even the lookup a write under an existing page starts with.
    harness
        .run(&["wiki", "delete", "users/ilubenets/runbook", "--dry-run"])
        .assert()
        .success();
    nothing_was_sent(&harness).await;
}

/// Stdin works as the source, and `--merge` and `--silent` reach the Wiki as
/// its own query flags.
#[tokio::test]
async fn an_update_from_stdin_is_sent_by_page_id() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521"))
        .and(query_param("allow_merge", "true"))
        .and(query_param("is_silent", "true"))
        .and(body_json(serde_json::json!({ "content": "new text\n" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_page.json")))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "update",
            "users/ilubenets/runbook",
            "--from",
            "-",
            "--merge",
            "--silent",
        ])
        .write_stdin("new text\n")
        .assert()
        .success()
        .stdout("updated users/ilubenets/runbook (id 4521)\n");
}

#[tokio::test]
async fn an_update_with_nothing_to_change_is_refused() {
    let harness = Harness::new().await;

    harness
        .run(&["wiki", "update", "users/ilubenets/runbook"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nothing to change"));
    nothing_was_sent(&harness).await;
}

/// Where appended text goes is sent the way the Wiki names it.
#[tokio::test]
async fn append_goes_to_the_top_or_an_anchor() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/append-content"))
        .and(body_json(serde_json::json!({
            "content": "entry\n", "body": { "location": "top" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_page.json")))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/append-content"))
        .and(body_json(serde_json::json!({
            "content": "entry\n", "anchor": { "name": "#deploy" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_page.json")))
        .mount(&harness.server)
        .await;

    for place in [&["--top"][..], &["--anchor", "#deploy"]] {
        let mut args = vec!["wiki", "append", "users/ilubenets/runbook", "--from", "-"];
        args.extend_from_slice(place);
        harness
            .run(&args)
            .write_stdin("entry\n")
            .assert()
            .success()
            .stdout("appended to users/ilubenets/runbook (id 4521)\n");
    }
}

/// The recovery token is shown once, ever, so the output carries it and the
/// exact command that uses it.
#[tokio::test]
async fn a_delete_prints_the_token_and_the_restore_command() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("DELETE"))
        .and(path("/v1/pages/4521"))
        .and(query_param_is_missing("recursive"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "recovery_token": TOKEN })),
        )
        .mount(&harness.server)
        .await;

    harness
        .run(&["wiki", "delete", "users/ilubenets/runbook"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("recovery token {TOKEN}")))
        .stdout(predicate::str::contains(format!(
            "ytcli wiki restore {TOKEN}"
        )));
}

/// Taking the subpages too is refused without --yes, before any request.
#[tokio::test]
async fn a_recursive_delete_needs_yes() {
    let harness = Harness::new().await;

    harness
        .run(&["wiki", "delete", "users/ilubenets/runbook", "--recursive"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));
    nothing_was_sent(&harness).await;
}

#[tokio::test]
async fn a_page_is_restored_by_its_token() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/recovery_tokens/{TOKEN}/recover")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 4521, "slug": "users/ilubenets/runbook", "pages_count": 3
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["wiki", "restore", TOKEN])
        .assert()
        .success()
        .stdout("restored users/ilubenets/runbook (id 4521, 3 pages)\n");
}

/// A token that may read but not write is told which permission it lacks.
#[tokio::test]
async fn a_refused_write_names_wiki_write() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "create",
            "users/ilubenets/notes",
            "--title",
            "Notes",
        ])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("wiki:write"));
}

/// A 403 the Wiki explains — an editor may change a section but not add to
/// it — is reported as that explanation and as a rights refusal, not as a
/// token that lacks a permission it has.
#[tokio::test]
async fn a_refused_write_repeats_the_reason_the_wiki_gives() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error_code": "FORBIDDEN",
            "debug_message": "",
            "message": "No rights to create a page in this section",
            "details": null
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "create",
            "homepage/team/notes",
            "--title",
            "Notes",
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains(
            "the Wiki rejected the request (403 Forbidden): FORBIDDEN: No rights to create a page in this section",
        ))
        .stderr(predicate::str::contains("wiki:write").not());
}

/// Under `-f json` a write answers with an object, so a script takes the id
/// without parsing the sentence a person gets.
#[tokio::test]
async fn a_write_answers_json_when_asked() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 4600, "slug": "users/ilubenets/notes", "title": "Notes", "page_type": "wysiwyg"
        })))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&[
            "wiki",
            "create",
            "users/ilubenets/notes",
            "--title",
            "Notes",
            "-f",
            "json",
        ])
        .assert()
        .success();
    let answer: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("stdout is JSON");
    assert_eq!(
        answer,
        serde_json::json!({ "action": "created", "slug": "users/ilubenets/notes", "id": 4600 })
    );
}
