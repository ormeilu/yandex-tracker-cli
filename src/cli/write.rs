//! The gate every writing command passes through.
//!
//! Two rules, from ADR 1:
//!
//! * `--dry-run` prints the request that would be sent and sends nothing.
//! * A change that fans out across more than one issue needs `--yes`. A single
//!   issue does not: this is a tool for changing issues, and confirming each one
//!   would be theatre rather than safety.
//!
//! Both paths announce the profile and organisation first. A change applied to
//! the wrong organisation is expensive to reconstruct afterwards, and "which one
//! was I on" should never be a question the output leaves open.

use std::io::Write;

use crate::cli::Session;
use crate::exit::ExitCode;

/// What a write is about to do, in the words the user will see.
#[derive(Debug)]
pub struct Intent<'a> {
    /// Imperative summary, e.g. `create an issue in PROJ`.
    pub action: &'a str,
    /// The issue keys affected, when they are known ahead of time.
    pub targets: &'a [String],
    /// The request body that would be sent.
    pub body: &'a serde_json::Value,
    /// Ask for `--yes` even for a single target.
    ///
    /// For work that is irreversible in kind rather than at scale: a queue key
    /// is claimed once, and Tracker deletes a queue by hiding it, so the key
    /// stays spent whatever happens next.
    pub always_confirm: bool,
}

/// Outcome of the gate.
#[derive(Debug)]
pub enum Gate {
    /// Go ahead.
    Proceed,
    /// Stop, with this exit code.
    Stop(ExitCode),
}

/// Announce the write, then decide whether it may proceed.
#[must_use]
pub fn check(intent: &Intent<'_>, session: &Session) -> Gate {
    let mut err = anstream::stderr();

    // The same line every command prints, and only once: by the time a write
    // reaches the gate the client has usually already said which profile it is.
    if let Some(resolved) = &session.resolved {
        session.announce(resolved);
    }

    if session.global.dry_run {
        let _ = writeln!(err, "dry run: would {}", intent.action);
        let body =
            serde_json::to_string_pretty(intent.body).unwrap_or_else(|_| intent.body.to_string());
        let _ = writeln!(err, "{body}");
        return Gate::Stop(ExitCode::Success);
    }

    // One issue is the ordinary case and needs no ceremony. Several is different
    // in kind: irreversible at scale, and usually the result of a filter that
    // matched more than the caller pictured.
    if intent.targets.len() > 1 && !session.global.yes {
        let _ = writeln!(
            err,
            "refusing to {} across {} issues without --yes: {}",
            intent.action,
            intent.targets.len(),
            intent.targets.join(", "),
        );
        return Gate::Stop(ExitCode::ConfirmationRequired);
    }

    if intent.always_confirm && !session.global.yes {
        let _ = writeln!(
            err,
            "refusing to {} without --yes: this one cannot be undone",
            intent.action,
        );
        return Gate::Stop(ExitCode::ConfirmationRequired);
    }

    Gate::Proceed
}

/// Parse a `key=value` or `key:=json` pair from `--set`.
///
/// `key=value` guesses: the value is read as JSON when it parses as one, so
/// `--set storyPoints=3` sends a number and `--set tags=["a","b"]` sends an
/// array, and anything else is a string. Guessing from the queue's field
/// metadata instead would cost a request on every update to answer a question
/// the caller already knows.
///
/// The cost of the guess is that a summary which happens to look like a number
/// becomes one. `key:=json` is the way to say what was meant: the value is JSON
/// and only JSON, so `--set 'summary:="3"'` writes the string and invalid JSON
/// is a refusal rather than a silent fall back to text.
pub fn parse_assignment(raw: &str) -> Result<(String, serde_json::Value), String> {
    // `:=` only counts when it comes first: `--set summary=a:=b` is a value
    // containing a colon, not a JSON assignment with an odd key.
    if let Some((key, value)) = raw.split_once(":=")
        && !key.contains('=')
    {
        if key.is_empty() {
            return Err(format!("empty field name in `{raw}`"));
        }
        return match serde_json::from_str(value) {
            Ok(parsed) => Ok((key.to_owned(), parsed)),
            Err(error) => Err(format!(
                "`{raw}` says JSON with `:=` and is not valid JSON: {error}"
            )),
        };
    }

    let Some((key, value)) = raw.split_once('=') else {
        return Err(format!("expected key=value, got `{raw}`"));
    };
    if key.is_empty() {
        return Err(format!("empty field name in `{raw}`"));
    }

    let parsed =
        serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_owned()));

    Ok((key.to_owned(), parsed))
}

/// `me` on a write, replaced by the login it stands for.
///
/// A read hands `me` to YQL, where Tracker's own `me()` answers it. A write has
/// nothing like that: a field that holds people takes a login, `me` is sent as
/// one, and Tracker answers 422 saying no such user exists. So the login is
/// asked for once, through `GET /v3/myself`, and only when something says `me`.
///
/// `--assignee` only ever names a person. A `--set` field is replaced only when
/// the field registry says it holds people, because `summary=me` is a summary,
/// and a field's name is no evidence of what it holds. A queue's local field is
/// not in the registry, so a field the registry does not know is looked for
/// among the local fields of the queues being written to. One found in neither
/// is sent as written, and Tracker says what it makes of it.
#[derive(Debug)]
pub struct Me<'a> {
    client: &'a crate::api::Client,
    queues: Vec<String>,
    login: Option<String>,
}

impl<'a> Me<'a> {
    /// `queues` are where a local field could be defined: the queue an issue is
    /// created in, or the queues of the issues being changed.
    #[must_use]
    pub fn new(client: &'a crate::api::Client, queues: Vec<String>) -> Self {
        Self {
            client,
            queues,
            login: None,
        }
    }

    /// A value given where only a person can go, such as `--assignee`.
    pub async fn person(&mut self, who: &str) -> Result<String, ExitCode> {
        if is_me(who) {
            self.login().await
        } else {
            Ok(who.to_owned())
        }
    }

    /// A `--set` value, with `me` replaced if the field holds people.
    pub async fn field(
        &mut self,
        key: &str,
        mut value: serde_json::Value,
    ) -> Result<serde_json::Value, ExitCode> {
        if !mentions_me(&value) || !self.holds_people(key).await? {
            return Ok(value);
        }
        let login = self.login().await?;
        replace_me(&mut value, &login);
        Ok(value)
    }

    async fn login(&mut self) -> Result<String, ExitCode> {
        if let Some(login) = &self.login {
            return Ok(login.clone());
        }
        let user = self.client.myself().await.map_err(|error| {
            let code = error.exit_code();
            crate::cli::report(&error, code)
        })?;
        // A token without a login still has a uid, and Tracker takes either.
        let login = user
            .login
            .filter(|login| !login.is_empty())
            .or_else(|| Some(user.id).filter(|id| !id.is_empty()))
            .ok_or_else(|| {
                crate::cli::report(
                    &"this token has no login to put in place of `me`; name the login",
                    ExitCode::Auth,
                )
            })?;
        self.login = Some(login.clone());
        Ok(login)
    }

    async fn holds_people(&self, key: &str) -> Result<bool, ExitCode> {
        use crate::api::error::ApiError;

        let unknown = |error: &ApiError| {
            crate::cli::report(
                &format!(
                    "could not tell whether `{key}` holds people, so `me` cannot be resolved: {error}"
                ),
                error.exit_code(),
            )
        };

        match self.client.field(key).await {
            Ok(spec) => return Ok(spec.holds_people()),
            Err(ApiError::NotFound(_)) => {}
            Err(error) => return Err(unknown(&error)),
        }

        let short = key.rsplit("--").next().unwrap_or(key);
        for queue in &self.queues {
            match self.client.queue_local_fields(queue).await {
                Ok(fields) => {
                    if let Some(spec) = fields.iter().find(|spec| spec.key == short) {
                        return Ok(spec.holds_people());
                    }
                }
                Err(ApiError::NotFound(_)) => {}
                Err(error) => return Err(unknown(&error)),
            }
        }
        Ok(false)
    }
}

/// `me`, in any case and with any padding: `ME` is what a shift key produces,
/// and `me, ivan` is how people type a list.
#[must_use]
pub fn is_me(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case("me")
}

/// Whether a value says `me` anywhere a person could be named: the whole
/// string, one entry of a comma-separated list, or a string inside an array or
/// an object such as `{"add": ["me"]}`.
#[must_use]
pub fn mentions_me(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text.split(',').any(is_me),
        serde_json::Value::Array(items) => items.iter().any(mentions_me),
        serde_json::Value::Object(members) => members.values().any(mentions_me),
        _ => false,
    }
}

/// Put `login` wherever [`mentions_me`] finds `me`, and change nothing else:
/// a comma-separated list stays a string, an array stays an array.
fn replace_me(value: &mut serde_json::Value, login: &str) {
    match value {
        serde_json::Value::String(text) if text.split(',').any(is_me) => {
            *text = text
                .split(',')
                .map(|entry| if is_me(entry) { login } else { entry })
                .collect::<Vec<_>>()
                .join(",");
        }
        serde_json::Value::Array(items) => {
            for item in items {
                replace_me(item, login);
            }
        }
        serde_json::Value::Object(members) => {
            for member in members.values_mut() {
                replace_me(member, login);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_booleans_keep_their_type() {
        assert_eq!(
            parse_assignment("storyPoints=3"),
            Ok(("storyPoints".to_owned(), serde_json::json!(3)))
        );
        assert_eq!(
            parse_assignment("flagged=true"),
            Ok(("flagged".to_owned(), serde_json::json!(true)))
        );
    }

    #[test]
    fn plain_text_stays_a_string() {
        assert_eq!(
            parse_assignment("status=In Progress"),
            Ok(("status".to_owned(), serde_json::json!("In Progress")))
        );
    }

    #[test]
    fn json_arrays_pass_through_as_arrays() {
        assert_eq!(
            parse_assignment(r#"tags=["a","b"]"#),
            Ok(("tags".to_owned(), serde_json::json!(["a", "b"])))
        );
    }

    /// A value containing `=` belongs to the value, not to a second split.
    #[test]
    fn only_the_first_equals_separates() {
        assert_eq!(
            parse_assignment("summary=a=b"),
            Ok(("summary".to_owned(), serde_json::json!("a=b")))
        );
    }

    /// The escape hatch from the guess: JSON, and only JSON.
    #[test]
    fn a_json_assignment_is_not_guessed_at() {
        assert_eq!(
            parse_assignment(r#"summary:="3""#),
            Ok(("summary".to_owned(), serde_json::json!("3"))),
            "a number-shaped summary must be writable as a string"
        );
        assert_eq!(
            parse_assignment("storyPoints:=3"),
            Ok(("storyPoints".to_owned(), serde_json::json!(3)))
        );
    }

    /// Saying JSON and not writing JSON is a mistake worth naming, not
    /// something to quietly turn back into text.
    #[test]
    fn invalid_json_after_the_colon_is_refused() {
        assert!(parse_assignment("summary:=not json").is_err());
    }

    /// `:=` inside a value is part of the value.
    #[test]
    fn only_a_leading_colon_equals_means_json() {
        assert_eq!(
            parse_assignment("summary=a:=b"),
            Ok(("summary".to_owned(), serde_json::json!("a:=b")))
        );
    }

    #[test]
    fn a_pair_without_an_equals_is_rejected() {
        assert!(parse_assignment("storyPoints").is_err());
        assert!(parse_assignment("=3").is_err());
    }

    fn replaced(value: serde_json::Value) -> serde_json::Value {
        let mut value = value;
        replace_me(&mut value, "ilubenets");
        value
    }

    /// Every shape a list of people arrives in keeps its shape; only the `me`
    /// inside it changes.
    #[test]
    fn me_is_replaced_in_every_shape_a_person_can_take() {
        assert_eq!(replaced(serde_json::json!("me")), "ilubenets");
        assert_eq!(replaced(serde_json::json!("ME")), "ilubenets");
        assert_eq!(replaced(serde_json::json!("me,ivan")), "ilubenets,ivan");
        assert_eq!(
            replaced(serde_json::json!("ivan, me")),
            "ivan,ilubenets",
            "a padded entry is still `me`"
        );
        assert_eq!(
            replaced(serde_json::json!(["me", "ivan"])),
            serde_json::json!(["ilubenets", "ivan"])
        );
        assert_eq!(
            replaced(serde_json::json!({"add": ["me"]})),
            serde_json::json!({"add": ["ilubenets"]})
        );
    }

    /// `meeting` and `name` contain the letters; they are not the word.
    #[test]
    fn only_the_whole_word_is_me() {
        for text in ["meeting", "name", "me ivan", ""] {
            assert!(!mentions_me(&serde_json::json!(text)), "{text:?}");
            assert_eq!(replaced(serde_json::json!(text)), text);
        }
        assert!(!mentions_me(&serde_json::json!(3)));
        assert!(mentions_me(&serde_json::json!({"remove": "me"})));
    }
}
