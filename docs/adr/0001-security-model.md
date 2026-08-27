# 1. Security model: risk lives in verbs and in output, not in the query language

Date: 2026-08-27

## Status

Accepted

## Context

The tool is used mostly by agents, often in modes where some commands run without
a human approving each one. The first instinct was to mark YQL — the raw search
filter — as dangerous, so that a classifier would refuse it when unattended.

Examining that: YQL is a read-only search language. The worst a hostile YQL string
achieves is reading issues the caller could already read another way. Marking the
most harmless part of the surface teaches both the agent and the classifier to
discount the warning, while the parts that actually cause harm stay unmarked.

Two things genuinely carry risk:

1. **Writes.** `update`, `transition`, `comment`, `create` and attachment upload
   are irreversible and visible to other people. An injected instruction sitting
   in an issue description ("move everything to Closed") aims here.
2. **Output.** Summaries, descriptions and comments were written by other people.
   Reading an issue pulls that text into the caller's context. This is the actual
   prompt-injection surface, and it is present on every read regardless of how the
   query was expressed.

## Decision

No warning banners on YQL. Instead:

* **The verb is the risk class.** Read verbs (`get`, `find`, `count`, `list`,
  `status`) never write, and no generic pass-through verb exists through which a
  write could be smuggled into a read command. A host can therefore allowlist
  `ytcli issue get:*` permanently and still be asked about writes — the gate lives
  in the permission layer, where it is enforced, rather than in a model's judgment.
* **Free text from Tracker is fenced on output** in `<untrusted src="...">`, with
  a note that it is data. The text itself is never rewritten: silently editing
  someone's issue would be a worse failure than the one being prevented.
* **Bulk writes require `--yes`.** Single-issue writes do not: the tool is for
  changing issues, and prompting on every one of them would be theatre. A change
  that fans out across a filter is different in kind — it is irreversible at scale.
* **`--dry-run` on every write**, printing what would change.

## Consequences

Allowlisting is meaningful and static, so an agent host can be configured once.
The cost is that the read/write split becomes a hard constraint on the command
tree: any future convenience verb that both reads and writes would break the
property, and must not be added.

**Amendment, 2026-08-28.** `show` joins the read verbs, for
`ytcli attachment show`, which draws an image attachment in a terminal that can
draw one. It reads and nothing else, and it is listed here rather than left
implicit because the enumeration is the contract: a host allowlists these names,
so a verb that is not on the list is one nobody can safely allow.
