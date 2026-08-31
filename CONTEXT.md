# Domain model

The vocabulary this repository uses. Terms here are the ones that appear in code,
in `--help`, and in the docs; where Tracker's own naming is ambiguous, the
canonical term below wins.

## Identity and access

**Account** — an identity that holds one credential; *who you are*. `ytcli auth
login` stores a token per account, in the OS keychain. An account is not tied to
an organisation: the same login can be an admin in one organisation and a plain
member in another. It is never selected directly — it is reached through the
profile that names it — and it is removed by `auth logout`, which forgets the
token and deletes no profile.

**Organisation** — a Tracker tenant. It comes in two flavours that are addressed
by *different HTTP headers*, which is why the flavour is configuration rather
than something we detect: Yandex Cloud Organization (`X-Cloud-Org-Id`) and
Yandex 360 for Business (`X-Org-Id`). Sending the wrong header is a 403.

**Profile** — an organisation seen through an account, plus display defaults;
*where you work, as whom*. Profiles are the unit users select (`--profile`);
accounts are the unit credentials attach to. The relationship is many-to-many in
both directions: one account can reach several organisations, and one
organisation can be reached through several accounts (say, an admin identity and
a read-only one). A profile is removed by `auth remove`, which deletes the
organisation and its defaults from the config file and leaves the account and
its token where they are — undoing a login completely takes both commands.

**Repository pin** — a committed, secret-free `.tracker.toml` naming the profile
a checkout belongs to. Found by walking up from the working directory, the way
git finds `.git`. It exists so that "which organisation am I about to change" is
answered by the repository rather than by remembered global state.

**Profile source** — where the active profile name came from: `--profile`, the
environment, a repository pin, or the configured default. Always reported,
because a change applied to the wrong organisation is expensive to reconstruct
afterwards.

## Tracker entities

**Queue** — where issues live and get their numbers. The `PROJ` in `PROJ-42`.

**Issue** — one task. Identified by its key.

**Link** — a typed edge between two issues: blocks, is blocked by, parent,
subtask, duplicates, relates, epic. The *type* is part of the fact; a link
rendered without it is not useful.

**Project** — a project-management entity that groups issues across queues. Not a
queue, and addressed by its own id rather than by an issue key.

**Goal** — a tracked outcome, linked to projects and to other goals.

**Portfolio** — a container above projects. Out of scope for v1.

**Custom field** — a queue-defined field with an arbitrary key. Hidden from the
compact view by default and pinned explicitly per profile, because the set
differs per queue and an unstable field list defeats caching.

## Output

**Detail ladder** — the deliberate progression from cheapest to fullest:
compact text, then `--fields`, then `--full`, then `--json`, then `--json-raw`.
Callers start cheap and pay for detail only when they need it.

**Untrusted block** — free text originating from Tracker users (summaries,
descriptions, comments), marked as theirs on output: fenced in
`<untrusted src="...">` for a pipe, rendered as markdown behind a margin bar for
a terminal. Neither is sanitisation — the text is passed through unchanged, and
labelled so that whatever reads it can tell content from instruction.

**Tally** — the `shown N of M` line that closes every list. Its absence would let
a caller mistake one page for the whole result set, which is a worse failure than
any number of wasted tokens.

## Risk classes

**Read verb** — `get`, `find`, `count`, `list`, `status`, `show`. Cannot write, by
construction: there is no pass-through verb through which a write could be
smuggled. This is what makes an allowlist like `ytcli issue get:*` meaningful.

**Write verb** — `create`, `update`, `comment`, `transition`, `upload`, `login`,
`logout`, `remove`. A write that touches more than one issue additionally requires `--yes`.
