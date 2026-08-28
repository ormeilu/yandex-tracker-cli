//! People, against a stub Tracker.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

/// The directory, as one page.
async fn directory(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v3/users"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("users.json"))
                .append_header("X-Total-Count", "4"),
        )
        .mount(&harness.server)
        .await;
}

#[tokio::test]
async fn a_listing_ends_with_a_tally() {
    let harness = Harness::new().await;
    directory(&harness).await;

    let output = harness.run(&["user", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("ilubenets"), "{stdout}");
    assert!(stdout.ends_with("shown 4 of 4\n"), "{stdout}");
}

/// A dismissed account still owns everything it was ever assigned, so hiding it
/// would lose the answer to "who had this". Saying so in a column is the whole
/// difference between listing it and misleading with it.
#[tokio::test]
async fn a_departed_colleague_is_listed_and_labelled() {
    let harness = Harness::new().await;
    directory(&harness).await;

    let output = harness.run(&["user", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("departed"), "{stdout}");
    assert!(stdout.contains("dismissed"), "{stdout}");
    assert!(stdout.contains("external"), "{stdout}");
}

#[tokio::test]
async fn one_person_is_fetched_by_login() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/users/ilubenets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("user.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["user", "get", "ilubenets"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.starts_with("ilubenets  Ilya Lubenets\n"), "{stdout}");
    assert!(stdout.contains("state: active"), "{stdout}");
}

/// Tracker has no user search endpoint, so a match is found here — over login,
/// display name and email alike.
#[tokio::test]
async fn find_matches_login_name_and_email() {
    let harness = Harness::new().await;
    directory(&harness).await;

    for (needle, expected) in [
        ("ilubenets", "ilubenets"),
        ("outside", "contractor"),
        ("elsewhere.example", "contractor"),
    ] {
        let output = harness.run(&["user", "find", needle]).assert().success();
        let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");
        assert!(
            stdout.contains(expected),
            "{needle} did not find {expected}"
        );
    }
}

/// The tally counts what was read, not what the organisation holds: claiming
/// `1 of 4` for a search that read all four would be right, but claiming a
/// total nobody searched would not.
#[tokio::test]
async fn a_search_reports_how_many_people_it_read() {
    let harness = Harness::new().await;
    directory(&harness).await;

    let output = harness
        .run(&["user", "find", "ilubenets"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.ends_with("shown 1 of 4\n"), "{stdout}");
}

/// A search that stopped early has to say so on stderr. A short answer that
/// looks complete is the failure this whole command is one step away from.
#[tokio::test]
async fn a_search_that_hit_its_ceiling_says_so() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/users"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("users.json"))
                .append_header("X-Total-Count", "900"),
        )
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["user", "find", "nobody", "--scan", "2"])
        .assert()
        .success();
    let stderr = String::from_utf8(output.get_output().stderr.clone()).expect("utf-8");

    assert!(stderr.contains("searched 4 of 900"), "{stderr}");
    assert!(stderr.contains("--scan"), "{stderr}");
}

#[tokio::test]
async fn a_page_is_asked_for_by_number() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/users"))
        .and(query_param("page", "2"))
        .and(query_param("perPage", "10"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("users.json"))
                .append_header("X-Total-Count", "4"),
        )
        .mount(&harness.server)
        .await;

    harness
        .run(&["user", "list", "--page", "2", "--limit", "10"])
        .assert()
        .success();
}

/// `me` is not a user. Tracker answers 404 for it, and the command that knows
/// who you are is `auth status`.
#[tokio::test]
async fn an_unknown_login_exits_four() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/users/me"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "errorMessages": ["пользователь me не существует."],
            "statusCode": 404
        })))
        .mount(&harness.server)
        .await;

    harness.run(&["user", "get", "me"]).assert().code(4);
}
