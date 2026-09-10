//! Signing in through a device code, and renewing what it produced.
//!
//! The login cases run the binary with `--dry-run`, so the grant is really
//! fetched from the stub and verified against Tracker, and only the keychain
//! write is skipped. `auth refresh` reads the keychain before it asks anything,
//! so its exchange is covered one level down, against `oauth::App` itself.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use predicates::prelude::*;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod harness;
use harness::Harness;

fn code() -> serde_json::Value {
    serde_json::json!({
        "device_code": "device-code",
        "user_code": "ABCD1234",
        "verification_url": "https://oauth.yandex.ru/device",
        "interval": 0,
        "expires_in": 300
    })
}

fn myself() -> serde_json::Value {
    serde_json::json!({ "uid": 1, "login": "ilubenets", "display": "Ilya Lubenets" })
}

fn sign_in(harness: &Harness) -> assert_cmd::Command {
    let mut command = harness.run_raw(&[
        "auth",
        "login",
        "--account",
        "work",
        "--org-id",
        "12345",
        "--device",
        "--dry-run",
    ]);
    command
        .env("YTCLI_OAUTH_URL", harness.server.uri())
        .env("YTCLI_OAUTH_CLIENT_ID", "app-id")
        .env("YTCLI_OAUTH_CLIENT_SECRET", "app-secret");
    command
}

/// The whole point: a code on screen, a wait while it is confirmed, and a token
/// that is then verified like a pasted one — without anything being copied.
#[tokio::test]
async fn a_confirmed_code_becomes_a_verified_token() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/device/code"))
        .and(body_string_contains("client_id=app-id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(code()))
        .mount(&harness.server)
        .await;
    // Not confirmed yet on the first poll.
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(serde_json::json!({"error": "authorization_pending"})),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=device_code"))
        .and(body_string_contains("code=device-code"))
        .and(body_string_contains("client_secret=app-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "signed-in-token",
            "refresh_token": "renewal",
            "token_type": "bearer",
            "expires_in": 31_536_000
        })))
        .with_priority(2)
        .mount(&harness.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/myself"))
        .and(header("authorization", "OAuth signed-in-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(myself()))
        .mount(&harness.server)
        .await;

    sign_in(&harness)
        .assert()
        .success()
        .stderr(predicate::str::contains("  1. Copy the code   ABCD1234\n"))
        .stderr(predicate::str::contains(
            "  2. Open the page   https://oauth.yandex.ru/device   (expires in 5 min)\n",
        ))
        .stderr(predicate::str::contains(
            "verified as ilubenets in org 12345",
        ))
        .stderr(predicate::str::contains("dry run: would store a token"));
}

/// `--read-only` has to reach Yandex as a scope, or the token gets everything
/// the application may grant — and it covers the Wiki too, or a read-only
/// login would leave every `wiki` command refused.
#[tokio::test]
async fn read_only_asks_for_the_read_scope() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/device/code"))
        .and(body_string_contains("scope=tracker%3Aread%20wiki%3Aread"))
        .respond_with(ResponseTemplate::new(200).set_body_json(code()))
        .expect(1)
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": "access_denied"})),
        )
        .mount(&harness.server)
        .await;

    sign_in(&harness).arg("--read-only").assert().code(3);
}

/// Declining in the browser is an auth failure, said as such, and nothing is
/// stored or verified.
#[tokio::test]
async fn a_declined_sign_in_is_an_auth_failure() {
    let harness = Harness::new().await;
    Mock::given(method("POST"))
        .and(path("/device/code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(code()))
        .mount(&harness.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": "access_denied"})),
        )
        .mount(&harness.server)
        .await;

    sign_in(&harness)
        .assert()
        .code(3)
        .stderr(predicate::str::contains("declined in the browser"))
        .stderr(predicate::str::contains("would store").not());
}

async fn app_with_code(server: &MockServer) -> (ytcli::oauth::App, ytcli::oauth::DeviceCode) {
    Mock::given(method("POST"))
        .and(path("/device/code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(code()))
        .mount(server)
        .await;
    let app = ytcli::oauth::App::new(&server.uri(), "app-id".into(), "app-secret".into()).unwrap();
    let code = app.request_code(None).await.unwrap();
    (app, code)
}

/// Someone who confirms first and presses Enter afterwards is signed in
/// already, and must not be sent to the page a second time.
#[tokio::test]
async fn a_code_confirmed_before_enter_is_picked_up_at_once() {
    let server = MockServer::start().await;
    let (app, code) = app_with_code(&server).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "signed-in-token"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let grant = app.try_grant(&code).await.unwrap();
    assert_eq!(grant.unwrap().access_token, "signed-in-token");
}

/// Not confirmed yet is not a failure: it is the cue to open the page.
#[tokio::test]
async fn a_code_not_confirmed_yet_is_nothing_rather_than_an_error() {
    let server = MockServer::start().await;
    let (app, code) = app_with_code(&server).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(serde_json::json!({"error": "authorization_pending"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    assert!(app.try_grant(&code).await.unwrap().is_none());
}

/// Yandex may keep handing back the same token; what matters here is that the
/// refresh token and the application's secret are what gets sent.
#[tokio::test]
async fn a_refresh_spends_the_refresh_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=renewal"))
        .and(body_string_contains("client_secret=app-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "renewed",
            "refresh_token": "next-renewal"
        })))
        .mount(&server)
        .await;

    let app = ytcli::oauth::App::new(&server.uri(), "app-id".into(), "app-secret".into()).unwrap();
    let grant = app.refresh("renewal").await.unwrap();
    assert_eq!(grant.access_token, "renewed");
    assert_eq!(grant.refresh_token.as_deref(), Some("next-renewal"));
}

/// A refresh token that no longer works has to say why, in Yandex's words.
#[tokio::test]
async fn a_rejected_refresh_names_the_reason() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "expired refresh token"
        })))
        .mount(&server)
        .await;

    let app = ytcli::oauth::App::new(&server.uri(), "app-id".into(), "app-secret".into()).unwrap();
    let error = app.refresh("renewal").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Yandex OAuth refused the request: invalid_grant — expired refresh token"
    );
    assert_eq!(error.exit_code(), ytcli::exit::ExitCode::Auth);
}
