# 4. Talk to the API directly rather than through the official client

Date: 2026-08-27

## Status

Accepted

## Context

`yandex-tracker-client` 2.10 is Yandex's own Python client and was the initial
dependency. Reading what it actually provides:

* It is built on `requests`, synchronous, and would sit alongside the HTTP stack
  the rest of the tool needs.
* Responses decode into dynamically constructed objects. No types, so nothing a
  type checker can verify, and the normalised schema promised by ADR 3 has to be
  written on top regardless.
* It exposes seven collections: attachments, users, queues, issues, issue types,
  boards, sprints. **Projects and goals are absent**, and both are in scope for
  v1 — half the surface would be written outside the library anyway.
* It sets the organisation header itself, defaulting it to the literal string
  `not provided`. The account/profile model in ADR 2 decides that header, so the
  library's behaviour has to be worked around rather than used.
* Pagination and scroll are not surfaced in a usable way, and pagination is the
  substance of ADR 3's tally requirement.

## Decision

Write a thin HTTP layer directly against the REST API, with typed errors and
explicit control over headers, retries and pagination.

The choice of implementation language followed from this. Once the library was
gone, nothing tied the tool to Python, while two properties of a compiled binary
matter to both audiences: process start is milliseconds rather than the few
hundred that importing a Python CLI stack costs — paid back on every one of the
dozens of invocations in an agent session — and distribution is a single file with
no runtime to install. See ADR 5.

## Consequences

We own compatibility when the Tracker API changes, which is a real ongoing cost
and the strongest argument the other way. Recorded fixtures of real responses
(reused from the official client's BSD-3 licensed test suite, with attribution)
make that cost visible: a shape change breaks a test rather than a user.
