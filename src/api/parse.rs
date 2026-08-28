//! Turning Tracker's payloads into our models.
//!
//! Tracker returns most human-readable values as objects with `display`, `key`,
//! `id` and `self` members, and the useful part is not always in the same one.
//! Rather than deriving `Deserialize` against a moving target, we pull out what
//! we render and keep everything else in `extra`, so an upstream addition is
//! carried through instead of breaking the parse (ADR 4).

use serde_json::{Map, Value};

use crate::api::models::{
    Attachment, ChecklistItem, Comment, Entity, Issue, Link, LinkKind, User, Worklog,
};

/// System fields we map explicitly. Anything outside this set is treated as a
/// custom field and lands in `extra`.
const KNOWN: &[&str] = &[
    "key",
    // Counters and bookkeeping Tracker returns on every issue. They are not
    // custom fields, and counting them as such made every issue look like it had
    // several — which is exactly the noise the summary exists to avoid.
    "commentWithExternalMessageCount",
    "votes",
    "votedBy",
    "unique",
    "boards",
    "access",
    "followers",
    "checklistDone",
    "checklistTotal",
    "checklistItems",
    "emailCreatedBy",
    "emailTo",
    "emailFrom",
    "summary",
    "status",
    "type",
    "priority",
    "queue",
    "assignee",
    "createdBy",
    "createdAt",
    "updatedAt",
    "description",
    "commentWithoutExternalMessageCount",
    "id",
    "self",
    "version",
    "aliases",
    "lastCommentUpdatedAt",
    "statusStartTime",
    "updatedBy",
    "previousStatus",
    "previousStatusLastAssignee",
    "favorite",
    "pendingReplyFrom",
];

/// The label of a Tracker reference object: `display` if present, else `key`,
/// else `id`, else the value itself when it is a bare string.
fn label(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    for member in ["display", "key", "id"] {
        if let Some(text) = value.get(member).and_then(Value::as_str) {
            return Some(text.to_owned());
        }
    }
    None
}

/// The key of a reference object, falling back to its label.
///
/// Queues are addressed by key everywhere — it is the `PROJ` in `PROJ-1` — so
/// showing the prettier `display` name would print something the caller cannot
/// then type back into another command.
fn key_label(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(key) = value.get("key").and_then(Value::as_str) {
        return Some(key.to_owned());
    }
    label(Some(value))
}

fn user(value: Option<&Value>) -> Option<User> {
    let value = value?;
    Some(User {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        login: value
            .get("login")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        display: value
            .get("display")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

/// Parse a Tracker timestamp.
///
/// The API emits offsets as `+0300` rather than `+03:00`, which is valid ISO 8601
/// but not what a strict RFC 3339 parser accepts, so the compact form is widened
/// before parsing. An unparseable date is dropped rather than failing the whole
/// issue: a missing `updated` line is a much smaller problem than a command that
/// refuses to show an issue at all.
fn timestamp(value: Option<&Value>) -> Option<jiff::Timestamp> {
    let text = value?.as_str()?;

    if let Ok(parsed) = text.parse::<jiff::Timestamp>() {
        return Some(parsed);
    }

    let widened = widen_offset(text);
    match widened.parse::<jiff::Timestamp>() {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            tracing::debug!(%text, %error, "unparseable timestamp, omitted");
            None
        }
    }
}

/// `2026-08-27T10:00:00.000+0300` -> `2026-08-27T10:00:00.000+03:00`.
fn widen_offset(text: &str) -> String {
    let bytes = text.as_bytes();
    let Some(sign_at) = bytes
        .iter()
        .rposition(|byte| *byte == b'+' || *byte == b'-')
    else {
        return text.to_owned();
    };

    // Only the trailing `±HHMM` form needs widening, and only when it really is
    // a four-digit offset rather than part of the date.
    let offset = &text[sign_at + 1..];
    if offset.len() != 4 || !offset.bytes().all(|byte| byte.is_ascii_digit()) {
        return text.to_owned();
    }

    format!("{}{}:{}", &text[..=sign_at], &offset[..2], &offset[2..])
}

/// Map a link type id and our side of it onto our vocabulary.
///
/// Keyed on `type.id`, which is stable and English, rather than on the
/// `inward`/`outward` labels, which come back in the organisation's language —
/// a Russian-locale organisation answers "Связана", and an English word list
/// silently turns every link into "links".
///
/// `direction` decides which end we are on, and it does not mean the same thing
/// for every type: for `subtask`, being on the inward side makes the *other*
/// issue the parent, while for `depends` the inward side is the one that
/// depends. That asymmetry is why this is a table and not a rule.
fn link_kind(type_id: &str, inward: bool) -> LinkKind {
    match (type_id, inward) {
        ("subtask", true) => LinkKind::Parent,
        ("subtask", false) => LinkKind::Subtask,
        ("depends", true) => LinkKind::Depends,
        ("depends", false) => LinkKind::IsDependentBy,
        ("duplicate", true) => LinkKind::Duplicates,
        ("duplicate", false) => LinkKind::IsDuplicatedBy,
        ("epic", true) => LinkKind::HasEpic,
        ("epic", false) => LinkKind::Epic,
        ("relates", _) => LinkKind::Relates,
        _ => LinkKind::Other,
    }
}

/// Parse one entry of `GET /v3/issues/{key}/links`.
#[must_use]
pub fn link(value: &Value) -> Option<Link> {
    let object = value.get("object")?;
    let kind_object = value.get("type");
    let inward = value.get("direction").and_then(Value::as_str) == Some("inward");

    let type_id = kind_object
        .and_then(|kind| kind.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    // Tracker's own wording for our side of the link, kept for the types we do
    // not recognise so the output still says something true.
    let relation = kind_object
        .and_then(|kind| kind.get(if inward { "inward" } else { "outward" }))
        .and_then(Value::as_str)
        .map(str::to_lowercase);

    Some(Link {
        kind: link_kind(type_id, inward),
        relation,
        key: object.get("key").and_then(Value::as_str)?.to_owned(),
        summary: label(object.get("display")),
        status: label(object.get("status")),
    })
}

/// Parse one project, portfolio or goal.
///
/// Everything worth showing sits under `fields`; the envelope carries only
/// identity.
#[must_use]
pub fn entity(value: &Value) -> Option<Entity> {
    let fields = value.get("fields");
    let field = |name: &str| fields.and_then(|fields| fields.get(name));

    Some(Entity {
        id: value.get("id").and_then(Value::as_str)?.to_owned(),
        short_id: value.get("shortId").and_then(Value::as_i64),
        entity_type: value
            .get("entityType")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        summary: field("summary")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        status: label(field("entityStatus")),
        lead: user(field("lead")),
        start: field("start")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        end: field("end").and_then(Value::as_str).map(ToOwned::to_owned),
        description: field("description")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        parent: parent_entity(field("parentEntity")),
        version: value.get("version").and_then(Value::as_u64),
    })
}

/// The id out of `parentEntity`, in whichever shape it arrives.
///
/// Writes take an object (`{"primary": id, "secondary": [...]}`), and reads have
/// been seen to answer with the bare id. Both are accepted rather than guessed
/// at, because an entity whose parent silently reads as absent is worse than one
/// that costs a few lines here.
fn parent_entity(value: Option<&Value>) -> Option<String> {
    fn id_of(value: &Value) -> Option<String> {
        match value {
            Value::String(id) => Some(id.clone()),
            Value::Number(id) => Some(id.to_string()),
            // A read answers `{"primary": {"id": …, "display": …}}`; a write
            // takes `{"primary": "<id>"}`. Both arrive here.
            Value::Object(map) => map.get("id").and_then(id_of),
            _ => None,
        }
    }

    match value? {
        Value::Object(map) => map.get("primary").and_then(id_of),
        other => id_of(other),
    }
}

/// Parse one entry of `GET /v3/issues/{key}/attachments`.
#[must_use]
pub fn attachment(value: &Value) -> Option<Attachment> {
    Some(Attachment {
        id: match value.get("id")? {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        },
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        size: value.get("size").and_then(Value::as_u64),
        mimetype: value
            .get("mimetype")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        author: user(value.get("createdBy")),
        created_at: timestamp(value.get("createdAt")),
        content: value
            .get("content")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

/// Parse one entry of `GET /v3/issues/{key}/comments`.
#[must_use]
pub fn comment(value: &Value) -> Option<Comment> {
    Some(Comment {
        id: value
            .get("id")
            .map(|id| match id {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        text: value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        author: user(value.get("createdBy")),
        created_at: timestamp(value.get("createdAt")),
    })
}

/// Parse one entry of `GET /v3/issues/{key}/worklog`.
#[must_use]
pub fn worklog(value: &Value) -> Option<Worklog> {
    Some(Worklog {
        id: identifier(value.get("id"))?,
        duration: value
            .get("duration")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        author: user(value.get("createdBy")),
        start: timestamp(value.get("start")),
        comment: value
            .get("comment")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Parse one entry of `GET /v3/issues/{key}/checklistItems`.
#[must_use]
pub fn checklist_item(value: &Value) -> Option<ChecklistItem> {
    Some(ChecklistItem {
        id: identifier(value.get("id"))?,
        text: value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        checked: value
            .get("checked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        assignee: user(value.get("assignee")),
        deadline: value
            .get("deadline")
            .and_then(|deadline| deadline.get("date").or(Some(deadline)))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// An id, whether Tracker sent it as a string or a number.
///
/// Worklog and checklist ids come back as numbers on some endpoints and strings
/// on others, and a caller passes whichever they were given straight back.
fn identifier(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// The name a caller can actually type for a custom field.
///
/// Tracker returns them prefixed with the queue's opaque id —
/// `603bd9b6cdc7ba0d2f4b1a55--component` — and accepts the trailing segment
/// back. `queue fields` has always printed the short form; the issue view
/// printed the raw one, which meant the line whose whole purpose is to say what
/// to ask for next named a key `--fields` would not take.
fn custom_field_key(key: &str) -> String {
    key.rsplit("--").next().unwrap_or(key).to_owned()
}

/// Parse `GET /v3/issues/{key}` (or one element of a search result).
///
/// Returns `None` only when the payload has no key, which means it is not an
/// issue at all.
#[must_use]
pub fn issue(value: &Value) -> Option<Issue> {
    let object = value.as_object()?;

    let mut extra = Map::new();
    for (key, member) in object {
        if KNOWN.contains(&key.as_str()) {
            continue;
        }
        // A custom field that is set to nothing is not worth counting: it would
        // inflate the "N set" summary with fields nobody filled in.
        if member.is_null() {
            continue;
        }
        let key = custom_field_key(key);
        if let Some(text) = label(Some(member)) {
            extra.insert(key, Value::String(text));
        } else {
            extra.insert(key, member.clone());
        }
    }

    Some(Issue {
        key: object.get("key")?.as_str()?.to_owned(),
        summary: object
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        status: label(object.get("status")),
        issue_type: label(object.get("type")),
        priority: label(object.get("priority")),
        queue: key_label(object.get("queue")),
        assignee: user(object.get("assignee")),
        author: user(object.get("createdBy")),
        created_at: timestamp(object.get("createdAt")),
        updated_at: timestamp(object.get("updatedAt")),
        description: object
            .get("description")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        links: Vec::new(),
        comment_count: object
            .get("commentWithoutExternalMessageCount")
            .and_then(Value::as_u64)
            .and_then(|count| u32::try_from(count).ok()),
        extra,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn reference_objects_collapse_to_their_display_name() {
        let value = serde_json::json!({"display": "In Progress", "key": "inProgress"});
        assert_eq!(label(Some(&value)).as_deref(), Some("In Progress"));
    }

    #[test]
    fn a_reference_without_a_display_falls_back_to_its_key() {
        let value = serde_json::json!({"key": "PROJ", "id": "7"});
        assert_eq!(label(Some(&value)).as_deref(), Some("PROJ"));
    }

    /// Tracker writes offsets as `+0300`; a strict RFC 3339 parser wants `+03:00`.
    #[test]
    fn compact_offsets_are_widened_before_parsing() {
        let value = serde_json::json!("2026-08-27T10:00:00.000+0300");
        let parsed = timestamp(Some(&value)).expect("parsed");
        assert_eq!(parsed.to_string(), "2026-08-27T07:00:00Z");
    }

    #[test]
    fn utc_timestamps_parse_unchanged() {
        let value = serde_json::json!("2026-08-27T10:00:00Z");
        assert!(timestamp(Some(&value)).is_some());
    }

    /// A date we cannot read costs one line of output. Refusing to show the
    /// issue would cost the whole command.
    #[test]
    fn an_unparseable_date_is_dropped_not_fatal() {
        let value = serde_json::json!("yesterday");
        assert!(timestamp(Some(&value)).is_none());
    }

    #[test]
    fn unknown_members_become_custom_fields_and_nulls_are_skipped() {
        let value = serde_json::json!({
            "key": "PROJ-1",
            "summary": "s",
            "storyPoints": 3,
            "sprint": {"display": "S-12", "id": "9"},
            "emptyField": null,
        });
        let parsed = issue(&value).expect("parsed");

        assert_eq!(parsed.extra.len(), 2);
        assert_eq!(parsed.extra.get("sprint"), Some(&serde_json::json!("S-12")));
        assert!(!parsed.extra.contains_key("emptyField"));
    }

    #[test]
    fn a_payload_without_a_key_is_not_an_issue() {
        assert!(issue(&serde_json::json!({"summary": "s"})).is_none());
    }

    fn subtask_link(direction: &str, key: &str) -> Value {
        serde_json::json!({
            "type": {"id": "subtask", "inward": "Is subtask for", "outward": "Is parent task for"},
            "direction": direction,
            "object": {"key": key, "display": "some issue"},
        })
    }

    /// Tracker answers in the organisation's language. Reading the relationship
    /// from the localised label turned every link in a Russian organisation into
    /// the fallback, so the type id decides.
    #[test]
    fn a_russian_organisation_still_gets_real_link_types() {
        let value = serde_json::json!({
            "type": {"id": "relates", "inward": "Связана", "outward": "Связана"},
            "direction": "outward",
            "object": {"key": "LMS-1", "display": "какая-то задача"},
        });

        assert_eq!(link(&value).expect("link").kind, LinkKind::Relates);
    }

    /// An unfamiliar link type keeps Tracker's own wording rather than being
    /// rendered as the word "link".
    #[test]
    fn an_unknown_link_type_keeps_what_tracker_called_it() {
        let value = serde_json::json!({
            "type": {"id": "somethingNew", "inward": "Blocks release of", "outward": "x"},
            "direction": "inward",
            "object": {"key": "PROJ-5"},
        });

        let parsed = link(&value).expect("link");
        assert_eq!(parsed.kind, LinkKind::Other);
        assert_eq!(parsed.relation.as_deref(), Some("blocks release of"));
    }

    /// Bookkeeping counters are not custom fields; counting them made every
    /// issue look like it had several.
    #[test]
    fn tracker_bookkeeping_is_not_mistaken_for_custom_fields() {
        let value = serde_json::json!({
            "key": "PROJ-1",
            "summary": "s",
            "commentWithExternalMessageCount": 0,
            "votes": 3,
            "followers": [],
            "storyPoints": 5,
        });

        let parsed = issue(&value).expect("parsed");
        assert_eq!(parsed.extra.keys().collect::<Vec<_>>(), ["storyPoints"]);
    }

    #[test]
    fn link_direction_decides_parent_from_subtask() {
        assert_eq!(
            link(&subtask_link("inward", "PROJ-9")).expect("link").kind,
            LinkKind::Parent
        );
        assert_eq!(
            link(&subtask_link("outward", "PROJ-12"))
                .expect("link")
                .kind,
            LinkKind::Subtask
        );
    }

    /// `subtask` and `depends` invert relative to each other, so a table keyed
    /// on the type id alone renders one of them backwards.
    #[test]
    fn depends_is_not_inverted_the_way_subtask_is() {
        let inward = serde_json::json!({
            "type": {"id": "depends", "inward": "Depends on", "outward": "Is dependent by"},
            "direction": "inward",
            "object": {"key": "PROJ-3", "display": "blocker"},
        });

        assert_eq!(link(&inward).expect("link").kind, LinkKind::Depends);
    }

    #[test]
    fn a_queue_renders_as_its_key_not_its_display_name() {
        let value = serde_json::json!({
            "key": "PROJ-1",
            "summary": "s",
            "queue": {"key": "PROJ", "display": "Product"},
        });
        assert_eq!(
            issue(&value).expect("parsed").queue.as_deref(),
            Some("PROJ")
        );
    }

    /// The line whose whole purpose is to say what to ask for next has to name
    /// a key `--fields` accepts. Tracker prefixes custom fields with the
    /// queue's opaque id and takes the trailing segment back.
    #[test]
    fn a_custom_field_is_keyed_by_the_name_a_caller_can_type() {
        let value = serde_json::json!({
            "key": "PROJ-1",
            "summary": "x",
            "603bd9b6cdc7ba0d2f4b1a55--component": "backend",
            "sprint": "S-12",
        });

        let parsed = issue(&value).expect("parses");

        assert_eq!(
            parsed.extra.get("component"),
            Some(&serde_json::json!("backend"))
        );
        assert!(!parsed.extra.keys().any(|key| key.contains("--")));
        // A field with no prefix is left exactly as it came.
        assert_eq!(parsed.extra.get("sprint"), Some(&serde_json::json!("S-12")));
    }
}
