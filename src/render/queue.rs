//! Queue listings and field tables.

use std::fmt::Write as _;

use crate::api::{Queue, QueueField};

/// One line per queue: key first, because the key is what every other command
/// takes.
#[must_use]
pub fn queues(queues: &[Queue]) -> String {
    let mut out = String::with_capacity(queues.len() * 48 + 32);

    for queue in queues {
        let _ = writeln!(
            out,
            "{:<12} {:<28} {}",
            queue.key,
            queue.name,
            queue.lead.as_deref().unwrap_or("-"),
        );
    }

    let _ = writeln!(out, "shown {} of {}", queues.len(), queues.len());
    out
}

/// Field keys, types and names.
///
/// The key column comes first and is the point of the command: it is what
/// `--fields` and `--set` accept, and without it a caller is guessing. Custom
/// fields are marked, since those are the ones that differ per queue and are
/// therefore the ones worth pinning in a profile.
#[must_use]
pub fn fields(fields: &[QueueField]) -> String {
    let mut out = String::with_capacity(fields.len() * 56 + 32);

    for field in fields {
        let _ = writeln!(
            out,
            "{:<28} {:<12} {:<8} {}",
            field.key,
            field.field_type,
            if field.system { "system" } else { "custom" },
            field.name,
        );
    }

    let custom = fields.iter().filter(|field| !field.system).count();
    let _ = writeln!(
        out,
        "shown {} of {} ({custom} custom)",
        fields.len(),
        fields.len()
    );
    out
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn field_listing_marks_custom_fields_and_counts_them() {
        let listing = fields(&[
            QueueField {
                key: "summary".to_owned(),
                name: "Summary".to_owned(),
                field_type: "string".to_owned(),
                system: true,
            },
            QueueField {
                key: "storyPoints".to_owned(),
                name: "Story points".to_owned(),
                field_type: "number".to_owned(),
                system: false,
            },
        ]);

        assert!(listing.contains("summary"));
        assert!(listing.contains("system"));
        assert!(listing.contains("storyPoints"));
        assert!(listing.ends_with("shown 2 of 2 (1 custom)\n"));
    }
}
