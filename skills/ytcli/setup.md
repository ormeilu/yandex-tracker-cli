# Profiles, several organisations, CI, permissions

## Getting the binary

This skill is documentation; the CLI is a separate program. However the skill
arrived, the binary still has to be installed:

```bash
uvx --from yandex-tracker-cli ytcli --help    # run it without installing
uv tool install yandex-tracker-cli            # keep it
cargo install yandex-tracker-cli              # with a Rust toolchain
```

Or a prebuilt binary for Linux, macOS or Windows, x86-64 or arm64, from
[Releases](https://github.com/ormeilu/yandex-tracker-cli/releases).

`uvx ytcli` will not work: `uvx` looks for an executable named after the
package, and `ytcli` as a package name belongs to somebody else. The package is
`yandex-tracker-cli` on both PyPI and crates.io; the command it installs is
`ytcli`.

## Two words that are not the same thing

An **account** holds one credential. A **profile** is one organisation seen
through one account. A person with an admin login and a personal login in the
same organisation has two accounts; the same account seeing a work and a
personal organisation gives two profiles. Log in once per account; every profile
naming it works from then on.

```bash
ytcli auth status        # every profile: identity, org, queues, projects, your open issues
ytcli auth status --brief --active-only
ytcli auth list          # accounts and profiles, and whether a token is stored
```

## Which profile is in play

Highest wins:

1. `--profile NAME`
2. `YTCLI_PROFILE`
3. `.tracker.toml` in the working directory or above it
4. the configured default

`auth status` reports which of these the answer came from, so "it used the wrong
organisation" is a question with an answer rather than a guess.

**In a repository**, commit a `.tracker.toml` naming the profile. An agent handed
that directory and nothing else then reaches the right organisation with no
setup.

## Logging in

`ytcli auth login` is interactive: it asks for each value in turn and takes the
token the way a password prompt does, so it never lands in shell history.

**Do not attempt this on the user's behalf, and never ask for a token in the
conversation.** If credentials are missing, say so and let the user run it.
Outside a terminal the command takes flags only and reads the token from stdin,
which is what CI uses.

## CI and containers

```bash
export YTCLI_TOKEN=…      # checked before the keychain is opened at all
export YTCLI_ORG_ID=…
```

With `YTCLI_TOKEN` set, no keychain is touched, so a runner with no Secret
Service and no Keychain is not a problem. Without it, there is no plaintext
fallback: tokens live in the OS keychain or the command fails and says so.

There is no command that prints a stored token.

`YTCLI_TOKEN` is **not per account**: it is checked before the keychain and used
for whichever profile is in play, so with more than one profile configured it
makes them all the same identity. `ytcli auth status` marks each profile it read
that way with `(from YTCLI_TOKEN)` and warns once at the end. If two profiles
report the same person, that is the first thing to check — a shell that sources
`.env` on entering a directory sets it without anyone deciding to.

In a sandbox that is rebuilt per session — no keychain, nothing kept between
runs — `YTCLI_TOKEN` is the only mechanism there is. The user sets it in the
environment themselves. **Do not ask for a token in the conversation to put it
there**: a token in a transcript is a token to revoke, and an environment
variable set from a command line is visible to every process on the machine.

## Permission allowlists

The verb is the risk class, and no verb both reads and writes, so a static
allowlist is worth having. For Claude Code, in `.claude/settings.json`:

```json
{
  "permissions": {
    "allow": [
      "Bash(ytcli issue get:*)",
      "Bash(ytcli issue find:*)",
      "Bash(ytcli issue list:*)",
      "Bash(ytcli issue count:*)",
      "Bash(ytcli issue links:*)",
      "Bash(ytcli issue comments:*)",
      "Bash(ytcli issue changelog:*)",
      "Bash(ytcli queue list:*)",
      "Bash(ytcli queue get:*)",
      "Bash(ytcli queue fields:*)",
      "Bash(ytcli queue versions:*)",
      "Bash(ytcli queue tags:*)",
      "Bash(ytcli worklog find:*)",
      "Bash(ytcli dict list:*)",
      "Bash(ytcli user list:*)",
      "Bash(ytcli user get:*)",
      "Bash(ytcli user find:*)",
      "Bash(ytcli field list:*)",
      "Bash(ytcli field get:*)",
      "Bash(ytcli template list:*)",
      "Bash(ytcli board list:*)",
      "Bash(ytcli board get:*)",
      "Bash(ytcli board sprints:*)",
      "Bash(ytcli project list:*)",
      "Bash(ytcli project get:*)",
      "Bash(ytcli portfolio list:*)",
      "Bash(ytcli portfolio get:*)",
      "Bash(ytcli portfolio contents:*)",
      "Bash(ytcli goal list:*)",
      "Bash(ytcli goal get:*)",
      "Bash(ytcli issue worklogs:*)",
      "Bash(ytcli issue checklist:*)",
      "Bash(ytcli attachment list:*)",
      "Bash(ytcli attachment show:*)",
      "Bash(ytcli auth status:*)",
      "Bash(ytcli auth list:*)",
      "Bash(ytcli cheatsheet:*)"
    ],
    "ask": [
      "Bash(ytcli portfolio place:*)",
      "Bash(ytcli project create:*)",
      "Bash(ytcli project update:*)",
      "Bash(ytcli project delete:*)",
      "Bash(ytcli portfolio create:*)",
      "Bash(ytcli portfolio update:*)",
      "Bash(ytcli portfolio delete:*)",
      "Bash(ytcli goal create:*)",
      "Bash(ytcli goal update:*)",
      "Bash(ytcli goal delete:*)",
      "Bash(ytcli project place:*)",
      "Bash(ytcli queue create:*)",
      "Bash(ytcli issue create:*)",
      "Bash(ytcli issue update:*)",
      "Bash(ytcli issue comment:*)",
      "Bash(ytcli issue transition:*)",
      "Bash(ytcli issue move:*)",
      "Bash(ytcli issue worklog:*)",
      "Bash(ytcli issue check:*)",
      "Bash(ytcli issue link:*)",
      "Bash(ytcli attachment upload:*)",
      "Bash(ytcli auth login:*)",
      "Bash(ytcli auth use:*)",
      "Bash(ytcli auth logout:*)"
    ]
  }
}
```

Reading then stops prompting, and anything that changes someone else's Tracker
still asks. This is data you install, not something the plugin does to you: a
plugin cannot grant itself permissions, and one that could should not.
