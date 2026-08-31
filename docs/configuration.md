# Configuration

Everything here is optional except the first login. The tool works with one
account, one organisation and no config file beyond what `ytcli auth login`
writes for you.

## Accounts and profiles

An **account** is a login: one Yandex identity, one OAuth token, kept in the OS
keychain under the account's name. It says *who you are*.

A **profile** is one organisation seen through one account, plus the defaults
for working in it — the default queue, display settings, pinned custom fields.
It says *where you are working, as whom*.

They are separate because neither contains the other: one account can reach
several organisations, and one organisation can be reached through several
accounts (an admin identity and a read-only one, say). So:

| | account | profile |
|---|---|---|
| holds | a token, in the keychain | organisation id, org kind, defaults — in the config file |
| named by | `--account`, `auth logout` | `--profile`, `YTCLI_PROFILE`, `.tracker.toml`, `work/PROJ-1` |
| created by | `auth login` | `auth login` (same run) |
| deleted by | `auth logout --account NAME` | `auth remove NAME --yes` |

Every command runs as exactly one profile, and prints which one on stderr; the
account is only how that profile gets its token. Two profiles pointing at the
same account share one credential: log out and both stop working, while removing
one profile leaves the other untouched.

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
description = "production — customer data"

[profiles.work.display]
limit = 25
description_lines = 10
extra_fields = ["sprint", "storyPoints"]
```

`description` is optional and free text. It is shown by `ytcli auth list` and
`ytcli auth status`, and appended to the `→ profile=… org=…` line every command
prints on stderr — which is the point of it: `12345` identifies nothing until
you already know the number, and `work2` does not say whose data is behind it.
Set it without hand-editing:

```bash
ytcli auth edit work --description "production — customer data"
ytcli auth edit sandbox --clear-description
```

`ytcli auth login --description TEXT` sets the same field while creating a
profile, and logging in again without the flag leaves an existing note alone.

With `default_queue` set, a bare issue number is completed from it: `ytcli issue
get 42` and `ytcli issue get PROJ-42` name the same issue, which is what
somebody reading a board and typing a key by hand actually has in front of them.

## Changing a profile later

```bash
ytcli auth edit work --name prod                  # rename; default_profile follows
ytcli auth edit work --org-id 67890 --queue TEAM  # point it somewhere else
ytcli auth edit work --account admin              # use another credential
```

Like `auth use`, these read no token and send no request, so a profile can be
corrected whether or not its credentials currently work. What you do not pass is
not touched, and a rename moves the whole `[profiles.x]` table — display
settings and all. Nothing here is verified against Tracker, because nothing is
sent: `ytcli auth status --active-only` is the check afterwards.

Two things a local edit deliberately does not reach: a committed
`.tracker.toml` naming the old profile is reported rather than rewritten, since
it is shared with everyone else working in that checkout; and the credential
itself, which is keyed by account in the keychain — `ytcli auth login` replaces
that.

## Removing a profile

```bash
ytcli auth remove sandbox --yes     # what would go: --dry-run
```

The counterpart to `auth login`, and not the same thing as `auth logout`:
this drops the organisation and its defaults from the config file, while logout
drops the credential from the keychain. Undoing a login completely takes both,
in either order.

`--yes` is required even for one profile. Nothing is sent anywhere, but
`[profiles.x]` holds display settings and pinned custom fields that exist only
in that file, and logging in again does not bring them back.

If the profile was the default, `default_profile` goes with it rather than being
pointed at another organisation — which one a bare command touches is a decision
you make with `ytcli auth use NAME`. The account is left alone, because other
profiles may still use it; when none do, the command says so and prints the
`auth logout` line. A committed `.tracker.toml` naming the removed profile is
reported rather than rewritten.

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
- `ytcli auth logout --account NAME` forgets a token; the profiles using it
  stay, and logging back in makes them work again.
- `ytcli auth remove NAME --yes` deletes a profile; the account and its token
  stay.
