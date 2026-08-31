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
            table["org_kind"] = value(kind_name(profile.org_kind));
            match &profile.default_queue {
                Some(queue) => table["default_queue"] = value(queue),
                None => {
                    table.remove("default_queue");
                }
            }
            // Unlike the keys above, an absent description means "not said"
            // rather than "cleared": login knows the intended queue every time
            // it runs and does not ask about the note unless told to, so
            // rewriting the profile must leave a hand-written one alone.
            // `edit` is how it goes away.
            if let Some(description) = &profile.description {
                table["description"] = value(description);
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

/// Point `default_profile` at an existing profile.
///
/// Separate from [`upsert`] because changing which organisation a bare command
/// touches is its own decision, not a side effect of writing a profile — and
/// because it must not require a token: switching profiles is a local edit, and
/// asking the keychain for a credential to make one would be theatre.
pub fn set_default(path: &Path, name: &str) -> Result<String, StoreError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(StoreError::Read(error)),
    };

    let mut document: DocumentMut = existing.parse()?;
    document["default_profile"] = value(name);
    let rendered = document.to_string();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StoreError::Write)?;
    }
    write_private(path, &rendered).map_err(StoreError::Write)?;

    Ok(rendered)
}

/// What an edit changes about a profile.
///
/// Two levels of optionality, and they mean different things: the outer
/// `None` is "not mentioned, leave it alone", and `Some(None)` on the fields
/// that have it is "remove this key". A command that edits one thing must not
/// quietly rewrite the rest.
#[derive(Debug, Default)]
pub struct Edits<'a> {
    /// Rename the profile itself.
    pub name: Option<&'a str>,
    pub account: Option<&'a str>,
    pub org_id: Option<&'a str>,
    pub org_kind: Option<OrgKind>,
    pub description: Option<Option<&'a str>>,
    pub default_queue: Option<Option<&'a str>>,
}

impl Edits<'_> {
    /// Nothing to do. The caller refuses rather than rewriting the file for no
    /// reason: a no-op that reports success looks exactly like a change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.account.is_none()
            && self.org_id.is_none()
            && self.org_kind.is_none()
            && self.description.is_none()
            && self.default_queue.is_none()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("no profile called `{0}` in the configuration file")]
    Unknown(String),
    #[error("a profile called `{0}` already exists; pick another name or remove that one")]
    NameTaken(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Change an existing profile in place, optionally renaming it.
///
/// Its own function rather than a mode of [`upsert`], and for the same reason
/// [`set_default`] is: editing a profile is a local edit to a file the user
/// owns, and making them log in again — token, verification and all — to fix a
/// typo in an organisation id would be theatre.
///
/// A rename moves the table rather than copying its fields, so `[profiles.x.display]`
/// and anything hand-written inside it travel with it, and `default_profile` is
/// carried across because a default naming a profile that no longer exists
/// breaks every later command.
pub fn edit(path: &Path, profile: &str, edits: &Edits<'_>) -> Result<String, EditError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(StoreError::Read(error).into()),
    };

    let mut document: DocumentMut = existing.parse().map_err(StoreError::from)?;

    let profiles = implicit_table(&mut document, "profiles");
    if !profiles.contains_key(profile) {
        return Err(EditError::Unknown(profile.to_owned()));
    }
    if let Some(taken) = edits
        .name
        .filter(|name| *name != profile && profiles.contains_key(name))
    {
        return Err(EditError::NameTaken(taken.to_owned()));
    }

    let entry = profiles
        .entry(profile)
        .or_insert_with(|| Item::Table(Table::new()));
    if let Some(table) = entry.as_table_mut() {
        if let Some(account) = edits.account {
            table["account"] = value(account);
        }
        if let Some(org_id) = edits.org_id {
            table["org_id"] = value(org_id);
        }
        if let Some(org_kind) = edits.org_kind {
            table["org_kind"] = value(kind_name(org_kind));
        }
        if let Some(description) = edits.description {
            set_or_remove(table, "description", description);
        }
        if let Some(queue) = edits.default_queue {
            set_or_remove(table, "default_queue", queue);
        }
    }

    if let Some(new_name) = edits.name.filter(|name| *name != profile) {
        if let Some(moved) = profiles.remove(profile) {
            profiles.insert(new_name, moved);
        }
        if document.get("default_profile").and_then(Item::as_str) == Some(profile) {
            document["default_profile"] = value(new_name);
        }
    }

    let rendered = document.to_string();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StoreError::Write)?;
    }
    write_private(path, &rendered).map_err(StoreError::Write)?;

    Ok(rendered)
}

/// What removing a profile changed beyond the profile itself.
#[derive(Debug)]
pub struct Removed {
    /// `default_profile` named this profile and was dropped with it.
    pub cleared_default: bool,
    /// The file's new contents, so the caller need not read it back.
    pub contents: String,
}

/// Delete a profile from the config file.
///
/// The whole `[profiles.x]` table goes, display settings included: half a
/// profile is worse than none, because the keys left behind still resolve and
/// still send requests. `default_profile` naming it is dropped rather than
/// pointed somewhere else — guessing which organisation should inherit the
/// bare command is exactly the guess this tool does not make.
///
/// The account and its keychain token are untouched: an account can back
/// several profiles, and forgetting a credential is `auth logout`.
pub fn remove(path: &Path, profile: &str) -> Result<Removed, EditError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(StoreError::Read(error).into()),
    };

    let mut document: DocumentMut = existing.parse().map_err(StoreError::from)?;

    let profiles = implicit_table(&mut document, "profiles");
    if profiles.remove(profile).is_none() {
        return Err(EditError::Unknown(profile.to_owned()));
    }

    let cleared_default = document.get("default_profile").and_then(Item::as_str) == Some(profile);
    if cleared_default {
        // A comment written above `default_profile` is usually about the file
        // rather than about that one key, so it outlives the key: dropping it
        // with the line would quietly edit prose the user wrote by hand.
        let carried = document
            .as_table()
            .key("default_profile")
            .and_then(|key| key.leaf_decor().prefix().cloned());
        document.remove("default_profile");
        if let Some(text) = carried
            .as_ref()
            .and_then(toml_edit::RawString::as_str)
            .filter(|prefix| has_comment(prefix))
        {
            carry_prefix(document.as_table_mut(), text);
        }
    }

    let rendered = document.to_string();

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StoreError::Write)?;
    }
    write_private(path, &rendered).map_err(StoreError::Write)?;

    Ok(Removed {
        cleared_default,
        contents: rendered,
    })
}

/// Whether a chunk of decor holds anything a person wrote.
fn has_comment(prefix: &str) -> bool {
    prefix
        .lines()
        .any(|line| line.trim_start().starts_with('#'))
}

/// Move a departing key's leading comment onto whatever now comes first.
///
/// "First" is the first thing that is actually written out, which is not always
/// the first entry: `[accounts]` exists in the tree while only `[accounts.admin]`
/// appears in the file, and decor on a table nobody renders is decor nobody
/// reads.
fn carry_prefix(table: &mut Table, text: &str) -> bool {
    let Some(name) = table.iter().map(|(name, _)| name.to_owned()).next() else {
        return false;
    };

    if table
        .get(&name)
        .and_then(Item::as_table)
        .is_some_and(Table::is_implicit)
    {
        return table
            .get_mut(&name)
            .and_then(Item::as_table_mut)
            .is_some_and(|inner| carry_prefix(inner, text));
    }

    // A table wears its comment above its header; a plain key wears it above
    // the key itself.
    if let Some(inner) = table.get_mut(&name).and_then(Item::as_table_mut) {
        prepend(inner.decor_mut(), text);
        return true;
    }
    if let Some(mut key) = table.key_mut(&name) {
        prepend(key.leaf_decor_mut(), text);
        return true;
    }

    false
}

fn prepend(decor: &mut toml_edit::Decor, text: &str) {
    let existing = decor
        .prefix()
        .and_then(toml_edit::RawString::as_str)
        .unwrap_or("")
        .to_owned();
    decor.set_prefix(format!("{text}{existing}"));
}

fn set_or_remove(table: &mut Table, key: &str, wanted: Option<&str>) {
    match wanted {
        Some(text) => table[key] = value(text),
        None => {
            table.remove(key);
        }
    }
}

fn kind_name(kind: OrgKind) -> &'static str {
    match kind {
        OrgKind::Cloud => "cloud",
        OrgKind::Yandex360 => "yandex360",
    }
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
            description: None,
            default_queue: Some("PROJ".to_owned()),
            display: Display::default(),
        }
    }

    /// The config is a file people write by hand, and the docs tell them to.
    /// Switching the default must not cost them their comments.
    #[test]
    fn setting_the_default_keeps_what_was_written_around_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# my notes\ndefault_profile = \"work\"\n\n[profiles.home]\naccount = \"me\"\n",
        )
        .expect("write");

        let written = set_default(&path, "home").expect("set default");

        assert!(written.contains("# my notes"));
        assert!(written.contains(r#"default_profile = "home""#));
        assert!(written.contains("[profiles.home]"));
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

    /// The note is about the organisation, which has not changed because a
    /// token was renewed. A login that does not mention it must leave it be.
    #[test]
    fn a_login_that_says_nothing_about_the_description_keeps_the_one_on_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        let mut described = profile();
        described.description = Some("production — customer data".to_owned());
        upsert(&path, "work", None, Some(("work", &described)), true).expect("first");

        let written =
            upsert(&path, "work", None, Some(("work", &profile())), false).expect("second");

        assert!(written.contains(r#"description = "production — customer data""#));
    }

    #[test]
    fn a_description_can_be_set_and_removed_without_touching_anything_else() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# my notes\n\n[profiles.work]\naccount = \"me\"\norg_id = \"12345\"\n",
        )
        .expect("write");

        let written = edit(
            &path,
            "work",
            &Edits {
                description: Some(Some("sandbox")),
                ..Edits::default()
            },
        )
        .expect("set");
        assert!(written.contains(r#"description = "sandbox""#));
        assert!(written.contains("# my notes"));
        assert!(written.contains(r#"org_id = "12345""#));

        let cleared = edit(
            &path,
            "work",
            &Edits {
                description: Some(None),
                ..Edits::default()
            },
        )
        .expect("clear");
        assert!(!cleared.contains("description"));
        assert!(cleared.contains(r#"org_id = "12345""#));
    }

    /// A rename that loses the display settings, or leaves `default_profile`
    /// pointing at a name that no longer exists, is worse than no rename.
    #[test]
    fn renaming_moves_the_whole_profile_and_the_default_with_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"default_profile = "work"

[profiles.work]
account = "me"
org_id = "12345"
org_kind = "cloud"

[profiles.work.display]
limit = 5
"#,
        )
        .expect("write");

        let written = edit(
            &path,
            "work",
            &Edits {
                name: Some("prod"),
                description: Some(Some("production")),
                ..Edits::default()
            },
        )
        .expect("renamed");

        assert!(written.contains("[profiles.prod]"));
        assert!(written.contains("[profiles.prod.display]"));
        assert!(written.contains("limit = 5"));
        assert!(written.contains(r#"default_profile = "prod""#));
        assert!(written.contains(r#"description = "production""#));
        assert!(!written.contains("[profiles.work]"));
    }

    /// Renaming onto a name in use would merge two organisations into one
    /// profile, silently. It is refused instead.
    #[test]
    fn renaming_onto_an_existing_profile_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[profiles.work]\naccount = \"me\"\n\n[profiles.home]\naccount = \"me\"\n",
        )
        .expect("write");

        let error = edit(
            &path,
            "work",
            &Edits {
                name: Some("home"),
                ..Edits::default()
            },
        )
        .expect_err("refused");

        assert!(matches!(error, EditError::NameTaken(name) if name == "home"));
        let still = std::fs::read_to_string(&path).expect("readable");
        assert!(still.contains("[profiles.work]"));
    }

    #[test]
    fn editing_a_profile_that_does_not_exist_says_so() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[profiles.work]\naccount = \"me\"\n").expect("write");

        let error = edit(
            &path,
            "nope",
            &Edits {
                org_id: Some("1"),
                ..Edits::default()
            },
        )
        .expect_err("refused");

        assert!(matches!(error, EditError::Unknown(name) if name == "nope"));
    }

    /// The whole table goes, display settings included, and the profiles that
    /// share the file are left exactly as they were.
    #[test]
    fn removing_a_profile_takes_its_display_settings_and_nothing_else() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[accounts.me]\n\n[profiles.work]\naccount = \"me\"\n\n[profiles.work.display]\nwidth = 100\n\n[profiles.home]\naccount = \"me\"\n",
        )
        .expect("write");

        let removed = remove(&path, "work").expect("removed");

        assert!(!removed.cleared_default);
        assert!(!removed.contents.contains("[profiles.work"));
        assert!(!removed.contents.contains("width = 100"));
        assert!(removed.contents.contains("[profiles.home]"));
        assert!(removed.contents.contains("[accounts.me]"));
    }

    /// A `default_profile` naming a profile that no longer exists fails every
    /// later command, so it goes with it — and it is not silently pointed at
    /// somebody else's organisation.
    #[test]
    fn removing_the_default_profile_drops_the_default_rather_than_moving_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# mine\ndefault_profile = \"work\"\n\n[profiles.work]\naccount = \"me\"\n\n[profiles.home]\naccount = \"me\"\n",
        )
        .expect("write");

        let removed = remove(&path, "work").expect("removed");

        assert!(removed.cleared_default);
        assert!(!removed.contents.contains("default_profile"));
        assert!(
            removed.contents.starts_with("# mine"),
            "a comment written above the key outlives it: {}",
            removed.contents
        );
    }

    #[test]
    fn removing_a_profile_that_does_not_exist_says_so() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[profiles.work]\naccount = \"me\"\n").expect("write");

        let error = remove(&path, "nope").expect_err("refused");

        assert!(matches!(error, EditError::Unknown(name) if name == "nope"));
        assert!(
            std::fs::read_to_string(&path)
                .expect("readable")
                .contains("[profiles.work]")
        );
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
