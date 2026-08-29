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
}
