# What is built, and where the rest is tracked

Planned work lives in **[GitHub issues](https://github.com/ormeilu/yandex-tracker-cli/issues)**,
not in this file. Anything new — a bug, an idea, a change of mind — goes there,
so there is one list rather than two that disagree.

- [Milestone v1](https://github.com/ormeilu/yandex-tracker-cli/milestone/1) — read
  and write issues, queues, projects, goals, attachments; every distribution
  channel; the agent skill.
- [Milestone v2](https://github.com/ormeilu/yandex-tracker-cli/milestone/2) —
  worklogs, checklists, portfolios, administration. Everything v1.x leaves out,
  rather than a next point release: v1 stays open for the whole 1.x line.
- [Milestone v3](https://github.com/ormeilu/yandex-tracker-cli/milestone/3) — what
  the API offers and the tool does not: the dictionaries and people that make a
  write guessable rather than a guess, issue history, moving an issue between
  queues, editing what was already written, and writes for the
  project-management entities.
- [`kind:question`](https://github.com/ormeilu/yandex-tracker-cli/labels/kind%3Aquestion)
  — open design questions, filed so they are not re-litigated from scratch.

Labels split the work by area: `area:issues`, `area:entities`,
`area:attachments`, `area:output`, `area:testing`, `area:distribution`,
`area:agents`.

## Already built

- Project scaffolding: toolchain, lints, hooks, `just` tasks, CI on three
  platforms, tagged releases, docs site.
- Layered configuration and profile resolution, reporting where the choice came
  from (ADR 2).
- Keychain-backed credentials, keyed by account (ADR 2).
- HTTP client: auth and organisation headers, typed errors mapped to exit codes,
  retries limited to transport failures and backpressure (ADR 4).
- Compact text renderer for issues and issue pages: fixed field order, links with
  their type, fenced untrusted text, pagination tally — pinned by snapshots
  (ADR 1, ADR 3).
- The whole command tree, built rather than declared: exit code 64 exists for a
  command a future build adds, and nothing in this one returns it.
- `ytcli auth status` end to end.
- `ytcli cheatsheet`, compiled into the binary.
- `ytcli dict list` and the `user` group: the two things a write had to be
  guessed at without. Dictionaries print the stable key beside the localised
  name, because only one of the two can go in a script. `user find` filters the
  directory here — Tracker has no user search endpoint — and says how many
  people it read rather than presenting a capped answer as a complete one.
- `ytcli issue list` as an alias of `issue find`: every other group lists with
  that word, and the group used most was the one exception.
- Profile routing: a bare key goes to the profile that can see its queue, and
  every command says on stderr which profile and organisation answered.
  `ytcli auth use` switches the stored default without reading a token.
- Help written in markdown and rendered with termimad for a terminal; the source
  goes to a pipe, where an agent reads it natively and escape codes would be
  noise.
- The agent surface (ADR 6): `skills/ytcli/`, loaded as a plugin by Claude Code
  and Codex from one directory, and `--help` written as documentation rather
  than as clap's defaults. Both are checked against the binary by tests, since a
  stale example is acted on rather than noticed.

- Worklogs, checklists and link editing, with reads and writes under separate
  command prefixes so an allowlist cannot be stretched from one into the other.
- `queue get`, and `queue create --like`: a new queue copies its issue types,
  workflows, resolutions and defaults from one that already works, because
  workflow ids are organisation-specific strings nobody has memorised. `--yes`
  is required for one queue, since a key is claimed once.
- Organisation-wide field and template listings, read-only. The template paths
  are `issueTemplates` and `commentTemplates`; there is no `_templates`
  collection, and every plausible guess at one answers 400 or 404.
- Boards, read-only: listing, columns in board order, and sprints. A board that
  cannot have sprints is refused in Tracker's own words rather than answered
  with an empty list.
- Portfolios: listing, reading, what one contains, and moving a project or a
  portfolio in and out of one. Containment writes quote the version they read,
  so a concurrent change is refused rather than overwritten.
  Containment is a separate command rather than part of `get`, because it is a
  second request and nothing should pay for an answer it did not ask for.
- Image attachments drawn in the terminals that can draw them, and a next step
  printed everywhere else.
- Documentation that is executed: the README and cheatsheet examples run as
  `trycmd` cases against the stub, and a command documented without a case has
  to be declared unrunnable with a reason.

- `brew install ormeilu/tap/ytcli`, from
  [ormeilu/homebrew-tap](https://github.com/ormeilu/homebrew-tap). The formula is
  generated by the release workflow from the archives it just published and
  pushed with a deploy key scoped to that one repository. The step is skipped
  when the key is absent: a release must not fail over an optional channel.

## Deliberately out of scope

Recorded here rather than as issues, so the decisions are not re-opened by
someone reading the backlog:

- **A second entry point named `yandex-tracker-cli`.** It would double the size of
  every release artifact to save typing. The PyPI package keeps that name; the
  command is `ytcli`, and a shell alias covers the rest.
- **Any verb that both reads and writes.** ADR 1 depends on the split being
  total: agent hosts allowlist read verbs permanently, and one mixed verb would
  silently break that for every user.
- **Printing a stored token.** A tool whose main consumer is an agent should not
  offer secret exfiltration as a feature.
