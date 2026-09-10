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
