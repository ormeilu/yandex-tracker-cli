# 3. Output is a detail ladder, and its shape is a contract

Date: 2026-08-27

## Status

Accepted

## Context

The reason this tool exists is that an MCP server for Tracker costs tens of
thousands of context tokens before anything is asked, and then answers with raw
API payloads. A CLI that returns the same payloads saves the first cost and not
the second.

## Decision

**Decoration follows the terminal; format follows the flag.** When stdout is a
terminal, output is coloured and tabular. When it is a pipe, no colour and no box
drawing. `--format` chooses the data shape independently: `text` (default),
`json` (our normalised schema), `json-raw` (upstream payload), `toon`
(experimental, behind a feature flag).

**Detail is a ladder**, cheapest first: the compact view, then `--fields` for a
named subset, then `--full` for the whole description, then `--json`. A caller
starts cheap and pays for detail deliberately.

**Field order is fixed.** A view that reorders itself between calls invalidates an
agent's prompt cache on every invocation and breaks anything parsing the text.
Snapshot tests pin every renderer, so a change to a default shape appears as a
diff in review.

**Custom fields are summarised, not dumped.** They differ per queue, most are
empty, and printing them all makes the view unstable. The compact view names how
many are set and lists a few; profiles pin the ones that matter, in a fixed order.

**Links are always shown, with their type.** "What blocks this" is the question
that follows "what is this"; making the caller run a second command for it costs
more than the few lines it saves.

**Lists always end with a tally**, and offer the next page when one exists. A
caller who receives 25 rows must be able to tell a complete answer from a
truncated one — concluding "there are no open issues" from a truncated page is a
far worse failure than a few wasted tokens. For the same reason, truncation is
never signalled through the exit code, which stays a plain success/failure
channel for scripts.

**`--json` is our schema, not Tracker's.** Upstream field changes would otherwise
leak straight into users' scripts. `--json-raw` remains available for the cases
that genuinely need the original.

## Consequences

Every renderer needs a snapshot test, and adding a field to the compact view is a
deliberate act rather than a side effect.

**Amendment, 2026-08-28.** TOON was behind a build feature while its value was
unknown. Measured, it saves 7% against `json` on a page of issues and 13% on one
issue — its documented 30–55% needs a uniform array of flat objects, and an
issue is not that shape. That is not a reason to hide it: 33 KB on a 4 MB binary
is not a cost worth a feature flag, and a format that only exists in some builds
is one no caller can rely on. It ships in every build and stays off the default,
where the compact text renderer is 97% smaller than either.
