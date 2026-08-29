//! Ratios drawn as a bar, where there is a terminal to draw on.
//!
//! One rule, and it is the whole design: **decoration follows the painter.**
//! The numbers are the contract — `4 of 6` is what a caller reads, greps and
//! parses, and it appears in both modes unchanged. The blocks are chrome, and a
//! pipe gets none of them, so no piped output grows a byte for a person's
//! benefit and nothing an agent parses changes shape.
//!
//! This is the same switch colour is on (`Context::is_human`), for the same
//! reason: an agent pays per token for anything a terminal draws.

use crate::render::Context;
use crate::render::style::Palette;

/// How wide a bar is, in cells.
///
/// Short on purpose. It sits inside a line that already carries the numbers, so
/// its job is to be read at a glance rather than to be measured.
const CELLS: usize = 10;

/// `▓▓▓▓░░░░░░ 4 of 6`, or just `4 of 6`.
///
/// `total` of zero has no ratio to show: nothing is drawn, and the numbers are
/// still printed, because "0 of 0" is an answer and a blank line is not.
#[must_use]
pub fn ratio(done: u64, total: u64, ctx: &Context) -> String {
    let numbers = format!("{done} of {total}");
    if !ctx.is_human() || total == 0 {
        return numbers;
    }

    let filled = cells(done, total);
    let paint = ctx.painter();
    let colour = if done >= total {
        Palette::ok()
    } else {
        Palette::warn()
    };

    format!(
        "{}{} {numbers}",
        paint.paint(&"▓".repeat(filled), colour),
        paint.paint(&"░".repeat(CELLS - filled), Palette::label()),
    )
}

/// How many cells of [`CELLS`] are filled.
///
/// Rounded down, and never rounded up to full: a bar that reads as finished
/// while one item is outstanding is worse than no bar. The same holds at the
/// bottom — any progress at all shows one cell, so "started" and "not started"
/// never look alike.
fn cells(done: u64, total: u64) -> usize {
    if done >= total {
        return CELLS;
    }
    if done == 0 {
        return 0;
    }
    let cells = u64::try_from(CELLS).unwrap_or(u64::MAX);
    let filled = usize::try_from(done.saturating_mul(cells) / total.max(1)).unwrap_or(CELLS);
    filled.clamp(1, CELLS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Audience, Format};

    fn ctx(audience: Audience) -> Context {
        Context {
            format: Format::Text,
            audience,
            description_lines: None,
            extra_fields: Vec::new(),
            width: 80,
            images: false,
            inline: crate::render::image::Inline::default(),
        }
    }

    /// The rule this module exists for.
    #[test]
    fn a_pipe_gets_the_numbers_and_nothing_else() {
        assert_eq!(ratio(4, 6, &ctx(Audience::Machine)), "4 of 6");
        assert_eq!(ratio(0, 0, &ctx(Audience::Machine)), "0 of 0");
    }

    #[test]
    fn a_terminal_gets_the_same_numbers_with_a_bar_in_front() {
        let drawn = ratio(4, 6, &ctx(Audience::Human));
        assert!(drawn.ends_with("4 of 6"), "{drawn}");
        assert!(drawn.contains('▓'), "{drawn}");
        assert!(drawn.contains('░'), "{drawn}");
    }

    /// Nothing to divide by, so nothing is drawn — but the answer is still given.
    #[test]
    fn a_ratio_of_nothing_draws_no_bar() {
        assert_eq!(ratio(0, 0, &ctx(Audience::Human)), "0 of 0");
    }

    #[test]
    fn a_full_bar_means_finished_and_only_that() {
        assert_eq!(cells(6, 6), CELLS);
        assert_eq!(cells(7, 6), CELLS);
        // One item outstanding must not read as done.
        assert_eq!(cells(99, 100), CELLS - 1);
    }

    #[test]
    fn any_progress_at_all_is_visible() {
        assert_eq!(cells(0, 100), 0);
        assert_eq!(cells(1, 100), 1);
    }
}
