//! Per-OS locations and the upward search for a repository pin.

use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

use crate::config::ProjectPin;

/// Name of the committed, secret-free per-repository pin file.
pub const PROJECT_FILE: &str = ".tracker.toml";

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("no home directory found")]
    NoHome(#[from] etcetera::HomeDirError),
}

/// `~/.config/ytcli/config.toml` on Linux, `~/Library/Application Support/...`
/// on macOS, `%APPDATA%\...` on Windows.
pub fn config_file() -> Result<PathBuf, PathsError> {
    Ok(etcetera::choose_base_strategy()?
        .config_dir()
        .join("ytcli")
        .join("config.toml"))
}

/// Walk up from `start` looking for [`PROJECT_FILE`], the way git looks for `.git`.
///
/// A malformed pin is ignored rather than fatal: a broken file in some parent
/// directory must not make the tool unusable everywhere below it.
#[must_use]
pub fn find_project_pin(start: &Path) -> Option<(PathBuf, ProjectPin)> {
    for dir in start.ancestors() {
        let candidate = dir.join(PROJECT_FILE);
        if !candidate.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        match toml_from_str(&text) {
            Some(pin) => return Some((candidate, pin)),
            None => {
                tracing::warn!(path = %candidate.display(), "ignoring malformed .tracker.toml");
            }
        }
    }
    None
}

fn toml_from_str(text: &str) -> Option<ProjectPin> {
    use figment::Figment;
    use figment::providers::{Format, Toml};

    Figment::new().merge(Toml::string(text)).extract().ok()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn finds_pin_in_parent_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            root.path().join(PROJECT_FILE),
            "profile = \"work\"\nqueue = \"PROJ\"\n",
        )
        .expect("write pin");
        let nested = root.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("create nested");

        let (path, pin) = find_project_pin(&nested).expect("pin found");

        assert_eq!(path, root.path().join(PROJECT_FILE));
        assert_eq!(pin.profile.as_deref(), Some("work"));
        assert_eq!(pin.queue.as_deref(), Some("PROJ"));
    }

    #[test]
    fn malformed_pin_is_ignored_not_fatal() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join(PROJECT_FILE), "profile = [[[").expect("write pin");

        assert!(find_project_pin(root.path()).is_none());
    }
}
