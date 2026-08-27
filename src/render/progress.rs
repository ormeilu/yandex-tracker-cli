//! Saying that a long walk is still walking.
//!
//! `--all` fetches pages until there are none left, and twenty pages of silence
//! is indistinguishable from a hang. Three rules keep the cure from being worse
//! than the disease:
//!
//! 1. **Always stderr.** stdout is a data channel; a progress line in the middle
//!    of a result set would corrupt whatever is parsing it.
//! 2. **Only when stderr is a terminal**, not when stdout is. `find --all > out`
//!    run by a person has a piped stdout and a watching human — that is exactly
//!    the case progress is for. A command whose stderr is captured has no
//!    watcher, so it gets nothing.
//! 3. **Nothing is left behind.** The bar clears itself; the tally on stdout is
//!    the record of what happened.

use std::io::IsTerminal;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// A progress indicator, or nothing at all.
#[derive(Debug)]
pub struct Walk(Option<ProgressBar>);

impl Walk {
    /// Start reporting, if there is anyone to report to.
    #[must_use]
    pub fn start(what: &str) -> Self {
        if !std::io::stderr().is_terminal() {
            return Self(None);
        }

        let bar = ProgressBar::new_spinner();
        bar.set_draw_target(ProgressDrawTarget::stderr());
        if let Ok(style) = ProgressStyle::with_template("{spinner} {msg}") {
            bar.set_style(style);
        }
        bar.set_message(what.to_owned());
        // Without a steady tick the spinner only moves when a page arrives,
        // which is precisely when the caller is not wondering whether it hung.
        bar.enable_steady_tick(Duration::from_millis(120));
        Self(Some(bar))
    }

    /// Report what has been collected so far.
    pub fn page(&self, page: u32, collected: usize, total: Option<u64>) {
        let Some(bar) = &self.0 else { return };
        let of = total.map_or_else(|| "unknown total".to_owned(), |total| total.to_string());
        bar.set_message(format!("page {page}: {collected} of {of}"));
    }

    /// Take the indicator down, leaving the terminal as it was found.
    pub fn finish(self) {
        if let Some(bar) = self.0 {
            bar.finish_and_clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test environment has no terminal, which is the case that matters:
    /// nothing is drawn, and every call is still safe to make.
    #[test]
    fn a_captured_stderr_gets_no_progress() {
        let walk = Walk::start("searching");
        assert!(walk.0.is_none());
        walk.page(2, 50, Some(340));
        walk.finish();
    }
}
