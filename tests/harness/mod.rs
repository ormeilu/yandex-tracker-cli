//! Shared scaffolding for the end-to-end tests.
//!
//! Every test here runs the real binary against a `wiremock` stub, which is the
//! only way to cover what unit tests cannot: header construction, the number of
//! requests a command makes, format selection, and the exit codes callers
//! branch on.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    dead_code,
    // The module is included per test binary, so items used by one binary look
    // unreachable to another.
    unreachable_pub
)]

use assert_cmd::Command;
use wiremock::MockServer;

/// A profile with pinned display defaults, so the tests do not move when the
/// built-in defaults do.
const CONFIG: &str = r#"
[accounts.test]

[profiles.test]
account = "test"
org_id = "12345"
org_kind = "cloud"

[profiles.test.display]
description_lines = 3
extra_fields = ["storyPoints"]
"#;

/// Read one of the recorded response shapes from `tests/fixtures`.
pub fn fixture(name: &str) -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(path).expect("fixture readable");
    serde_json::from_str(&text).expect("fixture is valid json")
}

pub struct Harness {
    pub server: MockServer,
    _dir: tempfile::TempDir,
    config: std::path::PathBuf,
}

impl Harness {
    pub async fn new() -> Self {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().expect("temp dir");
        let config = dir.path().join("config.toml");
        std::fs::write(&config, CONFIG).expect("write config");
        Self {
            server,
            _dir: dir,
            config,
        }
    }

    /// The binary, pointed at the stub and given a token the way CI would.
    pub fn run(&self, args: &[&str]) -> Command {
        let mut command = Command::cargo_bin("ytcli").expect("binary built");
        command
            .args(args)
            .arg("--profile")
            .arg("test")
            .arg("--config")
            .arg(&self.config)
            .env("YTCLI_BASE_URL", self.server.uri())
            .env("YTCLI_TOKEN", "test-token")
            .env_remove("YTCLI_PROFILE");
        command
    }
}
