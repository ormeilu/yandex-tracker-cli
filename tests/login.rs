//! `auth login`.
//!
//! The real command writes to the OS keychain, which a test must not do — a CI
//! runner has no unlocked keychain, and a developer's should not collect entries
//! from a test run. Everything up to the writes is exercised through `--dry-run`,
//! which login honours like every other write; the file-writing half is unit
//! tested in `config::store`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::Harness;

fn myself() -> serde_json::Value {
    serde_json::json!({
        "self": "https://api.tracker.yandex.net/v3/myself",
        "uid": 1_120_000_000_000_219_i64,
        "login": "ilubenets",
        "display": "Ilya Lubenets",
        "email": "someone@example.com"
    })
}

/// A mistyped token that reaches the keychain fails later, somewhere else,
/// looking like a permissions problem. Checking first is one request.
#[tokio::test]
async fn the_token_is_verified_before_anything_is_stored() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .and(header("authorization", "OAuth test-token"))
        .and(header("x-cloud-org-id", "12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(myself()))
        .mount(&harness.server)
        .await;

    harness
        .run_raw(&[
            "auth",
            "login",
            "--account",
            "work",
            "--org-id",
            "12345",
            "--dry-run",
        ])
        .write_stdin("test-token\n")
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "verified as ilubenets in org 12345",
        ))
        .stderr(predicate::str::contains("dry run: would store a token"))
        .stderr(predicate::str::contains(
            "dry run: would write profile `work`",
        ));
}

/// The two organisation headers are not interchangeable, and the wrong one
/// answers 403 — which reads as a permissions problem rather than a
/// configuration mistake. Trying both here costs one request, once.
#[tokio::test]
async fn the_organisation_header_form_is_detected() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .and(header("x-cloud-org-id", "12345"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .and(header("x-org-id", "12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(myself()))
        .mount(&harness.server)
        .await;

    harness
        .run_raw(&[
            "auth",
            "login",
            "--account",
            "work",
            "--org-id",
            "12345",
            "--dry-run",
        ])
        .write_stdin("test-token\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("Yandex360"));
}

/// A rejected token is rejected under either header, so there is nothing to
/// retry and the message should say what is actually wrong.
#[tokio::test]
async fn a_rejected_token_fails_immediately_without_trying_the_other_header() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&harness.server)
        .await;

    harness
        .run_raw(&[
            "auth",
            "login",
            "--account",
            "work",
            "--org-id",
            "12345",
            "--dry-run",
        ])
        .write_stdin("wrong-token\n")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("token was rejected"));
}

#[tokio::test]
async fn a_wrong_org_id_reports_that_both_forms_were_tried() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&harness.server)
        .await;

    harness
        .run_raw(&[
            "auth",
            "login",
            "--account",
            "work",
            "--org-id",
            "99999",
            "--dry-run",
        ])
        .write_stdin("test-token\n")
        .assert()
        .code(5)
        .stderr(predicate::str::contains(
            "checked both organisation header forms",
        ));
}

/// Without an organisation there is nothing to write a profile from, and saying
/// so beats leaving someone with a token and no way to use it.
#[tokio::test]
async fn login_without_an_org_id_says_the_setup_is_unfinished() {
    let harness = Harness::new().await;

    harness
        .run_raw(&["auth", "login", "--account", "work", "--dry-run"])
        .write_stdin("test-token\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("no --org-id given"))
        .stderr(predicate::str::contains("rerun with --org-id"));
}

#[tokio::test]
async fn an_empty_token_is_refused_before_any_request() {
    let harness = Harness::new().await;

    harness
        .run_raw(&[
            "auth",
            "login",
            "--account",
            "work",
            "--org-id",
            "12345",
            "--dry-run",
        ])
        .write_stdin("   \n")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("no token given"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// The profile name defaults to the account name, and --queue lands in it.
#[tokio::test]
async fn the_profile_can_be_named_and_given_a_queue() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(myself()))
        .mount(&harness.server)
        .await;

    harness
        .run_raw(&[
            "auth",
            "login",
            "--account",
            "admin",
            "--org-id",
            "12345",
            "--profile",
            "work",
            "--queue",
            "PROJ",
            "--dry-run",
        ])
        .write_stdin("test-token\n")
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "would write profile `work` (account=admin, org=12345",
        ));
}
