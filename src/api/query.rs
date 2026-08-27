//! Building Tracker query strings from filter flags.
//!
//! Flags and `--yql` compile to the same thing — a query string — which keeps
//! one code path on the wire and makes the escape hatch nothing more than
//! skipping this module. Both are read-only: a query decides what can be seen,
//! never what changes (ADR 1).

/// Everything the search flags can express.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub queue: Option<String>,
    pub assignee: Option<String>,
    pub status: Option<String>,
    pub tags: Vec<String>,
}

impl Filter {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_none()
            && self.assignee.is_none()
            && self.status.is_none()
            && self.tags.is_empty()
    }

    /// Compile to a query string.
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut clauses: Vec<String> = Vec::new();

        if let Some(queue) = &self.queue {
            clauses.push(format!("Queue: {}", value(queue)));
        }
        if let Some(assignee) = &self.assignee {
            clauses.push(format!("Assignee: {}", assignee_value(assignee)));
        }
        if let Some(status) = &self.status {
            clauses.push(format!("Status: {}", value(status)));
        }
        if !self.tags.is_empty() {
            let tags: Vec<String> = self.tags.iter().map(|tag| value(tag)).collect();
            clauses.push(format!("Tags: {}", tags.join(", ")));
        }

        clauses.join(" AND ")
    }
}

/// `me` resolves server-side through Tracker's own `me()`, so the convenience
/// costs nothing: no extra request to find out who the token belongs to.
fn assignee_value(assignee: &str) -> String {
    if assignee.eq_ignore_ascii_case("me") {
        return "me()".to_owned();
    }
    value(assignee)
}

/// Quote a value.
///
/// Always quoted, never conditionally: a status like `In Progress` needs it, and
/// deciding per value would mean two shapes to reason about and one of them
/// wrong. Embedded quotes are escaped rather than stripped — a query that
/// silently drops part of what was asked for is worse than one that fails.
fn value(raw: &str) -> String {
    format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> Filter {
        Filter {
            queue: Some("PROJ".to_owned()),
            assignee: Some("ilubenets".to_owned()),
            status: Some("In Progress".to_owned()),
            tags: vec!["regression".to_owned()],
        }
    }

    #[test]
    fn clauses_join_with_and_in_a_fixed_order() {
        assert_eq!(
            filter().to_query(),
            r#"Queue: "PROJ" AND Assignee: "ilubenets" AND Status: "In Progress" AND Tags: "regression""#
        );
    }

    #[test]
    fn me_becomes_trackers_own_function_rather_than_a_lookup() {
        let filter = Filter {
            assignee: Some("me".to_owned()),
            ..Filter::default()
        };
        assert_eq!(filter.to_query(), "Assignee: me()");
    }

    #[test]
    fn me_is_matched_regardless_of_case() {
        let filter = Filter {
            assignee: Some("ME".to_owned()),
            ..Filter::default()
        };
        assert_eq!(filter.to_query(), "Assignee: me()");
    }

    /// A quote inside a value must not be able to end the quoted string and
    /// change what the query means.
    #[test]
    fn embedded_quotes_are_escaped_not_dropped() {
        let filter = Filter {
            status: Some(r#"say "hi""#.to_owned()),
            ..Filter::default()
        };
        assert_eq!(filter.to_query(), r#"Status: "say \"hi\"""#);
    }

    #[test]
    fn several_tags_become_one_clause() {
        let filter = Filter {
            tags: vec!["a".to_owned(), "b".to_owned()],
            ..Filter::default()
        };
        assert_eq!(filter.to_query(), r#"Tags: "a", "b""#);
    }

    #[test]
    fn an_empty_filter_is_recognisable() {
        assert!(Filter::default().is_empty());
        assert!(!filter().is_empty());
    }
}
