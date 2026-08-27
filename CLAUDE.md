# Working in this repository

Read `CONTEXT.md` first for the vocabulary; `docs/adr/` records why the awkward
parts are the way they are.

**Planned work lives in GitHub issues**, not in a file in the tree. Before
starting anything, check whether an issue already covers it, and work against
that issue rather than a private plan; when you find something worth doing that
is out of scope for the change at hand, file an issue instead of leaving a TODO
comment. `docs/TODO.md` records only what is already built and what was
deliberately ruled out.

## What this project is

A Yandex Tracker CLI whose reason to exist is cost. Its main consumer is an
agent, and the alternative it replaces — an MCP server — spends tens of thousands
of context tokens before anything is asked. Any change that makes output larger,
less stable, or slower to start is working against the point of the tool.

## Rules that are not negotiable

**Read verbs never write.** `get`, `find`, `count`, `list`, `status` are pure
reads, and no pass-through verb exists through which a write could be reached from
a read command. Agent hosts allowlist read verbs permanently; a single mixed verb
would silently break that for every user. See ADR 1.

**Output shape is a contract.** Field order is fixed. Every renderer has a
snapshot test. If a change alters what callers see, the snapshot diff must be read
and understood, not accepted to make the build green — `just snapshots`.

**Free text from Tracker is fenced, never rewritten.** Descriptions and comments
go out inside `<untrusted src="...">`. Do not add sanitisation that edits the
text: mangling someone's issue is a worse failure than the one being prevented.

**Lists always end with a tally.** `shown N of M`, plus the next page when one
exists. Never signal truncation through the exit code.

**Secrets stay in the keychain.** No plaintext fallback, no command that prints a
token, no token passed as a command-line argument (arguments are visible in `ps`).

**Every write reports which profile and organisation it is about to touch.**

## Conventions

- Everything is in English: code, comments, docs, help text, commit messages.
- `just check` is the gate: `cargo fmt`, `clippy -D warnings`, tests, `cargo deny`.
- Comments explain *why*, and only where the reason is not evident from the code.
  Do not narrate what the next line does.
- `unwrap`, `expect`, `panic!` and direct `println!` are denied by lints in
  library and binary code. Write to `anstream::stdout()` / `stderr()`; logs go to
  stderr through `tracing` so stdout stays pipeable.
- Errors: `thiserror` in the library with variants specific enough to map onto
  distinct exit codes and actionable messages; `anyhow` at the shell.
- Imports are absolute from the crate root.
- New commands go into `src/cli/<entity>.rs` and must be declared in the tree even
  before they work — `not_implemented()` keeps help and completions honest.

## Testing

- Renderers: `insta` snapshots.
- HTTP: `wiremock` against recorded fixtures. Do not write tests that reach the
  real API outside the `live` feature, which is ignored by default and takes its
  credentials from the environment or the configured profile — `just test-live`.
  Live tests answer what fixtures cannot: whether the payload still has the shape
  we believe it has. They do not re-test the parsing that mocks already cover.
- The binary's behaviour, exit codes and help: `tests/cli.rs` with `assert_cmd`.
- A test that only restates the implementation is not worth its maintenance. Test
  the promises: exit codes, field order, tallies, fencing, profile provenance.

## When adding a dependency

`cargo deny` gates licences and advisories. Prefer no new dependency over a small
one; release builds optimise for size and startup because the binary runs far more
often than it is built.
