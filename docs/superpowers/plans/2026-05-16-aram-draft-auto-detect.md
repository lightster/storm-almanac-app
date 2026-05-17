# ARAM Draft Overlay Auto-Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the ARAM draft win-rate overlay appear automatically when the HoTS "CHOOSE A HERO" screen shows, instead of only on a hotkey press.

**Architecture:** A new `draft_watcher` background module owns a *watch window* — armed for 2 minutes when a match starts (the existing `on_game_started` hook). While armed and HoTS is foreground, a 1-second poll captures the primary monitor, OCRs only the top header band (a cheap gate), and drives a pure state machine that runs the existing draft pipeline when the header appears and hides the overlay when it goes away. The pipeline, overlay window, and OCR/capture code are reused unchanged.

**Tech Stack:** Rust, Tauri 2, the `windows` crate (GDI capture + `Windows.Media.Ocr`, already wrapped by `screen_capture`/`ocr`), `image` crate, `std::sync` (`Mutex`/`Condvar`) for the poll loop.

**Notes for the implementer:**
- Work happens on the existing `aram-draft-auto-detect` branch. Do not create a new branch.
- Commit messages: imperative mood, capitalized, no trailing period, no `feat:`/`fix:` prefixes.
- All `cargo` commands run from the `src-tauri` directory. Use a subshell so the session's working directory is unchanged: `(cd src-tauri && cargo test ...)`.
- Tasks 1, 3, and 5 add code that nothing consumes until a later task. Interim `unused` / `dead_code` warnings between tasks are expected and are all resolved once Task 5 wires everything together. They do not fail the build.

---

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `src-tauri/src/draft_watcher.rs` *(new)* | Watch-window state machine, header-band crop, cheap detection gate, poll loop, arm/disarm | 1, 3, 4 |
| `src-tauri/src/draft_parse.rs` | Add the `looks_like_draft` text predicate (pure) | 2 |
| `src-tauri/src/draft_overlay.rs` | Use `looks_like_draft`; add `reveal` (re-show window) | 2, 4 |
| `src-tauri/src/lib.rs` | Declare the module; start the watcher; arm/disarm/wake from lifecycle hooks | 1, 5 |

---

## Task 1: Draft watch-window state machine

The pure, deterministic core: a state machine driven once per poll with the time since the window armed and whether the header was detected. No clock, no I/O — fully unit-testable.

**Files:**
- Create: `src-tauri/src/draft_watcher.rs`
- Modify: `src-tauri/src/lib.rs` (add the module declaration)

- [ ] **Step 1: Declare the module**

In `src-tauri/src/lib.rs`, the module declarations are alphabetical at the top of the file (lines 1-15). Add `mod draft_watcher;` immediately after the `mod draft_types;` line:

```rust
mod draft_types;
mod draft_watcher;
mod game_focus;
```

- [ ] **Step 2: Create `draft_watcher.rs` with the state machine and its tests**

Create `src-tauri/src/draft_watcher.rs` with this exact content. `tick` is deliberately stubbed to always return `Idle` so the crate compiles and the tests can run red:

```rust
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

    /// Advance the state machine by one poll.
    pub fn tick(&mut self, _armed_elapsed: Duration, _header_present: bool) -> TickOutcome {
        TickOutcome::Idle
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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `(cd src-tauri && cargo test draft_watcher)`
Expected: compiles, then FAIL — `detects_draft_and_runs_pipeline`, `ensures_shown_while_draft_persists`, `two_consecutive_misses_dismiss`, `a_hit_resets_the_miss_counter`, `expires_after_timeout_with_no_draft`, and `timeout_does_not_apply_once_draft_shown` fail because the stub always returns `Idle`. The three `Idle`-expecting tests pass.

- [ ] **Step 4: Implement `tick`**

Replace the stub `tick` body with the real state machine:

```rust
    /// Advance the state machine by one poll.
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `(cd src-tauri && cargo test draft_watcher)`
Expected: PASS — `test result: ok. 9 passed`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/draft_watcher.rs src-tauri/src/lib.rs
git commit -m "Add draft watch-window state machine"
```

---

## Task 2: "CHOOSE A HERO" header predicate

A pure text predicate, shared by the existing overlay pipeline and (in Task 4) the watcher's detection gate. Today the check is inline in `draft_overlay.rs`; extract it so there is one definition.

**Files:**
- Modify: `src-tauri/src/draft_parse.rs` (add `looks_like_draft` + tests)
- Modify: `src-tauri/src/draft_overlay.rs` (call it from `run_pipeline_inner`)

- [ ] **Step 1: Add `looks_like_draft` with a stub and tests to `draft_parse.rs`**

`draft_parse.rs` already imports `OcrLine` from `crate::draft_types` (it is the input type of `parse_draft`). Add this function near `parse_draft`, stubbed to return `false`:

```rust
/// Whether OCR output looks like the ARAM "CHOOSE A HERO" draft screen.
pub fn looks_like_draft(lines: &[OcrLine]) -> bool {
    let _ = lines;
    false
}
```

The `draft_parse.rs` test module already has a `line(text, x, y)` helper that builds an `OcrLine`. Add these tests inside that existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn looks_like_draft_detects_the_header() {
        let lines = vec![line("CHOOSE A HERO", 100.0, 50.0)];
        assert!(looks_like_draft(&lines));
    }

    #[test]
    fn looks_like_draft_is_case_insensitive() {
        let lines = vec![line("Choose a Hero", 100.0, 50.0)];
        assert!(looks_like_draft(&lines));
    }

    #[test]
    fn looks_like_draft_false_without_the_header() {
        let lines = vec![line("Nova", 100.0, 50.0)];
        assert!(!looks_like_draft(&lines));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `(cd src-tauri && cargo test looks_like_draft)`
Expected: FAIL — `looks_like_draft_detects_the_header` and `looks_like_draft_is_case_insensitive` fail (stub returns `false`); `looks_like_draft_false_without_the_header` passes.

- [ ] **Step 3: Implement `looks_like_draft`**

Replace the stub body:

```rust
/// Whether OCR output looks like the ARAM "CHOOSE A HERO" draft screen.
pub fn looks_like_draft(lines: &[OcrLine]) -> bool {
    lines
        .iter()
        .any(|l| l.text.to_uppercase().contains("CHOOSE A HERO"))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `(cd src-tauri && cargo test looks_like_draft)`
Expected: PASS — `test result: ok. 3 passed`.

- [ ] **Step 5: Use the shared predicate in the overlay pipeline**

In `src-tauri/src/draft_overlay.rs`, `run_pipeline_inner` currently has this inline check:

```rust
    let looks_like_draft = lines
        .iter()
        .any(|l| l.text.to_uppercase().contains("CHOOSE A HERO"));
    if !looks_like_draft {
        log::info!("draft overlay: no 'CHOOSE A HERO' found; ignoring");
        return Ok(());
    }
```

Replace it with a call to the shared function:

```rust
    if !crate::draft_parse::looks_like_draft(&lines) {
        log::info!("draft overlay: no 'CHOOSE A HERO' found; ignoring");
        return Ok(());
    }
```

- [ ] **Step 6: Verify the build**

Run: `(cd src-tauri && cargo test)`
Expected: PASS — all tests, including the existing 24, still pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/draft_parse.rs src-tauri/src/draft_overlay.rs
git commit -m "Extract the CHOOSE A HERO header predicate"
```

---

## Task 3: Header-band crop

The cheap detection gate OCRs only the top portion of the screen, where the "CHOOSE A HERO" header sits, instead of the whole monitor. This task adds the pure cropping helper.

**Files:**
- Modify: `src-tauri/src/draft_watcher.rs`

- [ ] **Step 1: Add `header_band` with a stub and tests**

In `src-tauri/src/draft_watcher.rs`, add this function after the `DraftWatch` impl block (above the `#[cfg(test)] mod tests` block), stubbed to return the image unchanged:

```rust
/// Crop the top 30% of a screenshot — the band that contains the ARAM
/// "CHOOSE A HERO" header — so the detection gate OCRs far fewer pixels
/// than a full-screen pass.
pub fn header_band(img: &image::RgbaImage) -> image::RgbaImage {
    img.clone()
}
```

Add these tests inside the existing `#[cfg(test)] mod tests` block in the same file:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `(cd src-tauri && cargo test header_band)`
Expected: FAIL — `header_band_crops_the_top_30_percent` fails (stub returns full 1000-px height); `header_band_is_never_zero_height` passes.

- [ ] **Step 3: Implement `header_band`**

Replace the stub body:

```rust
/// Crop the top 30% of a screenshot — the band that contains the ARAM
/// "CHOOSE A HERO" header — so the detection gate OCRs far fewer pixels
/// than a full-screen pass.
pub fn header_band(img: &image::RgbaImage) -> image::RgbaImage {
    let band_h = ((img.height() as f64 * 0.30).round() as u32).max(1);
    image::imageops::crop_imm(img, 0, 0, img.width(), band_h).to_image()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `(cd src-tauri && cargo test header_band)`
Expected: PASS — `test result: ok. 2 passed`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/draft_watcher.rs
git commit -m "Add header-band crop for the draft detection gate"
```

---

## Task 4: Cheap detection gate and poll loop

The OS-bound, threaded shell of the watcher: a `reveal` helper on the overlay, the capture+OCR gate, the app-managed watcher state, the `arm`/`disarm`/`wake_now` controls, and the 1-second poll loop. This code wraps OS APIs and a background thread, so — like `battle_lobby_probe` — it has no unit tests; it is verified by compilation and the regression suite, and exercised by the manual test at the end of this plan.

**Files:**
- Modify: `src-tauri/src/draft_overlay.rs` (add `reveal`)
- Modify: `src-tauri/src/draft_watcher.rs`

- [ ] **Step 1: Add `reveal` to `draft_overlay.rs`**

In `src-tauri/src/draft_overlay.rs`, add this function immediately after `hide_window`:

```rust
/// Re-show the overlay window if it exists — used to restore it after a
/// focus-loss hide while a draft is still on screen.
pub fn reveal(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(DRAFT_LABEL) {
        let _ = w.show();
    }
}
```

- [ ] **Step 2: Extend the `draft_watcher.rs` imports**

In `src-tauri/src/draft_watcher.rs`, change the single import line:

```rust
use std::time::Duration;
```

to:

```rust
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

use crate::draft_overlay;
```

- [ ] **Step 3: Add the detection gate, watcher state, controls, and poll loop**

In `src-tauri/src/draft_watcher.rs`, add the following after the `header_band` function (above the `#[cfg(test)] mod tests` block):

```rust
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
```

- [ ] **Step 4: Verify the build compiles**

Run: `(cd src-tauri && cargo check)`
Expected: compiles. Warnings that `start`, `arm`, `disarm`, and `wake_now` are unused are expected — Task 5 wires them in.

- [ ] **Step 5: Verify the regression suite still passes**

Run: `(cd src-tauri && cargo test)`
Expected: PASS — all tests still pass (24 existing + 9 from Task 1 + 3 from Task 2 + 2 from Task 3 = 38).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/draft_overlay.rs src-tauri/src/draft_watcher.rs
git commit -m "Add the draft detection gate and poll loop"
```

---

## Task 5: Wire the watcher into app startup and lifecycle

Connect the watcher: start it, arm it when a match begins, wake it when HoTS regains focus, and disarm it when the feature is turned off.

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Start the watcher at app startup**

In `src-tauri/src/lib.rs`, find the `battle_lobby_probe::start(app.handle().clone());` call (around line 1161, preceded by a `// Diagnostic probe` comment). Add the watcher start immediately before it — the watcher must be managed before the probe, because the probe can synchronously call `on_game_started` (which arms the watcher):

```rust
            // Auto-detect the ARAM draft screen once a match starts.
            draft_watcher::start(app.handle().clone());

            // Diagnostic probe — watches %TEMP% for *.battlelobby files and
```

(Leave the existing `// Diagnostic probe` comment and `battle_lobby_probe::start` line as they are.)

- [ ] **Step 2: Arm the watcher when a match starts**

In `src-tauri/src/lib.rs`, `on_game_started` currently ends like this:

```rust
    log::info!("game session: match started");
    refresh_blocker_visibility(app);
}
```

Add an `arm` call:

```rust
    log::info!("game session: match started");
    draft_watcher::arm(app);
    refresh_blocker_visibility(app);
}
```

- [ ] **Step 3: Wake the watcher when HoTS regains focus**

In `src-tauri/src/lib.rs`, `handle_focus_change` begins like this:

```rust
fn handle_focus_change(app: &tauri::AppHandle, focused: bool) {
    if !focused {
        draft_overlay::hide_window(app);
    }
    if !focused && !is_game_running() {
```

Insert a wake call between the two `if` blocks:

```rust
fn handle_focus_change(app: &tauri::AppHandle, focused: bool) {
    if !focused {
        draft_overlay::hide_window(app);
    }
    if focused {
        draft_watcher::wake_now(app);
    }
    if !focused && !is_game_running() {
```

- [ ] **Step 4: Disarm the watcher when the feature is turned off**

In `src-tauri/src/lib.rs`, `set_draft_overlay_enabled` has this `else` branch:

```rust
    } else {
        unregister_draft_overlay_hotkey(app);
        draft_overlay::hide_window(app);
    }
```

Add a `disarm` call:

```rust
    } else {
        unregister_draft_overlay_hotkey(app);
        draft_overlay::hide_window(app);
        draft_watcher::disarm(app);
    }
```

- [ ] **Step 5: Verify the build**

Run: `(cd src-tauri && cargo test)`
Expected: PASS — all 38 tests pass.

Run: `(cd src-tauri && cargo check)`
Expected: compiles with no `unused`/`dead_code` warnings from `draft_watcher` — everything is now wired in.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "Wire the draft watcher into app startup and lifecycle"
```

---

## Manual verification

After Task 5, build and run the dev app (`npm run tauri:dev`), enable "Enable Draft Overlay" in the tray, and confirm in a live ARAM game with HoTS in **Windowed Fullscreen**:

1. **Auto-show** — when the "CHOOSE A HERO" screen appears, the win-rate overlay shows within ~1-2 seconds without pressing the hotkey. The log shows `draft watcher: armed` at match start and `draft watcher: draft detected, running pipeline`.
2. **Alt-tab survival** — alt-tab away during the draft and back; the overlay reappears (log shows `draft watcher: ...` activity, the overlay is re-revealed).
3. **Auto-hide** — when the draft screen ends, the overlay disappears within ~2 seconds; the log shows `draft watcher: draft dismissed`.
4. **No false arming cost** — outside a draft (in-match, menus), there is no overlay and the log is quiet.
5. **Open investigation item from the spec** — note in the log whether `draft watcher: armed` (logged when `battle.lobby` is detected) lands before or after the draft screen appears, to confirm the 2-minute window comfortably covers the timing.

The hotkey (`Ctrl+Shift+D`) must still work as a manual trigger throughout.

---

## Self-Review

**Spec coverage** (against `docs/superpowers/specs/2026-05-16-aram-draft-auto-detect-design.md`):
- Watch window armed by the battlelobby signal → Task 5 Step 2 (`arm` in `on_game_started`, which the battlelobby probe drives). ✓
- 2-minute window, foreground-gated → `WATCH_TIMEOUT` (Task 1), `is_focused` gate in `poll_once` (Task 4). ✓
- 1-second cheap header-band gate → `run_loop` 1s `wait_timeout`, `header_band` + `detect_header` (Tasks 3, 4). ✓
- Full pipeline once per draft → `RunPipeline` fires only on the `BeforeDraft → DraftShown` transition (Task 1). ✓
- 2-poll dismiss debounce → `DISMISS_MISSES` (Task 1). ✓
- No re-arm → `Dismiss`/`Expire` call `disarm`; only `on_game_started` re-arms (Tasks 4, 5). ✓
- Hotkey fallback retained → `draft_overlay::toggle` untouched. ✓
- Single tray toggle → `arm` checks `draft_overlay_enabled`; `disarm` on disable (Tasks 4, 5). ✓
- Focus regain immediate re-check → `wake_now` via the condvar (Tasks 4, 5). ✓
- Error handling (capture/OCR failure → no draft, no spam) → `detect_header` debug-level logging (Task 4). ✓
- Testing: state-machine unit tests (Task 1), header-band tests (Task 3), predicate tests (Task 2); capture/OCR/loop exercised by the manual test. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code.

**Type consistency:** `DraftWatch`, `TickOutcome`, `tick(Duration, bool)`, `header_band(&RgbaImage) -> RgbaImage`, `looks_like_draft(&[OcrLine]) -> bool`, `SharedDraftWatcher`, `arm`/`disarm`/`wake_now`/`start`/`reveal` — names and signatures are consistent across all tasks.
