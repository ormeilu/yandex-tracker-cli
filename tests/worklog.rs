//! Worklogs, checklists and link editing, against a stub Tracker.
//!
//! The cases that matter are the translations: what a person types becomes what
//! the API takes, and what the API returns becomes something worth reading.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::Harness;

fn worklog_entries() -> serde_json::Value {
    serde_json::json!([
        {
            "id": 12345,
            "duration": "PT1H30M",
            "start": "2026-08-27T09:00:00.000+0300",
            "createdBy": {"id": "1", "login": "ilubenets", "display": "Ilya"},
            "comment": "pairing on the migration"
        },
        {
            "id": 12346,
            "duration": "PT45M",
            "start": "2026-08-27T14:00:00.000+0300",
            "createdBy": {"id": "1", "login": "ilubenets", "display": "Ilya"}
        }
    ])
}

async fn worklog_available(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/worklog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(worklog_entries()))
        .mount(&harness.server)
        .await;
}

/// Nobody types `PT1H30M` and nobody wants to read it either.
#[tokio::test]
async fn a_worklog_reads_in_the_units_people_use() {
    let harness = Harness::new().await;
    worklog_available(&harness).await;

    let output = harness
        .run(&["issue", "worklogs", "PROJ-1"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("1h 30m"));
    assert!(stdout.contains("45m"));
    assert!(!stdout.contains("PT1H30M"), "that is the API's wording");
    assert!(stdout.contains("ilubenets"));
    assert!(stdout.contains("pairing on the migration"));
    // The total is the reason to read a worklog at all.
    assert!(stdout.ends_with("shown 2 of 2 for PROJ-1 — 2h 15m total\n"));
}

/// JSON keeps Tracker's own vocabulary: a script is written against the API,
/// not against our shorthand.
#[tokio::test]
async fn json_keeps_the_iso_duration() {
    let harness = Harness::new().await;
    worklog_available(&harness).await;

    let output = harness
        .run(&["issue", "worklogs", "PROJ-1", "--format", "json"])
        .assert()
        .success();
    let value: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("json");

    assert_eq!(value[0]["duration"], "PT1H30M");
    assert_eq!(value[0]["id"], "12345");
}

#[tokio::test]
async fn a_typed_duration_becomes_the_one_tracker_takes() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/worklog"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({"id": 9, "duration": "PT1H30M"})),
        )
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "worklog", "add", "PROJ-1", "1h30m"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1 worklog 9 1h 30m"));

    let requests = harness.server.received_requests().await.expect("recorded");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(body["duration"], "PT1H30M");
    // Tracker requires a start, and "now" is what somebody logging time at the
    // end of the work means.
    assert!(body["start"].as_str().is_some_and(|s| s.contains('T')));
}

/// A duration nobody can parse must say what was wrong with it, before any
/// request is sent.
#[tokio::test]
async fn a_duration_with_no_unit_is_refused_before_anything_is_sent() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "worklog", "add", "PROJ-1", "90"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no unit"));

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
async fn a_checklist_shows_its_boxes_and_what_is_done() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/issues/PROJ-1/checklistItems"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": "41", "text": "migrate the audio tracks", "checked": true},
            {"id": "42", "text": "review", "checked": false,
             "assignee": {"id": "1", "login": "ilubenets"},
             "deadline": {"date": "2026-09-01"}}
        ])))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["issue", "checklist", "PROJ-1"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("41 [x] migrate the audio tracks"));
    assert!(stdout.contains("42 [ ] review @ilubenets due 2026-09-01"));
    // Piped: the numbers, and nothing drawn. The bar is for a terminal.
    assert!(
        stdout.ends_with("shown 2 of 2 for PROJ-1 — 1 of 2 done\n"),
        "{stdout}"
    );
}

/// Ticking prints the list as it now stands, so the result needs no second call.
#[tokio::test]
async fn ticking_a_line_shows_the_checklist_afterwards() {
    let harness = Harness::new().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/issues/PROJ-1/checklistItems/42"))
        .and(body_json(serde_json::json!({"checked": true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "PROJ-1",
            "checklistItems": [{"id": "42", "text": "review", "checked": true}]
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "check", "tick", "PROJ-1", "42"])
        .assert()
        .success()
        .stdout(predicate::str::contains("42 [x] review"));
}

#[tokio::test]
async fn linking_two_issues_sends_the_relationship_and_the_other_key() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/links"))
        .and(body_json(
            serde_json::json!({"relationship": "relates", "issue": "PROJ-7"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({})))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "link", "add", "PROJ-1", "relates", "PROJ-7"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1 relates PROJ-7"));
}

/// Tracker has no undelete for any of these, so a deletion goes through the
/// same gate every other write does — and under --dry-run reaches nothing.
#[tokio::test]
async fn every_deletion_is_gated_and_sends_nothing_under_dry_run() {
    for args in [
        vec!["issue", "worklog", "delete", "PROJ-1", "12345"],
        vec!["issue", "check", "delete", "PROJ-1", "42"],
        vec!["issue", "link", "delete", "PROJ-1", "987"],
        vec!["issue", "comment", "delete", "PROJ-1", "987654"],
    ] {
        let harness = Harness::new().await;
        let mut dry = args.clone();
        dry.push("--dry-run");

        harness
            .run(&dry)
            .assert()
            .success()
            .stderr(predicate::str::contains("dry run: would delete"));

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

/// ADR 1's property, for the commands added here: reading and writing never
/// share a command prefix, so `ytcli issue worklogs:*` cannot be stretched into
/// a write and `ytcli issue checklist:*` cannot either.
#[tokio::test]
async fn the_read_and_the_write_are_different_words() {
    let harness = Harness::new().await;

    for write in [
        vec!["issue", "worklogs", "add", "PROJ-1", "1h"],
        vec!["issue", "checklist", "add", "PROJ-1", "text"],
        vec!["issue", "links", "add", "PROJ-1", "relates", "PROJ-2"],
    ] {
        harness.run(&write).assert().failure();
    }
}

/// The old spelling is in every allowlist anyone wrote, so gaining `edit` and
/// `delete` must not cost it.
#[tokio::test]
async fn the_bare_comment_form_still_works_beside_the_subcommands() {
    for args in [
        vec!["issue", "comment", "PROJ-1", "text"],
        vec!["issue", "comment", "add", "PROJ-1", "text"],
    ] {
        let harness = Harness::new().await;
        Mock::given(method("POST"))
            .and(path("/v3/issues/PROJ-1/comments"))
            .and(body_json(serde_json::json!({"text": "text"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": 987_654, "text": "text"
            })))
            .mount(&harness.server)
            .await;

        harness
            .run(&args)
            .assert()
            .success()
            .stdout("PROJ-1 comment 987654\n");
    }
}

/// Editing replaces the body rather than appending to it: what is sent is the
/// whole text, and Tracker keeps no copy of what was there.
#[tokio::test]
async fn editing_a_comment_sends_the_whole_new_body() {
    let harness = Harness::new().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/issues/PROJ-1/comments/987654"))
        .and(body_json(serde_json::json!({"text": "corrected"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 987_654, "text": "corrected"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "comment", "edit", "PROJ-1", "987654", "corrected"])
        .assert()
        .success()
        .stdout("PROJ-1 comment 987654 edited\n");
}

#[tokio::test]
async fn a_comment_can_be_removed_by_its_own_id() {
    let harness = Harness::new().await;
    Mock::given(method("DELETE"))
        .and(path("/v3/issues/PROJ-1/comments/987654"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "comment", "delete", "PROJ-1", "987654"])
        .assert()
        .success()
        .stdout("PROJ-1 comment 987654 deleted\n");
}

/// A correction sends only what was corrected: sending the whole entry back
/// would overwrite the half the caller did not mention.
#[tokio::test]
async fn correcting_a_worklog_sends_only_what_changed() {
    let harness = Harness::new().await;
    Mock::given(method("PATCH"))
        .and(path("/v3/issues/PROJ-1/worklog/12345"))
        .and(body_json(serde_json::json!({"duration": "PT2H"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 12345, "duration": "PT2H"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "worklog", "edit", "PROJ-1", "12345", "-d", "2h"])
        .assert()
        .success()
        .stdout("PROJ-1 worklog 12345 2h\n");
}

/// Changing nothing is a mistake, and it is caught before a request the way an
/// update that sets no field is.
#[tokio::test]
async fn a_worklog_edit_that_changes_nothing_is_refused() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "worklog", "edit", "PROJ-1", "12345"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("nothing to change"));

    assert!(harness.server.received_requests().await.unwrap().is_empty());
}

/// The whole mechanism: a start kept locally, turned into a worklog on stop.
///
/// `start` and `cancel` send nothing at all, which is why they can be answered
/// without a Tracker fixture.
#[tokio::test]
async fn a_timer_starts_locally_and_stops_as_a_worklog() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/worklog"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 7,
            "duration": "PT1M",
            "createdBy": {"id": "1", "login": "ilubenets"},
            "start": "2026-08-29T09:00:00.000+0000"
        })))
        .expect(1)
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "timer", "start", "PROJ-1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("timer started"));

    // Nothing was sent to start it.
    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );

    harness
        .run(&["issue", "timers"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1"))
        .stdout(predicate::str::contains("shown 1 of 1"));

    harness
        .run(&["issue", "timer", "stop", "PROJ-1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJ-1 worklog 7"));

    // Stopped means gone: the same timer must not be recorded twice.
    harness
        .run(&["issue", "timers"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shown 0 of 0"));
}

/// Starting over a running timer would throw away the time it had collected,
/// which is the one thing the command exists to keep.
#[tokio::test]
async fn starting_a_timer_twice_is_refused() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "timer", "start", "PROJ-1"])
        .assert()
        .success();
    harness
        .run(&["issue", "timer", "start", "PROJ-1"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("has been timed since"));
}

/// Cancelling says how long it had been running: dropping a number in silence
/// is how somebody finds out afterwards that they lost an afternoon.
#[tokio::test]
async fn cancelling_records_nothing_and_says_what_was_dropped() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "timer", "start", "PROJ-1"])
        .assert()
        .success();
    harness
        .run(&["issue", "timer", "cancel", "PROJ-1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not recorded"));

    assert!(
        harness
            .server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

/// A refused worklog must leave the clock running, or the time is lost to an
/// error the caller could otherwise have retried.
#[tokio::test]
async fn a_failed_stop_keeps_the_timer() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/worklog"))
        .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
            "errors": {}, "errorMessages": ["nope"], "statusCode": 422
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "timer", "start", "PROJ-1"])
        .assert()
        .success();
    harness
        .run(&["issue", "timer", "stop", "PROJ-1"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("still running"));

    harness
        .run(&["issue", "timers"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shown 1 of 1"));
}

/// Nothing to stop is an answer; "no timer running" while one runs in the other
/// organisation is a true sentence that sends somebody looking in the wrong
/// place.
#[tokio::test]
async fn stopping_what_was_never_started_says_so() {
    let harness = Harness::new().await;

    harness
        .run(&["issue", "timer", "stop", "PROJ-1"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("no timer running for PROJ-1"));
}

/// The one write path with no mocked answer until now: unlinking.
///
/// `issue links` prints the id first precisely so this command can be given it,
/// and a delete that names the wrong path would have failed only against a real
/// Tracker.
#[tokio::test]
async fn unlinking_sends_a_delete_to_the_link_by_its_id() {
    let harness = Harness::new().await;
    Mock::given(method("DELETE"))
        .and(path("/v3/issues/PROJ-1/links/987"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&harness.server)
        .await;

    harness
        .run(&["issue", "link", "delete", "PROJ-1", "987"])
        .assert()
        .success();
}
