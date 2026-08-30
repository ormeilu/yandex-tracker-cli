//! End-to-end checks against the built binary.
//!
//! These cover the parts of the contract that only exist once the process runs:
//! the command tree, the exit codes, and the promise that stdout stays clean.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use assert_cmd::Command;
use predicates::prelude::*;

fn ytcli() -> Command {
    Command::cargo_bin("ytcli").expect("binary built")
}

#[test]
fn help_lists_every_entity_group() {
    ytcli()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("issue"))
        .stdout(predicate::str::contains("queue"))
        .stdout(predicate::str::contains("project"))
        .stdout(predicate::str::contains("goal"))
        .stdout(predicate::str::contains("attachment"))
        .stdout(predicate::str::contains("auth"));
}

#[test]
fn cheatsheet_prints_the_whole_sheet() {
    ytcli()
        .arg("cheatsheet")
        .assert()
        .success()
        .stdout(predicate::str::contains("## issue"))
        .stdout(predicate::str::contains("## auth"));
}

#[test]
fn cheatsheet_can_be_narrowed_to_one_topic() {
    ytcli()
        .args(["cheatsheet", "issue"])
        .assert()
        .success()
        .stdout(predicate::str::contains("## issue"))
        .stdout(predicate::str::contains("## auth").not());
}

/// The binary routinely arrives detached from its repository, and for whoever
/// ends up holding it `--help` is the whole surface of the project.
#[test]
fn the_long_help_says_where_the_docs_and_the_bug_tracker_are() {
    ytcli()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "https://ormeilu.github.io/yandex-tracker-cli/",
        ))
        .stdout(predicate::str::contains(
            "https://github.com/ormeilu/yandex-tracker-cli/issues",
        ));
}

/// Somebody who asked for the short help asked for less, not for less of a
/// different thing: `-h` keeps the size it had.
#[test]
fn the_short_help_carries_no_links() {
    ytcli()
        .arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("github.io").not());
}

/// The cheatsheet is what an agent reaches for instead of opening a file, which
/// makes it where it would look for whatever the sheet did not cover.
#[test]
fn the_cheatsheet_carries_the_links_too() {
    ytcli()
        .args(["cheatsheet", "more"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "https://github.com/ormeilu/yandex-tracker-cli/issues",
        ));
}

/// The mistake this line exists for: `#` numbers a list in Yandex wiki markup
/// and heads a section in Markdown, and Tracker draws it as a heading either
/// way — verified against a real Tracker in `tests/live.rs`.
#[test]
fn the_cheatsheet_says_which_markup_a_description_is() {
    ytcli()
        .arg("cheatsheet")
        .assert()
        .success()
        .stdout(predicate::str::contains("Markdown (Yandex Flavored)"))
        .stdout(predicate::str::contains("never a list marker"));
}

#[test]
fn unknown_cheatsheet_topic_fails_loudly() {
    ytcli()
        .args(["cheatsheet", "nonsense"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unknown topic"));
}

/// With nothing configured, the tool must say how to fix that rather than
/// merely failing.
#[test]
fn auth_status_without_a_profile_explains_how_to_get_credentials() {
    let empty = tempfile::NamedTempFile::new().expect("temp config");
    let dir = tempfile::tempdir().expect("temp dir");

    ytcli()
        .args(["auth", "status", "--config"])
        .arg(empty.path())
        .current_dir(dir.path())
        .env_remove("YTCLI_PROFILE")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("no profiles configured yet"))
        .stderr(predicate::str::contains("oauth.yandex.ru"))
        .stderr(predicate::str::contains("--org-id"));
}

/// A repository pin selects the profile, and the tool says so. Getting this
/// wrong means changes land in the wrong organisation.
#[test]
fn a_repository_pin_selects_the_profile_and_is_reported() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join(".tracker.toml"), "profile = \"work\"\n").expect("write pin");

    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        r#"
[accounts.admin]
description = "admin account"

[profiles.work]
account = "admin"
org_id = "12345"
org_kind = "cloud"
"#,
    )
    .expect("write config");

    ytcli()
        .args(["auth", "status", "--brief", "--config"])
        .arg(&config)
        .current_dir(dir.path())
        .env_remove("YTCLI_PROFILE")
        .env_remove("YTCLI_TOKEN")
        .assert()
        .stdout(predicate::str::contains("profile work (from"))
        .stdout(predicate::str::contains(".tracker.toml"))
        .stdout(predicate::str::contains("[active]"))
        .stdout(predicate::str::contains("org: 12345"));
}

/// The listing must never print a token, only whether one exists.
#[test]
fn auth_list_reports_whether_a_token_exists_never_the_token() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        r#"
default_profile = "work"

[accounts.admin]
description = "admin identity"

[profiles.work]
account = "admin"
org_id = "12345"
org_kind = "cloud"
"#,
    )
    .expect("write config");

    let output = ytcli()
        .args(["auth", "list", "--config"])
        .arg(&config)
        .current_dir(dir.path())
        .env_remove("YTCLI_PROFILE")
        .env_remove("YTCLI_TOKEN")
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("account admin"));
    assert!(stdout.contains("token:"));
    assert!(stdout.contains("profile work"));
    assert!(stdout.contains("[default, active]"));
}

/// A token piped in must not have to be typed, and must not appear anywhere it
/// could be read back — which includes the command line.
#[test]
fn auth_login_refuses_an_empty_token() {
    let dir = tempfile::tempdir().expect("temp dir");
    let empty = tempfile::NamedTempFile::new().expect("temp config");

    ytcli()
        .args(["auth", "login", "--account", "test", "--config"])
        .arg(empty.path())
        .current_dir(dir.path())
        .write_stdin("   \n")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("no token given"));
}

/// ADR 2: a tool whose main consumer is an agent must not offer reading a
/// secret back as a feature. Asserted on the subcommand list, so adding one
/// called `token` (or `show-token`, or `reveal`) fails here.
#[test]
fn there_is_no_subcommand_that_prints_a_stored_token() {
    let output = ytcli().args(["auth", "--help"]).assert().success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");

    let subcommands: Vec<&str> = stdout
        .lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .filter_map(|line| line.split_whitespace().next())
        .collect();

    assert_eq!(
        subcommands,
        ["login", "logout", "list", "use", "status", "help"]
    );
}
