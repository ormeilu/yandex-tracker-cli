# Reporting a bug in ytcli

The tool is small and its output is a contract, so a bug in it is usually
reproducible in one command. That makes a good report short.

**Offer, do not file.** An issue is public and permanent and it goes out in the
user's name. Draft it, show it to them, and let them post it — or post it
yourself only if they ask you to.

## Worth reporting

A crash. Output whose shape or field order changed between runs. A tally that
disagrees with what came back. A message that sent you the wrong way. A flag
that does not do what its help says.

And documentation that is missing or wrong: a thing `ytcli cheatsheet` should
have told you and did not is a real bug, because for an agent the cheatsheet
*is* the interface. Two of the issues fixed after 1.0.0 were exactly that.

## Not worth reporting

Tracker refusing a write it was always going to refuse. A key that does not
exist. Exit 2 for want of `--yes`, exit 3 with no credentials, exit 4 for a
queue this profile cannot see. These are answers, and the exit codes exist to
make them tellable apart. If the *message* about one of them was confusing, that
is worth reporting, but say so as a wording bug rather than as a failure.

## What belongs in one

- The exact command, with the flags it was given.
- The exit code.
- What you expected, and what happened instead.
- `ytcli --version`, and the platform if it looks platform-shaped.

## What has to come out first

The token, obviously — and not even redacted, since a redaction that slips is
worse than a mention that never happened. There is no command that prints a
stored token, so it should not be there to begin with.

Then everything that is nobody else's business, because a bug report is a public
document written from inside a private workspace:

| in your terminal | in the report |
|---|---|
| the real issue key | `PROJ-1` |
| the organisation id | `12345` |
| logins, names, e-mail addresses | `ilubenets` |
| summaries, descriptions, comments | leave them out, or replace with `text` |
| a queue key that names a customer or a project | `PROJ` |

The person reading it needs the *shape* of the failure, not its contents. If the
content is what triggers the bug — a summary of a certain length, a character
that broke the rendering — say that in words, and give the smallest made-up
value that reproduces it.

## Where

<https://github.com/ormeilu/yandex-tracker-cli/issues>

Also in the tool itself, for whoever does not have this skill loaded:
`ytcli cheatsheet more`, and the last three lines of `ytcli --help`.
