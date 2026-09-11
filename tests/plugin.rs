//! The Claude plugin must carry the version of the CLI it ships with.
//!
//! `claude plugin update` compares `.claude-plugin/plugin.json`'s version and
//! nothing else, so a release that forgets to bump it tells every user their
//! skill is current when it is not. It stayed at 0.1.0 through 2.0.0 that way.

#![allow(clippy::expect_used)]

#[test]
fn the_plugin_version_is_the_crate_version() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".claude-plugin/plugin.json");
    let text = std::fs::read_to_string(&path).expect("plugin.json readable");
    let manifest: serde_json::Value = serde_json::from_str(&text).expect("plugin.json is JSON");

    assert_eq!(
        manifest["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "bump `version` in .claude-plugin/plugin.json to match Cargo.toml"
    );
}
