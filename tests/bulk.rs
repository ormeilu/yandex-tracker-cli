//! Reading a bulk change back, after the command that started it has returned.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

#[tokio::test]
async fn status_reports_the_tally_of_a_finished_change() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d90773c59502bc8e028a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_complete.json")))
        .expect(1)
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["bulk", "status", "6a92d90773c59502bc8e028a"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("changed 2 of 2"), "{stdout}");
}

/// A change that failed is still an answer. Reading is not judging: the exit
/// code belongs to the command that made the change, not to the one that looks.
#[tokio::test]
async fn status_explains_a_failure_and_still_exits_zero() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d9387d41a060a2b5e6d9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_failed.json")))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d9387d41a060a2b5e6d9/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_issues.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["bulk", "status", "6a92d9387d41a060a2b5e6d9"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("changed 1 of 2"), "{stdout}");
    assert!(stdout.contains("PROJ-2"), "{stdout}");
}

/// One request while the work is still running: there is nothing per issue to
/// say yet, and asking would be a request spent on an empty answer.
#[tokio::test]
async fn status_of_a_running_change_asks_nothing_further() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/6a92d90773c59502bc8e028a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("bulkchange_created.json")))
        .expect(1)
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["bulk", "status", "6a92d90773c59502bc8e028a"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    // No tally is invented before Tracker has counted the issues.
    assert!(!stdout.contains("changed"), "{stdout}");
    assert!(stdout.contains("not counted yet"), "{stdout}");

    let requests = harness.server.received_requests().await.expect("recorded");
    assert_eq!(requests.len(), 1, "{requests:?}");
}

#[tokio::test]
async fn an_unknown_bulk_change_exits_four() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/bulkchange/nope"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&harness.server)
        .await;

    harness.run(&["bulk", "status", "nope"]).assert().code(4);
}
