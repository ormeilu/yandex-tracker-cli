# What is built, and where the rest is tracked

Planned work lives in **[GitHub issues](https://github.com/ormeilu/yandex-tracker-cli/issues)**,
not in this file. Anything new — a bug, an idea, a change of mind — goes there,
so there is one list rather than two that disagree.

**A milestone is named after the release that carries it**, and is closed when
that release is tagged. The names drifted once — milestones called `v1`…`v4`
against versions `0.1.0`…`0.6.0`, which read as major versions and were not — so
they were renamed to the versions they actually shipped in. An issue blocked on
something outside this repository sits on **no** milestone: a schedule it cannot
meet is how the drift started.

- [0.2.0](https://github.com/ormeilu/yandex-tracker-cli/milestone/1) — read and
  write issues, queues, projects, goals, attachments; every distribution
  channel; the agent skill. Shipped across 0.1.0 and 0.2.0.
- [0.3.0](https://github.com/ormeilu/yandex-tracker-cli/milestone/2) — worklogs,
  checklists, portfolios, administration.
- [0.5.0](https://github.com/ormeilu/yandex-tracker-cli/milestone/3) — what the
  API offered and the tool did not: the dictionaries and people that make a
  write guessable rather than a guess, issue history, moving an issue between
  queues, editing what was already written, and writes for the
  project-management entities. Shipped across 0.4.0 and 0.5.0.
- [0.6.0](https://github.com/ormeilu/yandex-tracker-cli/milestone/4) — the
  endpoints a survey of the API found unused, and the query language written
  down against a live Tracker rather than from memory.
- [0.7.0](https://github.com/ormeilu/yandex-tracker-cli/milestone/6) — the rest
  of the Tracker API: components, link types, queue access, and the writes a
  sweep found missing. **1.0.0 waits on this**, because a tool that claims a
  version 1 for a Tracker CLI should not still make a caller guess at what a
  component field takes.
- [Yandex Wiki](https://github.com/ormeilu/yandex-tracker-cli/milestone/5) —
  whether the other half of an organisation's writing belongs behind this binary
  at all. Not a version: research first, and the answer may be no.
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
- `issue changelog`, `issue move`, editing comments and worklogs, `worklog find`
  across issues, `queue versions` and `queue tags`, and create/update/delete for
  projects, portfolios and goals — the whole of milestone v3 apart from making
  the tool work inside Claude Cowork, which needs a session there to answer.
- `ytcli dict list` and the `user` group: the two things a write had to be
  guessed at without. Dictionaries print the stable key beside the localised
  name, because only one of the two can go in a script. `user find` filters the
  directory here — Tracker has no user search endpoint — and says how many
  people it read rather than presenting a capped answer as a complete one.
- Milestone 0.6.0: `skills/ytcli/yql.md`, every query on it sent to a real Tracker
  before it was written down — which is how `StoryPoints` turned out not to be a
  filter name while `"Story Points"` is. `field get` says what a field accepts,
  naming the command that lists the values when they live elsewhere.
  `issue remotelinks` shows what an issue is attached to outside Tracker.
  `queue automation` reports macros, autoactions and triggers, and says which
  section it was refused rather than counting it zero. `sprint list` and
  `queue local-fields` close the two listings that had no way in.
- Milestone 0.7.0 so far: `link types` prints the vocabulary a write takes beside
  the type ids it is not — a distinction that turned out to be encoded backwards
  in the parser, the fixture and a unit test at once. `queue access` answers who
  may do what in a queue, in two tables because Tracker gives two answers: the
  rule, roles and all, and the people that rule resolves to. Only the second can
  say whether the caller is one of them, and it says `?` rather than `no` when
  the token could not name its own user.
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
