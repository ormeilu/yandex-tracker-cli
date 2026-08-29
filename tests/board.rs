//! Boards and sprints, against a stub Tracker.
//!
//! The payloads here follow what a real organisation answered, which is where
//! the two surprises came from: a board's owner is `createdBy` and may have a
//! display name and no login, and a board that cannot have sprints answers the
//! sprint question with a refusal rather than an empty list.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{body_json, method, path};
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

/// One sprint, and the two ratios that say where it is: days, and issues.
///
/// Piped, so the numbers appear with nothing drawn around them — the bars are
/// chrome for a terminal, and an agent pays for every byte of them.
#[tokio::test]
async fn a_sprint_reports_its_dates_and_how_many_issues_are_resolved() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/sprints/21"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 21,
            "name": "Sprint 4",
            "board": {"id": "9", "display": "Infrastructure"},
            "status": "in_progress",
            "startDate": "2026-08-17",
            "endDate": "2026-08-28"
        })))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/_count"))
        .and(body_json(serde_json::json!({"query": "Sprint: 21"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(10))
        .expect(1)
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/issues/_count"))
        .and(body_json(
            serde_json::json!({"query": "Sprint: 21 AND Resolution: empty()"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(4))
        .expect(1)
        .mount(&harness.server)
        .await;

    let output = harness.run(&["sprint", "get", "21"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("2026-08-17 → 2026-08-28"), "{stdout}");
    assert!(stdout.contains("6 of 10 resolved"), "{stdout}");
    assert!(!stdout.contains('▓'), "a pipe was given a bar: {stdout}");
}

/// The counts are two requests, and a caller who only wanted the dates should
/// not pay for them.
#[tokio::test]
async fn no_issues_asks_for_the_sprint_and_nothing_else() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/sprints/21"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 21, "name": "Sprint 4", "status": "in_progress"
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["sprint", "get", "21", "--no-issues"])
        .assert()
        .success();

    let requests = harness.server.received_requests().await.expect("recorded");
    assert_eq!(requests.len(), 1, "{requests:?}");
}

/// Work planned now belongs to the next sprint, not to the one people are in
/// the middle of. The tally still counts what was found, so what was left out
/// is visible rather than implied.
#[tokio::test]
async fn planning_narrows_the_listing_to_the_sprint_to_plan_into() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/sprints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("all_sprints.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["sprint", "list", "--planning"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Infrastructure"), "{stdout}");
    assert!(
        !stdout.contains("Storage"),
        "the running sprint was chosen: {stdout}"
    );
    assert!(stdout.ends_with("shown 1 of 1\n"), "{stdout}");
}
