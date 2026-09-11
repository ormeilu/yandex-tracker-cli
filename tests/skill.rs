//! The shipped skill must describe the CLI that exists.
//!
//! A skill is documentation an agent acts on without checking, which makes a
//! stale example worse than a missing one: it turns into a failed command in
//! someone's session. Every command line in the skill is therefore run against
//! the real binary's help, so a renamed verb or a dropped flag fails here rather
//! than in the field.

// A failing assertion and a failing `expect` are the same event in a test: the
// build stops and says why.
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;

mod harness;

fn skill_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin/skills/ytcli")
}

fn markdown_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(skill_dir())
        .expect("skill directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "the skill has no files");
    files
}

/// The command path in a line like `ytcli issue get PROJ-1 --fields status`.
///
/// Lower-case words are verbs, hyphenated ones like `rows-add` included;
/// anything with a capital, a digit or a leading dash is an argument and ends
/// the path.
fn command_path(line: &str) -> Option<Vec<String>> {
    let rest = line.trim().strip_prefix("ytcli")?;
    let path: Vec<String> = rest
        .split_whitespace()
        .take_while(|word| {
            word.starts_with(|c: char| c.is_ascii_lowercase())
                && word.chars().all(|c| c.is_ascii_lowercase() || c == '-')
        })
        .take(2)
        .map(str::to_owned)
        .collect();
    (!path.is_empty()).then_some(path)
}

fn flags(line: &str) -> Vec<String> {
    line.split_whitespace()
        .filter(|word| word.starts_with("--") && word.len() > 2)
        .map(|word| {
            word.trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_owned()
        })
        .filter(|word| word.len() > 2)
        .collect()
}

fn help_for(path: &[String]) -> String {
    let mut command = Command::cargo_bin("ytcli").expect("binary");
    command.args(path).arg("--help");
    let output = command.output().expect("run help");
    assert!(
        output.status.success(),
        "`ytcli {} --help` failed — the skill names a command that does not exist",
        path.join(" ")
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Every `ytcli …` line in the skill names a command the binary has, and only
/// flags that command accepts.
#[test]
fn the_skill_only_promises_commands_that_exist() {
    for file in markdown_files() {
        let text = std::fs::read_to_string(&file).expect("read");
        let name = file.file_name().unwrap_or_default().to_string_lossy();

        for line in text.lines() {
            let Some(path) = command_path(line) else {
                continue;
            };
            let help = help_for(&path);

            for flag in flags(line) {
                assert!(
                    help.contains(&flag),
                    "{name}: `{flag}` is not a flag of `ytcli {}`",
                    path.join(" ")
                );
            }
        }
    }
}

/// The entry point stays resident once the skill triggers, so its size is a
/// running cost rather than a one-off. Topic files are the place for detail.
#[test]
fn the_entry_point_stays_small() {
    let text = std::fs::read_to_string(skill_dir().join("SKILL.md")).expect("SKILL.md");
    let lines = text.lines().count();
    assert!(lines <= 120, "SKILL.md has grown to {lines} lines");

    assert!(text.starts_with("---\n"), "SKILL.md needs frontmatter");
    let frontmatter = text.split("---").nth(1).unwrap_or_default();
    assert!(frontmatter.contains("name: ytcli"));
    assert!(frontmatter.contains("description:"));
}

/// The topic files are only useful if the entry point sends the reader to them.
#[test]
fn every_topic_file_is_referenced_from_the_entry_point() {
    let entry = std::fs::read_to_string(skill_dir().join("SKILL.md")).expect("SKILL.md");
    for file in markdown_files() {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        if name == "SKILL.md" {
            continue;
        }
        assert!(entry.contains(name.as_ref()), "{name} is never mentioned");
    }
}

/// Every query the skill teaches, taken off the page rather than kept in a list
/// beside it: a list beside it is a list that drifts.
fn documented_queries() -> Vec<String> {
    let text = std::fs::read_to_string(skill_dir().join("yql.md")).expect("yql.md");
    let queries: Vec<String> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("ytcli issue find --yql "))
        .map(|rest| rest.trim().trim_matches('\'').to_owned())
        .collect();

    assert!(
        queries.len() >= 10,
        "yql.md teaches {} queries; the extraction is probably broken",
        queries.len()
    );
    queries
}

/// The queries reach Tracker exactly as written.
///
/// This is not a test of the language — a stub accepts anything — but of the
/// path between the page and the wire: shell quoting, the `--yql` argument, and
/// the body the client builds. A documented query the CLI mangles is a
/// documented query that fails in the field. Whether Tracker *accepts* them is
/// the live suite's question, and the same list answers it there.
#[tokio::test]
async fn every_documented_query_survives_the_trip_to_the_request_body() {
    let harness = harness::Harness::new().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v3/issues/_search"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([]))
                .append_header("X-Total-Count", "0"),
        )
        .mount(&harness.server)
        .await;

    for query in documented_queries() {
        harness
            .run(&["issue", "find", "--yql", &query])
            .assert()
            .success();
    }

    let requests = harness
        .server
        .received_requests()
        .await
        .expect("recorded requests");
    let sent: Vec<String> = requests
        .iter()
        .filter_map(|request| serde_json::from_slice::<serde_json::Value>(&request.body).ok())
        .filter_map(|body| body["query"].as_str().map(ToOwned::to_owned))
        .collect();

    assert_eq!(sent, documented_queries());
}

/// No verb the allowlist lets through is the start of one it asks about.
///
/// Hosts match these patterns by prefix, so allowing `ytcli wiki grid:*` also
/// allows anything spelled `ytcli wiki grid…` — which is how a read verb and a
/// write verb sharing a prefix would turn an allowed read into an allowed
/// write (ADR 1). Checked on the published list rather than on a list of our
/// own, because that list is what people install.
#[test]
fn no_allowed_read_is_the_prefix_of_a_write() {
    let text = std::fs::read_to_string(skill_dir().join("setup.md")).expect("setup.md");
    let json = text
        .split("```json")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("setup.md has a JSON allowlist");
    let settings: serde_json::Value = serde_json::from_str(json).expect("the allowlist is JSON");
    let verbs = |list: &str| -> Vec<String> {
        settings["permissions"][list]
            .as_array()
            .expect("a list of patterns")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter_map(|pattern| pattern.strip_prefix("Bash(")?.strip_suffix(":*)"))
            .map(ToOwned::to_owned)
            .collect()
    };

    let (allowed, asked) = (verbs("allow"), verbs("ask"));
    assert!(
        allowed.len() > 10 && asked.len() > 10,
        "the extraction is broken"
    );
    for read in &allowed {
        for write in &asked {
            assert!(
                !write.starts_with(read.as_str()),
                "allowing `{read}:*` also allows `{write}`"
            );
        }
    }
}
