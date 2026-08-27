//! Where to get credentials.
//!
//! Printed when someone is stuck rather than on every run: a first login, a
//! rejected token, an organisation that answers 403. Getting a Tracker token
//! means creating an OAuth application and hand-assembling an authorize URL,
//! which is not something anyone guesses, and the organisation id lives behind a
//! page most people have never opened.

/// How to obtain an OAuth token.
pub const TOKEN: &str = "\
How to get an OAuth token:
  1. Create an application: https://oauth.yandex.com/client/new
     (https://oauth.yandex.ru/client/new for a Russian-locale account)
     — pick \"For API access or debugging\"
     — grant tracker:write for full access, or tracker:read to stay read-only
  2. Copy the application's ClientID from its page
  3. Open https://oauth.yandex.com/authorize?response_type=token&client_id=<ClientID>
     and sign in; the token comes back in the address bar you land on
  It looks like `y0__xAbc...`, roughly 60-90 characters.
  Docs: https://yandex.ru/support/tracker/en/api-ref/access";

/// How to find the organisation id, and which flavour it is.
pub const ORG: &str = "\
How to find your organisation id:
  Open https://tracker.yandex.ru/admin/orgs — it lists every organisation you are
  in, with its id, and tells you which kind each one is.
    Yandex 360 for Business  -> a number, e.g. 1234567          --org-kind yandex360
    Yandex Cloud Organization -> letters and digits, e.g. bpfaidqca8vd0m5jl3fp
                                                                --org-kind cloud
  Not sure which you have? Omit --org-kind: login tries both and reports which
  one answered. The two use different headers, and the wrong one returns 403 —
  which reads like a rights problem rather than a configuration mistake.
  Docs: https://yandex.ru/support/tracker/en/api-ref/access";

/// Both blocks, for a first run.
#[must_use]
pub fn full() -> String {
    format!("{TOKEN}\n\n{ORG}\n")
}
