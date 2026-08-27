# 5. Rust, distributed as both a binary and a Python wheel

Date: 2026-08-27

## Status

Accepted

## Context

With the official client dropped (ADR 4), the implementation language was open.
The tool has two audiences with different needs: agents invoke it dozens of times
per session, where process start dominates; people install it once, where not
having to think about a runtime dominates.

A Python implementation starts in a few hundred milliseconds after importing its
CLI, HTTP and model libraries. Across thirty invocations that is most of a minute
spent on imports by a tool whose entire purpose is to be cheap. It also drags a
Python installation into every environment where an agent might run.

The counterweight was `uvx`: a Python package can be run without installing
anything, which is a genuinely low-friction path for the audience that already
has uv.

## Decision

Rust, published through both channels.

The binary ships in GitHub Releases and on crates.io. The same binary is also
packaged as a Python wheel via maturin (`bindings = "bin"`), so
`uvx yandex-tracker-cli` keeps working — the wheel carries no Python code.

Supporting crates map onto the earlier decisions: `clap` for the command tree,
`figment` for layered configuration (ADR 2), `keyring` for per-account
credentials (ADR 2), `insta` and `trycmd` for the output contract (ADR 3),
`wiremock` for the API layer (ADR 4).

Release builds optimise for size and startup rather than compile time, since the
binary is executed far more often than it is built.

## Consequences

Releases need cross-compilation for Linux, macOS and Windows, which the release
workflow handles. Contributors need a Rust toolchain, which `rust-toolchain.toml`
pins to stable with no MSRV: this is an end-user tool, so nothing downstream
breaks when it moves forward.
