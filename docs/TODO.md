# What is built, and where the rest is tracked

Planned work lives in **[GitHub issues](https://github.com/ormeilu/yandex-tracker-cli/issues)**,
not in this file. Anything new — a bug, an idea, a change of mind — goes there,
so there is one list rather than two that disagree.

- [Milestone v1](https://github.com/ormeilu/yandex-tracker-cli/milestone/1) — read
  and write issues, queues, projects, goals, attachments; both distribution
  channels; the agent skill.
- [Milestone v1.1](https://github.com/ormeilu/yandex-tracker-cli/milestone/2) —
  worklogs, checklists, portfolios, administration.
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
- The full v1 command tree, so help and completions are honest; unbuilt verbs
  exit with code 64.
- `ytcli auth status` end to end.
- `ytcli cheatsheet`, compiled into the binary.
- The agent surface (ADR 6): `skills/ytcli/`, loaded as a plugin by Claude Code
  and Codex from one directory, and `--help` written as documentation rather
  than as clap's defaults. Both are checked against the binary by tests, since a
  stale example is acted on rather than noticed.

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
