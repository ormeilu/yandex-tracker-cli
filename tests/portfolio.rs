//! Portfolios, against a stub Tracker.
//!
//! A portfolio is one more entity type, so the interesting part is not the
//! listing: it is containment. Tracker types its endpoints and does not type
//! containment, so what one command answers takes two requests, and the tally
//! has to stay true across both.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

mod harness;
use harness::Harness;

fn portfolio(id: &str, short: i64, summary: &str, parent: Option<&str>) -> serde_json::Value {
    entity("portfolio", id, short, summary, parent)
}

fn entity(
    kind: &str,
    id: &str,
    short: i64,
    summary: &str,
    parent: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "shortId": short,
        "entityType": kind,
        "fields": {
            "summary": summary,
            "entityStatus": "in_progress",
            "parentEntity": parent,
        }
    })
}

async fn search(harness: &Harness, kind: &str, body: serde_json::Value, values: serde_json::Value) {
    let hits = values.as_array().map_or(0, Vec::len);
    Mock::given(method("POST"))
        .and(path(format!("/v3/entities/{kind}/_search")))
        .and(body_json(body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"hits": hits, "pages": 1, "values": values})),
        )
        .mount(&harness.server)
        .await;
}

#[tokio::test]
async fn portfolios_list_like_every_other_entity() {
    let harness = Harness::new().await;
    search(
        &harness,
        "portfolio",
        serde_json::json!({}),
        serde_json::json!([portfolio("aaa", 1, "Platform", None)]),
    )
    .await;

    let output = harness.run(&["portfolio", "list"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Platform"));
    assert!(stdout.contains("aaa"));
    assert!(stdout.ends_with("shown 1 of 1\n"));
}

/// The id of the portfolio a thing sits in is the one fact a caller cannot get
/// from anywhere else in the output.
#[tokio::test]
async fn a_portfolio_says_which_portfolio_it_sits_in() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/entities/portfolio/bbb"))
        .respond_with(ResponseTemplate::new(200).set_body_json(portfolio(
            "bbb",
            2,
            "Payments",
            Some("aaa"),
        )))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["portfolio", "get", "bbb"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Payments"));
    assert!(stdout.contains("in portfolio: aaa"));
}

/// Writes send `{"primary": id}`, and reads have been seen to answer with the
/// bare id. Either shape has to arrive as the same id.
#[tokio::test]
async fn a_parent_is_read_whichever_shape_it_arrives_in() {
    let harness = Harness::new().await;
    let mut body = portfolio("ccc", 3, "Billing", None);
    body["fields"]["parentEntity"] = serde_json::json!({"primary": "aaa", "secondary": []});
    Mock::given(method("GET"))
        .and(path("/v3/entities/portfolio/ccc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;

    let output = harness.run(&["portfolio", "get", "ccc"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("in portfolio: aaa"), "{stdout}");
}

/// Both types, one listing, and a count that adds up.
#[tokio::test]
async fn contents_asks_for_both_types_and_counts_them_together() {
    let harness = Harness::new().await;
    let filter = serde_json::json!({"filter": {"parentEntity": "aaa"}});
    search(
        &harness,
        "portfolio",
        filter.clone(),
        serde_json::json!([portfolio("bbb", 2, "Payments", Some("aaa"))]),
    )
    .await;
    search(
        &harness,
        "project",
        filter,
        serde_json::json!([
            entity("project", "p1", 10, "Card capture", Some("aaa")),
            entity("project", "p2", 11, "Refunds", Some("aaa")),
        ]),
    )
    .await;

    let output = harness
        .run(&["portfolio", "contents", "aaa"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("Payments"));
    assert!(stdout.contains("Card capture"));
    assert!(stdout.contains("Refunds"));
    // The type is what says which `get` reads a row back.
    assert!(stdout.contains("portfolio"));
    assert!(stdout.contains("project"));
    assert!(stdout.ends_with("shown 3 of 3\n"), "{stdout}");
}

/// Reading a portfolio must not cost the requests that answer a different
/// question: `get` is one request, containment is the command that asks twice.
#[tokio::test]
async fn get_does_not_pay_for_the_contents_nobody_asked_for() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/entities/portfolio/bbb"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(portfolio("bbb", 2, "Payments", None)),
        )
        .mount(&harness.server)
        .await;

    harness.run(&["portfolio", "get", "bbb"]).assert().success();

    let requests = harness.server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 1, "{requests:?}");
}

/// The write quotes the version it just read, so a portfolio somebody else
/// moved in between is Tracker's problem to refuse rather than ours to
/// overwrite.
#[tokio::test]
async fn placing_reads_the_version_first_and_sends_it_back() {
    let harness = Harness::new().await;
    let mut body = entity("project", "p1", 10, "Card capture", None);
    body["version"] = serde_json::json!(7);
    Mock::given(method("GET"))
        .and(path("/v3/entities/project/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
        .mount(&harness.server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v3/entities/project/p1"))
        .and(query_param("version", "7"))
        .and(body_json(serde_json::json!({
            "fields": {"parentEntity": {"primary": "aaa"}}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(entity(
            "project",
            "p1",
            10,
            "Card capture",
            Some("aaa"),
        )))
        .mount(&harness.server)
        .await;

    let output = harness
        .run(&["project", "place", "p1", "--into", "aaa"])
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("in portfolio: aaa"), "{stdout}");
}

/// Taking something out sends `null`, not an empty object: an empty object is a
/// change Tracker accepts and ignores, which reads as success and is not.
#[tokio::test]
async fn taking_an_entity_out_sends_a_null_parent() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/entities/portfolio/bbb"))
        .respond_with(ResponseTemplate::new(200).set_body_json(portfolio(
            "bbb",
            2,
            "Payments",
            Some("aaa"),
        )))
        .mount(&harness.server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v3/entities/portfolio/bbb"))
        .and(body_json(
            serde_json::json!({"fields": {"parentEntity": serde_json::Value::Null}}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(portfolio("bbb", 2, "Payments", None)),
        )
        .mount(&harness.server)
        .await;

    harness
        .run(&["portfolio", "place", "bbb", "--out"])
        .assert()
        .success();
}

/// Neither `--into` nor `--out` is a question, not a default: silently doing
/// one of the two would be the one outcome nobody asked for.
#[tokio::test]
async fn placing_nowhere_is_refused_before_anything_is_read() {
    let harness = Harness::new().await;

    harness
        .run(&["project", "place", "p1"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--into"));

    let requests = harness.server.received_requests().await.unwrap_or_default();
    assert!(requests.is_empty(), "{requests:?}");
}

/// A dry run reads, so it can fail on a bad id, and writes nothing.
#[tokio::test]
async fn a_dry_run_places_nothing() {
    let harness = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/v3/entities/project/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(entity(
            "project",
            "p1",
            10,
            "Card capture",
            None,
        )))
        .mount(&harness.server)
        .await;

    harness
        .run(&["project", "place", "p1", "--into", "aaa", "--dry-run"])
        .assert()
        .success();

    let requests = harness.server.received_requests().await.unwrap_or_default();
    assert!(
        requests.iter().all(|request| request.method == "GET"),
        "a dry run wrote something: {requests:?}"
    );
}

/// A version that moved on is a 412, and Tracker's own sentence explains it.
#[tokio::test]
async fn a_stale_version_is_reported_not_retried() {
    let harness = Harness::new().await;
    let mut body = entity("project", "p1", 10, "Card capture", None);
    body["version"] = serde_json::json!(1);
    Mock::given(method("GET"))
        .and(path("/v3/entities/project/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&harness.server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v3/entities/project/p1"))
        .respond_with(ResponseTemplate::new(412).set_body_json(serde_json::json!({
            "errorMessages": ["Could not save the change, try again."],
            "statusCode": 412
        })))
        .expect(1)
        .mount(&harness.server)
        .await;

    harness
        .run(&["project", "place", "p1", "--into", "aaa"])
        .assert()
        .code(5)
        .stderr(predicates::str::contains("try again"));
}
