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

use crate::render::image::Inline;
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
///
/// Where a line is nothing but a markdown image reference and `images` has the
/// picture it names, the picture is drawn in its place. That is where the
/// author put it, and a screenshot that appears three paragraphs from the
/// sentence about it is a different document.
#[must_use]
pub fn quoted(body: &str, width: usize, paint: Painter, images: &Inline) -> String {
    let bar = paint.paint(MARGIN, Palette::untrusted());
    let inner = width.saturating_sub(MARGIN.chars().count());
    let mut out = String::with_capacity(body.len() * 2);

    if images.is_empty() {
        push_rendered(&mut out, body, inner, &bar);
        return out;
    }

    // Text between the pictures is rendered in runs rather than line by line:
    // a list or a fenced block means nothing to markdown once it is split into
    // separate documents.
    let mut run = String::new();
    for line in body.lines() {
        if let Some(picture) = image_reference(line).and_then(|(_, url)| images.get(url)) {
            push_rendered(&mut out, &run, inner, &bar);
            run.clear();
            let _ = write!(out, "{bar}{}", picture.escape);
            // Under the picture: a caption read before there is anything to
            // attach it to is just a filename.
            let _ = writeln!(
                out,
                "{bar}{}",
                paint.paint(&picture.caption, Palette::label())
            );
        } else {
            run.push_str(line);
            run.push('\n');
        }
    }
    push_rendered(&mut out, &run, inner, &bar);

    out
}

fn push_rendered(out: &mut String, body: &str, inner: usize, bar: &str) {
    if body.trim().is_empty() {
        return;
    }
    for line in render(body, inner).lines() {
        // termimad pads lines out to the full width; the trailing run of spaces
        // is invisible until something copies it, so it goes.
        let _ = writeln!(out, "{bar}{}", line.trim_end());
    }
}

/// `![alt](url)`, when that is the whole line.
///
/// Only the whole-line form is substituted. An image reference in the middle of
/// a sentence is part of that sentence, and replacing it with a picture would
/// cut the sentence in half.
#[must_use]
pub fn image_reference(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let rest = line.strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let url = rest.strip_suffix(')')?;
    (!url.is_empty() && !url.contains(char::is_whitespace)).then_some((alt, url))
}

/// Every whole-line image reference in `body`, in the order they appear.
#[must_use]
pub fn image_references(body: &str) -> Vec<(&str, &str)> {
    body.lines().filter_map(image_reference).collect()
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
// A failing `expect` in a test is the test failing, and it says why.
#[allow(clippy::expect_used)]
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
        let out = quoted(
            "# Title\n\nbody text\n",
            40,
            Painter::plain(),
            &Inline::default(),
        );
        assert!(out.lines().count() >= 2);
        assert!(out.lines().all(|line| line.starts_with(MARGIN)));
    }

    /// The margin is the boundary marker, so an empty line inside the block
    /// still gets one — otherwise a blank line would look like the end of it.
    #[test]
    fn a_blank_line_inside_the_block_is_still_marked() {
        let out = quoted("one\n\ntwo", 40, Painter::plain(), &Inline::default());
        assert!(out.lines().any(|line| line.trim_end() == MARGIN.trim_end()));
    }

    fn picture() -> Inline {
        let mut inline = Inline::default();
        inline.insert(
            "/ajax/v2/attachments/29?inline=true".to_owned(),
            crate::render::image::Picture {
                escape: "<PICTURE>\n".to_owned(),
                caption: "screenshot.png".to_owned(),
            },
        );
        inline
    }

    /// The point of the whole exercise: the picture appears where the author
    /// put it, not three paragraphs later.
    #[test]
    fn a_picture_replaces_the_reference_that_names_it() {
        let body = "before\n\n![shot](/ajax/v2/attachments/29?inline=true)\n\nafter";
        let out = quoted(body, 60, Painter::plain(), &picture());

        let lines: Vec<&str> = out.lines().collect();
        let at = lines
            .iter()
            .position(|line| line.contains("<PICTURE>"))
            .expect("the picture was not drawn");

        assert!(lines[..at].iter().any(|line| line.contains("before")));
        assert!(lines[at..].iter().any(|line| line.contains("after")));
        // The caption goes under the picture, and it is still inside the block.
        assert!(lines[at + 1].contains("screenshot.png"));
        assert!(lines[at + 1].starts_with(MARGIN));
        // The markdown it replaced is gone.
        assert!(!out.contains("!["));
    }

    /// A reference nothing matches stays what it was. Substituting only what we
    /// actually fetched is what keeps a description honest.
    #[test]
    fn an_unmatched_reference_is_left_alone() {
        let body = "![shot](/ajax/v2/attachments/999)";
        let out = quoted(body, 60, Painter::plain(), &picture());
        assert!(out.contains("shot"));
        assert!(!out.contains("<PICTURE>"));
    }

    #[test]
    fn only_a_whole_line_reference_is_a_picture() {
        assert_eq!(
            image_reference("![alt](/a/b.png)"),
            Some(("alt", "/a/b.png"))
        );
        assert_eq!(image_reference("  ![](/a/b.png)  "), Some(("", "/a/b.png")));
        // Part of a sentence: replacing it would cut the sentence in half.
        assert_eq!(image_reference("see ![alt](/a/b.png) here"), None);
        assert_eq!(image_reference("[link](/a/b)"), None);
        assert_eq!(image_reference("![alt]()"), None);
    }

    #[test]
    fn lines_do_not_carry_trailing_padding() {
        let out = quoted("short", 60, Painter::plain(), &Inline::default());
        assert!(out.lines().all(|line| !line.ends_with(' ')));
    }
}
