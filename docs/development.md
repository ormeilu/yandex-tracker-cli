# Development

```bash
just install     # tooling and git hooks
just check       # fmt, clippy -D warnings, tests, cargo-deny
just run issue get PROJ-1
just snapshots   # review output-format changes
just docs-serve  # this site, with live reload
```

`just` with no arguments lists everything.

## Installing your own build

```bash
just signing-identity   # once per machine
just local-install      # build, install, sign
```

macOS binds a Keychain approval to a code signature, and Cargo ad-hoc signs
through the linker, so an unsigned local build asks for your password after
every `cargo install` — correctly, since to macOS it is a new application each
time. A self-signed code-signing certificate makes the approval hold. See
[Configuration](configuration.md) for the details and how to undo it.

If you would rather keep the keychain out of it entirely while working:

```bash
just dev-token ACCOUNT   # copies the token into .env
just run issue get PROJ-1
```

`YTCLI_TOKEN` is checked before the keychain is opened at all, and `just` loads
`.env` on its own. This is a plaintext token on disk — gitignored and mode 600,
but readable by anything running as you. It is the trade the tool refuses to
make for users, made deliberately for one machine; use an account whose rights
you would not mind losing.

`just test-live` runs a small suite against a real organisation, one test at a
time. It exists for the class of bug fixtures cannot catch — three shipped past
the mocked suite because the fixtures encoded the same wrong beliefs the code
did. Reads need credentials; the one write test only runs when `YTCLI_TEST_QUEUE`
names a queue, because Tracker has no delete.

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
- The documented examples — `trycmd` in `tests/docs/`, run against the same stub.
  Refresh them with `TRYCMD=overwrite cargo test --test docs`, then read the
  diff. A newly documented command must get a case there or be listed as
  unrunnable in `tests/docs.rs`, with the reason.
- Live tests need real credentials, are behind the `live` feature, and are
  ignored by default: `just test-live` with a populated `.env`.

Test the promises — exit codes, field order, tallies, fencing, profile provenance
— not the implementation restated.

## Hooks

`prek` runs formatting, clippy, secret scanning and a check for unreviewed
snapshots before each commit. `just hooks` runs the lot over the whole tree.
