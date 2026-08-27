# Reading issues

## The ladder

Five rungs, cheapest first. Take the lowest one that answers the question.

| command | size | use when |
|---|---|---|
| `ytcli issue count -q PROJ -s open` | one number | deciding whether to fetch at all |
| `ytcli issue get PROJ-1 --fields status,assignee` | one line | you need two fields |
| `ytcli issue get PROJ-1` | ~15 lines | you need to understand the issue |
| `ytcli issue get PROJ-1 --full` | + whole description | the first lines were cut and mattered |
| `ytcli issue get PROJ-1 --format json` | full payload | you need fidelity, not readability |

`--fields` takes any field name, custom ones included, and returns them in the
order you asked for. A field that is unknown or unset comes back as `-` rather
than vanishing, so the columns never shift under you.

`ytcli queue fields PROJ` lists what a queue actually has, custom keys included.
Guessing a custom field name and getting `-` back is indistinguishable from the
field being empty; this is how you tell.

## Search

```bash
ytcli issue find -q PROJ -a me -s open
ytcli issue find --tags QA --limit 50
ytcli issue find --yql 'Queue: PROJ AND Status: Open AND Updated: >now()-7d'
```

`--yql` is the full Yandex Query Language filter, and it is read-only like every
other search — there is no write reachable through it.

## Pagination

Lists end with `shown N of M`, plus `next: --page K` when more exist. Nothing
else signals truncation: the exit code stays `0`, because a truncated answer is
a successful one.

`--all` walks every page and refuses to run past `--max` rather than silently
truncating. Prefer a narrower filter to a larger `--max`.

## Keys from more than one organisation

A bare key is normal:

```bash
ytcli issue get PROJ-1
```

The qualified form `profile/KEY-1` is always accepted, so scripts can be
explicit. It becomes **required** only when two configured profiles are known to
share that queue key, in which case the bare form is refused and the message
names both candidates. The tool does not guess which organisation you meant.

## Custom fields

The compact view counts them (`custom: 4 set (components, epic, …) — see
--fields`) rather than dumping them: the set differs per queue, most are empty,
and printing all of them would make the view unstable between issues. Ask for
the ones you want by name.

Reference fields — components, tags, epics — render as their display names. The
ids are in `--format json` if you need to address them.

## Links

Links always appear, with their type: `parent`, `subtask`, `is blocked by`,
`relates`, `epic`, and so on. This is deliberate — "what blocks this" is the
question that follows "what is this", and a second command for it would cost
more than the four lines it saves.
