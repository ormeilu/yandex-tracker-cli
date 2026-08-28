//! Boards and sprints, against a stub Tracker.
//!
//! The payloads here follow what a real organisation answered, which is where
//! the two surprises came from: a board's owner is `createdBy` and may have a
//! display name and no login, and a board that cannot have sprints answers the
//! sprint question with a refusal rather than an empty list.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

fn boards() -> serde_json::Value {
    serde_json::json!([
        {
            "id": 6,
            "version": 6,
            "name": "Delivery",
            "createdBy": {"id": "1", "display": "Kim Novak"},
            "columns": [
                {"id": "1", "display": "Open"},
                {"id": "2", "display": "In Progress"},
                {"id": "3", "display": "Done"}
            ],
            "estimateBy": {"id": "storyPoints", "display": "Story Points"}
        },
        {
            "id": 9,
            "version": 1,
            "name": "Support",
            "createdBy": {"id": "2", "login": "ilubenets", "display": "Ilya"},
            "columns": [{"id": "1", "display": "Open"}]
        }
    ])
}

async fn one_board(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v3/boards/6"))
        .respond_with(ResponseTemplate::new(200).set_body_json(boards()[0].clone()))
        .mount(&harness.server)
        .await;
}

#[tokio::test]
async fn a_listing_says_which_board_not_how_it_is_built() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/boards"))
        .respond_with(ResponseTemplate::new(200).set_body_json(boards()))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["board", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Delivery"));
    assert!(stdout.contains("storyPoints"));
    // The count, not the columns: those are what `board get` is for.
    assert!(!stdout.contains("In Progress"), "{stdout}");
    assert!(stdout.ends_with("shown 2 of 2\n"), "{stdout}");
}

/// Columns in board order are the one thing a command line says better than the
/// web interface, so they are printed as a sequence rather than as a set.
#[tokio::test]
async fn a_board_prints_its_columns_in_order() {
    let harness = Harness::new().await;
    one_board(&harness).await;

    let output = harness.run(&["board", "get", "6"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Open → In Progress → Done"), "{stdout}");
}

/// A real organisation had a board owner with a display name and no login.
#[tokio::test]
async fn an_owner_without_a_login_is_still_named() {
    let harness = Harness::new().await;
    one_board(&harness).await;

    let output = harness.run(&["board", "get", "6"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Kim Novak"), "{stdout}");
}

/// "No sprints" and "cannot have sprints" are different answers.
#[tokio::test]
async fn a_board_that_cannot_have_sprints_says_so_rather_than_answering_empty() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/boards/6/sprints"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "errors": {},
            "errorMessages": ["A board of this type cannot have sprints."],
            "statusCode": 400
        })))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["board", "sprints", "6"]).assert().code(5);
    let stderr = String::from_utf8(output.get_output().stderr.clone()).expect("utf-8");

    assert!(stderr.contains("cannot have sprints"), "{stderr}");
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.is_empty(), "an empty listing was printed: {stdout}");
}

#[tokio::test]
async fn sprints_list_with_their_dates_and_a_tally() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/boards/9/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 21,
                "name": "Sprint 4",
                "status": "in_progress",
                "startDate": "2026-08-17",
                "endDate": "2026-08-28"
            }
        ])))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["board", "sprints", "9"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Sprint 4"));
    assert!(stdout.contains("2026-08-28"));
    assert!(stdout.ends_with("shown 1 of 1 for board 9\n"), "{stdout}");
}

/// A sprint name is a thing people say without knowing the board. The board
/// column is what makes two of them called "Sprint 1" tellable apart, and is
/// the whole difference from `board sprints`.
#[tokio::test]
async fn sprints_across_the_organisation_name_their_board() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("all_sprints.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["sprint", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert_eq!(stdout.matches("Sprint 1").count(), 2, "{stdout}");
    assert!(stdout.contains("Storage"), "{stdout}");
    assert!(stdout.contains("Infrastructure"), "{stdout}");
    assert!(stdout.ends_with("shown 2 of 2\n"), "{stdout}");
}
