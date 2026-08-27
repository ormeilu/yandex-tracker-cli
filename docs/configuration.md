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

You need two things: an OAuth token and an organisation id.

**Token** — create an application at [oauth.yandex.com/client/new](https://oauth.yandex.com/client/new),
choose "For API access or debugging", grant `tracker:write` (or `tracker:read`
to stay read-only), then open
`https://oauth.yandex.com/authorize?response_type=token&client_id=<ClientID>`
and sign in. The token comes back in the address bar and looks like `y0__xAbc…`.

**Organisation id** — [tracker.yandex.ru/admin/orgs](https://tracker.yandex.ru/admin/orgs)
lists every organisation you belong to, with its id and its kind. A Yandex 360
organisation has a numeric id; a Yandex Cloud organisation has one made of
letters and digits.

Full reference: [Tracker API access](https://yandex.ru/support/tracker/en/api-ref/access).

`ytcli` prints these steps itself when you need them — on a first login, on a
rejected token, on an organisation it cannot reach.

```bash
ytcli auth login
```

In a terminal that walks you through it one question at a time — account, token
(entered as a password, so it never reaches the scrollback or your shell
history), organisation, profile name, and a default queue picked from the queues
the token can actually see.

Anything you already know can be passed as a flag, and only the rest is asked
for:

```bash
ytcli auth login --account admin --org-id 12345 --queue PROJ
```

Outside a terminal — CI, a script, a pipe — the flags are all there is: the token
is read from stdin and a missing `--account` is an error rather than a prompt
that would hang.

Either way it does the whole path: it prompts for the token (or reads stdin
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

### If macOS keeps asking for your password

The Keychain grants "Always Allow" to the *exact binary* it was asked about, and
identifies it by its code signature. A release downloaded from GitHub is signed
once and keeps its approval; a binary you build yourself is ad-hoc signed by the
linker, so its signature changes on every `cargo install` and the Keychain
correctly treats the new one as an application it has never seen.

This is not a quirk of ytcli. `gh` is ad-hoc signed too, and asks again after a
`brew upgrade` for the same reason; you just do not rebuild it several times an
hour.

If you build ytcli yourself, give it a stable signature once:

```sh
just signing-identity   # once per machine: a self-signed code-signing cert
just local-install      # builds, installs, and signs with it
```

The certificate is trusted for code signing and nothing else, and the private
key is usable by `codesign` alone. Remove it with
`security delete-certificate -c ytcli-dev`.

Within a single command the token is read once, however many profiles share the
account, so a dialog per profile is not something you should see.

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

`ytcli auth status` is what to run when something is wrong. It checks **every**
profile, not just the active one — "it works with my other login" is the usual
next question — and reports where the active choice came from:

```
profile work (from /home/me/src/app/.tracker.toml)  [active]
  account: admin   org: 12345 (Cloud)   queue: PROJ
  token: ok   user: ilubenets (Ilya Lubenets)
  queues: 12   projects: 4   goals: 2   my open issues: 7
  projects: Storage rework (12), Billing (13), +2 more
  queues: PROJ, INFRA, DESIGN
profile my
  account: personal   org: 98765 (Yandex360)
  token: missing
```

The counts cost a handful of requests per profile, which is right for a
diagnostic and wrong for a hot path: `--brief` skips them, `--active-only` skips
the other profiles. The exit code follows the active profile, or reports failure
when no profile worked at all.

When a token is rejected or an organisation is not found, the output includes the
steps for getting the right value — creating an OAuth application is not
something anyone guesses.

## When two profiles share a queue key

Queue keys are unique inside an organisation, not across them: two profiles can
each see an `LMS`, and `LMS-12` then names two different issues. Whichever
profile is active decides — which is fine in a repository with a `.tracker.toml`,
and a trap when you are switching between organisations by hand.

A bare `LMS-12` keeps working — the prefix is only *required* when a collision
actually exists, and only once this tool has seen it:

```bash
ytcli issue get LMS-12          # fine, when only one profile sees LMS
ytcli issue get work/LMS-12     # always accepted, collision or not
```

When two profiles do share a queue key, the bare form is refused rather than
guessed at, and the message names both candidates. The prefix overrides
everything else for that command, and the command reports the profile it used.

The collision is known from what `ytcli auth status` and `ytcli auth login`
already had to look up — the queue lists they fetch are recorded next to the
config, so nothing extra is requested on a normal command. That means the
knowledge is only as current as the last `auth status`: an unknown collision
does not block anything, which is deliberate. Refusing on a guess would make the
common case worse to guard against a situation most people never have.

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
