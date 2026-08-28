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

A bare key is normal, and the key decides which profile answers:

```bash
ytcli issue get LMS-11
→ profile=work org=1234567 (from the only profile that sees LMS)
```

A queue only one profile can see is fetched through that profile, whatever the
default is. Two profiles on the *same* organisation are not a conflict — that is
one issue seen through two logins.

Two profiles in *different* organisations sharing a queue key is the real
ambiguity: `LMS-12` then names two issues, the bare form is refused, and the
message names both candidates. Write `work/LMS-12`, which is always accepted.

Every command prints the `→ profile=… org=…` line on stderr, once. stdout never
carries it, so it does not affect anything you parse.

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

## Beyond issues

Same ladder, same tallies:

```bash
ytcli queue list                     # keys and leads
ytcli queue get PROJ                 # the type and priority a new issue starts with
ytcli board list                     # id, name, column count
ytcli board sprints 6                # a kanban board is refused, in Tracker's words
ytcli field list                     # every field the organisation defines
ytcli template list --kind comment   # issue templates by default
ytcli portfolio contents 655…        # the portfolios and projects inside one
ytcli issue worklogs PROJ-1          # time logged, with the total
ytcli issue checklist PROJ-1         # lines, boxes and ids
```
