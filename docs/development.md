# Development

```bash
just install     # tooling and git hooks
just check       # fmt, clippy -D warnings, tests, cargo-deny
just run issue get PROJ-1
just snapshots   # review output-format changes
just docs-serve  # this site, with live reload
```

`just` with no arguments lists everything.

Planned work is tracked in
[GitHub issues](https://github.com/ormeilu/yandex-tracker-cli/issues); pick one
there rather than inventing a plan, and file a new issue for anything you find on
the way.

## Layout

```
src/
  main.rs        entry point: parse, resolve a profile, dispatch, exit code
  cli/           one file per entity; the command tree
  config/        layered profile resolution and per-OS paths
  secrets.rs     keychain access
  api/           HTTP client, typed errors, normalised models
  render/        the output ladder — this is the product
docs/            these pages, the ADRs, the TODO list, the cheatsheet
```

## The rules that bite

**Read verbs never write.** Agent hosts allowlist them permanently; one mixed verb
breaks that for every user (ADR 1).

**Output shape is a contract.** Field order is fixed, every renderer has a
snapshot test. When a snapshot changes, read the diff — `just snapshots` — rather
than accepting it to make the build green.

**`unwrap`, `expect`, `panic!` and `println!` are denied by lint** in library and
binary code. Write to `anstream::stdout()`; logs go to stderr via `tracing`, so
stdout stays pipeable.

**Secrets never reach a file, an argument or stdout.**

## Tests

- Renderers — `insta` snapshots.
- HTTP — `wiremock` against recorded fixtures.
- The binary, its exit codes and help — `assert_cmd` in `tests/cli.rs`.
- Live tests need real credentials, are behind the `live` feature, and are
  ignored by default: `just test-live` with a populated `.env`.

Test the promises — exit codes, field order, tallies, fencing, profile provenance
— not the implementation restated.

## Hooks

`prek` runs formatting, clippy, secret scanning and a check for unreviewed
snapshots before each commit. `just hooks` runs the lot over the whole tree.
