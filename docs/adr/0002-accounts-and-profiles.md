# 2. Accounts hold credentials, profiles select organisations

Date: 2026-08-27

## Status

Accepted

## Context

One person needs several identities against Tracker: an admin login and a
restricted one inside the same organisation, and the same personal login across a
work organisation and a private one. So the mapping between credentials and
organisations is many-to-many in both directions.

Two existing models were considered:

* **kubectl contexts** — a global "current context" mutated by `use-context`.
  The failure mode is well known: acting on the wrong cluster because the ambient
  state was not what you remembered.
* **glab** — a token per host, plus inference from the git remote. Closer, but it
  assumes one identity per host, which is exactly the assumption that breaks here.

## Decision

Two entities.

An **account** owns a credential. `auth login` stores one token per account in the
OS keychain. A **profile** is an organisation seen through an account, plus the
display defaults for that context. Several profiles may name the same account.

Selection precedence, highest first: `--profile`, `YTCLI_PROFILE`, the nearest
`.tracker.toml` walking up from the working directory, and the configured
`default_profile`. There is no `use-context`: no command mutates which profile is
active, so nothing can be stale.

The **repository pin** carries no secrets and is meant to be committed, so a
checkout selects its own organisation and an agent working in it lands in the
right place with no setup.

Every command reports which profile it resolved **and where that came from**.

Tokens live in the OS keychain only. If no keychain backend is available the tool
fails with instructions; it never silently falls back to a file. There is also no
command that prints a stored token: a tool whose main consumer is an agent should
not offer secret exfiltration as a feature.

## Consequences

`auth login` is per account, not per profile, so adding a second organisation for
an existing login costs one config entry and no re-authentication. The price is a
config file with two tables instead of one, which is a real cost in explaining the
tool and is why `CONTEXT.md` defines both terms first.
