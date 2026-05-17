//! Auto-detection of the ARAM draft screen. A watch window, armed when a
//! match starts, polls a cheap header-OCR gate and shows the draft overlay
//! when the "CHOOSE A HERO" screen appears, then hides it when it is gone.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

use crate::draft_overlay;

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

/// Crop the top 30% of a screenshot — the band that contains the ARAM
/// "CHOOSE A HERO" header — so the detection gate OCRs far fewer pixels
/// than a full-screen pass.
pub fn header_band(img: &image::RgbaImage) -> image::RgbaImage {
    let band_h = ((img.height() as f64 * 0.30).round() as u32).max(1);
    image::imageops::crop_imm(img, 0, 0, img.width(), band_h).to_image()
}

/// Capture the primary monitor, OCR only the header band, and report whether
/// the ARAM "CHOOSE A HERO" header is on screen. Any capture or OCR failure is
/// treated as "no draft" and logged at debug level, so a persistent failure
/// (e.g. HoTS in exclusive fullscreen) does not spam the log.
fn detect_header() -> bool {
    let img = match crate::screen_capture::capture_primary_monitor() {
        Ok(img) => img,
        Err(e) => {
            log::debug!("draft watcher: screen capture failed: {e}");
            return false;
        }
    };
    let band = header_band(&img);
    let lines = match crate::ocr::recognize_lines(&band) {
        Ok(lines) => lines,
        Err(e) => {
            log::debug!("draft watcher: header OCR failed: {e}");
            return false;
        }
    };
    crate::draft_parse::looks_like_draft(&lines)
}

struct WatcherInner {
    /// When the current watch window armed, or `None` when not watching.
    armed_at: Option<Instant>,
    watch: DraftWatch,
}

/// App-managed handle to the draft watcher: the watch state plus a condvar the
/// poll loop waits on, so `arm` and `wake_now` can prod it without waiting out
/// the 1-second poll interval.
pub struct SharedDraftWatcher {
    inner: Mutex<WatcherInner>,
    wake: Condvar,
}

/// Manage the watcher state and spawn the poll loop. Call once at startup,
/// before anything that can call `arm` (i.e. before `battle_lobby_probe`).
pub fn start(app: tauri::AppHandle) {
    app.manage(SharedDraftWatcher {
        inner: Mutex::new(WatcherInner {
            armed_at: None,
            watch: DraftWatch::new(),
        }),
        wake: Condvar::new(),
    });
    std::thread::spawn(move || run_loop(app));
}

/// Arm a fresh 2-minute watch window. No-op when the draft overlay feature is
/// disabled. Called when a match starts.
pub fn arm(app: &tauri::AppHandle) {
    if !crate::config::load_config(app).draft_overlay_enabled {
        return;
    }
    let state = app.state::<SharedDraftWatcher>();
    {
        let mut inner = state.inner.lock().unwrap();
        inner.armed_at = Some(Instant::now());
        inner.watch = DraftWatch::new();
    }
    state.wake.notify_one();
    log::info!("draft watcher: armed");
}

/// Stop the current watch window, if any.
pub fn disarm(app: &tauri::AppHandle) {
    let state = app.state::<SharedDraftWatcher>();
    let was_armed = state.inner.lock().unwrap().armed_at.take().is_some();
    if was_armed {
        log::info!("draft watcher: disarmed");
    }
}

/// Prod the poll loop to run a poll immediately rather than waiting out the
/// 1-second interval — used when HoTS regains focus.
pub fn wake_now(app: &tauri::AppHandle) {
    app.state::<SharedDraftWatcher>().wake.notify_one();
}

fn run_loop(app: tauri::AppHandle) {
    loop {
        {
            let state = app.state::<SharedDraftWatcher>();
            let guard = state.inner.lock().unwrap();
            let _ = state
                .wake
                .wait_timeout(guard, Duration::from_secs(1))
                .unwrap();
        }
        poll_once(&app);
    }
}

fn poll_once(app: &tauri::AppHandle) {
    let state = app.state::<SharedDraftWatcher>();

    let armed_at = match state.inner.lock().unwrap().armed_at {
        Some(at) => at,
        None => return,
    };
    // Polling is foreground-gated: no capture/OCR cost while alt-tabbed away.
    if !crate::game_focus::is_focused(app) {
        return;
    }

    let header_present = detect_header();

    let outcome = {
        let mut inner = state.inner.lock().unwrap();
        // A disarm may have raced the capture/OCR above.
        if inner.armed_at.is_none() {
            return;
        }
        inner.watch.tick(armed_at.elapsed(), header_present)
    };

    match outcome {
        TickOutcome::Idle => {}
        TickOutcome::RunPipeline => {
            log::info!("draft watcher: draft detected, running pipeline");
            draft_overlay::run_pipeline(app.clone());
        }
        TickOutcome::EnsureShown => draft_overlay::reveal(app),
        TickOutcome::Dismiss => {
            log::info!("draft watcher: draft dismissed");
            draft_overlay::hide_window(app);
            disarm(app);
        }
        TickOutcome::Expire => {
            log::info!("draft watcher: watch window expired");
            disarm(app);
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

    #[test]
    fn header_band_crops_the_top_30_percent() {
        let img = image::RgbaImage::new(800, 1000);
        let band = header_band(&img);
        assert_eq!(band.width(), 800);
        assert_eq!(band.height(), 300);
    }

    #[test]
    fn header_band_is_never_zero_height() {
        let img = image::RgbaImage::new(10, 1);
        assert_eq!(header_band(&img).height(), 1);
    }
}
