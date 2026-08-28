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
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

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

/// The entity endpoints accept exactly the field list we ask for.
///
/// This is the bug that shipped between two commits: `entityType` reads like a
/// field, comes back in every payload, and is not one — asking for it makes
/// Tracker refuse the whole search with 422, for projects, portfolios and goals
/// alike. Fixtures answer whatever they are asked, so only a real organisation
/// can say which names are real.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn every_entity_kind_accepts_the_fields_we_ask_for() {
    let client = client();
    for kind in ["project", "portfolio", "goal"] {
        client
            .entities(kind, None, 1, 5)
            .await
            .unwrap_or_else(|error| panic!("{kind} search rejected: {error}"));
    }
}

/// Boards, and the columns every board command prints.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn boards_parse_and_keep_their_columns() {
    let boards = client().boards().await.expect("boards");
    for board in &boards {
        assert!(!board.id.is_empty(), "a board with no id");
        assert!(
            !board.columns.is_empty(),
            "board {} has no columns, which no board has",
            board.id
        );
    }
}

/// A board that cannot have sprints is refused, not answered with an empty
/// list. Both outcomes are correct; what must not happen is a decode failure.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn asking_a_board_for_its_sprints_either_answers_or_is_refused() {
    let client = client();
    let Some(board) = client.boards().await.expect("boards").first().cloned() else {
        return;
    };

    match client.sprints(&board.id).await {
        Ok(sprints) => {
            for sprint in &sprints {
                assert!(!sprint.id.is_empty(), "a sprint with no id");
            }
        }
        Err(error) => {
            let said = error.to_string();
            assert!(
                said.contains("400") || said.contains("404"),
                "unexpected failure: {said}"
            );
        }
    }
}

/// The organisation-wide listings, through our parsers.
///
/// Templates are the half fixtures cannot vouch for: the organisation this runs
/// against has none, so the shape of a template is believed rather than known.
/// If one ever exists, a nameless template fails here rather than printing a
/// dash and looking like an empty template.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn fields_and_templates_parse() {
    let client = client();

    let fields = client.fields().await.expect("fields");
    assert!(!fields.is_empty(), "an organisation with no fields");
    for field in &fields {
        assert!(!field.key.is_empty(), "a field with no key: {field:?}");
    }

    for kind in [
        ytcli::api::TemplateKind::Issue,
        ytcli::api::TemplateKind::Comment,
    ] {
        for template in &client.templates(kind).await.expect("templates") {
            assert!(
                !template.name.is_empty(),
                "{kind:?}: a template with no name — the payload does not call it `name`"
            );
        }
    }
}

/// Containment, written and read back, against a portfolio this test makes.
///
/// Entities can be deleted, unlike issues, so this cleans up after itself and
/// runs on the same opt-in as the issue write below. It is here because the
/// shape of a containment write cannot be checked any other way: a read answers
/// `parentEntity.primary` as an object, a write takes it as a string, and a
/// mock can only agree with whichever we believed on the day.
#[tokio::test]
#[ignore = "creates and deletes real entities; needs YTCLI_TEST_QUEUE"]
async fn a_project_can_be_put_in_a_portfolio_and_taken_out() {
    if setting("YTCLI_TEST_QUEUE").is_none() {
        return;
    }
    let client = client();

    let portfolio = client
        .create_entity(
            "portfolio",
            &serde_json::json!({"summary": "ytcli live test — deleted by this test"}),
        )
        .await
        .expect("create portfolio");
    let project = client
        .create_entity(
            "project",
            &serde_json::json!({"summary": "ytcli live test — deleted by this test"}),
        )
        .await
        .expect("create project");

    let placed = client
        .place_entity("project", &project.id, Some(&portfolio.id), project.version)
        .await
        .expect("place");
    assert_eq!(
        placed.parent.as_deref(),
        Some(portfolio.id.as_str()),
        "the write did not come back as the read we believe in"
    );

    let removed = client
        .place_entity("project", &project.id, None, placed.version)
        .await
        .expect("remove");
    assert!(
        removed.parent.is_none(),
        "the parent survived being cleared"
    );

    client
        .delete_entity("project", &project.id)
        .await
        .expect("delete project");
    client
        .delete_entity("portfolio", &portfolio.id)
        .await
        .expect("delete portfolio");
}

/// A queue can be copied from, which is what `queue create --like` depends on.
///
/// Reading only. Creating a queue is not something a test suite should do to
/// somebody's organisation: Tracker deletes a queue by hiding it, and the key
/// stays spent. What can be checked without writing is the half that breaks —
/// that `expand=all` still answers with issue types, workflows and resolutions
/// under the names the create endpoint takes back.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn a_queue_can_still_be_used_as_a_blueprint() {
    let client = client();
    let queue = a_queue(&client).await;

    let blueprint = client.queue_blueprint(&queue).await.expect("blueprint");
    assert!(
        blueprint.default_type.is_some(),
        "{queue} has no default type"
    );
    for config in &blueprint.issue_types {
        assert!(
            config.get("workflow").and_then(|w| w.as_str()).is_some(),
            "an issue type with no workflow id: {config}"
        );
        assert!(
            config.get("issueType").and_then(|t| t.as_str()).is_some(),
            "an issue type with no key: {config}"
        );
    }
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

/// The four dictionaries, through our parser.
///
/// The claim worth checking against a real organisation is not that the request
/// succeeds but that `key` and `name` are genuinely different things: the whole
/// reason to print both is that a localised organisation answers `Ошибка` for
/// the type whose key is `bug`, and fixtures we wrote cannot prove that.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn every_dictionary_answers_with_keys() {
    let client = client();
    for kind in ytcli::api::Dictionary::ALL {
        let entries = client.dictionary(kind).await.expect("dictionary");
        assert!(!entries.is_empty(), "{} is empty", kind.label());
        for entry in &entries {
            assert!(!entry.key.is_empty(), "an entry with no key: {entry:?}");
        }
    }

    let statuses = client
        .dictionary(ytcli::api::Dictionary::Statuses)
        .await
        .expect("statuses");
    assert!(
        statuses.iter().any(|status| status.category.is_some()),
        "no status carried a category, which is the column that makes the list readable"
    );
}

/// The directory pages, and reports how many people it has.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn the_directory_pages_and_reports_its_size() {
    let page = client().users(1, 3).await.expect("users");
    assert!(!page.items.is_empty(), "an organisation with nobody in it");
    assert!(
        page.items.len() <= 3,
        "perPage was ignored: {} came back",
        page.items.len()
    );
    for person in &page.items {
        assert!(
            !person.login.is_empty(),
            "a person with no login: {person:?}"
        );
        assert!(!person.uid.is_empty(), "a person with no uid: {person:?}");
    }
}

/// A login taken from the directory can be fetched back by that login.
///
/// `/v3/users/{login}` and `/v3/users` are different endpoints, and this is the
/// round trip every `--assignee` depends on.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn a_login_from_the_listing_can_be_fetched_back() {
    let client = client();
    let page = client.users(1, 1).await.expect("users");
    let Some(first) = page.items.first() else {
        return;
    };

    let fetched = client.user(&first.login).await.expect("user");
    assert_eq!(fetched.login, first.login);
}

/// The organisation-wide worklog search, and the parameter that is easy to get
/// wrong: `createdBy` takes a login, and answers 422 for `me`. The CLI resolves
/// `me` before searching precisely because of this, so the belief is worth
/// checking against the real API rather than against our own mock.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn the_worklog_search_takes_a_login_and_not_me() {
    let client = client();
    let me = client.myself().await.expect("myself");
    let login = me.login.clone().unwrap_or(me.id.clone());

    // An empty result is fine: what is being checked is that the request is
    // accepted at all, and with a date range attached.
    client
        .worklog_search(Some(&login), Some("2020-01-01"), None, 5)
        .await
        .expect("worklog search by login");

    let refused = client.worklog_search(Some("me"), None, None, 5).await;
    assert!(
        refused.is_err(),
        "Tracker accepted `me` as a login; the CLI resolves it for nothing"
    );
}

/// The full life of an entity: created, changed, read back, deleted.
///
/// Safe to run for real because entities can be deleted — the asymmetry that
/// keeps the issue writes out of this suite — and the test cleans up after
/// itself even when the assertions in the middle fail.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn a_project_can_be_created_changed_and_deleted() {
    let client = client();
    let created = client
        .create_entity(
            "project",
            &serde_json::json!({"summary": "ytcli live test — deleted by this test"}),
        )
        .await
        .expect("create");

    let changed = client
        .update_entity(
            "project",
            &created.id,
            &serde_json::json!({"summary": "ytcli live test — renamed"}),
            created.version,
        )
        .await;

    let read_back = client.entity("project", &created.id).await;

    client
        .delete_entity("project", &created.id)
        .await
        .expect("delete");

    let changed = changed.expect("update");
    assert_eq!(changed.summary, "ytcli live test — renamed");
    assert_eq!(
        read_back.expect("read back").summary,
        "ytcli live test — renamed",
        "the rename did not survive being read through a different endpoint"
    );
}

/// Every query the skill teaches, sent to a real Tracker.
///
/// The stub-backed test proves the query survives the trip to the request body;
/// only this one proves the language is real. A filter name Tracker does not
/// know is a 422 naming it, which is exactly what an invented example looks
/// like — and inventing plausible syntax is the failure mode of writing
/// documentation about a language from memory.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn every_query_the_skill_teaches_is_accepted() {
    let client = client();
    let queue = a_queue(&client).await;
    let text =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/ytcli/yql.md"))
            .expect("yql.md");

    let queries: Vec<String> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("ytcli issue find --yql "))
        // The page is written against the queue this was developed in; any
        // organisation has a first queue, and the names being tested are the
        // filters rather than the queue.
        .map(|rest| {
            rest.trim()
                .trim_matches('\'')
                .replace("Queue: TRACKER", &format!("Queue: {queue}"))
        })
        .collect();
    assert!(queries.len() >= 10, "found {} queries", queries.len());

    let mut refused = Vec::new();
    for query in queries {
        if client.count(&query).await.is_err() {
            refused.push(query);
        }
    }

    assert!(
        refused.is_empty(),
        "Tracker refused queries the skill teaches: {refused:#?}"
    );
}
