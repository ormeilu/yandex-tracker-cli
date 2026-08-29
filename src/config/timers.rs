//! Timers running locally, kept between runs.
//!
//! A timer is not a Tracker object: there is no endpoint for "started working",
//! only for "worked this long". So the start is remembered here and turned into
//! a worklog on stop, which is the whole mechanism.
//!
//! Two decisions worth stating. It lives beside the config rather than in a
//! dotdir of our own, because it is derived state of the same account and
//! nobody should have to learn a second place to look. And it is keyed by
//! organisation as well as by issue: `PROJ-1` in two organisations is two
//! issues, and stopping the wrong one would log somebody else's time. By
//! organisation rather than by profile, because two profiles onto the same
//! organisation are two ways of saying the same issue — a timer started through
//! one should stop through the other.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped when the shape changes, so an old file is ignored rather than misread.
const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timers {
    #[serde(default)]
    pub version: u32,
    /// `org/KEY` -> when it started.
    #[serde(default)]
    pub running: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub org: String,
    /// The profile it was started through, for saying so. Not what it is keyed
    /// by: that is the organisation.
    pub profile: String,
    pub key: String,
    /// When the timer was started, as Tracker will be told.
    pub started: jiff::Timestamp,
}

/// Where the timers live: beside the config, like the queue cache.
#[must_use]
pub fn path_for(config_file: &Path) -> PathBuf {
    config_file.with_file_name("timers.json")
}

/// One timer's name in the file.
fn slot(org: &str, key: &str) -> String {
    format!("{org}/{key}")
}

impl Timers {
    /// Read them, or start empty.
    ///
    /// Unlike the queue cache, an unreadable file here loses work somebody did:
    /// it is still not an error to read, because refusing to start a new timer
    /// because an old file is corrupt helps nobody, but it is logged rather
    /// than passed over in silence.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(timers) if timers.version == VERSION => timers,
            Ok(_) => {
                tracing::warn!("ignoring timers written by another version");
                Self::default()
            }
            Err(error) => {
                tracing::warn!(%error, "ignoring an unreadable timers file");
                Self::default()
            }
        }
    }

    /// Write them back.
    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        self.version = VERSION;
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        std::fs::write(path, text + "\n")
    }

    /// The timer for this issue in this organisation.
    #[must_use]
    pub fn get(&self, org: &str, key: &str) -> Option<&Entry> {
        self.running.get(&slot(org, key))
    }

    /// Start one, refusing to replace a timer that is already running.
    ///
    /// Replacing it would throw away however long it had been running, which is
    /// exactly the data the command exists to keep.
    pub fn start(
        &mut self,
        org: &str,
        profile: &str,
        key: &str,
        at: jiff::Timestamp,
    ) -> Result<(), &Entry> {
        if self.running.contains_key(&slot(org, key)) {
            return Err(self
                .running
                .get(&slot(org, key))
                .unwrap_or_else(|| unreachable!("just checked")));
        }
        self.running.insert(
            slot(org, key),
            Entry {
                org: org.to_owned(),
                profile: profile.to_owned(),
                key: key.to_owned(),
                started: at,
            },
        );
        Ok(())
    }

    /// Take one off the list.
    pub fn take(&mut self, org: &str, key: &str) -> Option<Entry> {
        self.running.remove(&slot(org, key))
    }

    /// The same issue key running in some other organisation.
    ///
    /// What this answers is the confusing case: a timer was started as `work`
    /// and stopped as `personal`, and "no timer running" would be a true
    /// sentence that sends somebody looking in the wrong place.
    #[must_use]
    pub fn elsewhere(&self, org: &str, key: &str) -> Option<&Entry> {
        self.running
            .values()
            .find(|entry| entry.key == key && entry.org != org)
    }

    /// Everything running, oldest first — the one most likely to be forgotten.
    #[must_use]
    pub fn all(&self) -> Vec<&Entry> {
        let mut entries: Vec<&Entry> = self.running.values().collect();
        entries.sort_by_key(|entry| entry.started);
        entries
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn stamp(text: &str) -> jiff::Timestamp {
        text.parse().expect("timestamp")
    }

    /// The same key in two organisations is two issues.
    #[test]
    fn two_organisations_can_time_the_same_key_at_once() {
        let mut timers = Timers::default();
        assert!(
            timers
                .start("1", "work", "PROJ-1", stamp("2026-08-29T09:00:00Z"))
                .is_ok()
        );
        assert!(
            timers
                .start("2", "personal", "PROJ-1", stamp("2026-08-29T10:00:00Z"))
                .is_ok()
        );

        assert_eq!(timers.all().len(), 2);
        assert_eq!(
            timers.take("1", "PROJ-1").map(|entry| entry.started),
            Some(stamp("2026-08-29T09:00:00Z"))
        );
        assert!(timers.get("2", "PROJ-1").is_some());
    }

    /// Starting over a running timer would discard the time it had collected.
    #[test]
    fn starting_twice_is_refused_and_keeps_the_first_start() {
        let mut timers = Timers::default();
        let _ = timers.start("1", "work", "PROJ-1", stamp("2026-08-29T09:00:00Z"));

        let refused = timers.start("1", "work", "PROJ-1", stamp("2026-08-29T11:00:00Z"));
        assert_eq!(
            refused.err().map(|entry| entry.started),
            Some(stamp("2026-08-29T09:00:00Z"))
        );
    }

    /// "No timer running" is true and unhelpful when it is running as somebody
    /// else.
    #[test]
    fn a_timer_in_another_organisation_is_findable() {
        let mut timers = Timers::default();
        let _ = timers.start("1", "work", "PROJ-1", stamp("2026-08-29T09:00:00Z"));

        assert!(timers.get("2", "PROJ-1").is_none());
        assert_eq!(
            timers
                .elsewhere("2", "PROJ-1")
                .map(|entry| entry.profile.as_str()),
            Some("work")
        );
    }

    #[test]
    fn a_file_from_another_version_is_ignored_rather_than_misread() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("timers.json");
        std::fs::write(&path, r#"{"version": 99, "running": {"work/PROJ-1": {}}}"#).expect("write");

        assert!(Timers::load(&path).running.is_empty());
    }

    #[test]
    fn what_was_saved_comes_back() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("timers.json");

        let mut timers = Timers::default();
        let _ = timers.start("1", "work", "PROJ-1", stamp("2026-08-29T09:00:00Z"));
        timers.save(&path).expect("save");

        let loaded = Timers::load(&path);
        assert_eq!(
            loaded.get("1", "PROJ-1").map(|entry| entry.started),
            Some(stamp("2026-08-29T09:00:00Z"))
        );
    }
}
