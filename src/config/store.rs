//! Writing the config file back.
//!
//! `auth login` is the only path that edits configuration, and it edits a file
//! people also write by hand — the docs tell them to. So the edit is surgical:
//! `toml_edit` keeps existing comments, key order and formatting, and only the
//! touched keys change. Serialising the whole struct back would silently delete
//! whatever the user had written around it.

use std::path::Path;

use toml_edit::{DocumentMut, Item, Table, value};

use crate::config::{OrgKind, Profile};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not read the configuration file")]
    Read(#[source] std::io::Error),
    #[error("could not write the configuration file")]
    Write(#[source] std::io::Error),
    #[error("the configuration file is not valid TOML; fix or move it first")]
    Parse(#[from] toml_edit::TomlError),
}

/// Add or update an account and, optionally, a profile pointing at it.
///
/// Returns the file's new contents, so a caller can show what changed without
/// reading the file back.
pub fn upsert(
    path: &Path,
    account: &str,
    description: Option<&str>,
    profile: Option<(&str, &Profile)>,
    make_default: bool,
) -> Result<String, StoreError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(StoreError::Read(error)),
    };

    let mut document: DocumentMut = existing.parse()?;

    let accounts = implicit_table(&mut document, "accounts");
    let entry = accounts
        .entry(account)
        .or_insert_with(|| Item::Table(Table::new()));
    if let (Some(table), Some(description)) = (entry.as_table_mut(), description) {
        table["description"] = value(description);
    }

    if let Some((name, profile)) = profile {
        let profiles = implicit_table(&mut document, "profiles");
        let entry = profiles
            .entry(name)
            .or_insert_with(|| Item::Table(Table::new()));
        if let Some(table) = entry.as_table_mut() {
            table["account"] = value(&profile.account);
            table["org_id"] = value(&profile.org_id);
            table["org_kind"] = value(match profile.org_kind {
                OrgKind::Cloud => "cloud",
                OrgKind::Yandex360 => "yandex360",
            });
            match &profile.default_queue {
                Some(queue) => table["default_queue"] = value(queue),
                None => {
                    table.remove("default_queue");
                }
            }
        }

        // The caller decides whether to become the default; it never happens as
        // a side effect of writing a profile, because which profile is default
        // decides which organisation a bare command touches.
        if make_default {
            document["default_profile"] = value(name);
        }
    }

    let rendered = document.to_string();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StoreError::Write)?;
    }
    write_private(path, &rendered).map_err(StoreError::Write)?;

    Ok(rendered)
}

/// Write with owner-only permissions.
///
/// The file holds no secrets by design, but it does name organisations and
/// accounts, and a config a group can rewrite is a config someone else can point
/// at their own organisation.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// A `[accounts.x]`-style parent table, created without emitting an empty
/// `[accounts]` header of its own.
fn implicit_table<'a>(document: &'a mut DocumentMut, name: &str) -> &'a mut Table {
    let entry = document
        .entry(name)
        .or_insert_with(|| Item::Table(Table::new()));
    if let Some(table) = entry.as_table_mut() {
        table.set_implicit(true);
    }
    entry
        .as_table_mut()
        .unwrap_or_else(|| unreachable!("just inserted a table"))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::Display;

    fn profile() -> Profile {
        Profile {
            account: "work".to_owned(),
            org_id: "12345".to_owned(),
            org_kind: OrgKind::Cloud,
            default_queue: Some("PROJ".to_owned()),
            display: Display::default(),
        }
    }

    #[test]
    fn writes_a_file_that_did_not_exist() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested").join("config.toml");

        let written = upsert(
            &path,
            "work",
            Some("main"),
            Some(("work", &profile())),
            true,
        )
        .expect("written");

        assert!(written.contains("[accounts.work]"));
        assert!(written.contains("[profiles.work]"));
        assert!(written.contains(r#"org_kind = "cloud""#));
        assert!(written.contains(r#"default_profile = "work""#));
        assert!(path.exists());
    }

    /// People hand-write this file; the docs tell them to. An edit must not eat
    /// what they wrote around it.
    #[test]
    fn keeps_comments_and_unrelated_entries() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"# my notes about which org is which
default_profile = "other"

[accounts.personal]
description = "everyday login"

[profiles.other]
account = "personal"
org_id = "98765"
org_kind = "yandex360"
"#,
        )
        .expect("write");

        let written = upsert(
            &path,
            "work",
            Some("admin"),
            Some(("work", &profile())),
            false,
        )
        .expect("written");

        assert!(written.contains("# my notes about which org is which"));
        assert!(written.contains("[accounts.personal]"));
        assert!(written.contains("[profiles.other]"));
        assert!(written.contains("[profiles.work]"));
        // An existing default is not moved unless asked.
        assert!(written.contains(r#"default_profile = "other""#));
    }

    #[test]
    fn updating_an_existing_profile_replaces_its_fields() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        upsert(&path, "work", None, Some(("work", &profile())), true).expect("first");

        let mut moved = profile();
        moved.org_id = "999".to_owned();
        moved.org_kind = OrgKind::Yandex360;
        moved.default_queue = None;
        let written = upsert(&path, "work", None, Some(("work", &moved)), false).expect("second");

        assert!(written.contains(r#"org_id = "999""#));
        assert!(written.contains(r#"org_kind = "yandex360""#));
        assert!(!written.contains("default_queue"));
        assert_eq!(written.matches("[profiles.work]").count(), 1);
    }

    #[test]
    fn a_broken_file_is_reported_rather_than_overwritten() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is not [[[ toml").expect("write");

        assert!(upsert(&path, "work", None, None, false).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            "this is not [[[ toml"
        );
    }
}
