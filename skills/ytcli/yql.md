# The query language

`ytcli issue find --yql '<query>'` takes Tracker's own filter language. The flag
filters (`-q`, `-a`, `-s`, `--tags`) cover the common half; this covers the rest,
and it is read-only — there is no write reachable through a query, however it is
written.

Every query on this page was sent to a real Tracker and accepted. Names that
Tracker does not know are refused with `422` and the message
`Фильтр <name> не существует`, so a wrong guess costs one request and tells you
exactly which word was wrong.

## Shape

```bash
ytcli issue find --yql 'Queue: TRACKER AND Status: Open'
ytcli issue find --yql 'Queue: TRACKER AND Assignee: me()'
ytcli issue find --yql 'Queue: TRACKER AND (Status: Open OR Status: "In Progress")'
```

`<parameter>: <value>`, joined with `AND` and `OR`. Parentheses group; without
them `AND` binds tighter than `OR`. A space between two conditions means `AND`.

Quote a value containing a space: `Status: "In Progress"`.

## Comparison

```bash
ytcli issue find --yql 'Queue: TRACKER AND Status: !Closed'
ytcli issue find --yql 'Queue: TRACKER AND Votes: >0'
ytcli issue find --yql 'Queue: TRACKER AND "Story Points": 1..5'
ytcli issue find --yql 'Queue: TRACKER AND Priority: critical, blocker'
```

`!` negates. `>`, `<`, `>=`, `<=` compare. `1..5` is a numeric range. Several
values of one parameter, comma-separated, mean any of them.

## Functions

```bash
ytcli issue find --yql 'Queue: TRACKER AND Assignee: empty()'
ytcli issue find --yql 'Queue: TRACKER AND Deadline: notEmpty()'
ytcli issue find --yql 'Queue: TRACKER AND Resolution: unresolved()'
ytcli issue find --yql 'Queue: TRACKER AND Updated: week()'
ytcli issue find --yql 'Queue: TRACKER AND Created: today()'
```

`empty()` and `notEmpty()` for whether a field is set at all. `me()` for the
token's own user. `unresolved()` for issues with no resolution. `today()`,
`week()`, `month()`, `quarter()`, `year()` are intervals, not instants — they
match anything inside the period.

## Dates

```bash
ytcli issue find --yql 'Queue: TRACKER AND Updated: >now()-7d'
ytcli issue find --yql 'Queue: TRACKER AND Created: >= today() - "1w"'
```

A span is `"XXM XXw XXd XXh XXm XXs"` — `"2M 3d 5h"` is two months, three days,
five hours. Both spellings above work; the quoted form is the documented one.

## Sorting

```bash
ytcli issue find --yql 'Queue: TRACKER "Sort By": Updated DESC'
ytcli issue find --yql 'Queue: TRACKER AND Assignee: notEmpty() "Sort By": Created ASC, Updated DESC'
```

`"Sort By"` is a parameter like any other, quoted because of the space. Several
fields, comma-separated, are applied in order.

## The names

Verified against a live organisation:

| what | names |
|---|---|
| people | `Assignee`, `Author`, `Followers`, `Modifier`, `"Pending Reply From"` |
| state | `Status`, `Resolution`, `Priority`, `Type`, `Key` |
| dates | `Created`, `Updated`, `Resolved`, `Deadline` |
| grouping | `Queue`, `Project`, `Epic`, `Sprint`, `Components`, `Tags` |
| text | `Summary`, `Description`, `Comment` |
| effort | `"Story Points"`, `"Original Estimate"`, `"Time Spent"`, `Votes` |

**A filter name is not a field key.** `--set storyPoints=3` writes the field;
`"Story Points": 1..5` filters on it. `StoryPoints` as a filter is a 422. The
two vocabularies are separate, and `queue fields PROJ` lists the write side
only.

Not every name exists in every organisation — a filter for a field nobody
enabled is refused the same way a misspelling is.

## Cost

`--yql` conflicts with the flag filters on purpose: combining them would either
drop half of what was asked for or invent an `AND` nobody wrote.

Ask `issue count --yql '…'` before `issue find --yql '…'` when the size of the
answer matters. It is the same query and one number back.

The result is a page, and it ends with `shown N of M` plus `next: --page K` when
more exist. A short page is not evidence of a short answer.
