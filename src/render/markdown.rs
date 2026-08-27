//! Markdown, rendered for a terminal.
//!
//! Tracker's descriptions and comments are markdown, and printing them raw
//! makes a person read the syntax instead of the text. This module renders them
//! with [`termimad`], under two constraints that are not negotiable:
//!
//! 1. **Only for a terminal.** Machine output keeps the source text verbatim —
//!    an agent reads markdown perfectly well, and reflowing it would change
//!    bytes a caller may be diffing.
//! 2. **The block stays marked as someone else's.** Rendering makes the text
//!    readable; it must not make it look like the tool talking. Every line keeps
//!    a dim margin bar, so where the quoted block starts and ends is never a
//!    matter of interpretation (ADR 1).

use std::fmt::Write as _;

use termimad::MadSkin;
use termimad::crossterm::style::Attribute;
use termimad::minimad::Alignment;

use crate::render::style::{Painter, Palette};

/// The bar that marks every line of quoted text.
const MARGIN: &str = "\u{258f} ";

/// Render `body` as markdown, wrapped to `width`.
///
/// Returns the lines without the margin; [`quoted`] is what callers normally
/// want.
#[must_use]
pub fn render(body: &str, width: usize) -> String {
    skin().text(body, Some(width.max(20))).to_string()
}

/// Render `body` and mark every line as quoted text.
///
/// `width` is the width available to the whole block, margin included.
#[must_use]
pub fn quoted(body: &str, width: usize, paint: Painter) -> String {
    let bar = paint.paint(MARGIN, Palette::untrusted());
    let inner = width.saturating_sub(MARGIN.chars().count());
    let rendered = render(body, inner);

    let mut out = String::with_capacity(rendered.len() + rendered.lines().count() * bar.len());
    for line in rendered.lines() {
        // termimad pads lines out to the full width; the trailing run of spaces
        // is invisible until something copies it, so it goes.
        let _ = writeln!(out, "{bar}{}", line.trim_end());
    }
    out
}

/// The skin: structure, no palette of our own.
///
/// Bold, italics, bullets and quote marks say what the author meant. Colour
/// would say something else — that this text belongs to the tool — which is the
/// one thing the block must not claim. `termimad`'s default is built from grey
/// levels that hold on both light and dark terminals, so it needs only two
/// corrections.
fn skin() -> MadSkin {
    let mut skin = MadSkin::default();
    // The default centres H1 for a full-screen view. Inside a quoted block it
    // reads as a title of our output rather than of theirs.
    for header in &mut skin.headers {
        header.align = Alignment::Left;
    }
    skin.headers[0].add_attr(Attribute::Bold);
    skin
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_becomes_style_not_syntax() {
        let out = render("**loud**", 40);
        assert!(out.contains("loud"));
        assert!(!out.contains("**"));
    }

    #[test]
    fn every_line_of_a_quoted_block_carries_the_margin() {
        let out = quoted("# Title\n\nbody text\n", 40, Painter::plain());
        assert!(out.lines().count() >= 2);
        assert!(out.lines().all(|line| line.starts_with(MARGIN)));
    }

    /// The margin is the boundary marker, so an empty line inside the block
    /// still gets one — otherwise a blank line would look like the end of it.
    #[test]
    fn a_blank_line_inside_the_block_is_still_marked() {
        let out = quoted("one\n\ntwo", 40, Painter::plain());
        assert!(out.lines().any(|line| line.trim_end() == MARGIN.trim_end()));
    }

    #[test]
    fn lines_do_not_carry_trailing_padding() {
        let out = quoted("short", 60, Painter::plain());
        assert!(out.lines().all(|line| !line.ends_with(' ')));
    }
}
