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
did. Reads need credentials; the write tests only run when `YTCLI_TEST_QUEUE`
names a queue, because Tracker has no delete.

Two of them are about the fixtures rather than the code.
`every_fixture_still_has_the_shape_tracker_answers_with` checks each fixture
against the endpoint it came from and prints what it could not reach;
`one_of_everything_the_fixture_audit_cannot_otherwise_reach` makes the things no
organisation has by default — a queue version, a component, a local field, a
board with a sprint, a template, a link out of Tracker, a required field — so
that the audit reaches them at all. Point them at a scratch queue:

```bash
YTCLI_PROFILE=personal YTCLI_TEST_QUEUE=YTLIVE just test-live
```

`tests/fixtures/NOTICE` records which shapes have been confirmed that way and
what the runs turned up that the API reference does not say.

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

## Releasing

A tag starting `v` builds the binaries and wheels, publishes to crates.io and
PyPI through Trusted Publishing, attaches the archives to a GitHub release, and
regenerates the Homebrew formula in `ormeilu/homebrew-tap`.

Every credential-dependent step is skipped when its secret is absent rather than
failing the release: a fork can build the whole thing, and one optional channel
must not take the rest down with it.

| Secret | Used for |
|:-|:-|
| `TAP_DEPLOY_KEY` | pushing the generated formula to the tap |
| `APPLE_CERTIFICATE_P12` | base64 of a Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the password that `.p12` was exported with |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Name (TEAMID)` |
| `APPLE_API_KEY_P8` | base64 of an App Store Connect key, for notarising |
| `APPLE_API_KEY_ID` | that key's id |
| `APPLE_API_ISSUER` | the issuer id it belongs to |

The Apple half exists so the macOS Keychain's "Always Allow" survives a version
upgrade: the approval is tied to the signing identity, and an ad-hoc signature —
which is what Cargo's linker leaves — changes with every build. Notarisation is
the other half of the same story, so Gatekeeper does not quarantine a download.

The notarisation ticket binds to the binary's own hash, which is why only a zip
of the executable is submitted while the release still ships a tarball:
`stapler` staples bundles and installers, not bare executables, and the ticket
applies to the binary wherever it ends up.

Locally, `just signing-identity` creates a self-signed `ytcli-dev` certificate
for the same reason, and `just build`, `just test`, `just run` and
`just local-install` sign what they produce with it. A bare `cargo build` does
not, which is worth remembering when the Keychain starts asking again.
