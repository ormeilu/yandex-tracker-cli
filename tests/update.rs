//! Asking a registry what the newest version is.
//!
//! The notice itself is a pure function with unit tests beside it; what needs a
//! server is the part that talks to one — that both registry shapes are read,
//! and that every way the request can go wrong ends in silence rather than in
//! something a command would have to report.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use wiremock::matchers::{header_regex, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ytcli::update;

/// The crate registry answers with the newest version that is not a pre-release.
#[tokio::test]
async fn a_crates_io_answer_is_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/crates/yandex-tracker-cli"))
        // crates.io refuses a request that does not say who is asking, so the
        // agent is part of the contract rather than a nicety.
        .and(header_regex("user-agent", "^ytcli/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "crate": {"max_stable_version": "9.9.9", "newest_version": "9.9.9-rc.1"}
        })))
        .mount(&server)
        .await;

    let latest = update::latest(&format!(
        "{}/api/v1/crates/yandex-tracker-cli",
        server.uri()
    ))
    .await;

    assert_eq!(latest.as_deref(), Some("9.9.9"));
}

/// The wheel index is where a `uv` or `pipx` install came from, and it answers
/// in a shape of its own.
#[tokio::test]
async fn a_pypi_answer_is_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/pypi/yandex-tracker-cli/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "info": {"name": "yandex-tracker-cli", "version": "9.9.9"}
        })))
        .mount(&server)
        .await;

    let latest = update::latest(&format!("{}/pypi/yandex-tracker-cli/json", server.uri())).await;

    assert_eq!(latest.as_deref(), Some("9.9.9"));
}

/// A rate limit, an outage or a body in a shape we do not know all end the same
/// way: no answer, and nothing said about it. The check is an afterthought and
/// must never become something a command has to report.
#[tokio::test]
async fn a_registry_that_will_not_answer_is_simply_no_answer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rate-limited"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/nonsense"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&server)
        .await;

    assert_eq!(
        update::latest(&format!("{}/rate-limited", server.uri())).await,
        None
    );
    assert_eq!(
        update::latest(&format!("{}/nonsense", server.uri())).await,
        None
    );
    // Nothing listening at all, which is what no network looks like from here.
    assert_eq!(update::latest("http://127.0.0.1:1/crates").await, None);
}
