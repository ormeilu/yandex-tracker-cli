//! `--help` is the documentation, so it is tested like documentation.
//!
//! The promise made in ADR 6 is that `ytcli <command> --help` is enough to use
//! the command without loading anything else. That is a property of every leaf
//! command, not of the ones somebody remembered to write, so the command tree is
//! walked rather than listed: a new command with clap's default phrasing fails
//! here on the day it is added.

// A failing assertion and a failing `expect` are the same event in a test.
#![allow(clippy::expect_used)]

use assert_cmd::Command;

fn help(path: &[String], long: bool) -> String {
    let mut command = Command::cargo_bin("ytcli").expect("binary");
    command.args(path).arg(if long { "--help" } else { "-h" });
    let output = command.output().expect("run help");
    assert!(output.status.success(), "`ytcli {path:?} --help` failed");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The subcommand names clap lists, if any.
fn children(help: &str) -> Vec<String> {
    let Some(section) = help.split("Commands:\n").nth(1) else {
        return Vec::new();
    };
    section
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_owned)
        .collect()
}

/// Every command that actually does something, rather than grouping others.
fn leaves() -> Vec<Vec<String>> {
    let mut found = Vec::new();
    let mut queue = vec![Vec::new()];

    while let Some(path) = queue.pop() {
        let children = children(&help(&path, false));
        if children.is_empty() {
            if !path.is_empty() {
                found.push(path);
            }
            continue;
        }
        for child in children {
            let mut next = path.clone();
            next.push(child);
            queue.push(next);
        }
    }

    assert!(found.len() > 15, "only found {} commands", found.len());
    found.sort();
    found
}

/// Every leaf opens with a runnable example.
///
/// An agent reading help to decide how to call something needs the call, not a
/// paragraph about it. Anything before `Usage:` is ours to write; clap's flag
/// list below it is not the part in question.
#[test]
fn every_command_shows_an_example_before_its_usage_line() {
    for path in leaves() {
        let long = help(&path, true);
        let prose = long.split("Usage:").next().unwrap_or_default();

        assert!(
            prose.lines().any(|line| line.starts_with("  ytcli ")),
            "`ytcli {} --help` has no example",
            path.join(" ")
        );
    }
}

/// `-h` stays a summary, `--help` is the documentation. If they are identical,
/// one of the two audiences is being ignored.
#[test]
fn the_short_form_is_shorter_than_the_long_one() {
    for path in leaves() {
        let short = help(&path, false);
        let long = help(&path, true);
        assert!(
            long.len() > short.len(),
            "`ytcli {} --help` says no more than -h does",
            path.join(" ")
        );
    }
}

/// Flags in the examples are flags the command has.
///
/// A wrong flag in an example is worse than a missing example: it is a command
/// somebody runs and a failure they then have to diagnose.
#[test]
fn the_examples_only_use_flags_that_exist() {
    let mut paths = leaves();
    // The root's examples name other commands; resolve each against its own.
    paths.insert(0, Vec::new());

    for path in paths {
        let long = help(&path, true);
        let prose = long.split("Usage:").next().unwrap_or_default().to_owned();

        for line in prose.lines().filter(|line| line.starts_with("  ytcli ")) {
            let target = command_of(line);
            let target_help = help(&target, true);

            for flag in flags(line) {
                assert!(
                    target_help.contains(&flag),
                    "`{flag}` is not a flag of `ytcli {}` (in `{}`)",
                    target.join(" "),
                    line.trim()
                );
            }
        }
    }
}

/// The command an example line calls: lower-case words are verbs, and anything
/// with a capital, a digit or a dash is already an argument.
fn command_of(line: &str) -> Vec<String> {
    line.trim()
        .strip_prefix("ytcli")
        .unwrap_or_default()
        .split_whitespace()
        .take_while(|word| word.chars().all(|c| c.is_ascii_lowercase()))
        .take(2)
        .map(str::to_owned)
        .collect()
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
