//! The documented examples, run.
//!
//! README and the cheatsheet are the two places somebody copies a command out
//! of, and documentation rots quietly: nothing fails when a flag is renamed or a
//! line of output changes shape, until it fails in a user's terminal. So the
//! examples are test cases. `trycmd` runs each one against the same `wiremock`
//! stub the rest of the suite uses and compares the whole of stdout, which makes
//! a change in output shape a diff in these files rather than a surprise.
//!
//! Two tests, because docs rot in two ways:
//!
//! 1. [`the_documented_examples_still_produce_the_documented_output`] runs what
//!    can be run.
//! 2. [`every_documented_command_is_either_run_or_declared_unrunnable`] makes
//!    sure the first one keeps up: a newly documented command has to be given a
//!    case, or listed here with the reason it cannot have one.
//!
//! Update the expected output with `TRYCMD=overwrite cargo test --test docs`,
//! and then read the diff — the output shape is the product, not a detail.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

/// Everything the documented read commands ask for.
async fn tracker_answers(harness: &Harness) {
    let json = |name: &str| ResponseTemplate::new(200).set_body_json(fixture(name));

    for (route, body) in [
        ("/v3/issues/PROJ-1", "issue.json"),
        ("/v3/issues/PROJ-1/links", "issue_links.json"),
        ("/v3/issues/PROJ-1/comments", "issue_comments.json"),
        ("/v3/issues/PROJ-1/changelog", "changelog.json"),
        ("/v3/issues/PROJ-1/remotelinks", "issue_remotelinks.json"),
        ("/v3/queues", "queues.json"),
        ("/v3/queues/PROJ", "queue.json"),
        ("/v3/queues/PROJ/fields", "queue_fields.json"),
        ("/v3/queues/PROJ/versions", "queue_versions.json"),
        ("/v3/queues/PROJ/tags", "queue_tags.json"),
        ("/v3/queues/PROJ/macros", "queue_macros.json"),
        ("/v3/queues/PROJ/permissions", "queue_permissions.json"),
        ("/v3/queues/PROJ/access", "queue_access.json"),
        ("/v3/myself", "user.json"),
        (
            "/v3/bulkchange/6a92d90773c59502bc8e028a",
            "bulkchange_complete.json",
        ),
        ("/v3/queues/PROJ/localFields", "queue_local_fields.json"),
        ("/v3/sprints", "all_sprints.json"),
        ("/v3/queues/PROJ/autoactions", "queue_autoactions.json"),
        ("/v3/queues/PROJ/triggers", "queue_triggers.json"),
        ("/v3/fields", "fields.json"),
        ("/v3/components", "components.json"),
        ("/v3/linktypes", "linktypes.json"),
        ("/v3/fields/storyPoints", "field.json"),
        ("/v3/issueTemplates", "issue_templates.json"),
        ("/v3/boards", "boards.json"),
        ("/v3/boards/6", "board.json"),
        ("/v3/boards/9/sprints", "sprints.json"),
        ("/v3/issuetypes", "issuetypes.json"),
        ("/v3/priorities", "priorities.json"),
        ("/v3/statuses", "statuses.json"),
        ("/v3/resolutions", "resolutions.json"),
        ("/v3/users/ilubenets", "user.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
            .mount(&harness.server)
            .await;
    }

    // Search reports its total in a header, which is where the `shown N of M`
    // tally comes from.
    Mock::given(method("POST"))
        .and(path("/v3/issues/_search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("issue_search.json"))
                .append_header("X-Total-Count", "2"),
        )
        .mount(&harness.server)
        .await;

    entity_answers(harness).await;

    Mock::given(method("GET"))
        .and(path("/v3/boards/6/sprints"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "errors": {},
            "errorMessages": ["A board of this type cannot have sprints."],
            "statusCode": 400
        })))
        .mount(&harness.server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v3/worklog"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 1,
                "issue": {"key": "PROJ-1", "display": "Attachments are lost on move"},
                "duration": "PT1H30M",
                "start": "2026-08-24T09:00:00.000+0300",
                "createdBy": {"id": "1", "login": "ilubenets", "display": "Ilya Lubenets"},
                "comment": "pairing"
            }
        ])))
        .mount(&harness.server)
        .await;

    // The directory reports its size in a header, like every other listing.
    Mock::given(method("GET"))
        .and(path("/v3/users"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("users.json"))
                .append_header("X-Total-Count", "4"),
        )
        .mount(&harness.server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v3/issues/_count"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(2)))
        .mount(&harness.server)
        .await;
}

/// Projects, portfolios and goals, which are searched rather than fetched and
/// are told apart by the body of the search rather than by its path.
async fn entity_answers(harness: &Harness) {
    let json = |name: &str| ResponseTemplate::new(200).set_body_json(fixture(name));

    // The entity endpoints are typed and POST for everything, so the searches
    // are told apart by their body: an unfiltered listing sends `{}`, and
    // containment sends the parent it is asking about.
    let parent = serde_json::json!({
        "filter": {"parentEntity": "655a1d0c5f1b2c0011223344"}
    });
    for (kind, listing, contained) in [
        (
            "portfolio",
            "entities_portfolio.json",
            "entities_in_portfolio.json",
        ),
        (
            "project",
            "entities_project.json",
            "projects_in_portfolio.json",
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/v3/entities/{kind}/_search")))
            .and(body_json(parent.clone()))
            .respond_with(json(contained))
            .mount(&harness.server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/v3/entities/{kind}/_search")))
            .and(body_json(serde_json::json!({})))
            .respond_with(json(listing))
            .mount(&harness.server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/v3/entities/goal/_search"))
        .respond_with(json("entities_goal.json"))
        .mount(&harness.server)
        .await;

    for (route, body) in [
        (
            "/v3/entities/portfolio/655a1d0c5f1b2c0011223355",
            "entity_portfolio.json",
        ),
        (
            "/v3/entities/project/655a1d0c5f1b2c0011223366",
            "entity_project.json",
        ),
        (
            "/v3/entities/goal/655a1d0c5f1b2c0011223388",
            "entity_goal.json",
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
            .mount(&harness.server)
            .await;
    }

    // A kanban board refuses the sprint question. The documented case pins how
    // that reads, because passing Tracker's own words through is the behaviour.
}

#[tokio::test]
async fn the_documented_examples_still_produce_the_documented_output() {
    let harness = Harness::new().await;
    tracker_answers(&harness).await;

    trycmd::TestCases::new()
        .default_bin_name("ytcli")
        .env("YTCLI_CONFIG", harness.config_path().display().to_string())
        .env("YTCLI_BASE_URL", harness.server.uri())
        .env("YTCLI_TOKEN", "test-token")
        .env("YTCLI_PROFILE", "test")
        .case("tests/docs/*.md")
        .run();
}

/// Commands documented but not run here, each with the reason.
///
/// A write is not on this list because it is dangerous — `--dry-run` sends
/// nothing — but because what it prints names a profile and an organisation
/// belonging to whoever runs it. `login` is interactive and asks for a token.
const UNRUNNABLE: &[(&str, &str)] = &[
    ("auth login", "interactive; asks for a token"),
    ("auth logout", "would touch the keychain"),
    (
        "auth status",
        "several requests per profile, and prints identity",
    ),
    ("auth list", "prints the profiles of whoever runs it"),
    ("auth use", "rewrites the config of whoever runs it"),
    ("issue create", "a write"),
    ("queue create", "a write"),
    ("project create", "a write"),
    ("project update", "a write"),
    ("project delete", "a write"),
    ("issue update", "a write"),
    ("issue comment", "a write"),
    ("issue transition", "a write"),
    ("issue move", "a write"),
    ("issue worklogs", "no recorded worklog fixture yet"),
    ("issue checklist", "no recorded checklist fixture yet"),
    ("issue worklog", "a write"),
    ("issue check", "a write"),
    ("issue link", "a write"),
    ("portfolio place", "a write"),
    ("project place", "a write"),
    ("attachment list", "no recorded attachment fixture yet"),
    (
        "attachment show",
        "draws pixels, or names a download command with a real id",
    ),
    ("attachment download", "writes a file"),
    ("attachment upload", "a write"),
];

/// Every `ytcli <group> <verb>` in README or the cheatsheet is either exercised
/// above or admitted to be unrunnable.
///
/// Without this the first test decays into a snapshot of whatever was true when
/// it was written, while the documentation grows commands nothing checks.
#[test]
fn every_documented_command_is_either_run_or_declared_unrunnable() {
    let cases = read("tests/docs/readme.md") + &read("tests/docs/cheatsheet.md");

    for (source, text) in documentation() {
        for command in commands_in(&text) {
            if UNRUNNABLE.iter().any(|(name, _)| *name == command) {
                continue;
            }
            assert!(
                cases.contains(&format!("$ ytcli {command}")),
                "{source} documents `ytcli {command}`, which no case runs. \
                 Add one to tests/docs/, or add it to UNRUNNABLE with a reason."
            );
        }
    }
}

fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{relative} is readable"))
}

fn documentation() -> Vec<(&'static str, String)> {
    vec![
        ("README.md", read("README.md")),
        ("docs/cheatsheet.txt", read("docs/cheatsheet.txt")),
    ]
}

/// `ytcli issue get PROJ-1 --full` → `issue get`.
///
/// Lower-case words are verbs; the first word that is not ends the path, which
/// is how `PROJ-1`, `<group>` and `--full` stay out of it.
fn commands_in(text: &str) -> Vec<String> {
    let mut found: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("ytcli ")?;
            let path: Vec<&str> = rest
                .split_whitespace()
                .take_while(|word| word.chars().all(|c| c.is_ascii_lowercase()))
                .take(2)
                .collect();
            (path.len() == 2).then(|| path.join(" "))
        })
        .collect();
    found.sort();
    found.dedup();
    found
}
