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

/// A 403 from the Wiki, told apart from Tracker's.
///
/// Tracker's usual cause is the wrong organisation header; the Wiki's is a
/// token issued before `wiki:read` was granted, and the fix for that is one
/// command, which the error can name.
fn refused(error: ApiError) -> ApiError {
    match error {
        ApiError::Forbidden => ApiError::WikiForbidden,
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
