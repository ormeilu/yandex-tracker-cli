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

#[test]
fn unknown_cheatsheet_topic_fails_loudly() {
    ytcli()
        .args(["cheatsheet", "nonsense"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("unknown topic"));
}

/// Without a resolvable profile the tool must explain itself and exit with the
/// auth code, not crash and not pretend everything is fine.
#[test]
fn auth_status_without_a_profile_reports_the_auth_exit_code() {
    let empty = tempfile::NamedTempFile::new().expect("temp config");
    let dir = tempfile::tempdir().expect("temp dir");

    ytcli()
        .args(["auth", "status", "--config"])
        .arg(empty.path())
        .current_dir(dir.path())
        .env_remove("YTCLI_PROFILE")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("no profile selected"));
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
        .args(["auth", "status", "--config"])
        .arg(&config)
        .current_dir(dir.path())
        .env_remove("YTCLI_PROFILE")
        .assert()
        .stdout(predicate::str::contains("profile: work (from"))
        .stdout(predicate::str::contains(".tracker.toml"))
        .stdout(predicate::str::contains("org: 12345"));
}

/// Declared but unbuilt commands must be honest about it rather than failing in
/// a way that looks like a Tracker problem.
#[test]
fn unimplemented_commands_use_a_distinct_exit_code() {
    let dir = tempfile::tempdir().expect("temp dir");
    let empty = tempfile::NamedTempFile::new().expect("temp config");

    ytcli()
        .args(["goal", "list", "--config"])
        .arg(empty.path())
        .current_dir(dir.path())
        .assert()
        .code(64)
        .stderr(predicate::str::contains("not implemented"));
}
