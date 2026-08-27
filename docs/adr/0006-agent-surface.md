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

The Claude plugin additionally ships a permission set: read verbs allowed, write
verbs prompted. ADR 1 makes that split enforceable; shipping it means users get it
without configuring anything. No hooks and no subagents — this is a tool, not a
workflow.

## Consequences

The skill and the cheatsheet describe the same surface and can drift apart. The
cheatsheet is compiled into the binary from `docs/cheatsheet.txt`, and its
examples are exercised by `trycmd`, so the drift is at least testable on that
side.
