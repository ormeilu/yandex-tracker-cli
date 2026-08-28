//! `link types`, and the write vocabulary it exists to keep separate.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

async fn types(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v3/linktypes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("linktypes.json")))
        .mount(&harness.server)
        .await;
}

/// The whole point: two vocabularies, printed together. `depends` is a type id
/// and `depends on` is what a write takes, and confusing them is what this
/// command is for.
#[tokio::test]
async fn both_vocabularies_are_on_the_same_row() {
    let harness = Harness::new().await;
    types(&harness).await;

    let output = harness.run(&["link", "types"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    let depends = stdout
        .lines()
        .find(|line| line.starts_with("depends on"))
        .expect("no row for the direction that depends");
    assert!(depends.contains("зависит от"), "{stdout}");
    assert!(depends.ends_with("depends"), "{stdout}");

    assert!(stdout.contains("is dependent by"), "{stdout}");
    assert!(stdout.contains("is parent task for"), "{stdout}");
    assert!(stdout.contains("is subtask for"), "{stdout}");
}

/// A type nothing can write is still a type. Reads return links of it, and
/// leaving the row out would say it does not exist.
#[tokio::test]
async fn a_type_with_no_write_name_is_shown_with_a_dash() {
    let harness = Harness::new().await;
    types(&harness).await;

    let output = harness.run(&["link", "types"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    let cloners: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("cloners"))
        .collect();
    assert_eq!(cloners.len(), 2, "{stdout}");
    for line in cloners {
        assert!(line.starts_with('-'), "{line}");
    }
}

/// One relationship, not two rows saying the same thing twice.
#[tokio::test]
async fn a_type_that_reads_the_same_both_ways_is_one_row() {
    let harness = Harness::new().await;
    types(&harness).await;

    let output = harness.run(&["link", "types"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert_eq!(stdout.matches("relates").count(), 2, "{stdout}");
    assert!(stdout.ends_with("shown 7 of 7\n"), "{stdout}");
}

/// The four names that look right and are not. Refused here rather than sent:
/// Tracker's own rejection names the value it did not recognise and never what
/// it wanted instead, and no request is worth spending to learn that.
#[tokio::test]
async fn a_link_type_id_is_refused_as_a_relationship_without_a_request() {
    let harness = Harness::new().await;

    for (wrong, right) in [
        ("depends", "depends on"),
        ("parent", "is parent task for"),
        ("subtask", "is subtask for"),
        ("epic", "is epic of"),
    ] {
        let output = harness
            .run(&["issue", "link", "add", "PROJ-1", wrong, "PROJ-2", "--yes"])
            .assert()
            .code(2);
        let stderr = String::from_utf8(output.get_output().stderr.clone()).expect("utf-8");
        assert!(stderr.contains(right), "{wrong}: {stderr}");
        assert!(stderr.contains("ytcli link types"), "{wrong}: {stderr}");
    }

    // Nothing was mounted, so any request at all would have failed the run.
    assert!(
        harness
            .server
            .received_requests()
            .await
            .is_none_or(|r| r.is_empty())
    );
}

/// Hyphens are Tracker's business, not ours: it accepts them, so a name that
/// only differs by separator is passed through untouched.
#[tokio::test]
async fn a_hyphenated_relationship_is_not_second_guessed() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/PROJ-1/links"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": 5})))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "issue",
            "link",
            "add",
            "PROJ-1",
            "is-dependent-by",
            "PROJ-2",
            "--yes",
        ])
        .assert()
        .success();
}
