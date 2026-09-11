//! Wiki access: read through the page, changed through grants.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

async fn page_is_4521(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v1/pages"))
        .and(query_param("slug", "users/ilubenets/runbook"))
        .and(query_param_is_missing("fields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("wiki_page.json")))
        .mount(&harness.server)
        .await;
}

async fn nothing_was_sent(harness: &Harness) {
    let requests = harness.server.received_requests().await.expect("recorded");
    assert!(requests.is_empty(), "{requests:?}");
}

fn granted(id: &str, role: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "id": id, "role": role, "created_at": "2026-09-11T10:00:00Z",
        "user": {"id": 12, "username": "anna", "display_name": "Anna",
                 "is_dismissed": false, "affiliation": "staff"},
        "inheritance": "inherited"
    }))
}

/// Access is read off the page; the three lists come out as one, each grant
/// saying which list it was in.
#[tokio::test]
async fn access_is_read_from_the_page_in_one_list() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages"))
        .and(query_param("fields", "access_policy,access_lists"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 4521, "slug": "users/ilubenets/runbook", "title": "Deploy runbook",
            "page_type": "wysiwyg",
            "access_policy": {"access_type": "custom"},
            "access_lists": {
                "direct": [{"id": "a1", "role": "author",
                            "user": {"id": 11, "username": "ilubenets", "display_name": "Ilya",
                                     "is_dismissed": false, "affiliation": "staff"}}],
                "by_link": [],
                "inherited": [{"id": "g7", "role": "reader",
                               "group": {"identity": {"id": "42", "src": "dir"},
                                         "name": "Backend team", "type": "group"}}]
            }
        })))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["wiki", "access", "users/ilubenets/runbook"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.starts_with("users/ilubenets/runbook  access: custom"),
        "{stdout}"
    );
    assert!(stdout.contains("ilubenets"), "{stdout}");
    assert!(stdout.contains("Backend team"), "{stdout}");
    assert!(stdout.contains("inherited"), "{stdout}");
    assert!(stdout.trim_end().ends_with("shown 2 of 2"), "{stdout}");

    let output = harness
        .run(&[
            "wiki",
            "access",
            "users/ilubenets/runbook",
            "--format",
            "json",
        ])
        .assert()
        .success();
    let access: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(access["policy"], "custom");
    assert_eq!(access["entries"][1]["kind"], "group");
    assert_eq!(access["entries"][1]["via"], "inherited");
}

/// A login is turned into the uid the Wiki takes, through Tracker; the
/// self-lock guard is on unless asked off.
#[tokio::test]
async fn a_user_is_granted_by_login_through_tracker() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("GET"))
        .and(path("/v3/users/anna"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "login": "anna", "uid": 1_120_000_000_012_345_u64, "display": "Anna"
        })))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/access"))
        .and(query_param("prevent_selflock", "true"))
        .and(body_json(serde_json::json!({
            "role": "editor", "user": {"uid": "1120000000012345"}
        })))
        .respond_with(granted("a9", "editor"))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "grant",
            "users/ilubenets/runbook",
            "--role",
            "editor",
            "--user",
            "anna",
        ])
        .assert()
        .success()
        .stdout("granted editor on users/ilubenets/runbook to anna (access a9)\n")
        .stderr(predicate::str::contains("profile="));
}

/// A group is named with its source; `--no-inherit` and `--allow-selflock`
/// reach the Wiki as its own fields.
#[tokio::test]
async fn a_group_is_granted_by_source_and_id() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/access"))
        .and(query_param_is_missing("prevent_selflock"))
        .and(body_json(serde_json::json!({
            "role": "reader",
            "group": {"id": "42", "src": "dir"},
            "inheritance": "not_inherited"
        })))
        .respond_with(granted("g9", "reader"))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "grant",
            "users/ilubenets/runbook",
            "--role",
            "reader",
            "--group",
            "dir:42",
            "--no-inherit",
            "--allow-selflock",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("to group 42 (access g9)"));
}

/// Nobody named, or a group without its source: refused before any request.
#[tokio::test]
async fn a_grant_must_say_whom() {
    let harness = Harness::new().await;

    harness
        .run(&["wiki", "grant", "users/x", "--role", "reader"])
        .assert()
        .code(2);
    harness
        .run(&[
            "wiki", "grant", "users/x", "--role", "reader", "--group", "42",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("SOURCE:ID"));
    nothing_was_sent(&harness).await;
}

/// A dry run does not even ask Tracker for the uid.
#[tokio::test]
async fn a_dry_run_grant_sends_nothing() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "wiki",
            "grant",
            "users/ilubenets/runbook",
            "--role",
            "editor",
            "--user",
            "anna",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("uid of anna, from Tracker"));
    nothing_was_sent(&harness).await;
}

#[tokio::test]
async fn a_grant_changes_role() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/access/a9"))
        .and(query_param("prevent_selflock", "true"))
        .and(body_json(serde_json::json!({ "role": "reader" })))
        .respond_with(granted("a9", "reader"))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "regrant",
            "users/ilubenets/runbook",
            "a9",
            "--role",
            "reader",
        ])
        .assert()
        .success()
        .stdout("changed access a9 on users/ilubenets/runbook: reader\n");
}

/// One grant goes on request; every personal one needs --yes first.
#[tokio::test]
async fn revoking_all_needs_yes_and_one_does_not() {
    let harness = Harness::new().await;

    harness
        .run(&["wiki", "revoke", "users/ilubenets/runbook", "--all"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--yes"));
    nothing_was_sent(&harness).await;

    page_is_4521(&harness).await;
    Mock::given(method("DELETE"))
        .and(path("/v1/pages/4521/access/a9"))
        .and(query_param("prevent_selflock", "true"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&harness.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/pages/4521/access"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&harness.server)
        .await;

    harness
        .run(&["wiki", "revoke", "users/ilubenets/runbook", "a9"])
        .assert()
        .success()
        .stdout("revoked access a9 on users/ilubenets/runbook\n");
    harness
        .run(&[
            "wiki",
            "revoke",
            "users/ilubenets/runbook",
            "--all",
            "--yes",
        ])
        .assert()
        .success()
        .stdout("revoked every personal access on users/ilubenets/runbook\n");
}

/// `USER_NOT_FOUND` on a grant is about the caller's rights or the kind of
/// uid, not about the user: the message says so, and names the Wiki.
#[tokio::test]
async fn a_grant_the_wiki_calls_user_not_found_gets_advice() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/access"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error_code": "USER_NOT_FOUND",
            "debug_message": "User with such identity does not exist"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "grant",
            "users/ilubenets/runbook",
            "--role",
            "editor",
            "--uid",
            "8000000000000000",
        ])
        .assert()
        .code(5)
        .stderr(
            predicate::str::contains("the Wiki did not accept this identity")
                .and(predicate::str::contains(
                    "ytcli wiki access users/ilubenets/runbook",
                ))
                .and(predicate::str::contains("--cloud-uid"))
                .and(predicate::str::contains("Tracker rejected").not()),
        );

    harness
        .run(&[
            "wiki",
            "grant",
            "users/ilubenets/runbook",
            "--role",
            "editor",
            "--cloud-uid",
            "ajeabc",
        ])
        .assert()
        .code(5)
        .stderr(
            predicate::str::contains("USER_NOT_FOUND")
                .and(predicate::str::contains("--cloud-uid").not()),
        );
}

/// Any other refusal from the Wiki is named as the Wiki's, not Tracker's.
#[tokio::test]
async fn a_wiki_refusal_names_the_wiki() {
    let harness = Harness::new().await;
    page_is_4521(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v1/pages/4521/access"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error_code": "VALIDATION_ERROR",
            "debug_message": "role is not valid"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "wiki",
            "grant",
            "users/ilubenets/runbook",
            "--role",
            "editor",
            "--uid",
            "1",
        ])
        .assert()
        .code(5)
        .stderr(
            predicate::str::contains("the Wiki rejected the request (400 Bad Request)")
                .and(predicate::str::contains("VALIDATION_ERROR")),
        );
}
