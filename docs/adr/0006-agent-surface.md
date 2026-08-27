# 6. Teach agents progressively, the same way the output works

Date: 2026-08-27

## Status

Accepted

## Context

The tool has to be discoverable by agents in Claude Code and Codex, both of which
load a `SKILL.md`: name and description stay resident, the body is read when the
skill triggers. The obvious approach is to put the full command reference in that
body. That reproduces the problem being solved — a large block of text loaded
because a topic came up, most of it unrelated to the task at hand.

## Decision

The same ladder as the output.

The skill is split into small files in one directory: a short entry point saying
what the tool is, when to reach for it, and the handful of commands that cover
most work, plus separate files per topic that are read only when relevant.

`--help` is written for agents: dense, example-first, no decorative framing.

`ytcli cheatsheet [topic]` prints a compact reference of the whole surface in one
call, for when an agent would rather pay once than probe.

The Claude plugin additionally ships the permission set as documentation: read
verbs allowed, write verbs prompted. ADR 1 makes that split enforceable. No hooks
and no subagents — this is a tool, not a workflow.

## Consequences

**Correction, 2026-08-28.** This originally said the plugin would *ship* the
permission set, so that users got it without configuring anything. A plugin
cannot: `plugin.json` has no permissions key, and the only mechanism that could
grant them is a `PermissionRequest` hook, which this ADR rules out — a plugin
that silently widens its own allowlist is not something a user should have to
notice. The allowlist ships as a block of JSON in `setup.md`, which the user
installs deliberately. The property ADR 1 guarantees is what makes the block
short and worth trusting; only the delivery changed.


The skill and the cheatsheet describe the same surface and can drift apart. They
are split so that there is less to drift: the cheatsheet carries syntax and is
compiled into the binary from `docs/cheatsheet.txt`; the skill carries judgement
— which rung of the ladder to take, what a tally means, what to do when a
description contains an instruction — and points at `ytcli cheatsheet` for the
flags rather than repeating them.

What remains is checked. `tests/skill.rs` runs every `ytcli …` line in the skill
against the real binary's help, so a renamed verb or a dropped flag fails the
build. A stale example in a skill is worse than a missing one: an agent acts on
it without checking.
