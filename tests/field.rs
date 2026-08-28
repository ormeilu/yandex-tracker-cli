//! Organisation-wide fields and templates, against a stub Tracker.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::Harness;

async fn answer(harness: &Harness, route: &str, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(route.to_owned()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;
}

/// A field defined by the organisation rather than shipped with Tracker is the
/// reason to run this at all, so it is the one that stands out.
#[tokio::test]
async fn the_organisation_wide_listing_marks_what_is_not_tracker_s_own() {
    let harness = Harness::new().await;
    answer(
        &harness,
        "/v3/fields",
        serde_json::json!([
            {"id": "summary", "name": "Summary", "schema": {"type": "string"}},
            {
                "id": "60fa2c1e--storyPoints",
                "name": "Story points",
                "schema": {"type": "integer"}
            }
        ]),
    )
    .await;

    let output = harness.run(&["field", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    // The key a caller can type, not the prefixed id the payload carries.
    assert!(stdout.contains("storyPoints"));
    assert!(!stdout.contains("60fa2c1e--"), "{stdout}");
    assert!(stdout.ends_with("shown 2 of 2 (1 custom)\n"), "{stdout}");
}

/// There is no `_templates` collection; every plausible guess at one answers
/// 400 or 404. The paths are these two.
#[tokio::test]
async fn each_kind_of_template_has_its_own_path() {
    let harness = Harness::new().await;
    answer(
        &harness,
        "/v3/issueTemplates",
        serde_json::json!([{
            "id": 7,
            "name": "Incident",
            "queue": {"key": "PROJ"},
            "createdBy": {"id": "1", "login": "ilubenets"}
        }]),
    )
    .await;
    answer(
        &harness,
        "/v3/commentTemplates",
        serde_json::json!([{"id": 9, "name": "Asked for logs"}]),
    )
    .await;

    let issues = harness.run(&["template", "list"]).assert().success();
    let stdout = String::from_utf8(issues.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.contains("Incident"));
    assert!(stdout.contains("PROJ"));
    assert!(stdout.contains("ilubenets"));

    let comments = harness
        .run(&["template", "list", "--kind", "comment"])
        .assert()
        .success();
    let stdout = String::from_utf8(comments.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.contains("Asked for logs"), "{stdout}");
    assert!(stdout.contains("shown 1 of 1"));
}

/// An organisation with no templates is a normal answer, and a tally is how a
/// caller tells it apart from a command that printed nothing by accident.
#[tokio::test]
async fn nothing_to_list_still_ends_with_a_tally() {
    let harness = Harness::new().await;
    answer(&harness, "/v3/issueTemplates", serde_json::json!([])).await;

    let output = harness.run(&["template", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert_eq!(stdout, "shown 0 of 0\n");
}

/// The point of the command: a field with a fixed list of values says what they
/// are, so `--set` is not written blind.
#[tokio::test]
async fn a_fixed_list_of_values_is_printed_and_capped() {
    let harness = Harness::new().await;
    let values: Vec<String> = (1..=25).map(|n| format!("option-{n}")).collect();
    answer(
        &harness,
        "/v3/fields/component",
        serde_json::json!({
            "id": "60fa2c1e--component",
            "name": "Component",
            "schema": {"type": "string", "required": false},
            "readonly": false,
            "category": {"display": "Custom"},
            "optionsProvider": {"type": "FixedListOptionsProvider", "values": values}
        }),
    )
    .await;

    let output = harness
        .run(&["field", "get", "component"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("option-1,"), "{stdout}");
    assert!(!stdout.contains("option-25"), "{stdout}");
    assert!(
        stdout.ends_with("shown 20 of 25 values; --all for the rest\n"),
        "{stdout}"
    );

    let all = harness
        .run(&["field", "get", "component", "--all"])
        .assert()
        .success();
    let stdout = String::from_utf8(all.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.contains("option-25"), "{stdout}");
    assert!(stdout.ends_with("shown 25 of 25 values\n"), "{stdout}");
}

/// A provider that keeps its values elsewhere is the common case, and the only
/// useful thing to say about it is which command fetches them.
#[tokio::test]
async fn a_provider_without_values_names_the_command_that_has_them() {
    let harness = Harness::new().await;
    answer(
        &harness,
        "/v3/fields/followers",
        serde_json::json!({
            "id": "followers",
            "name": "Наблюдатели",
            "schema": {"type": "array", "items": "user"},
            "readonly": false,
            "optionsProvider": {"type": "TeamOptionsProvider"}
        }),
    )
    .await;

    let output = harness
        .run(&["field", "get", "followers"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    // `array` alone does not say what one element is, which is what decides
    // whether a write takes a value or a list.
    assert!(stdout.contains("type: array of user"), "{stdout}");
    assert!(stdout.contains("ytcli user list"), "{stdout}");
}

/// An unrecognised provider is passed through by name. Guessing at a command
/// for it would cost a request and read like a bug.
#[tokio::test]
async fn an_unknown_provider_is_named_rather_than_guessed_at() {
    let harness = Harness::new().await;
    answer(
        &harness,
        "/v3/fields/sla",
        serde_json::json!({
            "id": "sla",
            "name": "SLA",
            "schema": {"type": "sla"},
            "readonly": true,
            "optionsProvider": {"type": "OptionsProviderProto$SlaSettingsOptionsProviderProto"}
        }),
    )
    .await;

    let output = harness.run(&["field", "get", "sla"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("readonly: yes"), "{stdout}");
    assert!(
        stdout.contains("SlaSettingsOptionsProviderProto"),
        "{stdout}"
    );
    assert!(stdout.contains("not listed by this endpoint"), "{stdout}");
}
