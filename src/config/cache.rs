//! What we have learned about the organisations, kept between runs.
//!
//! There is exactly one thing here and one reason for it. A bare `LMS-12` is
//! ambiguous when two profiles can both see a queue called `LMS`, but finding
//! that out costs a request per profile — far too much to pay on every command.
//! So the map is recorded when something already had to list queues (`auth
//! status`, `auth login`) and consulted for free afterwards.
//!
//! The cache is an optimisation and is treated as one: missing, stale or
//! unreadable, the tool works exactly as it did before, it just cannot warn
//! about a collision it has never seen.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped when the shape changes, so an old file is ignored rather than
/// misread.
const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    pub version: u32,
    /// Queue key -> the profiles known to see it.
    #[serde(default)]
    pub queues: BTreeMap<String, Vec<String>>,
}

/// Where the cache lives: beside the config, not inside it. It is derived data,
/// and nobody should have to read it or keep it in version control.
#[must_use]
pub fn path_for(config_file: &Path) -> PathBuf {
    config_file.with_file_name("queues.json")
}

impl Cache {
    /// Read it, or start empty. A cache that cannot be read is not an error:
    /// the worst outcome is a warning we fail to give.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(cache) if cache.version == VERSION => cache,
            Ok(_) => {
                tracing::debug!("ignoring a queue cache written by another version");
                Self::default()
            }
            Err(error) => {
                tracing::debug!(%error, "ignoring an unreadable queue cache");
                Self::default()
            }
        }
    }

    /// Replace what is known about one profile.
    ///
    /// Replace rather than merge: a queue the profile can no longer see should
    /// stop being attributed to it, or the warning outlives the fact.
    pub fn record(&mut self, profile: &str, queue_keys: &[String]) {
        for profiles in self.queues.values_mut() {
            profiles.retain(|known| known != profile);
        }

        for key in queue_keys {
            let profiles = self.queues.entry(key.clone()).or_default();
            profiles.push(profile.to_owned());
            profiles.sort();
            profiles.dedup();
        }

        self.queues.retain(|_, profiles| !profiles.is_empty());
    }

    /// Follow a profile that was renamed.
    ///
    /// Stale names are already harmless — `profiles_for` filters against the
    /// configured ones — but dropping the knowledge would make the next bare
    /// key ambiguous again until something lists queues afresh. Returns whether
    /// anything moved, so an unchanged cache is not rewritten.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let mut moved = false;
        for profiles in self.queues.values_mut() {
            for profile in profiles.iter_mut() {
                if profile == from {
                    to.clone_into(profile);
                    moved = true;
                }
            }
            profiles.sort();
            profiles.dedup();
        }
        moved
    }

    /// Which profiles see this queue, among those still configured.
    ///
    /// Filtering against the current config matters: a profile deleted from the
    /// config must not keep making its old queues look ambiguous.
    #[must_use]
    pub fn profiles_for(&self, queue: &str, configured: &[String]) -> Vec<String> {
        let mut owners: Vec<String> = self
            .queues
            .get(queue)
            .map(|profiles| {
                profiles
                    .iter()
                    .filter(|profile| configured.iter().any(|known| known == *profile))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        // Sorted so the "write x/KEY-1 or y/KEY-1" message reads the same every
        // time, whatever order the cache file happened to be written in.
        owners.sort();
        owners.dedup();
        owners
    }

    /// Write it out. Failing to save a cache is not worth failing a command for.
    pub fn save(&mut self, path: &Path) {
        self.version = VERSION;
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(path, text) {
            tracing::debug!(%error, "could not write the queue cache");
        }
    }
}

/// The queue part of an issue key: `LMS` from `LMS-12`.
#[must_use]
pub fn queue_of(key: &str) -> Option<&str> {
    let (queue, number) = key.rsplit_once('-')?;
    if queue.is_empty() || number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(queue)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    /// A renamed profile keeps what was known about it: otherwise the next
    /// bare key is ambiguous again until something lists queues afresh.
    #[test]
    fn renaming_a_profile_carries_its_queues_over() {
        let mut cache = Cache::default();
        cache.record("work", &["LMS".to_owned(), "PROJ".to_owned()]);

        assert!(cache.rename("work", "prod"));
        assert_eq!(
            cache.profiles_for("LMS", &["prod".to_owned()]),
            vec!["prod".to_owned()]
        );
        assert!(!cache.rename("work", "prod"), "nothing left to move");
    }

    #[test]
    fn a_queue_key_is_the_part_before_the_number() {
        assert_eq!(queue_of("LMS-12"), Some("LMS"));
        assert_eq!(queue_of("TWO-PART-3"), Some("TWO-PART"));
    }

    #[test]
    fn something_that_is_not_an_issue_key_has_no_queue() {
        assert_eq!(queue_of("LMS"), None);
        assert_eq!(queue_of("LMS-"), None);
        assert_eq!(queue_of("LMS-abc"), None);
    }

    #[test]
    fn recording_a_profile_replaces_what_it_used_to_see() {
        let mut cache = Cache::default();
        cache.record("work", &["LMS".to_owned(), "OLD".to_owned()]);
        cache.record("work", &["LMS".to_owned()]);

        assert_eq!(cache.profiles_for("LMS", &["work".to_owned()]), ["work"]);
        assert!(cache.profiles_for("OLD", &["work".to_owned()]).is_empty());
    }

    #[test]
    fn two_profiles_seeing_one_queue_are_both_reported() {
        let mut cache = Cache::default();
        cache.record("work", &["LMS".to_owned()]);
        cache.record("personal", &["LMS".to_owned()]);

        let configured = vec!["work".to_owned(), "personal".to_owned()];
        assert_eq!(cache.profiles_for("LMS", &configured), ["personal", "work"]);
    }

    /// A profile removed from the config must stop making its old queues look
    /// ambiguous.
    #[test]
    fn a_profile_no_longer_configured_is_ignored() {
        let mut cache = Cache::default();
        cache.record("work", &["LMS".to_owned()]);
        cache.record("gone", &["LMS".to_owned()]);

        assert_eq!(cache.profiles_for("LMS", &["work".to_owned()]), ["work"]);
    }

    #[test]
    fn an_unreadable_cache_is_simply_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("queues.json");
        std::fs::write(&path, "not json").expect("write");

        assert!(Cache::load(&path).queues.is_empty());
    }

    #[test]
    fn a_saved_cache_round_trips() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("queues.json");

        let mut cache = Cache::default();
        cache.record("work", &["LMS".to_owned()]);
        cache.save(&path);

        assert_eq!(
            Cache::load(&path).profiles_for("LMS", &["work".to_owned()]),
            ["work"]
        );
    }
}
