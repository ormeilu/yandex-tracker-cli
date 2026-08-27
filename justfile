# Project tasks. `just` with no arguments lists them.
# Everything runs through cargo; no environment to activate.

set shell := ["bash", "-uc"]
set dotenv-load := true

cargo := "cargo"

default:
    @just --list --unsorted

# --- environment --------------------------------------------------------------

# Install the development tooling and the git hooks
install:
    {{cargo}} install cargo-nextest cargo-llvm-cov cargo-deny cargo-insta --locked
    prek install

# Refresh Cargo.lock
lock:
    {{cargo}} update

# --- checks -------------------------------------------------------------------

# Everything CI runs: format, lints, tests, dependency audit
check: fmt-check lint test deny

# rustfmt, no changes
fmt-check:
    {{cargo}} fmt --all --check

# rustfmt, with changes
fmt:
    {{cargo}} fmt --all

# clippy, warnings are errors
lint:
    {{cargo}} clippy --all-targets --all-features -- -D warnings

# clippy with autofix, then format
fix:
    {{cargo}} clippy --all-targets --all-features --fix --allow-dirty --allow-staged
    {{cargo}} fmt --all

# Tests
test *ARGS:
    {{cargo}} nextest run --all-features {{ARGS}}

# Tests without the live suite (the default; live needs real credentials)
test-fast:
    {{cargo}} nextest run --all-features

# Tests against a real organisation. Needs .env; creates issues in YTCLI_TEST_QUEUE
test-live:
    {{cargo}} nextest run --all-features --features live -- --ignored

# Coverage report
cov:
    {{cargo}} llvm-cov --all-features --html
    @echo "report: target/llvm-cov/html/index.html"

# Review snapshot changes: the output format is the product, so these are read, not rubber-stamped
snapshots:
    {{cargo}} insta review

# Dependency advisories, licences, duplicates
deny:
    {{cargo}} deny check

# All prek hooks over the whole tree
hooks:
    prek run --all-files

# Look for leaked secrets in the tree and in history
secrets:
    prek run gitleaks --all-files

# --- build and run ------------------------------------------------------------

# Debug build
build:
    {{cargo}} build

# Release build, as shipped
release:
    {{cargo}} build --release

# Run the CLI: just run issue get PROJ-1
run *ARGS:
    {{cargo}} run --quiet -- {{ARGS}}

# Regenerate shell completions into dist/completions
completions:
    mkdir -p dist/completions
    for sh in bash zsh fish powershell; do \
      {{cargo}} run --quiet -- completions $sh > dist/completions/ytcli.$sh; \
    done
    @echo "written to dist/completions/"

# --- docs ---------------------------------------------------------------------

# Build the mdBook site
docs:
    mdbook build

# Serve the docs with live reload
docs-serve:
    mdbook serve --open

# Open work, from GitHub issues
todo *ARGS:
    gh issue list --limit 50 {{ARGS}}
