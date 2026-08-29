//! Writing commands, and the gate in front of them.
//!
//! The interesting assertions are not that a POST happens but that it does not:
//! `--dry-run` must reach the network never, and the announcement of which
//! organisation is about to change must always be there.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

#[tokio::test]
async fn create_sends_the_queue_summary_and_returns_the_new_key() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/"))
        .and(body_json(serde_json::json!({
            "queue": {"key": "PROJ"},
            "summary": "Retry uploads on 5xx",
            "assignee": "ilubenets",
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(fixture("issue.json")))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "create",
            "-q",
            "PROJ",
            "-s",
            "Retry uploads on 5xx",
            "--assignee",
            "ilubenets",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("PROJ-1  "));
}

/// The whole point of a dry run: nothing reaches Tracker. The stub has no
/// mounted route, so any request at all would fail the test.
#[tokio::test]
async fn dry_run_sends_nothing_and_prints_the_body() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "issue",
            "create",
            "-q",
            "PROJ",
            "-s",
            "nothing to see",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "dry run: would create an issue in PROJ",
        ))
        .stderr(predicate::str::contains("\"summary\": \"nothing to see\""));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// "Which organisation did I just change" must never be a question the output
/// leaves open.
#[tokio::test]
async fn every_write_announces_the_profile_and_organisation() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/comments"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({"id": 205, "text": "done"})),
        )
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "comment", "PROJ-1", "done"])
        .assert()
        .success()
        .stderr(predicate::str::contains("→ profile=test org=12345"))
        .stdout(predicate::str::contains("PROJ-1 comment 205"));
}

#[tokio::test]
async fn update_types_values_by_reading_them_as_json() {
    let harness = Harness::new().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/issues/PROJ-1"))
        .and(body_json(serde_json::json!({
            "storyPoints": 3,
            "summary": "still a string",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue.json")))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "--set",
            "storyPoints=3",
            "--set",
            "summary=still a string",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn an_update_that_changes_nothing_is_refused() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "update", "PROJ-1"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nothing to change"));
}

#[tokio::test]
async fn a_malformed_assignment_is_refused_before_anything_is_sent() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "update", "PROJ-1", "--set", "storyPoints"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("expected key=value"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// Listing transitions is a read, and must not be gated as a write.
#[tokio::test]
async fn transition_without_an_id_lists_what_is_available() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/transitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": "close",
                "display": "Close",
                "to": {"id": "3", "key": "closed", "display": "Closed"}
            },
            {
                "id": "start_progress",
                "display": "Start progress",
                "to": {"id": "2", "key": "inProgress", "display": "In Progress"}
            }
        ])))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "transition", "PROJ-1"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("close"));
    assert!(stdout.contains("→ Closed"));
    assert!(stdout.ends_with("shown 2 of 2 for PROJ-1\n"));
}

#[tokio::test]
async fn transition_with_an_id_executes_it() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/transitions/close/_execute"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "transition", "PROJ-1", "close"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1 close"));
}

/// Closing an issue usually requires a resolution, and until this the command
/// could not supply one — half the statuses in an ordinary queue were out of
/// reach through this tool.
#[tokio::test]
async fn a_transition_carries_the_fields_it_is_given() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/transitions/close/_execute"))
        .and(body_json(serde_json::json!({
            "resolution": "wontFix",
            "comment": "not this quarter",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(1)
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "transition",
            "PROJ-1",
            "close",
            "-r",
            "wontFix",
            "--set",
            "comment=not this quarter",
        ])
        .assert()
        .success();
}

/// Tracker names the fields it wanted in the organisation's language and by
/// their display names, which are not what `--set` takes. Its words are kept;
/// the way to act on them is added.
#[tokio::test]
async fn a_transition_refused_for_want_of_a_field_says_how_to_supply_one() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/transitions/close/_execute"))
        .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
            "errorMessages": ["Вы должны указать значения для полей Резолюция."],
            "statusCode": 422,
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "transition", "PROJ-1", "close"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("Резолюция"))
        .stderr(predicate::str::contains("--resolution"));
}

/// The hint is for the caller who passed nothing. One who did pass fields and
/// was still refused is being told something else, and repeating the advice
/// would bury it.
#[tokio::test]
async fn a_transition_that_was_given_fields_is_not_told_to_pass_fields() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/transitions/close/_execute"))
        .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
            "errorMessages": ["резолюция nope не существует."],
            "statusCode": 422,
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "transition", "PROJ-1", "close", "-r", "nope"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("--resolution").not());
}

/// A create that timed out must not be retried: a duplicate issue is worse than
/// a clear failure.
#[tokio::test]
async fn a_failed_write_is_not_retried() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "create", "-q", "PROJ", "-s", "once"])
        .assert()
        .code(5);
}

/// The rule the `--yes` flag exists for: one issue is the ordinary case, several
/// is irreversible at scale. Nothing is sent while the answer is still no.
#[tokio::test]
async fn changing_several_issues_without_yes_exits_two_and_sends_nothing() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "PROJ-4",
            "--set",
            "storyPoints=3",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("without --yes"))
        // What it would have touched, so the caller can see whether the set is
        // the one they pictured.
        .stderr(predicate::str::contains("PROJ-1, PROJ-4"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// Several issues are one request, not one each, and the answer is a tally.
///
/// Tracker checks the whole list before it writes anything, which is the part
/// the issue-at-a-time path could never offer: there is no half-applied change
/// to reconstruct afterwards.
#[tokio::test]
async fn several_issues_with_yes_are_one_request_and_a_tally() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/bulkchange/_update"))
        .and(body_json(serde_json::json!({
            "issues": ["PROJ-1", "PROJ-4"],
            "values": {"storyPoints": 3},
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(fixture("bulkchange_created.json")))
        .expect(1)
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d90773c59502bc8e028a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_complete.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "PROJ-4",
            "--set",
            "storyPoints=3",
            "--yes",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("changed 2 of 2"), "{stdout}");
    // The only handle on the work afterwards, so it is printed even when
    // nothing went wrong.
    assert!(
        stdout.contains("bulkchange 6a92d90773c59502bc8e028a"),
        "{stdout}"
    );
    // Nothing per issue: the saving is the point, and repeating the keys would
    // spend it on the output.
    assert!(!stdout.contains("PROJ-1"), "{stdout}");
}

/// A change that finished having changed nothing must not exit zero, and the
/// per-issue reasons are worth the second request precisely then.
#[tokio::test]
async fn a_bulk_change_that_failed_names_each_issue_and_why() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/bulkchange/_update"))
        .respond_with(ResponseTemplate::new(201).set_body_json(fixture("bulkchange_created.json")))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d90773c59502bc8e028a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_failed.json")))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d9387d41a060a2b5e6d9/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_issues.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "PROJ-4",
            "--set",
            "priority=nonexistent",
            "--yes",
        ])
        .assert()
        .code(5);
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("changed 1 of 2"), "{stdout}");
    // Tracker's own words about the field, kept as written.
    assert!(stdout.contains("priority: "), "{stdout}");
    assert!(stdout.contains("PROJ-2"), "{stdout}");
    // The one that worked stays in the tally rather than in a line of its own.
    assert!(!stdout.contains("PROJ-1  "), "{stdout}");
}

/// An unknown key is refused before anything is written, and Tracker names it.
///
/// The issue-at-a-time path had already changed the earlier issues by the time
/// it found out.
#[tokio::test]
async fn a_key_that_does_not_exist_stops_the_whole_change() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/bulkchange/_update"))
        .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
            "errors": {"issues": "задачи [PROJ-999] не существуют"},
            "statusCode": 422,
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "PROJ-999",
            "--set",
            "storyPoints=3",
            "--yes",
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("PROJ-999"));
}

/// `--no-wait` claims only what happened: Tracker accepted it.
#[tokio::test]
async fn no_wait_returns_the_id_without_claiming_the_work_is_done() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/bulkchange/_update"))
        .respond_with(ResponseTemplate::new(201).set_body_json(fixture("bulkchange_created.json")))
        .expect(1)
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "PROJ-4",
            "--set",
            "storyPoints=3",
            "--yes",
            "--no-wait",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("bulkchange 6a92d90773c59502bc8e028a"),
        "{stdout}"
    );
    // No tally is invented from the keys we sent: Tracker has not counted them.
    assert!(!stdout.contains("changed 2 of 2"), "{stdout}");
}

/// Two organisations cannot be one request. The slow path stays, and says how
/// far it got in the same words.
#[tokio::test]
async fn keys_in_two_organisations_go_one_at_a_time() {
    let harness = Harness::new().await;
    harness.add_profile("other", "98765");
    for key in ["PROJ-1", "PROJ-4"] {
        Mock::given(method("PATCH"))
            .and(path(format!("/v3/issues/{key}")))
            .and(body_json(serde_json::json!({"storyPoints": 3})))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("issue.json")))
            .expect(1)
            .mount(&harness.server)
            .await;
    }

    let output = harness
        .run(&[
            "issue",
            "update",
            "test/PROJ-1",
            "other/PROJ-4",
            "--set",
            "storyPoints=3",
            "--yes",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("changed 2 of 2"), "{stdout}");
}

/// A dry run names every target, which is the only way to check the set before
/// committing to it.
#[tokio::test]
async fn a_dry_run_over_several_issues_lists_them_all() {
    let harness = Harness::new().await;

    harness
        .run(&[
            "issue",
            "update",
            "PROJ-1",
            "PROJ-4",
            "--set",
            "storyPoints=3",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("would update PROJ-1, PROJ-4"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// Half-applied is the worst outcome, so a bad key stops the run before the
/// first request rather than after the first success.
#[tokio::test]
async fn a_malformed_key_stops_the_run_before_anything_is_sent() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "update", "PROJ-1", "/", "--set", "x=1", "--yes"])
        .assert()
        .code(2);

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// Every write, not just the ones with a test of their own.
///
/// The gate is only worth having if nothing goes round it, and a new verb that
/// forgets to call it would otherwise be found by a user with a `--dry-run` that
/// wrote something.
#[tokio::test]
async fn no_write_verb_reaches_the_network_under_dry_run() {
    let file = tempfile::NamedTempFile::new().expect("temp file");
    let path = file.path().to_string_lossy().into_owned();

    let writes: Vec<Vec<&str>> = vec![
        vec!["issue", "create", "-q", "PROJ", "-s", "title"],
        vec!["issue", "update", "PROJ-1", "--set", "storyPoints=3"],
        vec!["issue", "comment", "PROJ-1", "text"],
        vec!["issue", "transition", "PROJ-1", "close"],
        vec!["issue", "move", "PROJ-1", "--to", "OPS"],
        vec!["attachment", "upload", "PROJ-1", &path],
    ];

    for args in writes {
        let harness = Harness::new().await;
        let mut with_flag = args.clone();
        with_flag.push("--dry-run");

        harness.run(&with_flag).assert().success();

        assert!(
            harness
                .server
                .received_requests()
                .await
                .expect("recorded")
                .is_empty(),
            "`ytcli {}` sent a request under --dry-run",
            args.join(" ")
        );
    }
}

/// The key changes and nothing puts it back, so a single issue needs `--yes`
/// here where an ordinary update does not.
#[tokio::test]
async fn moving_one_issue_still_needs_yes() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "move", "PROJ-1", "--to", "OPS"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be undone"));

    assert!(
        harness.server.received_requests().await.unwrap().is_empty(),
        "a refused move must send nothing"
    );
}

/// What the caller gets back is the new key: nothing they were holding still
/// addresses the issue.
#[tokio::test]
async fn a_move_reports_the_key_the_issue_now_has() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/_move"))
        .and(query_param("queue", "OPS"))
        .and(query_param("moveAllFields", "false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "OPS-17",
            "summary": "Attachments are lost on move",
            "queue": {"key": "OPS"}
        })))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "move", "PROJ-1", "--to", "OPS", "--yes"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert_eq!(stdout, "PROJ-1 → OPS-17\n");
}

/// Fields the target queue does not define are dropped by Tracker unless the
/// caller says otherwise, so the flag has to reach the request.
#[tokio::test]
async fn keep_fields_is_passed_through_to_tracker() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/_move"))
        .and(query_param("moveAllFields", "true"))
        .and(query_param("initialStatus", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "OPS-17",
            "summary": "Attachments are lost on move"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "move",
            "PROJ-1",
            "--to",
            "OPS",
            "--keep-fields",
            "--initial-status",
            "--yes",
        ])
        .assert()
        .success();
}

/// A workflow step applied to several issues is one request, with the fields the
/// workflow demands carried into it.
#[tokio::test]
async fn several_issues_take_one_transition_in_one_request() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/bulkchange/_transition"))
        .and(body_json(serde_json::json!({
            "issues": ["PROJ-1", "PROJ-4"],
            "transition": "close",
            "values": {"resolution": "fixed"},
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(fixture("bulkchange_created.json")))
        .expect(1)
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d90773c59502bc8e028a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_complete.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&[
            "issue",
            "transition",
            "PROJ-1",
            "PROJ-4",
            "--to",
            "close",
            "-r",
            "fixed",
            "--yes",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("changed 2 of 2"), "{stdout}");
}

/// With a list of keys there is nowhere unambiguous for a bare transition id, so
/// the command says which flag names it rather than guessing which word is which.
#[tokio::test]
async fn several_issues_without_to_are_refused_and_send_nothing() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "transition", "PROJ-1", "PROJ-4", "close", "--yes"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--to"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// Moving several issues is one request. `--yes` is still required for a single
/// one: the key change is irreversible in kind, not merely at scale.
#[tokio::test]
async fn several_issues_move_in_one_request() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/bulkchange/_move"))
        .and(body_json(serde_json::json!({
            "issues": ["PROJ-1", "PROJ-4"],
            "queue": "OPS",
            "moveAllFields": false,
            "initialStatus": false,
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(fixture("bulkchange_created.json")))
        .expect(1)
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d90773c59502bc8e028a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_complete.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "move", "PROJ-1", "PROJ-4", "--to", "OPS", "--yes"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("changed 2 of 2"), "{stdout}");
}

/// The confirmation for a move names every key it is about to change, because
/// after the request none of them address the issue any more.
#[tokio::test]
async fn moving_several_issues_without_yes_names_all_of_them() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "move", "PROJ-1", "PROJ-4", "--to", "OPS"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("PROJ-1, PROJ-4"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}
