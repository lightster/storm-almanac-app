//! Auto-detection of the ARAM draft screen. A watch window, armed when a
//! match starts, polls a cheap header-OCR gate and shows the draft overlay
//! when the "CHOOSE A HERO" screen appears, then hides it when it is gone.

use std::time::Duration;

/// How long the watch window stays armed waiting for a draft to appear.
const WATCH_TIMEOUT: Duration = Duration::from_secs(120);

/// Consecutive polls with no header before the overlay is dismissed —
/// debounces against momentary UI transitions mid-draft.
const DISMISS_MISSES: u32 = 2;

/// Which half of the watch window's lifecycle the watcher is in.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Armed, draft not yet detected.
    BeforeDraft,
    /// Draft detected; the overlay is up.
    DraftShown,
}

/// What a `tick` decided the caller should do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TickOutcome {
    /// Keep watching; nothing to do this tick.
    Idle,
    /// The draft screen just appeared — run the overlay pipeline.
    RunPipeline,
    /// The draft screen is still up — make sure the overlay is visible.
    EnsureShown,
    /// The draft screen is gone (debounced) — hide the overlay, stop watching.
    Dismiss,
    /// The watch window timed out with no draft — stop watching.
    Expire,
}

/// The pure watch-window state machine. `tick` is driven once per poll with
/// the time since arming and whether the header was detected; it owns no
/// clock or I/O, so it is fully deterministic and unit-testable.
pub struct DraftWatch {
    phase: Phase,
    miss_count: u32,
}

impl Default for DraftWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl DraftWatch {
    pub fn new() -> Self {
        Self {
            phase: Phase::BeforeDraft,
            miss_count: 0,
        }
    }

    /// Advance the state machine by one poll. `Dismiss` and `Expire` are
    /// terminal: the caller stops the watch window and does not tick again.
    pub fn tick(&mut self, armed_elapsed: Duration, header_present: bool) -> TickOutcome {
        match self.phase {
            Phase::BeforeDraft => {
                if header_present {
                    self.phase = Phase::DraftShown;
                    self.miss_count = 0;
                    TickOutcome::RunPipeline
                } else if armed_elapsed >= WATCH_TIMEOUT {
                    TickOutcome::Expire
                } else {
                    TickOutcome::Idle
                }
            }
            Phase::DraftShown => {
                if header_present {
                    self.miss_count = 0;
                    TickOutcome::EnsureShown
                } else {
                    self.miss_count += 1;
                    if self.miss_count >= DISMISS_MISSES {
                        TickOutcome::Dismiss
                    } else {
                        TickOutcome::Idle
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_secs(5);

    #[test]
    fn idle_before_draft_when_header_absent() {
        let mut w = DraftWatch::new();
        assert_eq!(w.tick(SHORT, false), TickOutcome::Idle);
    }

    #[test]
    fn detects_draft_and_runs_pipeline() {
        let mut w = DraftWatch::new();
        assert_eq!(w.tick(SHORT, true), TickOutcome::RunPipeline);
    }

    #[test]
    fn ensures_shown_while_draft_persists() {
        let mut w = DraftWatch::new();
        w.tick(SHORT, true);
        assert_eq!(w.tick(SHORT, true), TickOutcome::EnsureShown);
    }

    #[test]
    fn one_miss_does_not_dismiss() {
        let mut w = DraftWatch::new();
        w.tick(SHORT, true);
        assert_eq!(w.tick(SHORT, false), TickOutcome::Idle);
    }

    #[test]
    fn two_consecutive_misses_dismiss() {
        let mut w = DraftWatch::new();
        w.tick(SHORT, true);
        w.tick(SHORT, false);
        assert_eq!(w.tick(SHORT, false), TickOutcome::Dismiss);
    }

    #[test]
    fn a_hit_resets_the_miss_counter() {
        let mut w = DraftWatch::new();
        w.tick(SHORT, true); // DraftShown
        w.tick(SHORT, false); // miss 1
        w.tick(SHORT, true); // hit -> reset
        w.tick(SHORT, false); // miss 1 again
        assert_eq!(w.tick(SHORT, false), TickOutcome::Dismiss); // miss 2
    }

    #[test]
    fn expires_after_timeout_with_no_draft() {
        let mut w = DraftWatch::new();
        assert_eq!(w.tick(WATCH_TIMEOUT, false), TickOutcome::Expire);
    }

    #[test]
    fn does_not_expire_before_timeout() {
        let mut w = DraftWatch::new();
        let outcome = w.tick(WATCH_TIMEOUT - Duration::from_secs(1), false);
        assert_eq!(outcome, TickOutcome::Idle);
    }

    #[test]
    fn timeout_does_not_apply_once_draft_shown() {
        let mut w = DraftWatch::new();
        w.tick(SHORT, true); // DraftShown
        assert_eq!(w.tick(WATCH_TIMEOUT * 2, true), TickOutcome::EnsureShown);
    }
}
