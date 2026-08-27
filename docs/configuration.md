# Configuration

## Accounts and profiles are different things

An **account** holds a credential. A **profile** is an organisation seen through
an account, plus display defaults.

Keeping them apart is not pedantry — the real world is many-to-many in both
directions. The same login is often an admin in one organisation and an ordinary
member in another; the same organisation is often reached through two identities,
one with rights you would rather not hand to an automation.

```toml
# ~/.config/ytcli/config.toml
default_profile = "work"

[accounts.admin]
description = "admin identity"

[accounts.personal]
description = "everyday login"

[profiles.work]           # organisation, through the admin account
account = "admin"
org_id = "12345"
org_kind = "cloud"
default_queue = "PROJ"

[profiles.work-readonly]  # same organisation, restricted identity
account = "personal"
org_id = "12345"
org_kind = "cloud"

[profiles.my]             # different organisation, same identity
account = "personal"
org_id = "98765"
org_kind = "yandex360"
```

`org_kind` picks the header that carries the organisation id: `cloud` sends
`X-Cloud-Org-Id`, `yandex360` sends `X-Org-Id`. They are not interchangeable —
the wrong one is a 403 that looks like a permissions problem.

## Credentials

```bash
ytcli auth login --account admin --org-id 12345 --queue PROJ
```

That one command does the whole path: it prompts for the token (or reads stdin
when piped), checks it against the API, stores it in the OS keychain — macOS
Keychain, Windows Credential Manager, Secret Service on Linux — and writes the
account and profile into the config file.

`--org-kind` is detected when omitted. The two flavours use different headers and
the wrong one answers 403, which reads like a permissions problem rather than a
configuration mistake; one extra request at login saves that afternoon.

Add `--dry-run` to check a token and see what would be written without changing
anything. Add a second organisation for the same login by running it again with
a different `--org-id` and `--profile`.

Log in once per **account**; every profile naming it is then usable.

There is no plaintext fallback. If no keychain is available the command fails and
says so, rather than quietly writing a token to a file that a stray `git add -A`
would pick up. There is also no command that prints a stored token back: a tool
whose main consumer is an agent should not offer that as a feature.

For CI, where no keychain exists, `YTCLI_TOKEN` and `YTCLI_ORG_ID` take over.

## Which profile is active

Highest wins:

1. `--profile NAME`
2. `YTCLI_PROFILE`
3. the nearest `.tracker.toml`, walking up from the working directory
4. `default_profile`

There is no `use-context`-style switch, and no command mutates which profile is
active. Ambient state you set an hour ago is exactly how a change ends up in the
wrong organisation.

## Pinning a repository

`.tracker.toml`, committed, no secrets:

```toml
profile = "work"
queue = "PROJ"
```

Found the way git finds `.git`. Anyone working in the checkout — including an
agent that was handed the directory and no other context — reaches the right
organisation with no setup.

`ytcli auth status` reports the profile **and where it came from**:

```
profile: work (from /home/me/src/app/.tracker.toml)
account: admin   org: 12345 (Cloud)
queue: PROJ
token: ok   user: ilubenets
```

## Display defaults

Every default is overridable per profile:

```toml
[profiles.work.display]
limit = 25              # rows per page
max = 500               # ceiling for --all
description_lines = 10  # before the --full hint
extra_fields = ["sprint", "storyPoints"]
format = "text"         # when stdout is not a terminal
```

`extra_fields` is ordered, and that order is preserved on output. Custom fields
differ per queue, so the compact view counts them rather than dumping them; these
are the ones you have decided are worth the space. Find their keys with
`ytcli queue fields PROJ`.
