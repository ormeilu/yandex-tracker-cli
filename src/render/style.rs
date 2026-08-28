//! Colour and emphasis.
//!
//! Three rules hold here, and they are the reason this is a module rather than
//! scattered `\x1b[` literals:
//!
//! 1. **Styling never changes the data.** The same fields, in the same order,
//!    with the same words, whether or not anything is painted. Only the escape
//!    codes differ, so a pipe and a terminal disagree about nothing that matters.
//! 2. **Machine output is never styled.** Not stripped afterwards — never
//!    produced. Snapshot tests then pin the real bytes a caller receives.
//! 3. **Untrusted text is never given our chrome.** Descriptions and comments
//!    are dimmed and nothing more. Painting them like tool output would let an
//!    issue's text impersonate the tool talking, which is exactly the confusion
//!    the fence exists to prevent (ADR 1).

use anstyle::{AnsiColor, Color, Style};

/// The palette. Small on purpose: a listing that uses six colours communicates
/// less than one that uses two.
#[derive(Debug, Clone, Copy)]
pub struct Palette;

impl Palette {
    /// Identifiers a caller will type back: issue keys, queue keys, profile names.
    #[must_use]
    pub fn key() -> Style {
        Style::new().bold()
    }

    /// Field labels and other scaffolding.
    #[must_use]
    pub fn label() -> Style {
        Style::new().dimmed()
    }

    /// Something worked.
    #[must_use]
    pub fn ok() -> Style {
        Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)))
    }

    /// Something needs attention but is not broken.
    #[must_use]
    pub fn warn() -> Style {
        Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)))
    }

    /// Something is broken.
    #[must_use]
    pub fn bad() -> Style {
        Style::new().fg_color(Some(Color::Ansi(AnsiColor::Red)))
    }

    /// A link worth following.
    #[must_use]
    pub fn url() -> Style {
        Style::new()
            .fg_color(Some(Color::Ansi(AnsiColor::Cyan)))
            .underline()
    }

    /// A heading in a block of prose.
    #[must_use]
    pub fn heading() -> Style {
        Style::new().bold().underline()
    }

    /// Text somebody else wrote.
    #[must_use]
    pub fn untrusted() -> Style {
        Style::new().dimmed()
    }
}

/// Applies the palette, or does not.
#[derive(Debug, Clone, Copy)]
pub struct Painter {
    enabled: bool,
}

impl Painter {
    /// A painter that styles.
    #[must_use]
    pub fn colour() -> Self {
        Self { enabled: true }
    }

    /// A painter that leaves text exactly as it is.
    #[must_use]
    pub fn plain() -> Self {
        Self { enabled: false }
    }

    /// Style for a terminal, plain for anything else.
    #[must_use]
    pub fn for_stream(is_terminal: bool) -> Self {
        Self {
            enabled: is_terminal,
        }
    }

    /// Wrap `text` in `style`, or return it unchanged.
    #[must_use]
    pub fn paint(self, text: &str, style: Style) -> String {
        if !self.enabled {
            return text.to_owned();
        }
        format!("{style}{text}{style:#}")
    }

    /// Pad to `width` **before** styling.
    ///
    /// Escape codes have no width but plenty of bytes, so padding a styled
    /// string with `{:<12}` misaligns every column after it.
    #[must_use]
    pub fn paint_padded(self, text: &str, width: usize, style: Style) -> String {
        let visible = text.chars().count();
        let padding = width.saturating_sub(visible);
        format!("{}{}", self.paint(text, style), " ".repeat(padding))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_painter_changes_nothing() {
        assert_eq!(Painter::plain().paint("PROJ-1", Palette::key()), "PROJ-1");
    }

    #[test]
    fn a_colour_painter_wraps_and_resets() {
        let painted = Painter::colour().paint("PROJ-1", Palette::key());
        assert!(painted.starts_with('\u{1b}'));
        assert!(painted.contains("PROJ-1"));
        assert!(painted.ends_with("\u{1b}[0m"));
    }

    /// Columns must line up whether or not anything is painted: escape codes
    /// carry bytes but no width.
    #[test]
    fn padding_counts_visible_characters_only() {
        let plain = Painter::plain().paint_padded("PROJ-1", 12, Palette::key());
        let coloured = Painter::colour().paint_padded("PROJ-1", 12, Palette::key());

        assert_eq!(plain, "PROJ-1      ");
        assert!(coloured.ends_with("      "));
        assert_eq!(
            coloured.matches(' ').count(),
            plain.matches(' ').count(),
            "same visible width in both modes"
        );
    }

    #[test]
    fn text_longer_than_the_column_is_not_truncated_by_padding() {
        assert_eq!(
            Painter::plain().paint_padded("very-long-key", 4, Palette::key()),
            "very-long-key"
        );
    }
}
