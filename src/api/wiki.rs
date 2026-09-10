//! Yandex Wiki: a second host reached with the same account and organisation.
//!
//! The Wiki takes the token and the organisation header Tracker does, so one
//! client carries both and only the address differs
//! (`docs/adr/0007-yandex-wiki.md`). Pages are addressed by slug — the path in
//! their URL — because that is what anyone has to hand.

use serde::{Deserialize, Serialize};

use crate::api::Client;
use crate::api::error::ApiError;

/// Default Wiki API root. Overridable so tests can point at a `wiremock` server.
pub const DEFAULT_WIKI_URL: &str = "https://api.wiki.yandex.net";

/// One page, in our schema.
///
/// The date is lifted out of the Wiki's `attributes` block, where nobody reading
/// the JSON would think to look for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPage {
    pub id: u64,
    pub slug: String,
    pub title: String,
    /// `wysiwyg` pages are Markdown, `page` ones the legacy wiki markup; `grid`
    /// and `template` are the other two.
    #[serde(default)]
    pub page_type: Option<String>,
    #[serde(default)]
    pub modified_at: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

/// A page named by a listing: the Wiki sends its id and slug, and no title.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPageRef {
    pub id: u64,
    pub slug: String,
}

/// One page of a Wiki listing.
///
/// The Wiki pages by cursor and never says how many there are, so the only
/// honest tally is "this many, and there are more" (ADR 7). The cursor is kept
/// in the JSON form too: without it a script cannot ask for the next page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPage<T> {
    pub results: Vec<T>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// The last page of hits the Wiki's search will serve.
pub const LAST_SEARCH_PAGE: u32 = 500;

/// One search hit: a page, or a file attached to one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiHit {
    pub slug: String,
    pub title: String,
    /// `page` or `file`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub modified_at: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// The Wiki's excerpt of the matching text — somebody else's words. The
    /// Wiki calls it `content`; it is not the page's content, so our schema
    /// does not either.
    #[serde(default, rename(deserialize = "content"))]
    pub snippet: Option<String>,
}

/// A page of search hits.
///
/// Search is the one Wiki listing that pages by number, and it gives no total
/// either, so the next page number is all a caller can be told.
#[derive(Debug, Clone, Serialize)]
pub struct WikiHits {
    pub results: Vec<WikiHit>,
    pub next_page: Option<u32>,
}

#[derive(Deserialize)]
struct SearchAnswer {
    results: Vec<WikiHit>,
    #[serde(default)]
    next_cursor: Option<String>,
}

/// The page as the Wiki sends it.
#[derive(Deserialize)]
struct Answer {
    id: u64,
    slug: String,
    title: String,
    #[serde(default)]
    page_type: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    attributes: Option<Attributes>,
}

#[derive(Deserialize)]
struct Attributes {
    #[serde(default)]
    modified_at: Option<String>,
}

impl From<Answer> for WikiPage {
    fn from(answer: Answer) -> Self {
        Self {
            id: answer.id,
            slug: answer.slug,
            title: answer.title,
            page_type: answer.page_type,
            modified_at: answer
                .attributes
                .and_then(|attributes| attributes.modified_at),
            content: answer.content,
        }
    }
}

impl Client {
    /// `GET /v1/pages?slug=…`, with the text and the dates.
    pub async fn wiki_page(&self, slug: &str) -> Result<WikiPage, ApiError> {
        let url = format!(
            "{}/v1/pages?slug={}&fields=content,attributes",
            self.wiki_url,
            encode(slug)
        );
        let (value, _) = self
            .send_url(
                reqwest::Method::GET,
                &url,
                None,
                &format!("wiki page `{slug}`"),
            )
            .await
            .map_err(refused)?;
        serde_json::from_value::<Answer>(value)
            .map(WikiPage::from)
            .map_err(ApiError::Decode)
    }
}

impl Client {
    /// `GET /v1/pages/descendants?slug=…` — every page under one, at any depth.
    ///
    /// A page of results can hold fewer than `page_size` even when more follow,
    /// so only `next_cursor` says whether the listing is complete.
    pub async fn wiki_descendants(
        &self,
        slug: &str,
        cursor: Option<&str>,
        page_size: u32,
    ) -> Result<CursorPage<WikiPageRef>, ApiError> {
        use std::fmt::Write as _;

        let mut url = format!(
            "{}/v1/pages/descendants?slug={}&page_size={page_size}",
            self.wiki_url,
            encode(slug)
        );
        if let Some(cursor) = cursor {
            let _ = write!(url, "&cursor={}", encode(cursor));
        }
        let (value, _) = self
            .send_url(
                reqwest::Method::GET,
                &url,
                None,
                &format!("wiki page `{slug}`"),
            )
            .await
            .map_err(refused)?;
        serde_json::from_value(value).map_err(ApiError::Decode)
    }

    /// `POST /v1/search` — a read, whatever the method says.
    ///
    /// `page` is the Wiki's numbered `cursor` (1..=500); the string cursors it
    /// answers with only say whether another page exists.
    pub async fn wiki_search(
        &self,
        query: &str,
        kind: Option<&str>,
        page: u32,
        limit: u32,
    ) -> Result<WikiHits, ApiError> {
        let mut body = serde_json::json!({ "query": query, "cursor": page, "limit": limit });
        if let Some(kind) = kind {
            body["filters"] = serde_json::json!({ "type": kind });
        }
        let url = format!("{}/v1/search", self.wiki_url);
        let (value, _) = self
            .send_url(reqwest::Method::POST, &url, Some(&body), "wiki search")
            .await
            .map_err(refused)?;
        let answer: SearchAnswer = serde_json::from_value(value).map_err(ApiError::Decode)?;

        let more =
            answer.next_cursor.is_some_and(|cursor| !cursor.is_empty()) && page < LAST_SEARCH_PAGE;
        Ok(WikiHits {
            results: answer.results,
            next_page: more.then_some(page + 1),
        })
    }

    /// Whether the Wiki accepts this token in this organisation.
    ///
    /// `GET /v1/users/me`: the smallest request the Wiki answers, and one that
    /// needs both the token and the organisation header to be right. No page is
    /// read, so the answer costs one request and exposes nothing.
    pub async fn wiki_reachable(&self) -> Result<(), ApiError> {
        let url = format!("{}/v1/users/me", self.wiki_url);
        self.send_url(reqwest::Method::GET, &url, None, "the Wiki's current user")
            .await
            .map(|_| ())
            .map_err(refused)
    }
}

/// One comment, in our schema.
///
/// The author is reduced to the login, which is what the rest of the CLI
/// takes; the thread's size is lifted out of `thread_info`, and only the list
/// of a page's comments sends it.
#[derive(Debug, Clone, Serialize)]
pub struct WikiComment {
    pub id: u64,
    pub author: Option<String>,
    pub created_at: Option<String>,
    /// Somebody else's words, in markup the Wiki does not name.
    pub body: String,
    pub resolved: bool,
    pub deleted: bool,
    /// The passage of the page an inline comment is anchored to.
    pub quote: Option<String>,
    /// Posts in this comment's thread, itself included.
    pub thread_posts: Option<u64>,
}

#[derive(Deserialize)]
struct CommentAnswer {
    id: u64,
    #[serde(default)]
    body: String,
    #[serde(default)]
    author: Option<Person>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    is_deleted: bool,
    #[serde(default)]
    resolve_status: Option<String>,
    #[serde(default)]
    inline_text: Option<String>,
    #[serde(default)]
    thread_info: Option<ThreadInfo>,
}

#[derive(Deserialize)]
struct Person {
    username: String,
}

#[derive(Deserialize)]
struct ThreadInfo {
    total_posts: u64,
}

impl From<CommentAnswer> for WikiComment {
    fn from(answer: CommentAnswer) -> Self {
        Self {
            id: answer.id,
            author: answer.author.map(|person| person.username),
            created_at: answer.created_at,
            body: answer.body,
            resolved: answer.resolve_status.as_deref() == Some("resolved"),
            deleted: answer.is_deleted,
            quote: answer.inline_text.filter(|text| !text.is_empty()),
            thread_posts: answer.thread_info.map(|info| info.total_posts),
        }
    }
}

/// A file attached to a page, in our schema.
#[derive(Debug, Clone, Serialize)]
pub struct WikiAttachment {
    pub id: u64,
    /// Chosen by whoever uploaded it.
    pub name: String,
    /// As the Wiki sends it: a string, in units it does not name.
    pub size: String,
    pub mimetype: Option<String>,
    pub created_at: Option<String>,
    /// The uploader's login.
    pub author: Option<String>,
    pub download_url: Option<String>,
}

#[derive(Deserialize)]
struct AttachmentAnswer {
    id: u64,
    name: String,
    #[serde(default)]
    size: serde_json::Value,
    #[serde(default)]
    mimetype: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    user: Option<Person>,
    #[serde(default)]
    download_url: Option<String>,
}

impl From<AttachmentAnswer> for WikiAttachment {
    fn from(answer: AttachmentAnswer) -> Self {
        Self {
            id: answer.id,
            name: answer.name,
            // Documented as a string; a number is taken too rather than
            // failing the whole listing over one field's type.
            size: match answer.size {
                serde_json::Value::String(size) => size,
                serde_json::Value::Null => "-".to_owned(),
                other => other.to_string(),
            },
            mimetype: answer.mimetype,
            created_at: answer.created_at,
            author: answer.user.map(|person| person.username),
            download_url: answer.download_url,
        }
    }
}

/// A grid named by a listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiGridRef {
    /// A uuid, where pages have numbers.
    #[serde(deserialize_with = "text_id")]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// One thing a page holds: a file or a grid, told apart by `kind`.
#[derive(Debug, Clone, Serialize)]
pub struct WikiResource {
    /// `attachment` or `grid`.
    pub kind: String,
    pub id: String,
    /// A file's name, or a grid's title.
    pub name: String,
    pub created_at: Option<String>,
}

#[derive(Deserialize)]
struct ResourceAnswer {
    #[serde(rename = "type")]
    kind: String,
    item: serde_json::Value,
}

impl From<ResourceAnswer> for WikiResource {
    fn from(answer: ResourceAnswer) -> Self {
        let text = |field: &str| {
            answer.item.get(field).and_then(|value| match value {
                serde_json::Value::String(text) => Some(text.clone()),
                serde_json::Value::Null => None,
                other => Some(other.to_string()),
            })
        };
        Self {
            id: text("id").unwrap_or_default(),
            name: text("name").or_else(|| text("title")).unwrap_or_default(),
            created_at: text("created_at"),
            kind: answer.kind,
        }
    }
}

/// One dynamic table, in our schema.
///
/// Rows keep their cells in column order, as the Wiki sends them, so the
/// columns and the cells pair by position; the values stay as the Wiki typed
/// them — a user, a ticket, a list — for a script to use.
#[derive(Debug, Clone, Serialize)]
pub struct WikiGrid {
    pub id: String,
    pub title: String,
    pub page: Option<WikiPageRef>,
    /// Changes with every edit; a write can name the one it was made against.
    pub revision: String,
    pub columns: Vec<GridColumn>,
    pub rows: Vec<GridRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridColumn {
    /// What `--columns` and `--filter` name the column by.
    pub slug: String,
    pub title: String,
    /// `string`, `number`, `date`, `select`, `staff`, `checkbox`, `ticket`,
    /// `ticket_field`.
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GridRow {
    pub id: String,
    pub cells: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct GridAnswer {
    #[serde(deserialize_with = "text_id")]
    id: String,
    title: String,
    #[serde(default)]
    page: Option<WikiPageRef>,
    #[serde(default, deserialize_with = "text_id")]
    revision: String,
    // A grid just created may come back before it has any structure to show.
    #[serde(default)]
    structure: Structure,
    #[serde(default)]
    rows: Vec<RowAnswer>,
}

#[derive(Default, Deserialize)]
struct Structure {
    #[serde(default)]
    columns: Vec<GridColumn>,
}

#[derive(Deserialize)]
struct RowAnswer {
    #[serde(deserialize_with = "text_id")]
    id: String,
    #[serde(default)]
    row: Vec<serde_json::Value>,
}

impl From<GridAnswer> for WikiGrid {
    fn from(answer: GridAnswer) -> Self {
        Self {
            id: answer.id,
            title: answer.title,
            page: answer.page,
            revision: answer.revision,
            columns: answer.structure.columns,
            rows: answer
                .rows
                .into_iter()
                .map(|row| GridRow {
                    id: row.id,
                    cells: row.row,
                })
                .collect(),
        }
    }
}

/// An id the Wiki documents as a string and sometimes sends as a number.
fn text_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(text) => text,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}

/// What to read of a grid. The Wiki does the filtering, so a narrow question
/// costs a narrow answer.
#[derive(Debug, Clone, Copy, Default)]
pub struct GridQuery<'a> {
    /// `[slug] ~ text AND [n] < 3`, in the Wiki's own syntax.
    pub filter: Option<&'a str>,
    /// `slug, -other`.
    pub sort: Option<&'a str>,
    /// Column slugs, comma-separated.
    pub columns: Option<&'a str>,
    /// Row ids, comma-separated.
    pub rows: Option<&'a str>,
    pub revision: Option<u64>,
}

/// Who can read and edit a page, in our schema.
///
/// The Wiki has no access endpoint to read: the page carries it when asked,
/// in three lists — direct grants, grants by link, and those inherited from a
/// parent. They are flattened into one, each entry saying which list it was.
#[derive(Debug, Clone, Serialize)]
pub struct WikiAccess {
    pub slug: String,
    /// `inherited`, `all_staff` or `custom`.
    pub policy: Option<String>,
    /// What an inherited policy comes to: `all_staff` or `custom`.
    pub inherited_policy: Option<String>,
    /// The role everyone in the organisation has, when the policy is `all_staff`.
    pub all_staff_role: Option<String>,
    pub entries: Vec<AccessEntry>,
}

/// One grant: a role, held by a user or a group.
#[derive(Debug, Clone, Serialize)]
pub struct AccessEntry {
    /// What `wiki regrant` and `wiki revoke` take.
    pub id: String,
    /// `reader`, `editor`, `extra_editor` or `author`.
    pub role: String,
    /// `user` or `group`.
    pub kind: String,
    /// A user's login, or a group's name.
    pub who: String,
    /// `direct`, `by_link` or `inherited`.
    pub via: String,
    pub inheritance: Option<String>,
}

/// A string field, or a number written as one: the Wiki is not consistent.
fn scalar(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

impl WikiAccess {
    fn from_page(slug: &str, page: &serde_json::Value) -> Self {
        let policy = page.get("access_policy");
        let field = |name: &str| scalar(policy.and_then(|policy| policy.get(name)));
        let mut entries = Vec::new();
        if let Some(lists) = page.get("access_lists") {
            for via in ["direct", "by_link", "inherited"] {
                let items = lists.get(via).and_then(serde_json::Value::as_array);
                for item in items.into_iter().flatten() {
                    entries.push(AccessEntry::from_item(item, via));
                }
            }
        }
        Self {
            slug: slug.to_owned(),
            policy: field("access_type"),
            inherited_policy: field("inherited_access_type"),
            all_staff_role: field("all_staff_role"),
            entries,
        }
    }
}

impl AccessEntry {
    fn from_item(item: &serde_json::Value, via: &str) -> Self {
        let present = |name: &str| item.get(name).filter(|value| !value.is_null());
        let (kind, who) = if let Some(user) = present("user") {
            ("user", scalar(user.get("username")))
        } else if let Some(group) = present("group") {
            ("group", scalar(group.get("name")))
        } else {
            ("-", None)
        };
        Self {
            id: scalar(item.get("id")).unwrap_or_default(),
            role: scalar(item.get("role")).unwrap_or_default(),
            kind: kind.to_owned(),
            who: who.unwrap_or_default(),
            via: via.to_owned(),
            inheritance: scalar(item.get("inheritance")),
        }
    }
}

/// Work the Wiki does after it has answered: a clone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiOperation {
    pub id: String,
    /// `clone` for a page, `clone_inline_grid` for a grid.
    #[serde(rename = "type")]
    pub kind: String,
}

/// Where an operation has got to.
#[derive(Debug, Clone, Serialize)]
pub struct OperationStatus {
    /// `scheduled`, `in_progress`, `success` or `failed`.
    pub status: String,
    pub percentage: Option<f64>,
    pub details: Option<String>,
    /// What a finished clone made: the new page, and for a grid the new grid.
    pub result: Option<serde_json::Value>,
}

impl OperationStatus {
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self.status.as_str(), "success" | "failed")
    }

    /// The slug of the page a finished clone made, or landed its grid on.
    #[must_use]
    pub fn page_slug(&self) -> Option<&str> {
        self.result.as_ref()?.get("page")?.get("slug")?.as_str()
    }

    /// The id of the grid a finished grid clone made.
    #[must_use]
    pub fn grid_id(&self) -> Option<String> {
        scalar(self.result.as_ref()?.get("grid_id"))
    }
}

/// Which comments to list.
#[derive(Debug, Clone, Copy)]
pub enum CommentScope<'a> {
    /// The page's comments, optionally only `resolved` or `unresolved` ones.
    Page { status: Option<&'a str> },
    /// Every post in one comment's thread.
    Thread(u64),
}

impl Client {
    /// The numeric id behind a slug.
    ///
    /// Only the page read and the descendants listing take a slug; everything
    /// else under a page wants its id, so this costs one request first.
    pub async fn wiki_page_id(&self, slug: &str) -> Result<u64, ApiError> {
        #[derive(Deserialize)]
        struct Identity {
            id: u64,
        }

        let url = format!("{}/v1/pages?slug={}", self.wiki_url, encode(slug));
        let (value, _) = self
            .send_url(
                reqwest::Method::GET,
                &url,
                None,
                &format!("wiki page `{slug}`"),
            )
            .await
            .map_err(refused)?;
        serde_json::from_value::<Identity>(value)
            .map(|identity| identity.id)
            .map_err(ApiError::Decode)
    }

    /// A page's comments, or one thread of them.
    pub async fn wiki_comments(
        &self,
        slug: &str,
        scope: CommentScope<'_>,
        cursor: Option<&str>,
        page_size: u32,
    ) -> Result<CursorPage<WikiComment>, ApiError> {
        let id = self.wiki_page_id(slug).await?;
        let (tail, what) = match scope {
            CommentScope::Page { status } => (
                format!(
                    "comments?{}",
                    status.map_or_else(String::new, |status| format!("status_filter={status}&"))
                ),
                format!("comments on wiki page `{slug}`"),
            ),
            CommentScope::Thread(comment) => (
                format!("comments/{comment}/thread?"),
                format!("comment {comment} on wiki page `{slug}`"),
            ),
        };
        let page: CursorPage<CommentAnswer> = self
            .wiki_listing(id, &tail, cursor, page_size, &what)
            .await?;
        Ok(CursorPage {
            results: page.results.into_iter().map(WikiComment::from).collect(),
            next_cursor: page.next_cursor,
        })
    }

    /// The files attached to a page.
    pub async fn wiki_attachments(
        &self,
        slug: &str,
        cursor: Option<&str>,
        page_size: u32,
    ) -> Result<CursorPage<WikiAttachment>, ApiError> {
        let id = self.wiki_page_id(slug).await?;
        let page: CursorPage<AttachmentAnswer> = self
            .wiki_listing(
                id,
                "attachments?",
                cursor,
                page_size,
                &format!("attachments of wiki page `{slug}`"),
            )
            .await?;
        Ok(CursorPage {
            results: page.results.into_iter().map(WikiAttachment::from).collect(),
            next_cursor: page.next_cursor,
        })
    }

    /// The grids on a page.
    pub async fn wiki_grids(
        &self,
        slug: &str,
        cursor: Option<&str>,
        page_size: u32,
    ) -> Result<CursorPage<WikiGridRef>, ApiError> {
        let id = self.wiki_page_id(slug).await?;
        self.wiki_listing(
            id,
            "grids?",
            cursor,
            page_size,
            &format!("grids of wiki page `{slug}`"),
        )
        .await
    }

    /// What a page holds, files and grids together, optionally one kind or
    /// only those whose title matches `query`.
    pub async fn wiki_resources(
        &self,
        slug: &str,
        kind: Option<&str>,
        query: Option<&str>,
        cursor: Option<&str>,
        page_size: u32,
    ) -> Result<CursorPage<WikiResource>, ApiError> {
        use std::fmt::Write as _;

        let id = self.wiki_page_id(slug).await?;
        let mut tail = "resources?".to_owned();
        if let Some(kind) = kind {
            let _ = write!(tail, "types={}&", encode(kind));
        }
        if let Some(query) = query {
            let _ = write!(tail, "q={}&", encode(query));
        }
        let page: CursorPage<ResourceAnswer> = self
            .wiki_listing(
                id,
                &tail,
                cursor,
                page_size,
                &format!("resources of wiki page `{slug}`"),
            )
            .await?;
        Ok(CursorPage {
            results: page.results.into_iter().map(WikiResource::from).collect(),
            next_cursor: page.next_cursor,
        })
    }

    /// `GET /v1/grids/{id}`: every row that matches — the Wiki does not page
    /// a grid.
    pub async fn wiki_grid(&self, id: &str, query: GridQuery<'_>) -> Result<WikiGrid, ApiError> {
        use std::fmt::Write as _;

        let mut url = format!("{}/v1/grids/{}", self.wiki_url, encode(id));
        let mut separator = '?';
        for (name, value) in [
            ("filter", query.filter),
            ("sort", query.sort),
            ("only_cols", query.columns),
            ("only_rows", query.rows),
        ] {
            if let Some(value) = value {
                let _ = write!(url, "{separator}{name}={}", encode(value));
                separator = '&';
            }
        }
        if let Some(revision) = query.revision {
            let _ = write!(url, "{separator}revision={revision}");
        }
        let (value, _) = self
            .send_url(
                reqwest::Method::GET,
                &url,
                None,
                &format!("wiki grid `{id}`"),
            )
            .await
            .map_err(refused)?;
        serde_json::from_value::<GridAnswer>(value)
            .map(WikiGrid::from)
            .map_err(ApiError::Decode)
    }

    /// One file on a page, by its id or its name, and the page's id with it.
    ///
    /// The Wiki has no lookup by name, so the listing is read until the file
    /// turns up.
    pub async fn wiki_attachment_named(
        &self,
        slug: &str,
        wanted: &str,
    ) -> Result<(u64, WikiAttachment), ApiError> {
        // Enough for any page a person picks a file from by name; past it the
        // search stops instead of reading an unbounded listing.
        const PAGES: usize = 20;

        let id = self.wiki_page_id(slug).await?;
        let what = format!("attachments of wiki page `{slug}`");
        let mut cursor: Option<String> = None;
        for _ in 0..PAGES {
            let page: CursorPage<AttachmentAnswer> = self
                .wiki_listing(id, "attachments?", cursor.as_deref(), 100, &what)
                .await?;
            if let Some(found) = page
                .results
                .into_iter()
                .map(WikiAttachment::from)
                .find(|file| file.id.to_string() == wanted || file.name == wanted)
            {
                return Ok((id, found));
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Err(ApiError::NotFound(format!(
            "attachment `{wanted}` on wiki page `{slug}`"
        )))
    }

    /// A file's bytes, by page id and file id.
    pub async fn wiki_attachment_bytes(&self, page: u64, file: u64) -> Result<Vec<u8>, ApiError> {
        let url = format!(
            "{}/v1/pages/{page}/attachments/{file}/download",
            self.wiki_url
        );
        self.wiki_bytes(&url, &format!("attachment {file}")).await
    }

    /// A file's bytes, by its address: `<slug>/.files/<name>`. The Wiki follows
    /// a page that has moved.
    pub async fn wiki_file_bytes(&self, path: &str) -> Result<Vec<u8>, ApiError> {
        let url = format!(
            "{}/v1/pages/attachments/download_by_url?url={}",
            self.wiki_url,
            encode(path)
        );
        self.wiki_bytes(&url, &format!("wiki file `{path}`")).await
    }

    /// The body of a download, as bytes.
    ///
    /// The address is always built from the configured Wiki host, never taken
    /// from a payload, so the token goes nowhere else; a redirect to storage
    /// on another host loses the `Authorization` header on the way.
    async fn wiki_bytes(&self, url: &str, what: &str) -> Result<Vec<u8>, ApiError> {
        let response = self.http.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 | 403 => ApiError::WikiForbidden,
                404 => ApiError::NotFound(what.to_owned()),
                _ => ApiError::Rejected {
                    status,
                    message: String::new(),
                },
            });
        }
        Ok(response.bytes().await?.to_vec())
    }

    /// `GET /v1/pages/{id}/{tail}page_size=…&cursor=…`: one page of a listing
    /// under a page. `tail` ends in `?` or `&`, ready for the paging.
    async fn wiki_listing<T: serde::de::DeserializeOwned>(
        &self,
        id: u64,
        tail: &str,
        cursor: Option<&str>,
        page_size: u32,
        what: &str,
    ) -> Result<CursorPage<T>, ApiError> {
        use std::fmt::Write as _;

        let mut url = format!(
            "{}/v1/pages/{id}/{tail}page_size={page_size}",
            self.wiki_url
        );
        if let Some(cursor) = cursor {
            let _ = write!(url, "&cursor={}", encode(cursor));
        }
        let (value, _) = self
            .send_url(reqwest::Method::GET, &url, None, what)
            .await
            .map_err(refused)?;
        let page: CursorPage<T> = serde_json::from_value(value).map_err(ApiError::Decode)?;
        // An empty string is how some listings say "no more"; to the tally it
        // must mean the same as null.
        Ok(CursorPage {
            next_cursor: page.next_cursor.filter(|cursor| !cursor.is_empty()),
            results: page.results,
        })
    }
}

/// A page brought back from deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiRestored {
    pub id: u64,
    pub slug: String,
    /// The page and the subpages restored with it.
    #[serde(default)]
    pub pages_count: Option<u64>,
}

impl Client {
    /// `POST /v1/pages`. The parent is whatever the slug's path says.
    pub async fn wiki_create(
        &self,
        body: &serde_json::Value,
        silent: bool,
    ) -> Result<WikiPageRef, ApiError> {
        let url = format!(
            "{}/v1/pages{}",
            self.wiki_url,
            query(&[silent.then_some("is_silent=true")])
        );
        self.wiki_write(reqwest::Method::POST, &url, Some(body), "the new wiki page")
            .await
    }

    /// `POST /v1/pages/{id}` — an update, whatever the method says. Content
    /// replaces the page's text in full; `merge` lets the Wiki fold in edits
    /// made since, where it would otherwise refuse.
    pub async fn wiki_update(
        &self,
        id: u64,
        body: &serde_json::Value,
        merge: bool,
        silent: bool,
    ) -> Result<WikiPageRef, ApiError> {
        let url = format!(
            "{}/v1/pages/{id}{}",
            self.wiki_url,
            query(&[
                merge.then_some("allow_merge=true"),
                silent.then_some("is_silent=true"),
            ])
        );
        self.wiki_write(
            reqwest::Method::POST,
            &url,
            Some(body),
            &format!("wiki page {id}"),
        )
        .await
    }

    /// `POST /v1/pages/{id}/append-content`.
    pub async fn wiki_append(
        &self,
        id: u64,
        body: &serde_json::Value,
        silent: bool,
    ) -> Result<WikiPageRef, ApiError> {
        let url = format!(
            "{}/v1/pages/{id}/append-content{}",
            self.wiki_url,
            query(&[silent.then_some("is_silent=true")])
        );
        self.wiki_write(
            reqwest::Method::POST,
            &url,
            Some(body),
            &format!("wiki page {id}"),
        )
        .await
    }

    /// `DELETE /v1/pages/{id}`, answering with the one token that restores it.
    pub async fn wiki_delete(&self, id: u64, recursive: bool) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        struct Deleted {
            recovery_token: String,
        }

        // The reference names both flags and not the difference between them;
        // a recursive delete sends both, a plain one neither.
        let url = format!(
            "{}/v1/pages/{id}{}",
            self.wiki_url,
            query(&[
                recursive.then_some("recursive=true"),
                recursive.then_some("allow_recursive=true"),
            ])
        );
        let deleted: Deleted = self
            .wiki_write(
                reqwest::Method::DELETE,
                &url,
                None,
                &format!("wiki page {id}"),
            )
            .await?;
        Ok(deleted.recovery_token)
    }

    /// `POST /v1/pages/{id}/comments`: a comment, or a reply when the body
    /// names a `parent_id`.
    pub async fn wiki_comment(
        &self,
        page: u64,
        body: &serde_json::Value,
    ) -> Result<WikiComment, ApiError> {
        let url = format!("{}/v1/pages/{page}/comments", self.wiki_url);
        let answer: CommentAnswer = self
            .wiki_write(
                reqwest::Method::POST,
                &url,
                Some(body),
                &format!("wiki page {page}"),
            )
            .await?;
        Ok(WikiComment::from(answer))
    }

    /// `DELETE /v1/pages/{id}/comments/{comment}`, answering how many
    /// comments the page has left.
    pub async fn wiki_delete_comment(
        &self,
        page: u64,
        comment: u64,
    ) -> Result<Option<u64>, ApiError> {
        #[derive(Deserialize)]
        struct Left {
            #[serde(default)]
            comments_count: Option<u64>,
        }

        let url = format!("{}/v1/pages/{page}/comments/{comment}", self.wiki_url);
        let left: Left = self
            .wiki_write(
                reqwest::Method::DELETE,
                &url,
                None,
                &format!("comment {comment} on wiki page {page}"),
            )
            .await?;
        Ok(left.comments_count)
    }

    /// Who can read and edit a page: the page read, asked for its access.
    pub async fn wiki_access(&self, slug: &str) -> Result<WikiAccess, ApiError> {
        let url = format!(
            "{}/v1/pages?slug={}&fields=access_policy,access_lists",
            self.wiki_url,
            encode(slug)
        );
        let (value, _) = self
            .send_url(
                reqwest::Method::GET,
                &url,
                None,
                &format!("wiki page `{slug}`"),
            )
            .await
            .map_err(refused)?;
        Ok(WikiAccess::from_page(slug, &value))
    }

    /// `POST /v1/pages/{id}/access`. Unless `allow_selflock`, the Wiki is
    /// asked to refuse a change that would lock the caller out.
    pub async fn wiki_grant(
        &self,
        page: u64,
        body: &serde_json::Value,
        allow_selflock: bool,
    ) -> Result<AccessEntry, ApiError> {
        let url = format!(
            "{}/v1/pages/{page}/access{}",
            self.wiki_url,
            query(&[(!allow_selflock).then_some("prevent_selflock=true")])
        );
        let item: serde_json::Value = self
            .wiki_write(
                reqwest::Method::POST,
                &url,
                Some(body),
                &format!("access to wiki page {page}"),
            )
            .await?;
        Ok(AccessEntry::from_item(&item, "direct"))
    }

    /// `POST /v1/pages/{id}/access/{access}`: another role, or inheritance.
    pub async fn wiki_regrant(
        &self,
        page: u64,
        access: &str,
        body: &serde_json::Value,
        allow_selflock: bool,
    ) -> Result<AccessEntry, ApiError> {
        let url = format!(
            "{}/v1/pages/{page}/access/{}{}",
            self.wiki_url,
            encode(access),
            query(&[(!allow_selflock).then_some("prevent_selflock=true")])
        );
        let item: serde_json::Value = self
            .wiki_write(
                reqwest::Method::POST,
                &url,
                Some(body),
                &format!("access {access} on wiki page {page}"),
            )
            .await?;
        Ok(AccessEntry::from_item(&item, "direct"))
    }

    /// `DELETE /v1/pages/{id}/access/{access}`, or every personal grant on
    /// the page when no access is named.
    pub async fn wiki_revoke(
        &self,
        page: u64,
        access: Option<&str>,
        allow_selflock: bool,
    ) -> Result<(), ApiError> {
        let one = access.map_or_else(String::new, |access| format!("/{}", encode(access)));
        let url = format!(
            "{}/v1/pages/{page}/access{one}{}",
            self.wiki_url,
            query(&[(!allow_selflock).then_some("prevent_selflock=true")])
        );
        let _: serde_json::Value = self
            .wiki_write(
                reqwest::Method::DELETE,
                &url,
                None,
                &format!("access to wiki page {page}"),
            )
            .await?;
        Ok(())
    }

    /// `POST /v1/pages/{id}/clone`: accepted now, done later.
    pub async fn wiki_clone_page(
        &self,
        page: u64,
        body: &serde_json::Value,
    ) -> Result<WikiOperation, ApiError> {
        let url = format!("{}/v1/pages/{page}/clone", self.wiki_url);
        self.wiki_started(&url, body, &format!("wiki page {page}"))
            .await
    }

    /// `POST /v1/grids/{id}/clone`: accepted now, done later.
    pub async fn wiki_clone_grid(
        &self,
        grid: &str,
        body: &serde_json::Value,
    ) -> Result<WikiOperation, ApiError> {
        let url = format!("{}/v1/grids/{}/clone", self.wiki_url, encode(grid));
        self.wiki_started(&url, body, &format!("wiki grid `{grid}`"))
            .await
    }

    async fn wiki_started(
        &self,
        url: &str,
        body: &serde_json::Value,
        what: &str,
    ) -> Result<WikiOperation, ApiError> {
        #[derive(Deserialize)]
        struct Started {
            operation: WikiOperation,
        }

        let started: Started = self
            .wiki_write(reqwest::Method::POST, url, Some(body), what)
            .await?;
        Ok(started.operation)
    }

    /// `GET /v1/operations/{kind}/{id}`: a read, however often it is asked.
    pub async fn wiki_operation(
        &self,
        operation: &WikiOperation,
    ) -> Result<OperationStatus, ApiError> {
        let url = format!(
            "{}/v1/operations/{}/{}",
            self.wiki_url,
            encode(&operation.kind),
            encode(&operation.id)
        );
        let (value, _) = self
            .send_url(
                reqwest::Method::GET,
                &url,
                None,
                &format!("operation {}/{}", operation.kind, operation.id),
            )
            .await
            .map_err(refused)?;
        let progress = value.get("progress");
        Ok(OperationStatus {
            status: scalar(value.get("status")).unwrap_or_default(),
            percentage: progress
                .and_then(|progress| progress.get("percentage"))
                .and_then(serde_json::Value::as_f64),
            details: scalar(progress.and_then(|progress| progress.get("details")))
                .filter(|details| !details.is_empty()),
            result: value
                .get("result")
                .filter(|result| !result.is_null())
                .cloned(),
        })
    }

    /// `POST /v1/grids`: a new grid on a page, with no columns yet.
    pub async fn wiki_grid_create(&self, body: &serde_json::Value) -> Result<WikiGrid, ApiError> {
        let url = format!("{}/v1/grids", self.wiki_url);
        let answer: GridAnswer = self
            .wiki_write(reqwest::Method::POST, &url, Some(body), "the new wiki grid")
            .await?;
        Ok(WikiGrid::from(answer))
    }

    /// One change under `/v1/grids/{id}`, answered with what the Wiki says —
    /// the new revision among it.
    pub async fn wiki_grid_write(
        &self,
        method: reqwest::Method,
        grid: &str,
        tail: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, ApiError> {
        let url = format!("{}/v1/grids/{}{tail}", self.wiki_url, encode(grid));
        self.wiki_write(method, &url, body, &format!("wiki grid `{grid}`"))
            .await
    }

    /// `POST /v1/recovery_tokens/{token}/recover`.
    pub async fn wiki_restore(&self, token: &str) -> Result<WikiRestored, ApiError> {
        let url = format!(
            "{}/v1/recovery_tokens/{}/recover",
            self.wiki_url,
            encode(token)
        );
        self.wiki_write(
            reqwest::Method::POST,
            &url,
            Some(&serde_json::json!({})),
            &format!("recovery token `{token}`"),
        )
        .await
    }

    async fn wiki_write<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<&serde_json::Value>,
        what: &str,
    ) -> Result<T, ApiError> {
        let (value, _) =
            self.send_url(method, url, body, what)
                .await
                .map_err(|error| match error {
                    ApiError::Forbidden | ApiError::Unauthorized => ApiError::WikiWriteForbidden,
                    other => other,
                })?;
        serde_json::from_value(value).map_err(ApiError::Decode)
    }
}

/// `?a&b` from whichever parts are present, or nothing at all.
fn query(parts: &[Option<&str>]) -> String {
    let present: Vec<&str> = parts.iter().flatten().copied().collect();
    if present.is_empty() {
        String::new()
    } else {
        format!("?{}", present.join("&"))
    }
}

/// A refusal from the Wiki, told apart from Tracker's.
///
/// The Wiki answers 401 to a token it will not serve — the documented case — and
/// 403 to one that lacks rights, and both most often mean a token issued before
/// `wiki:read` was granted. Tracker's 401 means the token itself is dead; the
/// Wiki's is fixed by one command, which the error can name.
fn refused(error: ApiError) -> ApiError {
    match error {
        ApiError::Forbidden | ApiError::Unauthorized => ApiError::WikiForbidden,
        other => other,
    }
}

/// The slug in whatever was pasted: a slug already, or a page's full address.
///
/// Query and fragment go, and so do the slashes around the path, which the
/// browser adds and the API does not want. A copied address arrives
/// percent-encoded — every Cyrillic slug does — and is decoded here, or it
/// would be encoded a second time on its way to the API and name no page.
#[must_use]
pub fn slug_of(target: &str) -> String {
    let path = match target.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map_or("", |(_, path)| path),
        None => target,
    };
    decode(
        path.split(['?', '#'])
            .next()
            .unwrap_or_default()
            .trim_matches('/'),
    )
}

/// `%D0%B7` back to `з`. Text that does not decode to UTF-8 is kept as typed:
/// a slug with a literal `%` in it is rarer than a mangled one, but not ours
/// to guess at.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                at += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}

/// Percent-encoding for one query value. A slug is a path, and its slashes
/// have to survive being put inside another URL's query.
fn encode(text: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_is_taken_as_it_is() {
        assert_eq!(
            slug_of("users/ilubenets/runbook"),
            "users/ilubenets/runbook"
        );
    }

    /// What people actually have is the address bar.
    #[test]
    fn an_address_is_reduced_to_its_slug() {
        assert_eq!(
            slug_of("https://wiki.yandex.ru/users/ilubenets/runbook/?from=search#deploy"),
            "users/ilubenets/runbook"
        );
    }

    /// The browser hands over a Cyrillic slug percent-encoded; sent on as it
    /// is, it would be encoded twice and name no page.
    #[test]
    fn a_copied_address_is_decoded() {
        assert_eq!(
            slug_of(
                "https://wiki.yandex.ru/users/%D1%8F%D0%BD/%D0%B7%D0%B0%D0%BC%D0%B5%D1%82%D0%BA%D0%B8/"
            ),
            "users/ян/заметки"
        );
        // Not a valid escape, or not UTF-8 once decoded: kept as typed.
        assert_eq!(slug_of("users/100%/x"), "users/100%/x");
        assert_eq!(slug_of("users/%FF"), "users/%FF");
    }

    #[test]
    fn a_bare_host_names_no_page() {
        assert_eq!(slug_of("https://wiki.yandex.ru/"), "");
        assert_eq!(slug_of("https://wiki.yandex.ru"), "");
    }

    /// Cyrillic slugs exist; they have to reach the API byte for byte.
    #[test]
    fn a_slug_survives_being_put_in_a_query() {
        assert_eq!(
            encode("users/ян/заметки"),
            "users%2F%D1%8F%D0%BD%2F%D0%B7%D0%B0%D0%BC%D0%B5%D1%82%D0%BA%D0%B8"
        );
    }
}
