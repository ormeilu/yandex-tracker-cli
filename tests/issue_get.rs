//! `issue get` against a stub Tracker.
//!
//! These run the real binary against `wiremock`, so they cover what unit tests
//! cannot: header construction, the two requests the compact view needs, format
//! selection, and the exit codes callers branch on.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

/// Both requests the compact view makes, answered.
async fn issue_available(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1"))
        .and(header("authorization", "OAuth test-token"))
        .and(header("x-cloud-org-id", "12345"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue.json")))
        .mount(&harness.server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/links"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue_links.json")))
        .mount(&harness.server)
        .await;
}

#[tokio::test]
async fn compact_view_renders_fields_links_and_a_fenced_description() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    let output = harness.run(&["issue", "get", "PROJ-1"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.starts_with("PROJ-1  Attachments are lost"));
    assert!(stdout.contains("status: In Progress   type: Bug   prio: Critical"));
    assert!(stdout.contains("assignee: ilubenets   author: reporter   queue: PROJ"));
    assert!(stdout.contains("comments: 3"));

    // Pinned custom field is shown by name; the rest are only counted.
    assert!(stdout.contains("storyPoints: 3"));
    assert!(stdout.contains("custom: "));

    // Every link carries its type, and the direction decides which end we are.
    assert!(stdout.contains("depends on PROJ-3 [Open]"));
    assert!(stdout.contains("parent PROJ-9"));
    assert!(stdout.contains("relates PROJ-7"));

    assert!(stdout.contains("<untrusted src=\"PROJ-1/description\""));
    assert!(stdout.contains("(+"));
    assert!(stdout.contains("--full"));
}

/// The offset Tracker sends is `+0300`, which a strict RFC 3339 parser rejects.
#[tokio::test]
async fn timestamps_with_compact_offsets_are_rendered() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    harness
        .run(&["issue", "get", "PROJ-1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("updated: 2026-08-27T10:00:00Z"));
}

#[tokio::test]
async fn full_shows_the_whole_description_without_a_truncation_notice() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    harness
        .run(&["issue", "get", "PROJ-1", "--full"])
        .assert()
        .success()
        .stdout(predicate::str::contains("attachments survive the move"))
        .stdout(predicate::str::contains("more lines: --full").not());
}

#[tokio::test]
async fn fields_collapses_the_answer_to_one_line() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    let output = harness
        .run(&["issue", "get", "PROJ-1", "--fields", "status,storyPoints"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert_eq!(stdout, "PROJ-1  status=In Progress  storyPoints=3\n");
}

/// `--json` is our schema. Tracker's own field names appear only under
/// `--json-raw`, so a rename upstream cannot reach anyone's script.
#[tokio::test]
async fn json_is_the_normalised_schema_and_json_raw_is_the_payload() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    let output = harness
        .run(&["issue", "get", "PROJ-1", "--format", "json"])
        .assert()
        .success();
    let normalised: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("json");

    assert_eq!(normalised["key"], "PROJ-1");
    assert_eq!(normalised["status"], "In Progress");
    assert_eq!(normalised["assignee"]["login"], "ilubenets");
    assert_eq!(normalised["links"][0]["kind"], "depends");
    assert!(
        normalised
            .get("commentWithoutExternalMessageCount")
            .is_none()
    );

    let output = harness
        .run(&["issue", "get", "PROJ-1", "--format", "json-raw"])
        .assert()
        .success();
    let raw: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).expect("json");

    assert_eq!(raw["status"]["display"], "In Progress");
    assert_eq!(raw["commentWithoutExternalMessageCount"], 3);
}

#[tokio::test]
async fn a_missing_issue_exits_four_and_names_what_was_missing() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-404"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "get", "PROJ-404"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("issue PROJ-404 not found"));
}

#[tokio::test]
async fn a_rejected_token_exits_three() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&harness.server)
        .await;

    harness.run(&["issue", "get", "PROJ-1"]).assert().code(3);
}

/// Links live on a second endpoint. Losing them must not lose the issue: the
/// caller asked for the issue, and a missing links section is the smaller loss.
#[tokio::test]
async fn the_issue_still_renders_when_links_cannot_be_fetched() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue.json")))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/links"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "get", "PROJ-1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1  Attachments are lost"))
        .stdout(predicate::str::contains("links: none"));
}

/// Queue keys are unique inside an organisation, not across them, so a caller
/// can say which profile a key belongs to and the command follows it.
#[tokio::test]
async fn a_profile_qualified_key_selects_that_profile() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    harness
        .run_raw(&["issue", "get", "test/PROJ-1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1  Attachments are lost"))
        .stderr(predicate::str::contains(
            "profile=test org=12345 (from the key `test/PROJ-1`)",
        ));
}

#[tokio::test]
async fn an_unknown_profile_in_a_key_is_an_auth_error() {
    let harness = Harness::new().await;

    harness
        .run_raw(&["issue", "get", "nosuch/PROJ-1"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("`nosuch` is not defined"));
}

#[tokio::test]
async fn a_stray_slash_is_rejected_rather_than_guessed_at() {
    let harness = Harness::new().await;

    harness
        .run_raw(&["issue", "get", "/PROJ-1"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "write it as PROJ-1 or profile/PROJ-1",
        ));
}

/// A bare key is refused only when this tool has *seen* the collision — from a
/// previous `auth status` or login. Guessing would make the common case worse
/// to guard against a situation most people never have.
#[tokio::test]
async fn a_bare_key_is_refused_when_two_profiles_share_the_queue() {
    let harness = Harness::new().await;
    harness.add_profile("other", "99999");
    harness.write_queue_cache(&[("PROJ", &["test", "other"])]);

    harness
        .run_raw(&["issue", "get", "PROJ-1"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("`PROJ-1` is ambiguous"))
        .stderr(predicate::str::contains(
            "write other/PROJ-1 or test/PROJ-1",
        ));
}

/// Qualified always works, collision or not — scripts written before the
/// collision existed keep working, and so do scripts written after.
#[tokio::test]
async fn a_qualified_key_works_even_when_the_queue_is_shared() {
    let harness = Harness::new().await;
    harness.add_profile("other", "99999");
    harness.write_queue_cache(&[("PROJ", &["test", "other"])]);
    issue_available(&harness).await;

    harness
        .run_raw(&["issue", "get", "test/PROJ-1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1  Attachments are lost"));
}

/// One profile seeing the queue is not a collision.
#[tokio::test]
async fn a_bare_key_is_fine_when_only_one_profile_sees_the_queue() {
    let harness = Harness::new().await;
    harness.write_queue_cache(&[("PROJ", &["test"])]);
    issue_available(&harness).await;

    harness.run(&["issue", "get", "PROJ-1"]).assert().success();
}

/// A collision recorded for a profile that has since been removed from the
/// config must stop blocking.
#[tokio::test]
async fn a_collision_with_a_deleted_profile_stops_mattering() {
    let harness = Harness::new().await;
    harness.write_queue_cache(&[("PROJ", &["test", "deleted-long-ago"])]);
    issue_available(&harness).await;

    harness.run(&["issue", "get", "PROJ-1"]).assert().success();
}

/// The cost promise: drawing images must not make the cheap path more
/// expensive. Without a terminal that can draw, the attachments are not
/// fetched, so the view still costs the two requests it always cost.
#[tokio::test]
async fn a_pipe_never_pays_for_the_images_it_cannot_see() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    harness.run(&["issue", "get", "PROJ-1"]).assert().success();

    let paths: Vec<String> = harness
        .server
        .received_requests()
        .await
        .expect("recorded")
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();

    assert_eq!(
        paths,
        vec!["/v3/issues/PROJ-1", "/v3/issues/PROJ-1/links"],
        "the issue view asked for something it cannot show"
    );
}

/// The flag exists to be usable everywhere the images are, and turning them off
/// must not disturb anything else in the view.
#[tokio::test]
async fn no_images_changes_nothing_else_about_the_output() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    let plain = harness.run(&["issue", "get", "PROJ-1"]).assert().success();
    let without = harness
        .run(&["issue", "get", "PROJ-1", "--no-images"])
        .assert()
        .success();

    assert_eq!(
        plain.get_output().stdout,
        without.get_output().stdout,
        "--no-images altered output that has no images in it"
    );
}

/// TOON used to be a build feature, so `--format toon` could fail on a binary
/// that looked identical to one where it worked. It is in every build now, and
/// this is the test that says so.
#[tokio::test]
async fn toon_is_in_the_build() {
    let harness = Harness::new().await;
    issue_available(&harness).await;

    let output = harness
        .run(&["issue", "get", "PROJ-1", "--format", "toon"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("PROJ-1"));
    // TOON is key: value without the quoting and bracing of JSON.
    assert!(!stdout.contains("\"key\":"), "that is JSON, not TOON");
}
