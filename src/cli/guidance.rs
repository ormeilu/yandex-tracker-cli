//! Where to get credentials.
//!
//! Printed when someone is stuck rather than on every run: a first login, a
//! rejected token, an organisation that answers 403. Getting a Tracker token
//! means creating an OAuth application and hand-assembling an authorize URL,
//! which is not something anyone guesses, and the organisation id lives in a
//! different console depending on which flavour of organisation it is.

/// How to obtain an OAuth token.
pub const TOKEN: &str = "\
How to get an OAuth token:
  1. Create an application at https://oauth.yandex.ru/client/new
     — pick \"For API access or debugging\" (Веб-сервисы is not what you want)
     — grant tracker:write for full access, or tracker:read to stay read-only
  2. Copy the application's ClientID from its page
  3. Open https://oauth.yandex.ru/authorize?response_type=token&client_id=<ClientID>
     and sign in; the token is in the address bar of the page you land on
  The token looks like `y0__xAbc...`, roughly 60-90 characters.
  Docs: https://yandex.ru/support/tracker/ru/concepts/access";

/// How to find the organisation id, and which flavour it is.
pub const ORG: &str = "\
How to find your organisation id:
  Yandex 360 for Business — https://admin.yandex.ru, Administration -> Organizations,
    field `identifier`. A number, e.g. 1234567. Use --org-kind yandex360.
  Yandex Cloud Organization — https://console.yandex.cloud/org, the organisation's
    id. Letters and digits, e.g. bpfaidqca8vd0m5jl3fp. Use --org-kind cloud.
  If you are not sure which one you have, omit --org-kind: login tries both.";

/// Both blocks, for a first run.
#[must_use]
pub fn full() -> String {
    format!("{TOKEN}\n\n{ORG}\n")
}
