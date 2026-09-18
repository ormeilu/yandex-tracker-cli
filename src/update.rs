//! Whether a newer ytcli exists — asked at most once a day, and only where a
//! person is there to read the answer.
//!
//! The binary is invoked dozens of times in an agent session, so a check on the
//! way in would be a network round-trip charged to every command. Three rules
//! keep it from costing anything that matters: it happens after the command's
//! own output rather than before it, it is skipped entirely when stdout is not
//! a terminal — which is every agent, every pipe and every CI job — and the
//! answer is kept for a day. Anything that goes wrong, from no network to a
//! body in a shape we do not know, leaves no trace: a notice we failed to give
//! is not worth a word of complaint.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the published versions are listed for a binary that came from a crate.
/// Overridable so tests can point at a stub.
pub const CRATES_IO: &str = "https://crates.io/api/v1/crates/yandex-tracker-cli";
/// The same for a binary that came from a wheel.
pub const PYPI: &str = "https://pypi.org/pypi/yandex-tracker-cli/json";

const URL_ENV: &str = "YTCLI_UPDATE_URL";
const DISABLE_ENV: &str = "YTCLI_NO_UPDATE_CHECK";

/// How long an answer is trusted before asking again.
const INTERVAL: u64 = 24 * 60 * 60;
/// The check is an afterthought and has to behave like one: a registry that is
/// slow to answer is a registry we do not wait for.
const TIMEOUT: Duration = Duration::from_secs(2);

/// Bumped when the shape changes, so an old file is ignored rather than misread.
const VERSION: u32 = 1;

/// What the last check found, and when.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Stamp {
    #[serde(default)]
    pub version: u32,
    /// Seconds since the epoch.
    #[serde(default)]
    pub checked_at: i64,
    #[serde(default)]
    pub latest: Option<String>,
}

/// Beside the config, like the queue cache: derived data nobody should have to
/// read, and nobody should keep in version control.
#[must_use]
pub fn path_for(config_file: &Path) -> PathBuf {
    config_file.with_file_name("update.json")
}

impl Stamp {
    /// Read it, or start empty. An unreadable stamp is not an error; it costs
    /// one request.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(stamp) if stamp.version == VERSION => stamp,
            _ => Self::default(),
        }
    }

    /// Best effort: a stamp that cannot be written costs one extra request a
    /// day, which is not worth failing a command over.
    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// A clock that went backwards — a corrected system time, a stamp copied
    /// from another machine — asks again rather than waiting out a day that
    /// may never pass.
    #[must_use]
    pub fn is_due(&self, now: i64) -> bool {
        let elapsed = now - self.checked_at;
        elapsed < 0 || elapsed >= i64::try_from(INTERVAL).unwrap_or(i64::MAX)
    }
}

/// Where this copy came from, inferred from where its executable sits.
///
/// It decides two things. Which registry to ask: a wheel and a crate are built
/// from the same tag but published by different jobs, and either can fail on
/// its own, so a `uv` install is asked about at pypi.org rather than told about
/// a version that has no wheel yet. And which single line of advice to print.
/// A guess that lands on `Unknown` costs nothing — the notice then names the
/// version and leaves the upgrade to the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `uv tool install` — upgradable in place.
    UvTool,
    /// `uvx`, which runs out of the cache and installs nothing.
    Uvx,
    Pipx,
    Cargo,
    Homebrew,
    Unknown,
}

impl Source {
    /// The registry this copy actually came from.
    #[must_use]
    pub fn registry(self) -> &'static str {
        match self {
            Self::UvTool | Self::Uvx | Self::Pipx => PYPI,
            Self::Cargo | Self::Homebrew | Self::Unknown => CRATES_IO,
        }
    }

    /// The one line that says what to do about it.
    #[must_use]
    pub fn upgrade(self) -> &'static str {
        match self {
            Self::UvTool => "uv tool upgrade yandex-tracker-cli",
            // uvx installs nothing, so there is nothing to upgrade: the pin is
            // what makes the next run fetch the new version.
            Self::Uvx => "uvx --from yandex-tracker-cli@latest ytcli",
            Self::Pipx => "pipx upgrade yandex-tracker-cli",
            Self::Cargo => "cargo install yandex-tracker-cli",
            Self::Homebrew => "brew upgrade ytcli",
            Self::Unknown => "https://github.com/ormeilu/yandex-tracker-cli/releases",
        }
    }
}

/// Recognise the installer from the path of the running executable.
///
/// Matched on whole path components rather than as substrings: a home
/// directory called `uv` is not a uv install.
#[must_use]
pub fn source_of(exe: &Path) -> Source {
    let parts: Vec<String> = exe
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    let has = |names: &[&str]| {
        parts
            .windows(names.len())
            .any(|window| window.iter().zip(names).all(|(part, name)| part == name))
    };
    // The directories uv unpacks a `uvx` run into. The suffix is a cache format
    // version, so only the prefix can be relied on.
    let cached = parts
        .iter()
        .any(|part| part.starts_with("archive-v") || part.starts_with("builds-v"));

    if has(&["uv", "tools"]) {
        Source::UvTool
    } else if cached && parts.iter().any(|part| part == "uv") {
        Source::Uvx
    } else if has(&["pipx", "venvs"]) {
        Source::Pipx
    } else if has(&[".cargo", "bin"]) {
        Source::Cargo
    } else if has(&["cellar"]) || has(&["homebrew", "bin"]) || has(&["linuxbrew"]) {
        Source::Homebrew
    } else {
        Source::Unknown
    }
}

/// Where this run should ask, with the environment able to redirect it.
fn registry_url(source: Source) -> String {
    std::env::var(URL_ENV)
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| source.registry().to_owned())
}

/// Ask a registry what the newest published version is.
///
/// Every failure is `None`: no network, a rate limit, a body we cannot read.
/// There is nothing useful to say about any of them.
pub async fn latest(url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("ytcli/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    parse_latest(&response.text().await.ok()?)
}

/// Pull the version out of whichever of the two answers this is.
///
/// crates.io puts it in `crate.max_stable_version`, which already skips
/// pre-releases; the wheel index answers with `info.version`, its newest release
/// that is not yanked. Both are tried rather than chosen by registry: the shape
/// of an answer is what identifies it, and picking wrong would silently give
/// nothing.
fn parse_latest(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    json.pointer("/crate/max_stable_version")
        .or_else(|| json.pointer("/info/version"))?
        .as_str()
        .map(str::to_owned)
}

/// Is `latest` a version after `current`?
///
/// Three numbers compared as numbers: `2.10.0` is after `2.9.0`, which is the
/// comparison a string gets wrong. Anything that does not parse is not an
/// upgrade — nobody is sent after a version we cannot read.
#[must_use]
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parts(latest), parts(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

fn parts(version: &str) -> Option<(u64, u64, u64)> {
    // A pre-release or build suffix is dropped rather than ordered: comparing
    // the numbers alone makes `2.2.0-rc.1` equal to `2.2.0`, so it is never
    // offered as an upgrade over the release it precedes.
    let core = version.split(['-', '+']).next()?;
    let mut numbers = core.split('.');
    let major = numbers.next()?.trim().parse().ok()?;
    let minor = numbers.next()?.parse().ok()?;
    let patch = numbers.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// The line itself, or nothing when there is nothing to say.
///
/// Kept separate from everything that touches the network or the clock, because
/// this is the part callers actually see.
#[must_use]
pub fn notice(latest: &str, current: &str, source: Source) -> Option<String> {
    is_newer(latest, current).then(|| {
        format!(
            "ytcli {latest} is out (this is {current}) — {}",
            source.upgrade()
        )
    })
}

/// Would the person running this see the notice at all?
///
/// Not a terminal means a pipe, a script or an agent: none of them can act on
/// it and all of them pay for it, an agent in tokens and a script in a line it
/// did not expect. `CI` is the same case wearing a terminal, and the variable
/// is for the terminals that want none of it.
#[must_use]
pub fn wanted() -> bool {
    use std::io::IsTerminal;

    std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
        && std::env::var_os(DISABLE_ENV).is_none()
        && std::env::var_os("CI").is_none()
}

/// Check if it is time to, and say so if there is something to say.
///
/// Called after the command has printed everything it had: a notice is worth a
/// line at the end, never a delay at the start.
pub async fn notify(config_file: &Path) {
    use std::io::Write as _;

    if !wanted() {
        return;
    }

    let path = path_for(config_file);
    let mut stamp = Stamp::load(&path);
    let now = jiff::Timestamp::now().as_second();
    let source = std::env::current_exe().map_or(Source::Unknown, |exe| source_of(&exe));

    if stamp.is_due(now) {
        stamp = Stamp {
            version: VERSION,
            checked_at: now,
            latest: latest(&registry_url(source)).await,
        };
        stamp.save(&path);
    }

    let Some(latest) = stamp.latest.as_deref() else {
        return;
    };
    let Some(notice) = notice(latest, env!("CARGO_PKG_VERSION"), source) else {
        return;
    };

    let paint = crate::render::style::Painter::for_stream(true);
    let _ = writeln!(
        anstream::stderr(),
        "{}",
        paint.paint(&notice, crate::render::style::Palette::label())
    );
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_later_version_is_newer_and_an_earlier_one_is_not() {
        assert!(is_newer("2.2.0", "2.1.2"));
        assert!(is_newer("2.10.0", "2.9.0"), "numbers, not strings");
        assert!(!is_newer("2.1.2", "2.1.2"));
        assert!(!is_newer("2.1.1", "2.1.2"));
        // A release candidate for the version we are already on is not an
        // upgrade, and neither is anything we cannot parse.
        assert!(!is_newer("2.1.2-rc.1", "2.1.2"));
        assert!(!is_newer("nightly", "2.1.2"));
    }

    #[test]
    fn both_registries_answer_in_a_shape_we_can_read() {
        assert_eq!(
            parse_latest(r#"{"crate":{"max_stable_version":"2.2.0"}}"#).as_deref(),
            Some("2.2.0")
        );
        assert_eq!(
            parse_latest(r#"{"info":{"version":"2.2.0"}}"#).as_deref(),
            Some("2.2.0")
        );
        assert_eq!(parse_latest("not json"), None);
        assert_eq!(parse_latest("{}"), None);
    }

    /// The installer decides which registry holds the answer: a wheel and a
    /// crate come from the same tag but not from the same job.
    #[test]
    fn a_uv_install_is_asked_about_on_pypi() {
        let uv = source_of(Path::new(
            "/home/ilya/.local/share/uv/tools/yandex-tracker-cli/bin/ytcli",
        ));
        assert_eq!(uv, Source::UvTool);
        assert_eq!(uv.registry(), PYPI);
        assert_eq!(uv.upgrade(), "uv tool upgrade yandex-tracker-cli");

        let uvx = source_of(Path::new("/home/ilya/.cache/uv/archive-v0/9Qf1/bin/ytcli"));
        assert_eq!(uvx, Source::Uvx);
        assert_eq!(uvx.registry(), PYPI);

        let pipx = source_of(Path::new(
            "/home/ilya/.local/pipx/venvs/yandex-tracker-cli/bin/ytcli",
        ));
        assert_eq!(pipx, Source::Pipx);
        assert_eq!(pipx.registry(), PYPI);
    }

    #[test]
    fn a_crate_or_a_bottle_is_asked_about_on_crates_io() {
        let cargo = source_of(Path::new("/home/ilya/.cargo/bin/ytcli"));
        assert_eq!(cargo, Source::Cargo);
        assert_eq!(cargo.registry(), CRATES_IO);

        let brew = source_of(Path::new("/opt/homebrew/bin/ytcli"));
        assert_eq!(brew, Source::Homebrew);
        assert_eq!(brew.upgrade(), "brew upgrade ytcli");
        assert_eq!(
            source_of(Path::new("/usr/local/Cellar/ytcli/2.1.2/bin/ytcli")),
            Source::Homebrew
        );
    }

    /// A path is matched by its components, so a directory that merely spells
    /// an installer's name is not that installer.
    #[test]
    fn a_directory_that_only_looks_like_an_installer_is_not_one() {
        assert_eq!(source_of(Path::new("/home/uv/bin/ytcli")), Source::Unknown);
        assert_eq!(source_of(Path::new("/opt/uv-tools/ytcli")), Source::Unknown);
        assert_eq!(
            source_of(Path::new("/usr/local/bin/ytcli")),
            Source::Unknown
        );
    }

    /// The whole point of the line: what it says, and that it says nothing when
    /// there is nothing to say.
    #[test]
    fn the_notice_names_the_version_and_the_way_to_get_it() {
        assert_eq!(
            notice("2.2.0", "2.1.2", Source::UvTool).as_deref(),
            Some("ytcli 2.2.0 is out (this is 2.1.2) — uv tool upgrade yandex-tracker-cli")
        );
        assert_eq!(
            notice("2.2.0", "2.1.2", Source::Unknown).as_deref(),
            Some(
                "ytcli 2.2.0 is out (this is 2.1.2) — https://github.com/ormeilu/yandex-tracker-cli/releases"
            )
        );
        assert_eq!(notice("2.1.2", "2.1.2", Source::Cargo), None);
    }

    #[test]
    fn a_stamp_from_today_is_not_due_and_one_from_last_week_is() {
        let fresh = Stamp {
            version: VERSION,
            checked_at: 1_000_000,
            latest: None,
        };
        assert!(!fresh.is_due(1_000_000 + 60));
        assert!(fresh.is_due(1_000_000 + i64::try_from(INTERVAL).unwrap_or_default()));
        // A clock that went backwards asks again rather than waiting out a day
        // that may never arrive.
        assert!(fresh.is_due(999_000));
    }

    #[test]
    fn a_stamp_from_another_version_is_ignored_rather_than_misread() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("update.json");
        std::fs::write(&path, r#"{"version":99,"checked_at":5,"latest":"9.9.9"}"#)
            .expect("write stamp");

        let stamp = Stamp::load(&path);

        assert_eq!(stamp.checked_at, 0);
        assert_eq!(stamp.latest, None);
    }

    #[test]
    fn a_saved_stamp_reads_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = path_for(&dir.path().join("config.toml"));
        Stamp {
            version: VERSION,
            checked_at: 42,
            latest: Some("2.2.0".to_owned()),
        }
        .save(&path);

        let stamp = Stamp::load(&path);

        assert_eq!(path.file_name().unwrap_or_default(), "update.json");
        assert_eq!(stamp.checked_at, 42);
        assert_eq!(stamp.latest.as_deref(), Some("2.2.0"));
    }
}
