# 7. Yandex Wiki reads go behind the same binary, as one command group

Date: 2026-09-10

## Status

Accepted

## Context

Yandex Wiki sits next to Tracker in the same organisation. Runbooks, decisions
and the pages issues point at live there, and an agent using this tool cannot
read any of them. Issue #56 asked four questions before anything is built.

**Does an API exist, and on what terms?** Yes: `https://api.wiki.yandex.net/v1/`.
It authenticates exactly as Tracker does — `Authorization: OAuth <token>` (or
`Bearer` with an IAM token in Yandex Cloud), plus `X-Org-Id` for Yandex 360 or
`X-Cloud-Org-Id` for a Cloud organisation. The organisation flavour a profile
already records selects the right header with no new configuration. What differs
is the scope: `wiki:read` and `wiki:write` are separate from `tracker:read` and
`tracker:write`. A Yandex OAuth application can hold both sets, and a token issued
for it carries all of them, but a token issued before `wiki:read` was added to the
application does not, and gets a 403. Service accounts are refused outright; the
Wiki API only serves user identities.

**What can be read, and by key or only by id?** By slug, which is the path in a
page's URL, so anything a person can paste is addressable:

| Read | Endpoint | Addressed by |
| --- | --- | --- |
| Page | `GET /v1/pages?slug=…` (or `/v1/pages/{id}`) | slug |
| Subpages | `GET /v1/pages/descendants?slug=…` | slug |
| Search | `POST /v1/search` | text |
| Comments | `GET /v1/pages/{id}/comments` | id |
| Attachments | `GET /v1/pages/{id}/attachments` | id |

Comments and attachments take the numeric id only. The page read returns it, so
a slug costs one extra request, which the tool can make rather than the caller.

**What does a page body arrive as?** A string in `content`, requested with
`fields=content`. Pages made in the current editor are Yandex Flavored Markdown —
the same dialect Tracker descriptions use, which `render::markdown` already
draws. Pages from the legacy editor, which can no longer be created but still
exist, come back in the old wiki markup. Both are readable as text; neither needs
conversion to be useful to an agent, and `page_type` says which one it is.

**Does it belong in this tool at all?** See the decision.

## Decision

One binary, one new command group: `ytcli wiki`.

Everything that makes a second product expensive is already built: the keychain
account, the profile and its provenance, the organisation header, the untrusted
fence, the tally, the detail ladder. A sibling binary would share all of it
through a library and still have to ship, install, and be allowlisted separately.
The cost that remains is surface, and it is contained the way ADR 6 contains
everything else: one group name in the top-level help, a `wiki` topic in
`ytcli cheatsheet`, and a separate skill file read only when a wiki page comes up.

The first version is reads only:

```
ytcli wiki get <slug|url>         title, slug, id, type, modified; body fenced
ytcli wiki list <slug|url>        subpages
ytcli wiki find <text>            search hits
ytcli wiki comments <slug|url>    comments, fenced
ytcli wiki attachments <slug|url> file names, sizes, types
```

All five are read verbs in the ADR 1 sense and follow the rules the rest of the
tool follows. Page bodies and comments go out as
`<untrusted src="wiki:<slug>">`, unchanged. Writes — create, update, append,
comment — come later, each as its own issue, and each reporting the profile and
organisation it is about to touch.

## Consequences

**Scope is the first thing users meet.** An existing token has no `wiki:read`,
so the first `wiki` command most people run fails. That failure has to name the
fix — add Wiki permissions to the OAuth application and log in again — rather
than surface a bare 403. `auth login` guidance names both scopes, and
`auth status` reports per profile whether the Wiki answers, so the gap is found
before an agent trips over it.

**The tally loses its total.** Descendants, comments and attachments paginate by
cursor and do not report how many there are; search pages by number up to 500
and reports no total either. `shown N of M` cannot be printed honestly. The tally
becomes `shown N of more than N`, with the next cursor, whenever a next page
exists, and `shown N of N` when none does. The rule it serves — never let a
caller mistake one page for all of them — survives; the number does not.

**A second host.** Requests go to `api.wiki.yandex.net`, and the HTTP client's
check that download URLs stay on the API host has to learn a second one rather
than be loosened.

**Fixtures need a live check.** Every Wiki shape starts borrowed from the
documentation, which does not state the markup of `content` or of comment bodies.
The `live` suite verifies them against a real organisation before 1.x promises
anything about them.
