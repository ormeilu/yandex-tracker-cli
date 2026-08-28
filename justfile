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
    # A throwaway password, because macOS refuses to verify the MAC on a
    # PKCS#12 written with an empty one. It never leaves this shell, and the
    # bundle it protects is deleted a few lines below.
    pw=$(openssl rand -hex 16)
    openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" \
        -name ytcli-dev -out "$work/id.p12" -passout "pass:$pw"
    keychain="$HOME/Library/Keychains/login.keychain-db"
    # -T grants codesign, and only codesign, use of the key without a prompt.
    security import "$work/id.p12" -k "$keychain" -P "$pw" -T /usr/bin/codesign
    # Trusted for code signing alone: this certificate must not become something
    # that can vouch for a website or an email.
    security add-trusted-cert -r trustRoot -p codeSign -k "$keychain" "$work/cert.pem"
    security find-identity -v -p codesigning | grep ytcli-dev
    echo "done — now run: just local-install"

# Build and install the binary locally, signed so macOS stops asking
local-install:
    #!/usr/bin/env bash
    set -euo pipefail
    # --force because the point of this recipe is to replace the copy on the
    # PATH; without it cargo refuses as soon as one is already there.
    {{cargo}} install --path . --locked --force
    if [ "$(uname)" != "Darwin" ]; then exit 0; fi
    if security find-identity -v -p codesigning | grep -q ytcli-dev; then
        codesign --force --sign ytcli-dev "$(command -v ytcli)"
        # The requirement is what the Keychain records an approval against, so
        # printing it is how you tell a stable identity from an ad-hoc one that
        # will ask again tomorrow.
        codesign -d -r- "$(command -v ytcli)" 2>&1 | grep '^designated' || true
        echo "signed with ytcli-dev; the Keychain approval survives the next build"
    else
        echo "unsigned: macOS will ask for your password again after each build"
        echo "run \`just signing-identity\` once to stop that"
    fi

# Copy a stored token into .env, so local runs skip the keychain entirely
dev-token account:
    #!/usr/bin/env bash
    # This writes a token to a file in plaintext. It is the trade ADR 2 refuses
    # to make for users, made deliberately for one developer machine: .env is
    # gitignored, mode 600, and read only by `just`. Any process running as you
    # can still read it, so use an account whose rights you would not mind
    # losing, and delete the line when you are done with it.
    set -euo pipefail
    if [ "$(uname)" != "Darwin" ]; then
        echo "macOS only; on Linux read the token out with secret-tool" >&2
        exit 1
    fi
    umask 077
    token=$(security find-generic-password -s ytcli -a "{{account}}" -w)
    touch .env
    grep -v "^YTCLI_TOKEN=" .env > .env.tmp || true
    printf "YTCLI_TOKEN=%s\n" "$token" >> .env.tmp
    mv .env.tmp .env
    chmod 600 .env
    echo "YTCLI_TOKEN written to .env"
    echo "  just run issue get PROJ-1     reads it already"
    echo "  set -a; source .env; set +a   for a bare ytcli in this shell"

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
test *ARGS: && (sign "target/debug/ytcli")
    {{cargo}} nextest run --all-features {{ARGS}}

# Tests without the live suite (the default; live needs real credentials)
test-fast: && (sign "target/debug/ytcli")
    {{cargo}} nextest run --all-features

# Tests against a real organisation. Needs credentials; writes only if YTCLI_TEST_QUEUE
#
# One at a time on purpose: Tracker rate-limits, and a suite that fails on its
# own concurrency reports its own noise rather than the API's behaviour.
test-live:
    {{cargo}} test --all-features --test live -- --ignored --test-threads=1

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

# Debug build, signed so the Keychain approval survives it
build: && (sign "target/debug/ytcli")
    {{cargo}} build

# Release build, as shipped
release: && (sign "target/release/ytcli")
    {{cargo}} build --release

# Run the CLI: just run issue get PROJ-1
run *ARGS: build
    ./target/debug/ytcli {{ARGS}}

# Sign a locally built binary with the ytcli-dev identity, if there is one
#
# Cargo links an ad-hoc signature that changes with every build, and the
# Keychain grants "Always Allow" to a signature rather than to a path — so
# without this, every rebuild is a new application asking for the password
# again. Signing with a stable identity makes the approval outlive the build.
#
# Silent when there is no identity or no such binary: this hangs off `build`,
# and a build must not fail because a convenience is not set up.
[private]
sign BINARY:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" != "Darwin" ] || [ ! -f "{{BINARY}}" ]; then exit 0; fi
    if security find-identity -v -p codesigning 2>/dev/null | grep -q ytcli-dev; then
        codesign --force --sign ytcli-dev "{{BINARY}}" 2>/dev/null
    fi

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
