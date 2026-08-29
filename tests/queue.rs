//! `queue list` and `queue fields` against a stub Tracker.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::{Harness, fixture};

#[tokio::test]
async fn queue_list_shows_key_name_and_lead() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queues.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("PROJ"));
    assert!(stdout.contains("Product"));
    assert!(stdout.contains("ilubenets"));
    // A queue without a lead still occupies its column.
    assert!(stdout.contains("INFRA"));
    assert!(stdout.contains(" -\n"));
    assert!(stdout.ends_with("shown 2 of 2\n"));
}

/// The point of the command: custom field keys are what `--fields` and `--set`
/// accept, and Tracker prefixes them with an opaque id that nobody can type.
#[tokio::test]
async fn queue_fields_exposes_typeable_custom_field_keys() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/fields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queue_fields.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "fields", "PROJ"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("storyPoints"));
    assert!(!stdout.contains("603bd9b6cdc7ba0d2f4b1a55--"));
    assert!(stdout.contains("integer"));
    assert!(stdout.contains("custom"));
    assert!(stdout.contains("summary"));
    assert!(stdout.contains("system"));
    assert!(stdout.ends_with("shown 4 of 4 (2 custom)\n"));
}

#[tokio::test]
async fn queue_fields_as_json_carries_the_same_keys() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/fields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queue_fields.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["queue", "fields", "PROJ", "--format", "json"])
        .assert()
        .success();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.get_output().stdout).expect("json");

    assert_eq!(parsed[2]["key"], "storyPoints");
    assert_eq!(parsed[2]["field_type"], "integer");
    assert_eq!(parsed[2]["system"], false);
}

#[tokio::test]
async fn an_unknown_queue_exits_four() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/NOPE/fields"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&harness.server)
        .await;

    harness
        .run(&["queue", "fields", "NOPE"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("queue NOPE fields not found"));
}

/// The defaults are the reason to read a queue: `issue create -q PROJ` with no
/// type and no priority gets them, and nothing else says what they are.
#[tokio::test]
async fn queue_get_says_what_a_new_issue_starts_as() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queue.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "get", "PROJ"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("PROJ"));
    assert!(stdout.contains("default type: task"), "{stdout}");
    assert!(stdout.contains("default priority: normal"), "{stdout}");
    // The key, not the localised label: that is what `issue create` takes.
    assert!(!stdout.contains("Normal"), "{stdout}");
}

/// A queue nobody can see is a 404, and a 404 is exit code 4.
#[tokio::test]
async fn reading_an_unknown_queue_exits_four() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/NOPE"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "errorMessages": ["Queue does not exist."],
            "statusCode": 404
        })))
        .mount(&harness.server)
        .await;

    harness
        .run(&["queue", "get", "NOPE"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("queue NOPE not found"));
}

fn blueprint() -> serde_json::Value {
    serde_json::json!({
        "id": 1,
        "key": "PROJ",
        "version": 2,
        "name": "Product",
        "lead": {"id": "1", "login": "ilubenets"},
        "defaultType": {"id": "2", "key": "task", "display": "Task"},
        "defaultPriority": {"id": "3", "key": "normal", "display": "Normal"},
        "issueTypesConfig": [{
            "issueType": {"id": "2", "key": "task", "display": "Task"},
            "workflow": {"id": "quickStartV2PresetWorkflow", "display": "Preset"},
            "resolutions": [
                {"id": "1", "key": "fixed", "display": "Fixed"},
                {"id": "2", "key": "wontFix", "display": "Won\'t fix"}
            ]
        }]
    })
}

async fn a_queue_to_copy(harness: &Harness) {
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ"))
        .respond_with(ResponseTemplate::new(200).set_body_json(blueprint()))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "1", "login": "ilubenets", "display": "Ilya"
        })))
        .mount(&harness.server)
        .await;
}

/// The whole point of `--like`: workflow ids are organisation-specific strings
/// nobody has memorised, so they are copied rather than asked for — and copied
/// as the keys the create endpoint takes, not the objects the read answers with.
#[tokio::test]
async fn creating_copies_the_issue_types_as_keys_and_ids() {
    let harness = Harness::new().await;
    a_queue_to_copy(&harness).await;
    Mock::given(method("POST"))
        .and(path("/v3/queues"))
        .and(body_json(serde_json::json!({
            "key": "OPS",
            "name": "Operations",
            "lead": "ilubenets",
            "defaultType": "task",
            "defaultPriority": "normal",
            "issueTypesConfig": [{
                "issueType": "task",
                "workflow": "quickStartV2PresetWorkflow",
                "resolutions": ["fixed", "wontFix"]
            }]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": 9,
            "key": "OPS",
            "version": 1,
            "name": "Operations",
            "lead": {"id": "1", "login": "ilubenets"},
            "defaultType": {"id": "2", "key": "task"},
            "defaultPriority": {"id": "3", "key": "normal"}
        })))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&[
            "queue",
            "create",
            "-k",
            "OPS",
            "-n",
            "Operations",
            "--like",
            "PROJ",
            "--yes",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("OPS"));
    assert!(stdout.contains("Operations"));
}

/// A queue key is claimed once — Tracker deletes a queue by hiding it, and the
/// key stays spent. One target, and `--yes` all the same.
#[tokio::test]
async fn creating_a_queue_without_yes_sends_nothing() {
    let harness = Harness::new().await;
    a_queue_to_copy(&harness).await;

    harness
        .run(&[
            "queue",
            "create",
            "-k",
            "OPS",
            "-n",
            "Operations",
            "--like",
            "PROJ",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be undone"));

    let requests = harness.server.received_requests().await.unwrap_or_default();
    assert!(
        requests.iter().all(|request| request.method == "GET"),
        "{requests:?}"
    );
}

/// `--dry-run` prints the body `--like` decided on, which is the cheap way to
/// find out what a queue would be created with.
#[tokio::test]
async fn a_dry_run_prints_what_like_decided() {
    let harness = Harness::new().await;
    a_queue_to_copy(&harness).await;

    let output = harness
        .run(&[
            "queue",
            "create",
            "-k",
            "OPS",
            "-n",
            "Operations",
            "--like",
            "PROJ",
            "--yes",
            "--dry-run",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8(output.get_output().stderr.clone()).expect("utf-8");

    assert!(stderr.contains("quickStartV2PresetWorkflow"), "{stderr}");
    let requests = harness.server.received_requests().await.unwrap_or_default();
    assert!(requests.iter().all(|request| request.method == "GET"));
}

/// A queue with no issue types cannot be copied from, and saying so beats
/// sending a body Tracker will refuse for a reason nobody can read.
#[tokio::test]
async fn a_model_queue_without_issue_types_is_refused_before_the_write() {
    let harness = Harness::new().await;
    let mut thin = blueprint();
    thin["issueTypesConfig"] = serde_json::json!([]);
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ"))
        .respond_with(ResponseTemplate::new(200).set_body_json(thin))
        .mount(&harness.server)
        .await;

    harness
        .run(&[
            "queue",
            "create",
            "-k",
            "OPS",
            "-n",
            "Operations",
            "--like",
            "PROJ",
            "--yes",
        ])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("issue types of queue PROJ"));
}

/// Versions are what an issue's `fixVersions` points at, and the state is the
/// column that decides whether new work may still point at one.
#[tokio::test]
async fn versions_carry_the_state_that_says_whether_they_are_still_open() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"id": 1, "name": "1.0", "released": true, "archived": false, "dueDate": "2026-06-01"},
            {"id": 2, "name": "1.1", "released": false, "archived": false},
            {"id": 3, "name": "0.9", "released": true, "archived": true}
        ])))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["queue", "versions", "PROJ"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("released"), "{stdout}");
    assert!(stdout.contains("open"), "{stdout}");
    // Archived wins over released: it was both, and only one of the two says
    // that nothing new should point at it.
    let archived = stdout
        .lines()
        .find(|line| line.starts_with("3 "))
        .expect("the archived version");
    assert!(archived.contains("archived"), "{archived}");
    assert!(stdout.ends_with("shown 3 of 3 for PROJ\n"), "{stdout}");
}

/// Tracker has answered tags as bare strings and as named objects depending on
/// where you read; a listing that silently drops every row would be worse than
/// reading a member it did not need.
#[tokio::test]
async fn tags_are_read_whichever_shape_they_arrive_in() {
    for body in [
        serde_json::json!(["backend", "urgent"]),
        serde_json::json!([{"name": "backend"}, {"name": "urgent"}]),
    ] {
        let harness = Harness::new().await;
        Mock::given(method("GET"))
            .and(path("/v3/queues/PROJ/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&harness.server)
            .await;

        let output = harness.run(&["queue", "tags", "PROJ"]).assert().success();
        let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

        assert!(stdout.contains("backend"), "{stdout}");
        assert!(stdout.contains("urgent"), "{stdout}");
    }
}

/// Three endpoints, one answer. An issue changed by the Tracker robot was
/// changed by one of these, and nothing in this tool used to say what they are.
#[tokio::test]
async fn automation_reports_all_three_kinds() {
    let harness = Harness::new().await;
    for (route, body) in [
        ("/v3/queues/PROJ/macros", "queue_macros.json"),
        ("/v3/queues/PROJ/autoactions", "queue_autoactions.json"),
        ("/v3/queues/PROJ/triggers", "queue_triggers.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture(body)))
            .mount(&harness.server)
            .await;
    }

    let output = harness
        .run(&["queue", "automation", "PROJ"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Ask for logs"), "{stdout}");
    // The key a caller could type, not the prefixed id the payload carries.
    assert!(stdout.contains("tags, component"), "{stdout}");
    assert!(!stdout.contains("60fa2c1e--"), "{stdout}");
    // Milliseconds on the wire, seconds in the answer.
    assert!(stdout.contains("3600s"), "{stdout}");
    assert!(stdout.contains("Nudge stale issues"), "{stdout}");
    assert!(stdout.contains("Close on merge"), "{stdout}");
    assert!(stdout.contains("Transition"), "{stdout}");
    assert_eq!(
        stdout.matches("shown 1 of 1 for PROJ").count(),
        3,
        "{stdout}"
    );
}

/// The case the command is shaped around: triggers need queue-owner rights, so
/// most callers get two sections and a reason for the third. Two answers out of
/// three beat a command that fails wholesale.
#[tokio::test]
async fn a_section_that_is_refused_says_so_instead_of_counting_zero() {
    let harness = Harness::new().await;
    for route in ["/v3/queues/PROJ/macros", "/v3/queues/PROJ/autoactions"] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&harness.server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/triggers"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({"errors": {}})))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["queue", "automation", "PROJ"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    // "none" and "you may not see them" are different answers, and a tally
    // under a refused section would state the wrong one.
    assert!(stdout.contains("queue owner only"), "{stdout}");
    assert_eq!(
        stdout.matches("shown 0 of 0 for PROJ").count(),
        2,
        "{stdout}"
    );
}

/// All three refused is not a partial answer; it is a queue that is not there
/// or a token that cannot see it, and it exits like one.
#[tokio::test]
async fn a_queue_that_answers_nothing_is_an_error() {
    let harness = Harness::new().await;
    for route in [
        "/v3/queues/NOPE/macros",
        "/v3/queues/NOPE/autoactions",
        "/v3/queues/NOPE/triggers",
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(404))
            .mount(&harness.server)
            .await;
    }

    harness
        .run(&["queue", "automation", "NOPE"])
        .assert()
        .code(4);
}

/// A local field belongs to its queue: `field list` cannot see it and
/// `field get` cannot fetch it. So this listing carries what each one accepts,
/// because no other command could.
#[tokio::test]
async fn local_fields_say_what_they_accept() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/localFields"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queue_local_fields.json")))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["queue", "local-fields", "PROJ"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    // The key a caller types, not the prefixed id the payload carries.
    assert!(stdout.contains("rollout"), "{stdout}");
    assert!(!stdout.contains("6054ae3a"), "{stdout}");
    assert!(stdout.contains("canary, partial, full"), "{stdout}");
    // A provider that keeps its values elsewhere names where they are instead.
    assert!(stdout.contains("ytcli user list"), "{stdout}");
    assert!(stdout.ends_with("shown 2 of 2 for PROJ\n"), "{stdout}");
}

/// Two endpoints, two different answers. `permissions` names roles, `access`
/// names people, and collapsing them into one table would lose the distinction
/// the command exists to show.
#[tokio::test]
async fn access_separates_the_rule_from_the_people_it_comes_out_as() {
    let harness = Harness::new().await;
    for (route, body) in [
        ("/v3/queues/PROJ/permissions", "queue_permissions.json"),
        ("/v3/queues/PROJ/access", "queue_access.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture(body)))
            .mount(&harness.server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("user.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "access", "PROJ"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    // Roles belong to the rule; only the resolved side lists writeNoAssign.
    assert!(stdout.contains("queue-lead"), "{stdout}");
    assert!(stdout.contains("writeNoAssign"), "{stdout}");
    // A group holds a right the way a role does, so it shares that column.
    assert!(stdout.contains("group:Storage team"), "{stdout}");
    // Counted before truncated: a cell cut off mid-name must not hide how many.
    assert!(stdout.contains("4: Ada Lovelace"), "{stdout}");
    assert!(stdout.contains("shown 4 of 4 for PROJ"), "{stdout}");
    assert!(stdout.contains("shown 5 of 5 for PROJ"), "{stdout}");
    // Without this line, a caller in none of the user lists reads the table as
    // "locked out" while being the assignee of every issue they care about.
    assert!(stdout.contains("decided per issue"), "{stdout}");
}

/// The question behind every 403: not whether you were refused, but whether you
/// are one of the people who would not be.
#[tokio::test]
async fn access_says_which_operations_are_yours() {
    let harness = Harness::new().await;
    for (route, body) in [
        ("/v3/queues/PROJ/permissions", "queue_permissions.json"),
        ("/v3/queues/PROJ/access", "queue_access.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture(body)))
            .mount(&harness.server)
            .await;
    }
    // Alan Turing: in create, read and writeNoAssign, and in neither write nor
    // grant.
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "uid": 1_120_000_000_000_003i64,
            "login": "aturing",
            "display": "Alan Turing"
        })))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "access", "PROJ"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    // Only the second table has a `YOU` column, and both tables name the same
    // operations — so the first `grant` line is the wrong one to read.
    let resolved = stdout
        .split("\naccess\n")
        .nth(1)
        .expect("an access section");
    let yours = |operation: &str| {
        resolved
            .lines()
            .find(|line| line.starts_with(operation))
            .unwrap_or_default()
            .to_owned()
    };
    assert!(yours("writeNoAssign").contains("yes"), "{stdout}");
    assert!(yours("grant").contains("no"), "{stdout}");
}

/// `?` is not `no`. A caller whose own user could not be read has not been told
/// they are excluded, and saying so would be the mistake this command is against.
#[tokio::test]
async fn access_admits_when_it_does_not_know_who_you_are() {
    let harness = Harness::new().await;
    for (route, body) in [
        ("/v3/queues/PROJ/permissions", "queue_permissions.json"),
        ("/v3/queues/PROJ/access", "queue_access.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture(body)))
            .mount(&harness.server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({"errors": {}})))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "access", "PROJ"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains('?'), "{stdout}");
    assert!(!stdout.contains("no"), "{stdout}");
}

/// Reading queue rights is itself a right. One section refused leaves the other
/// standing, and says why rather than tallying zero.
#[tokio::test]
async fn a_refused_access_section_says_so_instead_of_counting_zero() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/permissions"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({"errors": {}})))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/queues/PROJ/access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("queue_access.json")))
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("user.json")))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["queue", "access", "PROJ"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("those who may see queue rights"),
        "{stdout}"
    );
    assert!(stdout.contains("shown 5 of 5 for PROJ"), "{stdout}");
    assert!(!stdout.contains("shown 0 of 0"), "{stdout}");
}

/// Both refused is not a partial answer, and a queue that is not there is
/// reported as the queue rather than as the first endpoint tried.
#[tokio::test]
async fn a_queue_with_no_readable_rights_is_an_error() {
    let harness = Harness::new().await;
    for route in ["/v3/queues/NOPE/permissions", "/v3/queues/NOPE/access"] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(404))
            .mount(&harness.server)
            .await;
    }

    let output = harness.run(&["queue", "access", "NOPE"]).assert().code(4);
    let stderr = String::from_utf8(output.get_output().stderr.clone()).expect("utf-8");
    assert!(stderr.contains("queue NOPE not found"), "{stderr}");
}
