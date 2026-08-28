//! Normalised entities.
//!
//! These are *our* schema, not Tracker's. `--json` emits these so that scripts
//! keep working across Tracker API changes; `--json-raw` exists for the cases
//! where someone genuinely needs the upstream payload. Every unmapped field is
//! preserved in `extra`, which is also where a queue's custom fields arrive.

use serde::{Deserialize, Serialize};

/// A person, reduced to what output ever shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub display: Option<String>,
}

/// One entry of an organisation-wide dictionary: an issue type, a priority, a
/// status or a resolution.
///
/// All four endpoints answer with the same shape, so one type covers them.
///
/// `key` and `name` are not interchangeable and the difference is the reason
/// this is worth listing at all: `name` comes back in the organisation's own
/// language — a Russian organisation answers `Ошибка` — while `key` is the
/// stable English handle a write has to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    pub key: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Where Tracker sorts it. Priorities and resolutions have one; issue types
    /// do not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
    /// A status's category — `new`, `inProgress`, `paused`, `done`, `cancelled`.
    /// Only statuses carry it, and it is what makes a status list readable
    /// without knowing the workflow.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// A person, in full.
///
/// Distinct from [`User`] on purpose: that one is a *reference* to somebody,
/// the shape a login arrives in on an issue, and it is deliberately small
/// because it appears in every answer. This one is the directory record, and
/// only the user commands return it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub login: String,
    pub uid: String,
    pub display: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Left the organisation. Still assignable in old issues, which is why the
    /// listing says so rather than hiding them.
    pub dismissed: bool,
    /// Somebody outside the organisation with access to it.
    pub external: bool,
}

/// How two issues relate. Rendered on every issue view, because "what blocks
/// this" is the question an agent asks right after "what is this".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    Blocks,
    IsBlockedBy,
    Parent,
    Subtask,
    Duplicates,
    IsDuplicatedBy,
    Depends,
    IsDependentBy,
    Relates,
    Epic,
    HasEpic,
    Other,
}

impl LinkKind {
    /// Short label used in compact output.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::IsBlockedBy => "is blocked by",
            Self::Parent => "parent",
            Self::Subtask => "subtask",
            Self::Duplicates => "duplicates",
            Self::IsDuplicatedBy => "is duplicated by",
            Self::Depends => "depends on",
            Self::IsDependentBy => "is depended on by",
            Self::Relates => "relates",
            Self::Epic => "epic",
            Self::HasEpic => "has epic",
            Self::Other => "related to",
        }
    }
}

/// One edge from an issue to another issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub kind: LinkKind,
    /// Tracker's own wording for the relationship, in whatever language it
    /// answered in. Shown when `kind` is [`LinkKind::Other`], so an unrecognised
    /// relation still says what it is instead of reading as "link".
    #[serde(default)]
    pub relation: Option<String>,
    pub key: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

/// An issue, normalised.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub key: String,
    pub summary: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub issue_type: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub queue: Option<String>,
    #[serde(default)]
    pub assignee: Option<User>,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub created_at: Option<jiff::Timestamp>,
    #[serde(default)]
    pub updated_at: Option<jiff::Timestamp>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub comment_count: Option<u32>,
    /// Custom and unmapped fields, kept out of the compact view by default.
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One page of results plus the totals needed to say "shown N of M".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub per_page: u32,
    /// `None` when Tracker did not report a total for this query.
    pub total: Option<u64>,
}

impl<T> Page<T> {
    /// Whether more results exist beyond this page.
    #[must_use]
    pub fn has_more(&self) -> bool {
        match self.total {
            Some(total) => u64::from(self.page) * u64::from(self.per_page) < total,
            None => u32::try_from(self.items.len()).is_ok_and(|len| len == self.per_page),
        }
    }
}

/// One comment, reduced to what output shows.
///
/// `text` is the single most attacker-influenced string this tool handles: it is
/// free-form, written by anyone with access to the issue, and read by whatever
/// called us. It is never rewritten, only fenced (`render::untrusted`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub created_at: Option<jiff::Timestamp>,
}

/// One entry in an issue's worklog.
///
/// `duration` stays in Tracker's ISO 8601 form here — the API's word for it,
/// which is what a `--format json` caller is scripting against.
/// [`crate::api::duration::human`] is what turns it into something to read.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Worklog {
    pub id: String,
    pub duration: String,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub start: Option<jiff::Timestamp>,
    /// What the time was spent on, when whoever logged it said.
    #[serde(default)]
    pub comment: Option<String>,
}

/// One line of an issue's checklist.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChecklistItem {
    pub id: String,
    pub text: String,
    pub checked: bool,
    /// A checklist item can carry an assignee and a deadline of its own.
    #[serde(default)]
    pub assignee: Option<User>,
    #[serde(default)]
    pub deadline: Option<String>,
}

/// A project, portfolio or goal.
///
/// These live behind one endpoint family and differ only in which fields are
/// populated, so they share a model rather than three near-identical ones. Note
/// `short_id`: it is what an issue's `project` field refers to, and it is not
/// the `id` the entity endpoints take — confusing the two is the obvious
/// mistake, so both are carried.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub short_id: Option<i64>,
    pub entity_type: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub lead: Option<User>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// The portfolio this sits in, when it sits in one.
    #[serde(default)]
    pub parent: Option<String>,
    /// Tracker's optimistic-concurrency counter. A write that quotes a stale
    /// one is refused rather than applied over somebody else's change.
    #[serde(default)]
    pub version: Option<u64>,
}

/// One attachment of an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    /// The filename as uploaded. Chosen by whoever uploaded it, so it is never
    /// used to decide a path on disk without sanitising.
    pub name: String,
    pub size: Option<u64>,
    pub mimetype: Option<String>,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub created_at: Option<jiff::Timestamp>,
    /// Where the bytes are. Checked against the configured API host before it
    /// is followed.
    pub content: Option<String>,
}
