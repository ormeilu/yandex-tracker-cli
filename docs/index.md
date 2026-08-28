# ytcli

Yandex Tracker from the command line, for people and for agents.

The tool exists because of a cost. An MCP server for Tracker loads its whole tool
surface into an agent's context before a single question is asked, and then
answers with raw API payloads. A CLI costs nothing until it is called. This one
also answers in about fifteen lines instead of five kilobytes.

That single goal explains most of what follows: why the default view is terse and
its field order fixed, why lists always say how much they did not show, and why
`--json` is a schema of our own rather than whatever the API happened to return.

> **Status: 0.3.0.** Everything in the command tree is built: issues, worklogs,
> checklists, links, queues, boards and sprints, organisation-wide fields and
> templates, projects, portfolios, goals and attachments — reads and writes.
> Published on crates.io, PyPI, Homebrew and GitHub Releases. What was
> deliberately ruled out is in [TODO](TODO.md).

## Two audiences, one tool

A person wants Tracker in a terminal without a browser tab. An agent wants a
predictable, cheap interface it can call dozens of times in a session.

They mostly want the same thing. Where they differ, the terminal decides:
colour and tables when stdout is a terminal, plain stable lines when it is a pipe.
The data itself is the same either way.

## Where to go next

- [Install](install.md)
- [Configuration](configuration.md) — accounts, profiles, and pinning a repository
- [Output](output.md) — the detail ladder, and what the fences around text mean
- [Using it from an agent](agents.md) — allowlists, exit codes, the skill
- [Decisions](adr/index.md) — why the awkward parts are the way they are

Two things worth knowing before the rest: every command prints one stderr line
saying which profile and organisation answered, and an issue key whose queue
only one profile can see is fetched through that profile, whatever the default
is.
