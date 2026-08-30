//! Layered configuration and profile resolution.
//!
//! Two entities that are easy to conflate (see `CONTEXT.md`):
//!
//! * an **account** owns a credential — one `auth login` per account, one keychain entry;
//! * a **profile** is an *organisation seen through an account*, plus display defaults.
//!
//! One account can serve many profiles (same login, several organisations) and one
//! organisation can be reached through several accounts (admin and read-only).
//! That is why the token is keyed by account and never by profile.

pub mod cache;
pub mod paths;
pub mod store;
pub mod timers;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use serde::{Deserialize, Serialize};

use crate::render::Format as OutputFormat;

/// Which header carries the organisation id. Sending the wrong one is a 403,
/// so it is a profile-level decision rather than something we probe at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum OrgKind {
    /// Yandex Cloud Organization — `X-Cloud-Org-Id`.
    Cloud,
    /// Yandex 360 for Business — `X-Org-Id`.
    Yandex360,
}

impl OrgKind {
    /// Lowercase on purpose: header names are case-insensitive, and
    /// `HeaderName::from_static` only accepts the lowercase form.
    #[must_use]
    pub fn header_name(self) -> &'static str {
        match self {
            Self::Cloud => "x-cloud-org-id",
            Self::Yandex360 => "x-org-id",
        }
    }
}

/// An identity that holds a credential. The struct is intentionally empty of
/// secrets: the token lives in the OS keychain under this account's name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Account {
    /// Human note about who this is; shown by `auth list`.
    #[serde(default)]
    pub description: Option<String>,
}

/// Display defaults. Every one of these is overridable per profile and per
/// repository, because a default that cannot be moved becomes someone's papercut.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    /// Rows returned by list commands before pagination kicks in.
    pub limit: usize,
    /// Hard ceiling for `--all` page walking.
    pub max: usize,
    /// Description lines shown before the `--full` hint, when the output is
    /// being piped or read by an agent.
    pub description_lines: usize,
    /// The same, for a terminal. `None` — the default — means no limit: a person
    /// reading their own screen is not paying for context.
    pub description_lines_human: Option<usize>,
    /// Custom field keys pinned into the compact view, in this exact order.
    /// Order is fixed on purpose: a shuffling field list breaks an agent's
    /// prompt cache on every call.
    pub extra_fields: Vec<String>,
    /// Output format when stdout is not a terminal.
    pub format: OutputFormat,
    /// Draw image attachments inline where the terminal can draw them.
    ///
    /// On by default: a screenshot is usually the most informative thing on a
    /// bug, and this costs nothing anywhere it cannot be used — no terminal that
    /// draws means no request for the attachments in the first place.
    pub images: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            limit: 25,
            max: 500,
            description_lines: 10,
            description_lines_human: None,
            extra_fields: Vec::new(),
            format: OutputFormat::Text,
            images: true,
        }
    }
}

/// An organisation reached through an account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Key into [`Config::accounts`].
    pub account: String,
    /// Organisation id sent in the header chosen by `org_kind`.
    pub org_id: String,
    pub org_kind: OrgKind,
    /// Human note about which organisation this is. An org id is a number
    /// nobody recognises and a profile name is whatever was typed at login, so
    /// this is what answers "am I about to write to production" — which is why
    /// it rides along on the provenance banner and not only in `auth list`.
    #[serde(default)]
    pub description: Option<String>,
    /// Queue assumed when a command needs one and none was given.
    #[serde(default)]
    pub default_queue: Option<String>,
    #[serde(default)]
    pub display: Display,
}

/// The user-level config file, `$XDG_CONFIG_HOME/ytcli/config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub default_profile: Option<String>,
    pub accounts: BTreeMap<String, Account>,
    pub profiles: BTreeMap<String, Profile>,
}

/// The committed, secret-free `.tracker.toml` found by walking up from the cwd.
/// It pins a repository to a profile so that an agent working in a checkout
/// lands in the right organisation without any global mutable state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectPin {
    pub profile: Option<String>,
    pub queue: Option<String>,
}

/// Where the active profile name came from. Always reported by `auth status`
/// and by every writing command: "which organisation am I about to change" must
/// never be a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileSource {
    Flag,
    Env,
    ProjectFile(PathBuf),
    DefaultProfile,
    /// Chosen because it is the profile that can see the queue in the key.
    QueueOwner(String),
    /// Named in the key itself, as `profile/PROJ-1`.
    Qualified(String),
}

impl std::fmt::Display for ProfileSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flag => f.write_str("--profile"),
            Self::Env => f.write_str("YTCLI_PROFILE"),
            Self::ProjectFile(path) => write!(f, "{}", path.display()),
            Self::DefaultProfile => f.write_str("config default_profile"),
            Self::QueueOwner(queue) => write!(f, "the only profile that sees {queue}"),
            Self::Qualified(target) => write!(f, "the key `{target}`"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "no profile selected: pass --profile, set YTCLI_PROFILE, add .tracker.toml, or set default_profile"
    )]
    NoProfile,
    #[error("profile `{0}` is not defined in the config file")]
    UnknownProfile(String),
    #[error("profile `{profile}` refers to account `{account}`, which is not defined")]
    UnknownAccount { profile: String, account: String },
    #[error("could not read configuration")]
    Read(#[from] figment::Error),
    #[error("could not locate the configuration directory")]
    Paths(#[from] paths::PathsError),
}

/// A fully resolved profile plus the provenance of that choice.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub name: String,
    pub profile: Profile,
    pub source: ProfileSource,
    /// Queue override coming from `.tracker.toml`, if any.
    pub queue: Option<String>,
}

impl Config {
    /// Read the user config file, letting `YTCLI_*` environment variables win.
    pub fn load(config_file: &Path) -> Result<Self, ConfigError> {
        Ok(Figment::new()
            .merge(Toml::file(config_file))
            .merge(Env::prefixed("YTCLI_").split("__"))
            .extract()?)
    }

    /// Resolve the active profile.
    ///
    /// Precedence, highest first: `--profile`, `YTCLI_PROFILE`, the nearest
    /// `.tracker.toml` walking up from `start_dir`, `default_profile`.
    pub fn resolve(
        &self,
        flag: Option<&str>,
        env: Option<&str>,
        start_dir: &Path,
    ) -> Result<Resolved, ConfigError> {
        let pin = paths::find_project_pin(start_dir);

        let (name, source) = match (flag, env, &pin) {
            (Some(name), _, _) => (name.to_owned(), ProfileSource::Flag),
            (None, Some(name), _) => (name.to_owned(), ProfileSource::Env),
            (None, None, Some((path, pinned))) if pinned.profile.is_some() => {
                let Some(name) = pinned.profile.clone() else {
                    return Err(ConfigError::NoProfile);
                };
                (name, ProfileSource::ProjectFile(path.clone()))
            }
            _ => {
                let name = self.default_profile.clone().ok_or(ConfigError::NoProfile)?;
                (name, ProfileSource::DefaultProfile)
            }
        };

        let profile = self
            .profiles
            .get(&name)
            .cloned()
            .ok_or_else(|| ConfigError::UnknownProfile(name.clone()))?;

        if !self.accounts.contains_key(&profile.account) {
            return Err(ConfigError::UnknownAccount {
                profile: name,
                account: profile.account,
            });
        }

        Ok(Resolved {
            name,
            queue: pin
                .as_ref()
                .and_then(|(_, pinned)| pinned.queue.clone())
                .or_else(|| profile.default_queue.clone()),
            profile,
            source,
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    /// A screenshot is usually the most informative thing on a bug, so the
    /// default is to show it. Turning it off is a profile's decision.
    #[test]
    fn images_are_on_by_default_and_can_be_turned_off() {
        assert!(Display::default().images);

        let display: Display = figment::Figment::new()
            .merge(figment::providers::Toml::string("images = false"))
            .extract()
            .expect("parses");
        assert!(!display.images);

        // And the rest of the defaults survive naming only one of them.
        assert_eq!(display.limit, Display::default().limit);
    }
}
