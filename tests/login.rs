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
        .stderr(predicate::str::contains(
            "ytcli auth login --account work --org-id",
        ));
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

/// `auth status` is the command someone runs when something is wrong, so it
/// answers the questions that get asked: who am I, what can I reach.
#[tokio::test]
async fn status_reports_identity_and_what_the_profile_can_see() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(myself()))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/queues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": 7, "key": "PROJ", "name": "Product"},
            {"id": 8, "key": "INFRA", "name": "Infrastructure"}
        ])))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/entities/project/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "hits": 3,
            "pages": 1,
            "values": [
                {"id": "a1", "shortId": 12, "entityType": "project",
                 "fields": {"summary": "Storage rework"}},
                {"id": "a2", "shortId": 13, "entityType": "project",
                 "fields": {"summary": "Billing"}}
            ]
        })))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/entities/goal/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "hits": 1, "pages": 1, "values": []
        })))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/_count"))
        .respond_with(ResponseTemplate::new(200).set_body_json(7))
        .mount(&harness.server)
        .await;

    let output = harness.run_raw(&["auth", "status"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("profile test"));
    assert!(stdout.contains("org: 12345 (Cloud)"));
    assert!(stdout.contains("token: ok   user: ilubenets (Ilya Lubenets)"));
    assert!(stdout.contains("queues: 2   projects: 3   goals: 1   my open issues: 7"));
    assert!(stdout.contains("Storage rework (12), Billing (13), +1 more"));
    assert!(stdout.contains("PROJ, INFRA"));
}

/// A profile that cannot see projects should still report its queues rather
/// than losing the whole line.
#[tokio::test]
async fn status_survives_a_partly_unavailable_organisation() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(myself()))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/queues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": 7, "key": "PROJ", "name": "Product"}
        ])))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/entities/project/_search"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&harness.server)
        .await;

    let output = harness.run_raw(&["auth", "status"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("queues: 1   projects: -"));
    assert!(stdout.contains("PROJ"));
}

/// A rejected token should come with the instructions for getting a new one.
#[tokio::test]
async fn status_with_a_rejected_token_points_at_the_token_docs() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&harness.server)
        .await;

    harness
        .run_raw(&["auth", "status"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains("token: rejected"))
        .stderr(predicate::str::contains("oauth.yandex.ru/client/new"));
}

#[tokio::test]
async fn brief_skips_the_counts_and_their_requests() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(myself()))
        .mount(&harness.server)
        .await;

    let output = harness
        .run_raw(&["auth", "status", "--brief"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("token: ok"));
    assert!(!stdout.contains("my open issues"));

    let requests = harness.server.received_requests().await.expect("recorded");
    assert_eq!(requests.len(), 1);
}

/// The help must answer "where do I get these" without anyone having to fail
/// first — the same text the wizard shows.
#[test]
fn login_help_carries_the_credential_instructions() {
    let output = assert_cmd::Command::cargo_bin("ytcli")
        .expect("binary built")
        .args(["auth", "login", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("oauth.yandex.com/client/new"));
    assert!(stdout.contains("tracker:write"));
    assert!(stdout.contains("tracker.yandex.ru/admin/orgs"));
    assert!(stdout.contains("--org-kind yandex360"));
    assert!(stdout.contains("walks you through each step"));
}

/// Outside a terminal there is nobody to answer, so a missing account is an
/// error rather than a prompt that would hang a script.
#[tokio::test]
async fn without_a_terminal_a_missing_account_is_an_error_not_a_prompt() {
    let harness = Harness::new().await;

    harness
        .run_raw(&["auth", "login", "--dry-run"])
        .write_stdin("some-token\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "--account is required when not running in a terminal",
        ));
}

/// Switching the default profile is a local edit, and nothing more.
///
/// No token is read and no request is made: asking the keychain for a
/// credential in order to change a line in a config file would be theatre.
#[tokio::test]
async fn use_switches_the_default_profile_without_touching_the_network() {
    let harness = Harness::new().await;
    harness.add_profile("other", "99999");

    harness
        .run_raw(&["auth", "use", "other"])
        .assert()
        .success()
        .stderr(predicate::str::contains("default profile:"))
        .stderr(predicate::str::contains("other"));

    let config = std::fs::read_to_string(harness.config_path()).expect("read config");
    assert!(config.contains(r#"default_profile = "other""#), "{config}");

    let requests = harness.server.received_requests().await.unwrap_or_default();
    assert!(requests.is_empty(), "{requests:?}");
}

/// A default naming a profile that does not exist is a config every later
/// command fails on, with a worse message than this one.
#[tokio::test]
async fn use_refuses_a_profile_that_does_not_exist() {
    let harness = Harness::new().await;

    harness
        .run_raw(&["auth", "use", "nope"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("no profile called `nope`"))
        .stderr(predicate::str::contains("configured: test"));
}

/// `--dry-run` says what it would do and leaves the file alone, like every
/// other write here.
#[tokio::test]
async fn use_under_dry_run_changes_nothing() {
    let harness = Harness::new().await;
    harness.add_profile("other", "99999");
    let before = std::fs::read_to_string(harness.config_path()).expect("read config");

    harness
        .run_raw(&["auth", "use", "other", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dry run: would make `other`"));

    let after = std::fs::read_to_string(harness.config_path()).expect("read config");
    assert_eq!(before, after);
}

/// Two profiles on one organisation are not a collision. `FINANSY-1` names one
/// issue there, and either login fetches it — the tool routes rather than
/// refuses, so a warning saying it will be refused describes a rule that was
/// removed and sends the reader to qualify keys that never needed it.
#[tokio::test]
async fn two_profiles_on_one_organisation_are_not_warned_about() {
    let harness = Harness::new().await;
    harness.add_profile("second", "12345");
    status_answers(&harness).await;

    let output = harness.run_raw(&["auth", "status"]).assert().success();
    let stderr = String::from_utf8(output.get_output().stderr.clone()).expect("utf-8");

    assert!(!stderr.contains("PROJ"), "{stderr}");
}

/// One organisation apart, the same two queue keys *are* ambiguous, and that is
/// the case the warning exists for.
#[tokio::test]
async fn a_queue_key_shared_across_organisations_is_warned_about() {
    let harness = Harness::new().await;
    harness.add_profile("other", "99999");
    status_answers(&harness).await;

    let output = harness.run_raw(&["auth", "status"]).assert().success();
    let stderr = String::from_utf8(output.get_output().stderr.clone()).expect("utf-8");

    assert!(stderr.contains("queue PROJ is visible in"), "{stderr}");
    assert!(stderr.contains("different organisations"), "{stderr}");
}

/// Enough of an organisation for `auth status` to finish for every profile.
async fn status_answers(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(myself()))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/queues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": 7, "key": "PROJ", "name": "Product"}
        ])))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/entities/project/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "hits": 0, "pages": 1, "values": []
        })))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/entities/goal/_search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "hits": 0, "pages": 1, "values": []
        })))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/_count"))
        .respond_with(ResponseTemplate::new(200).set_body_json(0))
        .mount(&harness.server)
        .await;
}
