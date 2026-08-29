# Configuration

Everything here is optional except the first login. The tool works with one
account, one organisation and no config file beyond what `ytcli auth login`
writes for you.

## Accounts and profiles

An **account** holds a credential; a **profile** is an organisation seen through
an account. One login can reach several organisations, and one organisation can
be reached through several logins — so the two are separate.

```bash
ytcli auth login
```

In a terminal it walks you through each step and takes the token as a password,
so it never lands in your scrollback or shell history. Pass what you already
know and only the rest is asked for:

```bash
ytcli auth login --account work --org-id 12345 --queue PROJ
```

You need an OAuth token
([how to get one](https://yandex.ru/support/tracker/en/api-ref/access)) and an
organisation id
([tracker.yandex.ru/admin/orgs](https://tracker.yandex.ru/admin/orgs) lists
yours). `ytcli` prints both sets of steps itself when you need them.

It checks the token against the API, stores it in the OS keychain — macOS
Keychain, Windows Credential Manager, Secret Service on Linux — and writes the
profile for you. The token is never written to a config file, never passed as an
argument, and no command prints it back.

`--org-kind` is detected if you do not know it: the two organisation flavours
use different headers, and the wrong one answers 403 in a way that looks like a
permissions problem. `--dry-run` checks the token and reports what would be
written without touching anything.

## The config file

That leaves `~/.config/ytcli/config.toml` looking like this. Hand-edit it
freely: `auth login` preserves your comments and only touches the keys it owns.

```toml
default_profile = "work"

[accounts.work]
description = "admin identity"

[profiles.work]
account = "work"
org_id = "12345"
org_kind = "cloud"      # cloud -> X-Cloud-Org-Id, yandex360 -> X-Org-Id
default_queue = "PROJ"

[profiles.work.display]
limit = 25
description_lines = 10
extra_fields = ["sprint", "storyPoints"]
```

With `default_queue` set, a bare issue number is completed from it: `ytcli issue
get 42` and `ytcli issue get PROJ-42` name the same issue, which is what
somebody reading a board and typing a key by hand actually has in front of them.

## Per repository

In a repository, commit a `.tracker.toml`:

```toml
profile = "work"
queue = "PROJ"
```

Anyone — or any agent — working in that checkout now talks to the right
organisation with no global state to get wrong. To change the stored default
instead, `ytcli auth use work`: a local edit that reads no token and sends no
request.

## Which profile is in play

Highest wins:

1. `--profile NAME`
2. `YTCLI_PROFILE`
3. `.tracker.toml` in the working directory or above it
4. the configured default

Every command says which of these answered, on stderr, so "it used the wrong
organisation" is a question with an answer rather than a guess. With more than
one profile configured, a bare `PROJ-1` is routed to the profile that can
actually see that queue rather than to the default one — two profiles in
*different* organisations sharing a queue key is refused rather than guessed at,
and `work/PROJ-1` says which outright.

## CI and containers

```bash
export YTCLI_TOKEN=…      # checked before the keychain is opened at all
export YTCLI_ORG_ID=…
```

With `YTCLI_TOKEN` set, no keychain is touched, so a runner with no Secret
Service and no Keychain is not a problem. Without it there is no plaintext
fallback: tokens live in the OS keychain or the command fails and says so.

`YTCLI_TOKEN` is **not per account**. It is checked before the keychain and used
for whichever profile is in play, so with more than one profile configured it
makes them all the same identity. `ytcli auth status` marks every profile it
read that way and warns once at the end — a shell that sources `.env` on
entering a directory sets it without anyone deciding to.

## Everything else

- `YTCLI_CONFIG` points at another config file; `--config PATH` does it for one
  command.
- `ytcli auth list` shows the accounts and profiles and whether a token is
  stored for each. There is no command that prints a stored token.
- `ytcli auth logout --account NAME` forgets one.
