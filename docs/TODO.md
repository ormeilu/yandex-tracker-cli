# TODO

What is built, what is next, and what was deliberately left out. Scope decisions
live in `docs/adr/`; this file is the running state.

## Done

- [x] Project scaffolding: toolchain, lints, hooks, `just` tasks, CI, release, docs site.
- [x] Layered configuration and profile resolution, with provenance reporting (ADR 2).
- [x] Keychain-backed credential storage, per account (ADR 2).
- [x] HTTP client skeleton: auth and organisation headers, typed errors, retries (ADR 4).
- [x] Compact text renderer for issues and issue pages, with fenced untrusted
      text, fixed field order and the pagination tally (ADR 1, ADR 3).
- [x] Command tree for every v1 entity, with `--help` and shell completions.
- [x] Vertical slice: `ytcli auth status` end to end.
- [x] `ytcli cheatsheet`, compiled into the binary.

## v1

The commands below are declared in the CLI and currently exit with code 64.

### Issues
- [ ] `issue get` — fetch, normalise, render; `--fields` including custom keys.
- [ ] `issue find` — flag-based filters plus `--yql`; pagination, `--all` with `--max`.
- [ ] `issue count` — count only.
- [ ] `issue links`, `issue comments`.
- [ ] `issue create`, `issue update` (`--set key=value`), `issue comment`.
- [ ] `issue transition` — list transitions when the id is omitted.
- [ ] `--dry-run` and `--yes` enforcement on writes (ADR 1).

### Queues, projects, goals
- [ ] `queue list`, `queue fields` — the discovery path for custom field keys.
- [ ] `project list`, `project get`.
- [ ] `goal list`, `goal get`.

### Attachments
- [ ] `attachment list`, `download`, `upload`. Destinations stay explicit: a
      server-supplied filename must not decide where bytes land.

### Output
- [ ] `--format json` over the normalised schema; `--format json-raw`.
- [ ] `--format toon` behind the `toon` feature, with a measurement of what it
      actually saves on issue lists before it is recommended anywhere.
- [ ] Human-facing table rendering via `tabled` when stdout is a terminal.
- [ ] Progress reporting for `--all`, terminal only.

### Testing
- [ ] `wiremock` fixtures from real API responses, seeded from the official
      client's BSD-3 test suite with attribution in `tests/fixtures/NOTICE`.
- [ ] `trycmd` cases so the README and cheatsheet examples are executable.
- [ ] Live suite behind a feature flag and `.env`, ignored by default.

### Distribution
- [ ] First release: binaries for Linux, macOS and Windows plus the maturin wheel.
- [ ] Register the PyPI trusted publisher before the first tag — PyPI matches on
      the workflow filename, `publish.yml`.
- [ ] crates.io publish.
- [ ] Homebrew tap, if anyone asks.

### Agent surface (ADR 6)
- [ ] `SKILL.md` plus per-topic files.
- [ ] Claude Code plugin: the skill and a permission set (read allowed, write prompted).
- [ ] Codex packaging of the same skill.
- [ ] Rewrite `--help` text for density once the commands exist.

## v1.1

- [ ] Worklogs, checklists, link editing.
- [ ] Portfolios.
- [ ] Queue administration, global fields, templates.
- [ ] Boards and sprints.

## Deliberately out of scope

- A second entry point named `yandex-tracker-cli`: it would double the size of
  every release artifact to save typing. The PyPI package keeps that name; the
  command is `ytcli`, and a shell alias covers the rest.
- Any verb that both reads and writes. ADR 1 depends on the split being total.
- Printing a stored token.

## Open questions

- Does `--all` need Tracker's scroll API for result sets beyond ~10k, or is the
  `--max` ceiling enough in practice?
- Is `me` worth special-casing in `--assignee`, or does it hide an extra call?
