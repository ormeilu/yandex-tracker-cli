# Decisions

Architecture decision records: the reasoning behind the parts of this tool that
look odd until you know why.

- [1. Security model](0001-security-model.md) — why the risk marker is on verbs
  and on output rather than on the query language.
- [2. Accounts and profiles](0002-accounts-and-profiles.md) — why credentials and
  organisations are separate entities, and why there is no `use-context`.
- [3. Output ladder](0003-output-ladder.md) — why the default view is terse, its
  field order fixed, and every list ends with a tally.
- [4. Own HTTP client](0004-own-http-client.md) — why the official client was
  dropped.
- [5. Rust and a Python wheel](0005-rust-with-a-python-wheel.md) — why a compiled
  binary, and why it is still installable with `uvx`.
- [6. Agent surface](0006-agent-surface.md) — why the skill is split into small
  files instead of one large reference.
