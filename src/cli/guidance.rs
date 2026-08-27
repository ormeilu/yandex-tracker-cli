//! Where to get credentials.
//!
//! Printed when someone is stuck rather than on every run: a first login, a
//! rejected token, an organisation that answers 403. Getting a Tracker token
//! means creating an OAuth application and hand-assembling an authorize URL,
//! which is not something anyone guesses, and the organisation id lives behind a
//! page most people have never opened.

/// The token block, as a literal so it can be `concat!`ed into clap's help.
macro_rules! token_help {
    () => {
        "\
How to get an OAuth token:
  1. Create an application: https://oauth.yandex.com/client/new
     (https://oauth.yandex.ru/client/new for a Russian-locale account)
     — pick \"For API access or debugging\"
     — grant tracker:write for full access, or tracker:read to stay read-only
  2. Copy the application's ClientID from its page
  3. Open https://oauth.yandex.com/authorize?response_type=token&client_id=<ClientID>
     and sign in; the token comes back in the address bar you land on
  It looks like `y0__xAbc...`, roughly 60-90 characters.
  Docs: https://yandex.ru/support/tracker/en/api-ref/access"
    };
}

/// The organisation block, likewise.
macro_rules! org_help {
    () => {
        "\
How to find your organisation id:
  Open https://tracker.yandex.ru/admin/orgs — it lists every organisation you are
  in, with its id, and tells you which kind each one is.
    Yandex 360 for Business  -> a number, e.g. 1234567          --org-kind yandex360
    Yandex Cloud Organization -> letters and digits, e.g. bpfaidqca8vd0m5jl3fp
                                                                --org-kind cloud
  Not sure which you have? Omit --org-kind: login tries both and reports which
  one answered. The two use different headers, and the wrong one returns 403 —
  which reads like a rights problem rather than a configuration mistake.
  Docs: https://yandex.ru/support/tracker/en/api-ref/access"
    };
}

/// How to obtain an OAuth token.
pub const TOKEN: &str = token_help!();

/// How to find the organisation id, and which flavour it is.
pub const ORG: &str = org_help!();

/// The long help of `auth login`: the same two blocks, so `--help` answers the
/// question without anyone having to fail first.
pub const LOGIN_HELP: &str = concat!(
    "Store a token for an account, and set up a profile to use it with.\n\n",
    "  ytcli auth login\n",
    "  ytcli auth login --account work --org-id 1234567 --profile work\n",
    "  ytcli auth login --account work --org-id 1234567 --dry-run\n\n",
    "Run it with no arguments in a terminal and it walks you through each step,\n",
    "taking the token as a hidden password. Pass whatever you already know as\n",
    "flags and only the rest is asked for. Outside a terminal the flags are all\n",
    "there is, and the token is read from stdin.\n\n",
    token_help!(),
    "\n\n",
    org_help!(),
);

/// Both blocks, for a first run.
#[must_use]
pub fn full() -> String {
    format!("{TOKEN}\n\n{ORG}\n")
}
