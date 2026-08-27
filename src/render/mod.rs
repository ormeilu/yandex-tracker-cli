//! Output rendering — the detail ladder.
//!
//! The shape of what we print *is* the product (`docs/adr/0003-output-ladder.md`).
//! Two rules hold everywhere in this module:
//!
//! 1. **Field order is fixed.** A view that reorders itself between calls breaks
//!    an agent's prompt cache and any script that reads the output.
//! 2. **Free text coming from Tracker is fenced.** Summaries, descriptions and
//!    comments were written by other people and may contain instructions aimed
//!    at whatever reads them; they are data, and they are labelled as such.
//!
//! Snapshot tests cover every renderer, so changing a default shape shows up as
//! a diff in review rather than as a surprise in someone's pipeline.

pub mod entity;
pub mod image;
pub mod markdown;
pub mod progress;
pub mod queue;
pub mod style;
pub mod table;
pub mod text;
pub mod untrusted;

use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Output format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// Compact key/value text. The default, and the only format tuned for tokens.
    #[default]
    Text,
    /// Our normalised schema, stable across Tracker API changes.
    Json,
    /// The upstream payload, verbatim. Escape hatch, never the default.
    JsonRaw,
    /// Token-Oriented Object Notation. Experimental, behind the `toon` feature;
    /// only pays off on uniform lists.
    Toon,
}

impl FromStr for Format {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "json-raw" => Ok(Self::JsonRaw),
            "toon" => Ok(Self::Toon),
            other => Err(format!(
                "unknown format `{other}` (expected text, json, json-raw or toon)"
            )),
        }
    }
}

/// How the output is meant to be consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// stdout is a terminal: colour and tables are welcome.
    Human,
    /// stdout is a pipe: no colour, no box drawing, stable line shapes.
    Machine,
}

impl Audience {
    /// Decide from the environment. Explicit `--format` still overrides the
    /// format; this only decides decoration.
    #[must_use]
    pub fn detect() -> Self {
        use std::io::IsTerminal;
        if std::io::stdout().is_terminal() {
            Self::Human
        } else {
            Self::Machine
        }
    }
}

/// Rendering context assembled once per command.
#[derive(Debug, Clone)]
pub struct Context {
    pub format: Format,
    pub audience: Audience,
    /// Description lines before the `--full` hint; `None` means no limit.
    pub description_lines: Option<usize>,
    /// Custom field keys to surface, in this exact order.
    pub extra_fields: Vec<String>,
    /// Columns available for wrapped prose. Fixed for machine output, so a pipe
    /// gets the same bytes whatever the window happens to be.
    pub width: usize,
    /// Whether image attachments may be drawn inline. Says nothing about whether
    /// the terminal can: that is [`image::protocol`].
    pub images: bool,
    /// Pictures to put where the text references them, keyed by URL. Filled in
    /// by the command, once it knows there is a terminal to draw on.
    pub inline: image::Inline,
}

impl Context {
    #[must_use]
    pub fn is_human(&self) -> bool {
        self.audience == Audience::Human
    }

    /// The painter for this context.
    ///
    /// Machine output is never styled — not stripped after the fact, never
    /// produced — so what a snapshot test pins is exactly what a pipe receives.
    #[must_use]
    pub fn painter(&self) -> style::Painter {
        style::Painter::for_stream(self.is_human())
    }
}

/// Serialise a value in whichever machine format was asked for.
///
/// `text` is not handled here: it is per-entity and lives in [`text`].
pub fn machine<T: serde::Serialize>(value: &T, format: Format) -> Result<String, RenderError> {
    match format {
        Format::Json | Format::JsonRaw => {
            Ok(serde_json::to_string_pretty(value).map(|json| json + "\n")?)
        }
        #[cfg(feature = "toon")]
        Format::Toon => Ok(toon_format::encode_default(value)? + "\n"),
        #[cfg(not(feature = "toon"))]
        Format::Toon => Err(RenderError::ToonUnavailable),
        Format::Text => Err(RenderError::NotMachineReadable),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("could not serialise the result")]
    Serialise(#[from] serde_json::Error),
    #[error("this build has no TOON support; rebuild with `--features toon`")]
    ToonUnavailable,
    #[error("text output is rendered per entity, not generically")]
    NotMachineReadable,
    #[cfg(feature = "toon")]
    #[error("could not encode as TOON")]
    Toon(#[from] toon_format::ToonError),
}
