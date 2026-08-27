//! Tests against a real Tracker organisation.
//!
//! Every other test in this repository runs against `wiremock` and fixtures we
//! wrote, which means they prove the code agrees with our beliefs about the API.
//! Three bugs shipped past that suite because the beliefs were wrong: link types
//! come back in the organisation's language, every issue carries bookkeeping
//! counters that looked like custom fields, and dates arrive with a `+0300`
//! offset no RFC 3339 parser accepts. Fixtures cannot catch that class of
//! mistake, because they are the mistake.
//!
//! So this suite is deliberately not more of the same. It asks the questions
//! only a real organisation can answer: does the payload still have the shape
//! our fixtures claim, and does what we parse survive a round trip.
//!
//! It is behind the `live` feature and `#[ignore]`, needs credentials, and never
//! runs in CI. One test at a time: Tracker rate-limits, and a suite that fails
//! on its own concurrency teaches nothing.
//!
//! ```sh
//! just test-live
//! ```
//!
//! Reads are unconditional. Writes happen only when `YTCLI_TEST_QUEUE` names a
//! queue you are willing to have issues created in — Tracker has no delete, so
//! whatever a write test makes is permanent.

#![cfg(feature = "live")]
// A test that cannot reach its fixture has nothing to say; failing loudly is the
// correct behaviour.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::path::Path;

use ytcli::api::{Client, ClientConfig};
use ytcli::config::OrgKind;

/// Credentials, from the environment or from `.env` beside the manifest.
///
/// `.env` is read rather than exported into the process: setting environment
/// variables at runtime is unsound in the 2024 edition, and a test that mutates
/// global state for its neighbours deserves what it gets.
fn setting(name: &str) -> Option<String> {
    if let Ok(value) = std::env::var(name)
        && !value.is_empty()
    {
        return Some(value);
    }

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
}

/// The client the CLI itself would build.
///
/// Deliberately through the production path — the configured profile for the
/// organisation and its header flavour, `secrets::token` for the credential —
/// because "does my own configuration work" is one of the questions this suite
/// exists to answer. `YTCLI_TOKEN` and `YTCLI_ORG_ID` still win where they are
/// set, which is how CI would run this if it ever did.
fn client() -> Client {
    let (org, kind, account) = organisation();
    let token = setting("YTCLI_TOKEN")
        .or_else(|| account.and_then(|account| ytcli::secrets::token(&account).ok()))
        .expect("no token: set YTCLI_TOKEN, or run `ytcli auth login`");

    Client::new(&ClientConfig::new(token, org, kind)).expect("client")
}

/// The organisation to talk to: the environment, else the active profile.
fn organisation() -> (String, OrgKind, Option<String>) {
    if let Some(org) = setting("YTCLI_ORG_ID") {
        let kind = match setting("YTCLI_ORG_KIND").as_deref() {
            Some("cloud") => OrgKind::Cloud,
            _ => OrgKind::Yandex360,
        };
        return (org, kind, None);
    }

    let file = ytcli::config::paths::config_file().expect("config path");
    let config = ytcli::config::Config::load(&file).expect("config");
    let here = std::env::current_dir().expect("cwd");
    let resolved = config
        .resolve(None, None, &here)
        .expect("no profile: set YTCLI_ORG_ID, or run `ytcli auth login`");

    (
        resolved.profile.org_id.clone(),
        resolved.profile.org_kind,
        Some(resolved.profile.account.clone()),
    )
}

/// A queue to read from: whichever one the organisation lists first, unless one
/// was named.
async fn a_queue(client: &Client) -> String {
    if let Some(queue) = setting("YTCLI_TEST_QUEUE") {
        return queue;
    }
    let queues = client.queues().await.expect("queues");
    queues
        .first()
        .expect("the organisation has no queues")
        .key
        .clone()
}

fn keys(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "needs real credentials"]
async fn the_token_belongs_to_somebody() {
    let user = client().myself().await.expect("myself");
    assert!(!user.id.is_empty(), "a user with no id");
}

/// Every queue the organisation has, through our parser. A key we cannot read is
/// a queue no command can address.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn every_queue_parses_and_has_a_key() {
    let queues = client().queues().await.expect("queues");
    assert!(!queues.is_empty(), "no queues to test against");
    for queue in &queues {
        assert!(!queue.key.is_empty(), "a queue with no key: {queue:?}");
    }
}

/// The round trip a caller actually makes: find something, then fetch it.
///
/// Both halves parse the same payload shape through different endpoints, and
/// they have disagreed before.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn an_issue_found_by_search_can_be_fetched() {
    let client = client();
    let queue = a_queue(&client).await;

    let page = client
        .search(&format!("Queue: {queue}"), 1, 5)
        .await
        .expect("search");
    let Some(first) = page.items.first() else {
        return; // An empty queue proves nothing, and failing on it proves less.
    };

    let (fetched, _) = client.issue(&first.key).await.expect("issue");
    assert_eq!(fetched.key, first.key);
    assert!(!fetched.summary.is_empty(), "an issue with no summary");
}

/// Links carry a type, in any language.
///
/// This is the bug that shipped: the relation was read from the human-readable
/// label, which is localised, so a Russian organisation produced links whose
/// type was the word "link".
#[tokio::test]
#[ignore = "needs real credentials"]
async fn links_are_typed_whatever_language_the_organisation_speaks() {
    let client = client();
    let queue = a_queue(&client).await;
    let page = client
        .search(&format!("Queue: {queue}"), 1, 25)
        .await
        .expect("search");

    for issue in &page.items {
        let links = client.issue_links(&issue.key).await.expect("links");
        for link in &links {
            assert!(
                link.kind != ytcli::api::models::LinkKind::Other || link.relation.is_some(),
                "{}: a link with neither a known type nor Tracker's own wording",
                issue.key
            );
        }
    }
}

/// A real issue carries every field the compact view prints.
///
/// This started out comparing the payload against `tests/fixtures/issue.json`
/// and could never have passed: the fixture has `storyPoints`, `sprint` and a
/// component, and custom fields are per-queue and per-issue, so any other issue
/// legitimately lacks them. A test that cannot go green is worse than no test.
///
/// What is worth asserting is the part that is not per-issue — the system fields
/// every command depends on. Those are a contract with the API, and if one of
/// them disappears or is renamed, everything downstream renders a dash and says
/// nothing about why.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn a_real_issue_has_the_fields_every_command_depends_on() {
    let client = client();
    let queue = a_queue(&client).await;
    let page = client
        .search(&format!("Queue: {queue}"), 1, 1)
        .await
        .expect("search");
    let Some(found) = page.items.first() else {
        return;
    };

    let (issue, raw) = client.issue(&found.key).await.expect("issue");

    for field in [
        "key",
        "summary",
        "status",
        "queue",
        "createdAt",
        "updatedAt",
    ] {
        assert!(
            keys(&raw).contains(field),
            "the API no longer returns `{field}`"
        );
    }

    // And the parser got them out again, which is the half a key-set check
    // cannot see.
    assert!(!issue.key.is_empty());
    assert!(!issue.summary.is_empty());
    assert!(issue.status.is_some(), "status did not parse");
    assert!(issue.queue.is_some(), "queue did not parse");
    assert!(
        issue.updated_at.is_some(),
        "updatedAt did not parse — Tracker sends `+0300`, not `+03:00`"
    );
}

/// Writing, only into a queue somebody named on purpose.
///
/// Tracker has no delete. Whatever this creates stays, so it is opt-in twice
/// over: the feature, and the variable.
#[tokio::test]
#[ignore = "creates a real issue; needs YTCLI_TEST_QUEUE"]
async fn a_created_issue_can_be_read_back_and_commented_on() {
    let Some(queue) = setting("YTCLI_TEST_QUEUE") else {
        return;
    };
    let client = client();

    let created = client
        .create_issue(&serde_json::json!({
            "queue": queue,
            "summary": "ytcli live test — safe to close",
            "description": "Created by the ytcli live suite.",
        }))
        .await
        .expect("create");

    let (fetched, _) = client.issue(&created.key).await.expect("read back");
    assert_eq!(fetched.summary, "ytcli live test — safe to close");

    client
        .add_comment(&created.key, "ytcli live test comment")
        .await
        .expect("comment");
    let comments = client.issue_comments(&created.key).await.expect("comments");
    assert!(
        comments
            .iter()
            .any(|c| c.text.contains("live test comment")),
        "the comment did not come back"
    );
}
