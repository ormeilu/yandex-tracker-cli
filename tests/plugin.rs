//! The plugins must carry the version of the CLI they ship with.
//!
//! `claude plugin update` compares the manifest's version and nothing else, so
//! a release that forgets to bump it tells every user their skill is current
//! when it is not. Both manifests stayed at 0.1.0 through 2.0.0 that way.

#![allow(clippy::expect_used)]

use std::path::Path;

fn version_of(manifest: &str) -> Option<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest);
    let text = std::fs::read_to_string(&path).expect("manifest readable");
    let manifest: serde_json::Value = serde_json::from_str(&text).expect("manifest is JSON");
    manifest["version"].as_str().map(ToOwned::to_owned)
}

#[test]
fn the_plugin_versions_are_the_crate_version() {
    for manifest in [
        "plugin/.claude-plugin/plugin.json",
        "plugin/.codex-plugin/plugin.json",
    ] {
        assert_eq!(
            version_of(manifest).as_deref(),
            Some(env!("CARGO_PKG_VERSION")),
            "bump `version` in {manifest} to match Cargo.toml"
        );
    }
}

/// The marketplace installs the plugin directory, not the checkout: a local
/// install copies whatever `source` names, `target/` included when it is `./`.
#[test]
fn the_marketplace_points_at_the_plugin_directory() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".claude-plugin/marketplace.json");
    let text = std::fs::read_to_string(&path).expect("marketplace readable");
    let marketplace: serde_json::Value = serde_json::from_str(&text).expect("marketplace is JSON");
    assert_eq!(marketplace["plugins"][0]["source"], "./plugin");
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugin/skills/ytcli/SKILL.md")
            .is_file()
    );
}
