//! Where to get credentials.
//!
//! Printed when someone is stuck rather than on every run: a first login, a
//! rejected token, an organisation that answers 403. Getting a Tracker token
//! means creating an OAuth application and hand-assembling an authorize URL,
//! which is not something anyone guesses, and the organisation id lives behind a
//! page most people have never opened.
//!
//! The blocks are written as markdown and rendered for a terminal, for the same
//! reason issue descriptions are: a numbered procedure with a table in it reads
//! as a procedure, not as punctuation. Where nobody is watching — a pipe, a log,
//! a CI run — the markdown source goes out unrendered, because reflowed text
//! with escape codes in it is worse to read in a log than the source was.

use std::io::IsTerminal;

/// How to obtain an OAuth token.
pub const TOKEN: &str = "\
## Get an OAuth token

1. Create an application at https://oauth.yandex.com/client/new — pick
   **For API access or debugging**, and grant `tracker:write` for full access
   or `tracker:read` to stay read-only.
   (https://oauth.yandex.ru/client/new for a Russian-locale account.)
2. Copy the application's **ClientID** from its page.
3. Open `https://oauth.yandex.com/authorize?response_type=token&client_id=<ClientID>`
   and sign in. The token comes back in the address bar you land on.

It looks like `y0__xAbc...`, roughly 60-90 characters.

Docs: https://yandex.ru/support/tracker/en/api-ref/access
";

/// How to find the organisation id, and which flavour it is.
pub const ORG: &str = "\
## Find your organisation id

Open https://tracker.yandex.ru/admin/orgs — it lists every organisation you are
in, with its id, and says which kind each one is.

| Kind | Id looks like | Flag |
|:-|:-|:-|
| Yandex 360 for Business | `1234567` | `--org-kind yandex360` |
| Yandex Cloud Organization | `bpfaidqca8vd0m5jl3fp` | `--org-kind cloud` |

Not sure which you have? Omit `--org-kind`: login tries both and reports which
one answered. The two use different headers, and the wrong one returns **403** —
which reads like a rights problem rather than a configuration mistake.

Docs: https://yandex.ru/support/tracker/en/api-ref/access
";

/// What `auth login` is, above the two blocks.
const INTRO: &str = "\
Store a token for an account, and set up a profile to use it with.

```
ytcli auth login
ytcli auth login --account work --org-id 1234567 --profile work
ytcli auth login --account work --org-id 1234567 --dry-run
```

Run it with no arguments in a terminal and it walks you through each step,
taking the token as a hidden password. Pass whatever you already know as flags
and only the rest is asked for. Outside a terminal the flags are all there is,
and the token is read from stdin.
";

/// Render one block for whoever is reading it.
///
/// Rendered only for a terminal, and to the terminal's own width: markdown
/// wrapped to 80 columns in a 200-column window looks like a mistake, and
/// escape codes in a captured log are one.
#[must_use]
pub fn block(markdown: &str) -> String {
    if !std::io::stderr().is_terminal() {
        return markdown.to_owned();
    }
    crate::render::markdown::render(markdown, width())
}

/// Both blocks, for a first run.
#[must_use]
pub fn full() -> String {
    block(&format!("{TOKEN}\n{ORG}"))
}

/// The long help of `auth login`: what the command is, then the same two
/// blocks, so `--help` answers the question without anyone having to fail first.
#[must_use]
pub fn login_help() -> String {
    crate::cli::help::md(&format!("{INTRO}\n{TOKEN}\n{ORG}"))
}

/// The window, narrowed to a width prose is readable at.
///
/// A procedure wrapped to 200 columns is one long line with a number in front
/// of it; nobody reads that as steps.
fn width() -> usize {
    crate::cli::terminal_width().clamp(40, 92)
}
