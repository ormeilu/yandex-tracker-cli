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
            Self::Other => "links",
        }
    }
}

/// One edge from an issue to another issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub kind: LinkKind,
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
