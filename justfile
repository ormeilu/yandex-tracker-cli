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

# Create the local code-signing identity macOS needs (once per machine)
signing-identity:
    #!/usr/bin/env bash
    # A binary built by Cargo is ad-hoc signed by the linker, so its signature
    # changes on every build. The Keychain grants "Always Allow" to a signature,
    # not to a path, which is why a locally built ytcli asks for a password after
    # every rebuild — correctly: to macOS it really is a new application.
    #
    # A self-signed code-signing certificate fixes that, because the approval is
    # then tied to the certificate rather than to the bytes. This creates one and
    # trusts it for code signing only. macOS will ask for your password twice —
    # once to trust it, once when codesign first uses the key — and then stop.
    #
    # Undo with: security delete-certificate -c ytcli-dev
    set -euo pipefail
    if [ "$(uname)" != "Darwin" ]; then echo "macOS only; nothing to do"; exit 0; fi
    if security find-identity -v -p codesigning | grep -q ytcli-dev; then
        echo "ytcli-dev already exists"; exit 0
    fi
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    # The private key lives in the keychain from here on; these files do not
    # outlive the command.
    openssl req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes \
        -keyout "$work/key.pem" -out "$work/cert.pem" -subj "/CN=ytcli-dev" \
        -addext "basicConstraints=critical,CA:false" \
        -addext "keyUsage=critical,digitalSignature" \
        -addext "extendedKeyUsage=critical,codeSigning" 2>/dev/null
    openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" \
        -name ytcli-dev -out "$work/id.p12" -passout pass:
    keychain="$HOME/Library/Keychains/login.keychain-db"
    # -T grants codesign, and only codesign, use of the key without a prompt.
    security import "$work/id.p12" -k "$keychain" -P "" -T /usr/bin/codesign
    # Trusted for code signing alone: this certificate must not become something
    # that can vouch for a website or an email.
    security add-trusted-cert -r trustRoot -p codeSign -k "$keychain" "$work/cert.pem"
    security find-identity -v -p codesigning | grep ytcli-dev
    echo "done — now run: just local-install"

# Build and install the binary locally, signed so macOS stops asking
local-install:
    #!/usr/bin/env bash
    set -euo pipefail
    {{cargo}} install --path . --locked
    if [ "$(uname)" != "Darwin" ]; then exit 0; fi
    if security find-identity -v -p codesigning | grep -q ytcli-dev; then
        codesign --force --sign ytcli-dev "$(command -v ytcli)"
        echo "signed with ytcli-dev; the Keychain approval survives the next build"
    else
        echo "unsigned: macOS will ask for your password again after each build"
        echo "run \`just signing-identity\` once to stop that"
    fi

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
