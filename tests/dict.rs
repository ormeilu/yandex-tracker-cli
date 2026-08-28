//! The dictionaries a write has to quote from, against a stub Tracker.
//!
//! The promise being tested is not "it prints a list" but that the list is
//! usable: the stable key is present, the endpoint spellings are the ones
//! Tracker actually serves, and asking for one dictionary costs one request.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

async fn dictionaries(harness: &Harness) {
    for (route, body) in [
        ("/v3/issuetypes", "issuetypes.json"),
        ("/v3/priorities", "priorities.json"),
        ("/v3/statuses", "statuses.json"),
        ("/v3/resolutions", "resolutions.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture(body)))
            .mount(&harness.server)
            .await;
    }
}

/// One call, before a write, answering all four questions it could raise.
#[tokio::test]
async fn all_four_dictionaries_come_back_in_one_answer() {
    let harness = Harness::new().await;
    dictionaries(&harness).await;

    let output = harness.run(&["dict", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    for heading in ["types", "priorities", "statuses", "resolutions"] {
        assert!(stdout.contains(heading), "{heading} missing from {stdout}");
    }
    assert_eq!(
        harness.server.received_requests().await.unwrap().len(),
        4,
        "one request per dictionary, no more"
    );
}

/// The whole reason the key is printed: `name` arrives in the organisation's
/// language, and a script that quoted it would break in the next organisation.
#[tokio::test]
async fn the_stable_key_is_shown_next_to_the_translated_name() {
    let harness = Harness::new().await;
    dictionaries(&harness).await;

    let output = harness
        .run(&["dict", "list", "--kind", "types"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("bug"), "{stdout}");
    assert!(stdout.contains("Ошибка"), "{stdout}");
}

/// Asking for one dictionary asks Tracker for one dictionary.
#[tokio::test]
async fn a_single_kind_costs_a_single_request() {
    let harness = Harness::new().await;
    dictionaries(&harness).await;

    harness
        .run(&["dict", "list", "--kind", "statuses"])
        .assert()
        .success();

    let requests = harness.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/v3/statuses");
}

/// A status's category is what makes the list readable without knowing the
/// workflow, and only statuses have one.
#[tokio::test]
async fn statuses_carry_their_category_and_the_others_do_not() {
    let harness = Harness::new().await;
    dictionaries(&harness).await;

    let statuses = harness
        .run(&["dict", "list", "--kind", "statuses"])
        .assert()
        .success();
    let statuses = String::from_utf8(statuses.get_output().stdout.clone()).expect("utf-8");
    let open = statuses
        .lines()
        .find(|line| line.starts_with("open"))
        .expect("the open status");
    assert!(open.ends_with("new"), "{open}");

    let priorities = harness
        .run(&["dict", "list", "--kind", "priorities"])
        .assert()
        .success();
    let priorities = String::from_utf8(priorities.get_output().stdout.clone()).expect("utf-8");
    let trivial = priorities
        .lines()
        .find(|line| line.starts_with("trivial"))
        .expect("the trivial priority");
    assert_eq!(
        trivial.split_whitespace().count(),
        2,
        "a third column of dashes on a dictionary that has no category: {trivial}"
    );
}

/// Four lists of `{key, name}` concatenated would be four lists nobody can tell
/// apart; the machine format keys them by which dictionary they came from.
#[tokio::test]
async fn json_keys_each_list_by_its_dictionary() {
    let harness = Harness::new().await;
    dictionaries(&harness).await;

    let output = harness
        .run(&["dict", "list", "--format", "json"])
        .assert()
        .success();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("json");

    assert_eq!(parsed["types"][0]["key"], "bug");
    assert_eq!(parsed["statuses"][0]["type"], "new");
    assert_eq!(parsed["resolutions"][1]["key"], "wontFix");
}
