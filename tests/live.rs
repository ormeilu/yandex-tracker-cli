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
// The fixture audit prints what it could *not* check: an organisation with no
// boards leaves the board fixtures unverified, and that is a fact about the run
// rather than a success. A test's own report belongs on stdout.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stdout
)]

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
    let from_account = || {
        account
            .as_deref()
            .and_then(|account| ytcli::secrets::token(account).ok())
    };

    // Naming a profile means naming the identity that goes with it. Otherwise
    // `YTCLI_TOKEN` — which is not per profile, and which this suite also reads
    // out of `.env` — would be sent to somebody else's organisation, and a 401
    // is the friendly version of what that could do.
    let token = if setting("YTCLI_PROFILE").is_some() {
        from_account().or_else(|| setting("YTCLI_TOKEN"))
    } else {
        setting("YTCLI_TOKEN").or_else(from_account)
    }
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
    // `YTCLI_PROFILE` has to be passed in: resolution does not read the
    // environment on its own. Leaving it out silently ran this suite — writes
    // included — against the default profile while it had been told another
    // one, which is exactly the mistake every command's profile line exists to
    // prevent.
    let resolved = config
        .resolve(None, setting("YTCLI_PROFILE").as_deref(), &here)
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

/// The direction of a link, read from both ends.
///
/// One end proves nothing: an inverted mapping is perfectly self-consistent
/// when you only ever look at one side of the link, which is why `depends` was
/// rendered backwards for months with a passing test suite behind it. So this
/// writes a link whose meaning is known — the first issue depends on the second
/// — and insists that each end says the opposite thing about the other.
///
/// The link is removed afterwards; the two issues stay, as everything this
/// suite creates does.
#[tokio::test]
#[ignore = "creates real issues; needs YTCLI_TEST_QUEUE"]
async fn a_link_reads_the_same_way_from_both_ends() {
    let Some(queue) = setting("YTCLI_TEST_QUEUE") else {
        return;
    };
    let client = client();

    let make = |summary: &'static str| {
        let queue = queue.clone();
        let client = &client;
        async move {
            client
                .create_issue(&serde_json::json!({
                    "queue": queue,
                    "summary": summary,
                }))
                .await
                .expect("create")
        }
    };
    let dependent = make("ytcli live test — depends on the other").await;
    let blocker = make("ytcli live test — blocks the other").await;

    client
        .add_link(&dependent.key, "depends on", &blocker.key)
        .await
        .expect("link");

    let from_dependent = client.issue_links(&dependent.key).await.expect("links");
    let from_blocker = client.issue_links(&blocker.key).await.expect("links");

    let one = from_dependent
        .first()
        .expect("no link on the dependent end");
    let other = from_blocker.first().expect("no link on the blocking end");

    assert_eq!(
        one.kind,
        ytcli::api::models::LinkKind::Depends,
        "{} depends on {}, and its own listing says otherwise",
        dependent.key,
        blocker.key
    );
    assert_eq!(
        other.kind,
        ytcli::api::models::LinkKind::IsDependentBy,
        "{} is depended on, and its own listing says otherwise",
        blocker.key
    );

    // The id the help promises `issue link delete` can be given.
    assert!(!one.id.is_empty(), "a link with no id: {one:?}");
    client
        .delete_link(&dependent.key, &one.id)
        .await
        .expect("unlink");
}

/// The tags an issue carries, straight from the payload.
fn tags_of(raw: &serde_json::Value) -> Vec<String> {
    raw.get("tags")
        .and_then(serde_json::Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// What a bulk change refuses, which is the half the safety of it rests on.
///
/// Neither call writes anything: both are refusals, and that is the point.
/// Tracker requires the keys — a query is not accepted — so a bulk change
/// touches exactly what the caller named and the confirmation that names them
/// is the whole story. And a list containing one key that does not exist is
/// refused entire, rather than applied up to the bad one.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn a_bulk_change_takes_keys_and_refuses_a_list_it_cannot_do_whole() {
    let client = client();
    let queue = a_queue(&client).await;
    let page = client
        .search(&format!("Queue: {queue}"), 1, 1)
        .await
        .expect("search");
    let Some(issue) = page.items.first() else {
        return;
    };

    // A key that cannot exist beside one that does. Refused, and nothing on the
    // real one is touched — which is what the issue-at-a-time path cannot say.
    let missing = format!("{queue}-99999999");
    let error = client
        .bulk_update(
            &[issue.key.clone(), missing.clone()],
            &serde_json::json!({"tags": {"add": ["ytcli-live-should-not-happen"]}}),
        )
        .await
        .expect_err("a change naming an issue that does not exist was accepted");
    assert!(
        error.to_string().contains(&missing),
        "the refusal does not name the key it refused: {error}"
    );

    // Read from the raw payload: `tags` is not a field the compact view keeps,
    // and the question here is what Tracker holds, not what we render.
    let (_, raw) = client.issue(&issue.key).await.expect("issue");
    assert!(
        !tags_of(&raw).contains(&"ytcli-live-should-not-happen".to_owned()),
        "{}: a refused bulk change wrote a tag anyway",
        issue.key
    );
}

/// The whole round trip: one request, polled to an outcome, and a tally that
/// matches what was asked for.
///
/// The two issues stay, as everything this suite creates does; the tag it adds
/// is taken off again.
#[tokio::test]
#[ignore = "creates real issues; needs YTCLI_TEST_QUEUE"]
async fn a_bulk_change_finishes_and_counts_what_it_changed() {
    const TAG: &str = "ytcli-live-bulk";

    let Some(queue) = setting("YTCLI_TEST_QUEUE") else {
        return;
    };
    let client = client();

    let mut keys = Vec::new();
    for summary in [
        "ytcli live test — bulk change, one of two",
        "ytcli live test — bulk change, two of two",
    ] {
        let issue = client
            .create_issue(&serde_json::json!({"queue": queue, "summary": summary}))
            .await
            .expect("create");
        keys.push(issue.key);
    }

    let mut change = client
        .bulk_update(&keys, &serde_json::json!({"tags": {"add": [TAG]}}))
        .await
        .expect("bulk update");

    // Tracker answers with an operation, not a result. Waiting for it is what
    // makes an exit code mean anything.
    for _ in 0..60 {
        if change.finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        change = client.bulk_change(&change.id).await.expect("bulk status");
    }

    assert!(change.finished(), "{} never finished", change.id);
    assert_eq!(change.total, Some(keys.len() as u64), "{change:?}");
    assert_eq!(change.done, change.total, "{change:?}");
    assert!(change.succeeded(), "{change:?}");

    for key in &keys {
        let (_, raw) = client.issue(key).await.expect("issue");
        assert!(
            tags_of(&raw).iter().any(|tag| tag == TAG),
            "{key}: the change said it worked and the issue disagrees"
        );
    }

    client
        .bulk_update(&keys, &serde_json::json!({"tags": {"remove": [TAG]}}))
        .await
        .expect("undo");
}

/// What the other two bulk endpoints refuse, which is what makes a list of keys
/// safe to offer at all.
///
/// Nothing here writes. `_transition` is given a list holding one key that
/// cannot exist and refused entire — the real issue in the list keeps the status
/// it had, which is the promise a loop over the single-issue endpoint could
/// never make. `_move` is given a queue that does not exist and refused for that
/// reason alone, so the keys it names are still the keys they were.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn bulk_transition_and_move_refuse_a_list_they_cannot_do_whole() {
    let client = client();
    let queue = a_queue(&client).await;
    let page = client
        .search(&format!("Queue: {queue}"), 1, 1)
        .await
        .expect("search");
    let Some(issue) = page.items.first() else {
        return;
    };
    let before = client.issue(&issue.key).await.expect("issue").0;

    let missing = format!("{queue}-99999999");
    let refused = client
        .bulk_transition(
            &[issue.key.clone(), missing.clone()],
            "close",
            &serde_json::json!({}),
        )
        .await
        .expect_err("a transition naming an issue that does not exist was accepted");
    assert!(
        refused.to_string().contains(&missing),
        "the refusal does not name the key it refused: {refused}"
    );

    let refused = client
        .bulk_move(
            std::slice::from_ref(&issue.key),
            "NOSUCHQUEUE0",
            false,
            false,
        )
        .await
        .expect_err("a move into a queue that does not exist was accepted");
    assert!(
        refused.to_string().contains("NOSUCHQUEUE0"),
        "the refusal does not name the queue it refused: {refused}"
    );

    let after = client.issue(&issue.key).await.expect("issue").0;
    assert_eq!(
        before.status_key, after.status_key,
        "{}: a refused bulk transition moved the issue anyway",
        issue.key
    );
    assert_eq!(
        before.key, after.key,
        "a refused bulk move changed a key anyway"
    );
}

/// What `markupType` decides, asked of Tracker rather than of its reference.
///
/// The question from #77: every write this tool makes omits `markupType`, and
/// the documentation implies that field is what selects Markdown. The issue API
/// cannot answer it — the same body written three ways is stored byte-identical
/// and no field says which was used — so this asks the one endpoint that
/// renders on read. `expand=html` returns the drawn comment, and `#` disagrees
/// between the two markups: a heading in Markdown, a numbered list in wiki.
#[tokio::test]
#[ignore = "writes comments; needs YTCLI_TEST_QUEUE"]
async fn markup_type_decides_how_a_comment_is_drawn() {
    // A body the two markups disagree about, in the one place that costs a
    // reader something: `#` numbers a list in wiki markup and heads a section
    // in Markdown.
    const BODY: &str = "# one\n# two\n\n== Heading ==\n\n**md bold** !!wf bold!!\n\n1. first\n- item\n\n[md link](https://example.com) ((https://example.com wf link))\n\n`code`";

    let Some(queue) = setting("YTCLI_TEST_QUEUE") else {
        return;
    };
    let client = client();
    let issue = client
        .create_issue(&serde_json::json!({
            "queue": queue,
            "summary": "ytcli live test — what markupType decides",
        }))
        .await
        .expect("create");

    let path = format!("/v3/issues/{}/comments", issue.key);
    for body in [
        serde_json::json!({ "text": BODY }),
        serde_json::json!({ "text": BODY, "markupType": "md" }),
        serde_json::json!({ "text": BODY, "markupType": "wf" }),
    ] {
        client.probe_post(&path, &body).await.expect("comment");
    }

    let drawn = client
        .probe_get(&format!("{path}?expand=html"))
        .await
        .expect("comments as html");
    let html: Vec<String> = drawn
        .as_array()
        .expect("an array of comments")
        .iter()
        .filter_map(|comment| comment.get("textHtml").and_then(|html| html.as_str()))
        .map(ToOwned::to_owned)
        .collect();
    assert_eq!(
        html.len(),
        3,
        "expand=html returned no rendered text: {drawn}"
    );

    if let Ok(path) = std::env::var("YTCLI_CAPTURE") {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&drawn).unwrap_or_default(),
        )
        .expect("capture");
    }

    // `#` heads a section in both modes. That is the trap #75 hit, and it is
    // not conditional on anything.
    for (mode, rendered) in ["bare", "md", "wf"].iter().zip(&html) {
        assert!(
            rendered.contains("<h1"),
            "{mode}: `#` did not draw as a heading: {rendered}"
        );
    }

    // The answer #77 exists for. A write with no markupType — which is every
    // write this tool makes — is drawn exactly as `wf` is, and `wf` is the
    // permissive mode: it honours the wiki markup *as well as* Markdown.
    assert_eq!(
        html.first(),
        html.get(2),
        "a bare write is no longer drawn the way `wf` is"
    );
    assert_ne!(
        html.first(),
        html.get(1),
        "`md` and the default now draw the same, so the default may have changed"
    );

    // What `md` gives up: the wiki spellings stop being markup and stay text.
    let md = html.get(1).map(String::as_str).unwrap_or_default();
    assert!(md.contains("== Heading =="), "md drew a wiki heading: {md}");
    assert!(md.contains("!!wf bold!!"), "md drew wiki emphasis: {md}");

    // What the default gives: both vocabularies at once.
    let bare = html.first().map(String::as_str).unwrap_or_default();
    assert!(
        bare.contains("<h1 id=\"heading\""),
        "the default stopped honouring a wiki heading: {bare}"
    );
}

/// Every fixture in the repository, against what Tracker actually returns.
///
/// The fixtures were written from the shapes an upstream client documents, not
/// recorded from the API, and the mocked suite is only as true as they are. So
/// this asks the real thing once per endpoint and checks that every key a
/// fixture claims still exists in the answer — the direction that matters, since
/// a field Tracker has added is nothing to us and a field it has removed is a
/// renderer reading `None` forever.
///
/// It reports what it could not check rather than passing quietly: an
/// organisation with no boards leaves the board fixtures unverified, and that is
/// a fact about the run, not a success.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn every_fixture_still_has_the_shape_tracker_answers_with() {
    let client = client();
    let queue = a_queue(&client).await;
    let me = client.myself().await.expect("myself");

    // Every issue in the queue, not one of them: Tracker omits a field that is
    // unset, so a single bare issue would look like a payload that had lost
    // `assignee` and `description` rather than one that never had them.
    let found = client
        .search(&format!("Queue: {queue}"), 1, 25)
        .await
        .map(|page| page.items)
        .unwrap_or_default();
    let issue = found.first().map(|issue| issue.key.clone());
    let mut issue_keys: BTreeSet<String> = BTreeSet::new();
    for one in &found {
        if let Ok((_, raw)) = client.issue(&one.key).await {
            issue_keys.extend(keys(&raw));
        }
    }
    let board = client
        .boards()
        .await
        .ok()
        .and_then(|boards| boards.into_iter().next())
        .map(|board| board.id);

    // (fixture, path) — the fixture is a listing when the payload is an array,
    // and its first element is what carries the keys.
    let mut pairs: Vec<(&str, String)> = vec![
        ("queues.json", "/v3/queues".to_owned()),
        ("queue.json", format!("/v3/queues/{queue}")),
        ("queue_fields.json", format!("/v3/queues/{queue}/fields")),
        (
            "queue_versions.json",
            format!("/v3/queues/{queue}/versions"),
        ),
        (
            "queue_local_fields.json",
            format!("/v3/queues/{queue}/localFields"),
        ),
        ("components.json", "/v3/components".to_owned()),
        ("fields.json", "/v3/fields".to_owned()),
        ("linktypes.json", "/v3/linktypes".to_owned()),
        ("issuetypes.json", "/v3/issuetypes".to_owned()),
        ("priorities.json", "/v3/priorities".to_owned()),
        ("statuses.json", "/v3/statuses".to_owned()),
        ("resolutions.json", "/v3/resolutions".to_owned()),
        ("issue_templates.json", "/v3/issueTemplates".to_owned()),
        ("users.json", "/v3/users?perPage=1".to_owned()),
        ("user.json", format!("/v3/users/{}", me.id)),
        ("all_sprints.json", "/v3/sprints".to_owned()),
        ("boards.json", "/v3/boards".to_owned()),
    ];
    if let Some(key) = &issue {
        pairs.extend([
            ("issue_links.json", format!("/v3/issues/{key}/links")),
            ("issue_comments.json", format!("/v3/issues/{key}/comments")),
            ("changelog.json", format!("/v3/issues/{key}/changelog")),
            (
                "issue_remotelinks.json",
                format!("/v3/issues/{key}/remotelinks"),
            ),
        ]);
    }
    if let Some(id) = &board {
        pairs.push(("board.json", format!("/v3/boards/{id}")));
    }

    let (mut unchecked, mut drift) = (Vec::new(), Vec::new());

    // The issue payload, against every issue in the queue at once: Tracker omits
    // a field that is unset, so one bare issue would look like a payload that
    // had lost `assignee` rather than one that never had it.
    if issue_keys.is_empty() {
        unchecked.push("no issue in the queue: issue.json".to_owned());
    } else if let Some(missing) = fixture_gap("issue.json", &issue_keys) {
        unchecked.push(format!(
            "issue.json claims {missing:?}, which no issue in {queue} sets — \
             unset fields are omitted, so this proves nothing either way"
        ));
    }

    for (fixture, path) in pairs {
        match compare(&client, fixture, &path).await {
            Verdict::Same => {}
            Verdict::Drifted(missing) => {
                drift.push(format!(
                    "{fixture} claims {missing:?}, which {path} no longer returns"
                ));
            }
            Verdict::Unchecked(why) => unchecked.push(format!("{fixture}: {path} {why}")),
        }
    }

    for line in &unchecked {
        println!("unchecked — {line}");
    }
    assert!(
        drift.is_empty(),
        "fixtures have drifted:\n{}",
        drift.join("\n")
    );
}

/// What one endpoint had to say about one fixture.
enum Verdict {
    Same,
    Drifted(Vec<String>),
    Unchecked(&'static str),
}

/// Ask Tracker for `path` and check the fixture's keys against the answer.
async fn compare(client: &Client, fixture: &str, path: &str) -> Verdict {
    let Ok(live) = client.probe_get(path).await else {
        return Verdict::Unchecked("could not be read");
    };
    let Some(live) = first_object(&live) else {
        return Verdict::Unchecked("answered with nothing to compare");
    };
    let recorded = fixture_value(fixture);
    let Some(recorded) = first_object(&recorded) else {
        return Verdict::Unchecked("has a fixture with nothing to compare");
    };

    match fixture_gap_between(recorded, &keys(live)) {
        Some(missing) => Verdict::Drifted(missing),
        None => Verdict::Same,
    }
}

/// The keys a fixture claims that `live` does not have.
fn fixture_gap(fixture: &str, live: &BTreeSet<String>) -> Option<Vec<String>> {
    let recorded = fixture_value(fixture);
    fixture_gap_between(first_object(&recorded)?, live)
}

fn fixture_gap_between(
    recorded: &serde_json::Value,
    live: &BTreeSet<String>,
) -> Option<Vec<String>> {
    let missing: Vec<String> = keys(recorded)
        .difference(live)
        // A custom field is named after the field's id, so a fixture's invented
        // one is never going to appear in somebody else's organisation.
        .filter(|key| !key.contains("--"))
        .cloned()
        .collect();
    (!missing.is_empty()).then_some(missing)
}

/// A fixture, as JSON.
fn fixture_value(name: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("fixture readable");
    serde_json::from_str(&text).expect("fixture is valid json")
}

/// The object a payload carries: itself, or the first element of a listing.
fn first_object(value: &serde_json::Value) -> Option<&serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items.first(),
        object @ serde_json::Value::Object(_) => Some(object),
        _ => None,
    }
}

/// The subresource payloads, which only exist once something has written one.
///
/// `issue_links.json` and the checklist, worklog and attachment shapes are the
/// fixtures no read-only audit can reach: an organisation with no links answers
/// `[]`, and an empty array agrees with every fixture ever written. So this
/// makes one of each in the test queue and looks at what comes back.
///
/// Everything it creates stays, like everything else here — Tracker has no
/// delete for an issue — but the link, the worklog and the checklist item are
/// taken off again, since those it can undo.
#[tokio::test]
#[ignore = "creates real issues; needs YTCLI_TEST_QUEUE"]
async fn the_subresources_have_the_shape_their_fixtures_claim() {
    let Some(queue) = setting("YTCLI_TEST_QUEUE") else {
        return;
    };
    let client = client();

    let mut made = Vec::new();
    for summary in [
        "ytcli live test — subresource shapes, one",
        "ytcli live test — subresource shapes, two",
    ] {
        made.push(
            client
                .create_issue(&serde_json::json!({"queue": queue, "summary": summary}))
                .await
                .expect("create")
                .key,
        );
    }
    let (Some(key), Some(other)) = (made.first(), made.get(1)) else {
        panic!("two issues were created and fewer than two came back");
    };

    client
        .add_link(key, "relates", other)
        .await
        .expect("add link");
    client
        .add_worklog(
            key,
            // `start` is required, which the fixtures never said: a worklog is
            // when the work happened, not when it was recorded.
            &serde_json::json!({
                "duration": "PT1M",
                "start": jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S%.3f%z").to_string(),
                "comment": "ytcli live test",
            }),
        )
        .await
        .expect("add worklog");
    client
        .add_checklist_item(key, &serde_json::json!({"text": "ytcli live test"}))
        .await
        .expect("add checklist item");

    // Each payload against the keys we believe it has. `issue_links.json` is a
    // recorded fixture; the other two are only ever seen through our parsers,
    // so what is asserted is what those parsers read.
    let links = client
        .probe_get(&format!("/v3/issues/{key}/links"))
        .await
        .expect("links");
    let live = first_object(&links).map(keys).unwrap_or_default();
    assert!(!live.is_empty(), "a link was added and none came back");
    assert_eq!(
        fixture_gap("issue_links.json", &live),
        None,
        "issue_links.json claims keys the real payload does not have"
    );

    for (path, required) in [
        (
            format!("/v3/issues/{key}/worklog"),
            ["id", "issue", "duration", "createdBy", "createdAt"].as_slice(),
        ),
        (
            format!("/v3/issues/{key}/checklistItems"),
            ["id", "text", "checked"].as_slice(),
        ),
    ] {
        let payload = client.probe_get(&path).await.expect("subresource");
        let live = first_object(&payload).map(keys).unwrap_or_default();
        assert!(!live.is_empty(), "{path} answered with nothing");
        for field in required {
            assert!(
                live.contains(*field),
                "{path} no longer carries `{field}`: {live:?}"
            );
        }
    }

    // Put back what can be put back.
    if let Ok(links) = client.issue_links(key).await {
        for link in links {
            let _ = client.delete_link(key, &link.id).await;
        }
    }
}

/// Both link vocabularies, and the fact that they are two.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn link_types_are_not_the_names_a_write_takes() {
    let types = client().link_types().await.expect("link types");
    assert!(!types.is_empty(), "an organisation with no link types");

    for kind in &types {
        assert!(
            kind.outward.is_some() || kind.inward.is_some(),
            "{} has no wording for either direction",
            kind.id
        );
    }

    // The trap this command exists for: `depends` is a type id and is refused
    // as a relationship. If Tracker ever starts accepting it, this failing is
    // how we find out rather than by leaving the warning in place for years.
    let refused = client()
        .add_link("PROJ-0", "depends", "PROJ-0")
        .await
        .expect_err("a type id was accepted as a relationship");
    assert!(
        format!("{refused}").contains("depends"),
        "unexpected refusal: {refused}"
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

/// A field key from the listing can be fetched back on its own, and says what
/// it accepts.
///
/// The claim under test is that `optionsProvider` is real and is the thing that
/// decides values: a fixture we wrote proves only that we can read our own
/// invention. A live organisation is where `assignee` genuinely answers with a
/// provider and `storyPoints` genuinely answers with none.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn a_field_says_what_it_accepts() {
    let client = client();
    let fields = client.fields().await.expect("fields");
    assert!(!fields.is_empty(), "an organisation with no fields");

    for field in fields.iter().take(5) {
        let spec = client
            .field(&field.key)
            .await
            .unwrap_or_else(|error| panic!("field {}: {error}", field.key));
        assert_eq!(spec.key, field.key);
        assert!(
            !spec.field_type.is_empty() && spec.field_type != "unknown",
            "{} has no schema type",
            field.key
        );
    }

    let constrained = client.field("assignee").await.expect("assignee");
    assert!(
        constrained.options.is_some(),
        "assignee accepts anybody? {constrained:?}"
    );
}

/// The links that leave Tracker still answer with a list.
///
/// The organisation this runs against has none, so what is under test is the
/// endpoint and the envelope, not the parsing — and saying so here is cheaper
/// than somebody later reading an empty pass as proof of more than it is.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn remote_links_answer_with_a_list() {
    let client = client();
    let queue = a_queue(&client).await;
    let page = client
        .search(&format!("Queue: {queue}"), 1, 1)
        .await
        .expect("search");
    let Some(issue) = page.items.first() else {
        return;
    };

    client
        .issue_remote_links(&issue.key)
        .await
        .expect("remote links");
}

/// Automation, including the part most tokens may not read.
///
/// The 403 on triggers is the reason this command is shaped the way it is, and
/// it is a live fact rather than one a fixture can assert: whether a queue is
/// readable at all and whether its triggers are are two different rights, and
/// this organisation answers differently for two of its own queues.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn automation_survives_a_section_it_may_not_read() {
    let client = client();
    let queue = a_queue(&client).await;

    let automation = client
        .queue_automation(&queue)
        .await
        .unwrap_or_else(|error| panic!("automation of {queue}: {error}"));

    for refusal in &automation.unreadable {
        assert!(
            !refusal.reason.is_empty(),
            "{} was refused with no reason",
            refusal.section
        );
    }
    assert!(
        automation.unreadable.len() < 3,
        "every section refused should have been an error, not an answer"
    );
}

/// The two halves of "who may do what", and why one command prints both.
///
/// No fixture can establish this: that `permissions` answers with roles and
/// `access` answers with the people those roles come out as is a claim about
/// Tracker, and the whole shape of the command rests on it. The organisation
/// this runs against refuses both halves on some of its queues and allows both
/// on others, which is also the refusal path being exercised for real.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn access_resolves_the_roles_that_permissions_only_names() {
    let client = client();
    let queues = client.queues().await.expect("queues");

    let mut refusals = 0;
    for queue in &queues {
        let Ok(access) = client.queue_access(&queue.key).await else {
            refusals += 1;
            continue;
        };

        // Roles live on the rule side only. If Tracker ever starts answering
        // `access` with roles too, the second table is claiming something it
        // cannot deliver and the `YOU` column is a guess.
        assert!(
            access.access.iter().all(|entry| entry.roles.is_empty()),
            "access answered with roles: it is not a resolved list of people"
        );
        assert!(
            access
                .permissions
                .iter()
                .any(|entry| !entry.roles.is_empty()),
            "permissions answered without a single role"
        );
        assert!(
            access.you.is_some(),
            "the token could not name its own user"
        );
        return;
    }

    // Not a pass by default: every queue refusing is a fact about the token,
    // and it is asserted rather than allowed to look like a verified answer.
    assert_eq!(
        refusals,
        queues.len(),
        "no queue answered and none was counted as refused"
    );
}

/// Two small listings that used to be unreachable.
///
/// Both answer 200 and both are empty in the organisation this runs against, so
/// what is verified is the path and the envelope. That is worth saying out
/// loud: an empty pass here is not evidence that the parsing is right.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn sprints_and_local_fields_answer_with_lists() {
    let client = client();
    let queue = a_queue(&client).await;

    client.all_sprints().await.expect("sprints");
    client
        .queue_local_fields(&queue)
        .await
        .unwrap_or_else(|error| panic!("local fields of {queue}: {error}"));
}

/// Components, which this organisation actually uses.
///
/// Worth a live test rather than a fixture alone: half of the ten it has carry
/// no lead, which is the shape a fixture we wrote would not have thought to
/// include, and the queue-scoped path has to agree with the organisation-wide
/// one about which queue each belongs to.
#[tokio::test]
#[ignore = "needs real credentials"]
async fn components_belong_to_the_queue_they_say_they_do() {
    let client = client();
    let all = client.components(None).await.expect("components");
    let Some(queue) = all.iter().find_map(|component| component.queue.clone()) else {
        return;
    };

    let scoped = client
        .components(Some(&queue))
        .await
        .unwrap_or_else(|error| panic!("components of {queue}: {error}"));

    assert!(!scoped.is_empty(), "{queue} answered with none of its own");
    for component in &scoped {
        assert_eq!(component.queue.as_deref(), Some(queue.as_str()));
    }
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
