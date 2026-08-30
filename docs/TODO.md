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
  sweep found missing; then a study of another Tracker CLI, which added bars,
  a timer, `sprint get`, transition by status and two ways to stop fighting the
  shell. Shipped across 0.7.0 and 1.0.0.
- 1.0.1, from issues filed by people using it: `--help` and
  `ytcli cheatsheet more` carry the docs, source and bug-tracker URLs, because a
  binary installed through `uv`, `cargo` or a tap arrives detached from its
  repository and `--help` is then the whole project; and the skill says which
  markup a description is written in — Markdown, not the Yandex wiki markup
  whose `#` numbers a list. The skill also offers to file an issue whenever the
  tool surprises somebody, and `reporting.md` says what to strip out of a report
  before it becomes a public document. The README's example of what a person
  sees was the machine rendering, fence tags and all — a person sees markdown
  behind a margin bar, and now that is what it shows.
- What `markupType` decides, settled live: the default is `wf`, which honours
  the old wiki spellings *as well as* Markdown, and `md` is the stricter mode
  that drops them. So this sends neither, and `#` heads a section under both —
  which is the only part a caller has to know.
- A fixture audit, `every_fixture_still_has_the_shape_tracker_answers_with`.
  The fixtures were written from an upstream client's documented shapes rather
  than recorded, so the mocked suite is only as true as they are; the audit asks
  the real API once per endpoint and fails on a key a fixture claims and Tracker
  no longer returns. It prints what it could **not** check rather than passing
  quietly: an organisation with no sprints leaves the sprint fixture unverified,
  and that is a fact about the run rather than a success. `tests/fixtures/NOTICE`
  names the seven that remain only borrowed, and issue #78 is where closing that
  gap is tracked.
- **1.0.0** — no new surface of its own. It is the version that says the surface
  is finished: every endpoint a survey of the API found is reachable, the output
  shape has been stable for several releases, and what was deliberately left out
  is written down below rather than pending. The number is a promise about
  breakage from here, not a claim that there is nothing left to build.
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
- Milestone 0.7.0: `link types` prints the vocabulary a write takes beside
  the type ids it is not — a distinction that turned out to be encoded backwards
  in the parser, the fixture and a unit test at once. `queue access` answers who
  may do what in a queue, in two tables because Tracker gives two answers: the
  rule, roles and all, and the people that rule resolves to. Only the second can
  say whether the caller is one of them, and it says `?` rather than `no` when
  the token could not name its own user.
  `--format json` carries `status_key` and `priority_key` beside the displayed
  status and priority, and the terminal colours a status by that key: a Russian
  organisation answers `Закрыт`, and everything that matched on `closed` matched
  nothing and said so in no way at all. A bare issue number is completed from
  the profile's default queue.
  `attachment delete` closes the one gap in the sweep of the API that a user
  hits by accident, naming the file rather than the id it is about to lose.
  `issue update` over several issues is one request rather than one each, which
  turned out to be the safer shape as well as the cheaper one: Tracker validates
  the whole list before it writes, so an unknown key refuses the change instead
  of leaving half of it applied. It answers `changed N of M` and names every
  issue that did not change; `ytcli bulk status` reads the result back later.
  `issue transition` and `issue move` take a list on the same terms, verified
  live to be refused entire when one key or the target queue does not exist. A
  list of keys leaves no unambiguous place for a bare transition id, so with
  several issues the transition is named with `--to`.
- `ytcli issue list` as an alias of `issue find`: every other group lists with
  that word, and the group used most was the one exception.
- Profile routing: a bare key goes to the profile that can see its queue, and
  every command says on stderr which profile and organisation answered.
  `ytcli auth use` switches the stored default without reading a token, and
  `ytcli auth edit` changes an existing profile — its name, the organisation it
  points at, and the free-text note saying which organisation that is — on the
  same terms. The note rides along on that stderr line, because an organisation
  id is a number nobody recognises.
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
- **Creating versions, sprints, fields and boards.** A sweep of the API for
  verbs this tool does not have turned up five, and one of them — deleting an
  attachment — was worth building, because `attachment upload` had no undo and
  the mistake is one anybody makes. The other four are all the configuration of
  a queue rather than work in it: `POST /v3/queues/{key}/versions` and the
  version writes, `POST /v3/sprints` and the board writes, `POST /v3/fields` and
  `POST /v3/queues/{key}/localFields`. They are done once, by somebody with the
  rights to do them, in an interface that shows what the choice affects — the
  same argument that keeps triggers read-only here. A board is a view of work
  and not the work, and `board` and `sprint` stay read-only for that reason.
- **Box-drawn tables, and wrapping a cell instead of cutting it.** Both come
  from looking at another Tracker CLI to see what was worth taking. Borders cost
  tokens on every row and stop the output being `cut` and `grep`ed; a wrapped
  cell breaks one-line-per-issue, which is the property that makes a listing
  greppable at all. What was worth taking from that comparison was colouring by
  the status *key* rather than the words, and completing a bare issue number
  from the default queue — both are in.
- **A JSON error envelope on stderr under `--format json`.** The exit code
  already carries the class of failure — 2, 3, 4, 5 mean distinct things and are
  the documented contract — and a second, parseable copy of the message would be
  one more shape to keep stable for no question it answers.
- **Deleting a queue.** Tracker deletes a queue by hiding it, so the key stays
  spent and no `--yes` buys it back. `queue create` already asks for exactly
  that reason; a delete would be the same irreversible act with less to show
  for it.
