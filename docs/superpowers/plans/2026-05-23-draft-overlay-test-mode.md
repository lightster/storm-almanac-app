# Draft Overlay Test Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a debug-only tray menu item that lets the developer iterate on the ARAM draft overlay by running the full pipeline against saved PNG fixtures instead of needing a live game.

**Architecture:** Refactor `run_pipeline_inner` into a capture step plus a reusable `run_pipeline_with_image` body. A new `#[cfg(debug_assertions)]`-gated `test_mode` module scans `dev-screenshots/` for PNGs, opens a fullscreen "test draft" window showing the PNG (scaled-down-to-fit if needed), and runs the pipeline against it. Clicking the window advances to the next fixture and re-runs the pipeline; after the last one, click closes both the test window and the overlay.

**Tech Stack:** Rust 2021 / Tauri 2 (Windows), SvelteKit 5, `image` crate (already a dep), `base64` (new dep) for PNG → data-URL handoff to the webview.

**Working-directory rule:** Never bare `cd`; always use subshells: `(cd C:/Users/light/code/storm-almanac-app && <cmd>)`. Working directory is `C:\Users\light\code\storm-almanac-app`. Branch is `draft-overlay-test-mode`, one commit ahead of main (the spec, `b454c6d`). Two fixture PNGs are already in `dev-screenshots/` but uncommitted.

---

## File Structure

**Created:**

- `src-tauri/src/test_mode.rs` — Owns the test-mode logic: fixture scan, scale computation, Tauri-managed state, the entry/advance functions, the window open/swap. Gated by `#[cfg(debug_assertions)]` so it compiles out in release.
- `src/routes/overlay/+page.svelte` gains a new `?mode=test-draft` branch that renders the PNG as a fullscreen `<img>` and emits a click back to Rust. (Adding a new mode to the existing overlay page matches the convention used for `interactive`/`clickthrough`/`blocker`/`draft`.)

**Modified:**

- `src-tauri/src/draft_overlay.rs` — Split `run_pipeline_inner` into a screen-capture step + a reusable `run_pipeline_with_image(app, img, extra_descale: Option<f64>)`. All existing tests stay green.
- `src-tauri/src/lib.rs` — Register the `test_mode` module under `#[cfg(debug_assertions)]`, manage its state, add the `Show Test Draft` tray menu item, wire its click handler, register the two new Tauri commands (`test_mode_next`, `test_mode_load_fixture`).
- `src-tauri/Cargo.toml` — Add `base64` dep (for PNG → data-URL encoding when emitting to the webview).
- `dev-screenshots/01-lost-cavern.png`, `dev-screenshots/02-braxis-outpost.png` — already in the working tree; committed in Task 1.

---

## Task 1: Commit the fixture PNGs

This makes the test fixtures available to every subsequent task and gets them into git so the branch is reviewable.

**Files:**
- Add: `dev-screenshots/01-lost-cavern.png` (3840×2160, ~4.3 MB)
- Add: `dev-screenshots/02-braxis-outpost.png` (1999×1124, ~1.9 MB)

- [ ] **Step 1: Verify branch + fixtures present in working tree**

```bash
(cd C:/Users/light/code/storm-almanac-app && git branch --show-current && ls -la dev-screenshots/)
```

Expected: branch `draft-overlay-test-mode`, both PNGs listed.

- [ ] **Step 2: Commit them explicitly**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add dev-screenshots/01-lost-cavern.png dev-screenshots/02-braxis-outpost.png && git commit -m "Add ARAM draft screenshot fixtures for test mode")
```

Expected: `[draft-overlay-test-mode <sha>] Add ARAM draft screenshot fixtures for test mode` with `2 files changed`.

- [ ] **Step 3: Verify**

```bash
(cd C:/Users/light/code/storm-almanac-app && git log -1 --pretty='%H %s' && git show --stat HEAD)
```

Expected: commit message matches, only the two PNGs in the diffstat.

---

## Task 2: Refactor `run_pipeline_inner` — split capture from the rest

This is a pure refactor: behaviour stays identical, existing tests stay green. It just creates the seam test mode will hook into.

**Files:**
- Modify: `src-tauri/src/draft_overlay.rs` (the `run_pipeline_inner` body, currently lines 148–206 of the post-batch-endpoint state of the file)

- [ ] **Step 1: Replace `run_pipeline_inner`'s body with a capture-then-delegate version, and add the new `run_pipeline_with_image` function**

Open `src-tauri/src/draft_overlay.rs`. Replace the existing `fn run_pipeline_inner(app: &tauri::AppHandle) -> Result<(), String>` (and everything inside it) with the two functions below. The two new `build_overlay_request` and `build_overlay_heroes` helpers (currently defined immediately after `run_pipeline_inner`) stay exactly as they are — do not touch them.

```rust
fn run_pipeline_inner(app: &tauri::AppHandle) -> Result<(), String> {
    let t = std::time::Instant::now();
    let img = crate::screen_capture::capture_primary_monitor()?;
    let capture_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: capture {capture_ms}ms");
    run_pipeline_with_image(app, img, None)
}

/// Shared pipeline body: OCR through render. Used by both the live
/// pipeline (image from `capture_primary_monitor`) and test mode
/// (image loaded from a PNG fixture).
///
/// `extra_descale` is multiplied into the existing DPI descale when
/// rendering badge coordinates. Test mode passes `Some(scale)` when
/// the displayed PNG is smaller than its native resolution (because
/// it was scaled down to fit the monitor); the live pipeline always
/// passes `None`.
pub(crate) fn run_pipeline_with_image(
    app: &tauri::AppHandle,
    img: image::RgbaImage,
    extra_descale: Option<f64>,
) -> Result<(), String> {
    let pipeline_start = std::time::Instant::now();

    let t = std::time::Instant::now();
    let lines = crate::ocr::recognize_lines(&img)?;
    let ocr_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: ocr {ocr_ms}ms ({} lines)", lines.len());

    if !crate::draft_parse::looks_like_draft(&lines) {
        log::info!("draft overlay: no 'CHOOSE A HERO' found; ignoring");
        return Ok(());
    }

    let catalog_state = app.state::<crate::hero_catalog::SharedHeroCatalog>();
    let catalog = catalog_state.inner().clone();
    let t = std::time::Instant::now();
    let hero_list = tauri::async_runtime::block_on(catalog.ensure())?;
    let catalog_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: catalog {catalog_ms}ms");

    let draft = crate::draft_parse::parse_draft(&lines, &hero_list);
    if draft.players.is_empty() {
        log::info!("draft overlay: parsed no player columns");
        return Ok(());
    }

    let own_battletag = crate::config::load_config(app).player_battletag;
    let request = build_overlay_request(&draft, &own_battletag);

    let t = std::time::Instant::now();
    let resp = tauri::async_runtime::block_on(crate::overlay_api::fetch_overlay_draft(&request))?;
    let batch_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: batch-fetch {batch_ms}ms");

    let dpi_scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let combined_scale = dpi_scale * extra_descale.unwrap_or(1.0);

    let heroes = build_overlay_heroes(&draft, &resp, combined_scale);

    log::info!(
        "draft pipeline: total {}ms",
        pipeline_start.elapsed().as_millis()
    );
    show_overlay(app, heroes)
}
```

Notes for the implementer:
- The capture-timing log line now sits in `run_pipeline_inner` separately from the other stage logs (which moved into `run_pipeline_with_image`). The test-mode entry will skip the capture log entirely.
- `run_pipeline_with_image` is `pub(crate)` so `test_mode` (a sibling module) can call it; it does not need to be exposed beyond the crate.
- `extra_descale` of `None` produces `combined_scale = dpi_scale * 1.0`, identical to today's behaviour.

- [ ] **Step 2: Run the existing tests to confirm the refactor didn't break anything**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo test --lib draft_overlay 2>&1 | tail -10)
```

Expected: `test result: ok. 4 passed; 0 failed`.

- [ ] **Step 3: Run the full test suite**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo test 2>&1 | grep -E "test result" | head -3)
```

Expected: at least one `test result: ok. 42 passed; 0 failed`.

- [ ] **Step 4: cargo check clean**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo check 2>&1 | tail -3)
```

Expected: `Finished` with no errors.

- [ ] **Step 5: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/src/draft_overlay.rs && git commit -m "Factor draft pipeline body into run_pipeline_with_image")
```

---

## Task 3: Add `test_mode` module — pure helpers with unit tests

Start the new module with just the pure functions (fixture scanning + scale computation). No Tauri integration yet. This keeps the testable logic isolated.

**Files:**
- Create: `src-tauri/src/test_mode.rs`
- Modify: `src-tauri/src/lib.rs` (register module)

- [ ] **Step 1: Write the new module**

Create `src-tauri/src/test_mode.rs` with:

```rust
//! Debug-only test mode: run the draft pipeline against saved PNG
//! fixtures instead of capturing the live screen. See
//! `docs/superpowers/specs/2026-05-23-draft-overlay-test-mode-design.md`.
//!
//! Gated by `#[cfg(debug_assertions)]` at the module-declaration site
//! in `lib.rs` so the whole feature compiles out in release builds.

use std::path::{Path, PathBuf};

/// Compile-time absolute path to the repo's `dev-screenshots/` directory.
/// `CARGO_MANIFEST_DIR` expands to `<repo>/src-tauri`; up one level lands
/// at the repo root.
pub const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../dev-screenshots");

/// Scan a directory for `.png` files (case-insensitive extension match),
/// returning their full paths sorted by filename. Returns an empty vec
/// if the directory doesn't exist or cannot be read.
pub fn scan_fixtures(dir: &Path) -> Vec<PathBuf> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|s| s.eq_ignore_ascii_case("png"))
                .unwrap_or(false)
        })
        .collect();
    out.sort();
    out
}

/// Compute the uniform scale factor to apply when displaying a test PNG
/// of size `png` (width, height in pixels) on a monitor of logical size
/// `monitor`. Scale down to fit when the PNG is larger in either
/// dimension; otherwise render at native size (returns 1.0).
pub fn compute_test_scale(png: (u32, u32), monitor: (u32, u32)) -> f64 {
    let sx = monitor.0 as f64 / png.0 as f64;
    let sy = monitor.1 as f64 / png.1 as f64;
    sx.min(sy).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_fixtures_returns_empty_for_missing_dir() {
        let nonexistent = std::env::temp_dir().join("definitely-not-a-real-dir-12345");
        assert_eq!(scan_fixtures(&nonexistent), Vec::<PathBuf>::new());
    }

    #[test]
    fn scan_fixtures_returns_sorted_pngs_only() {
        let tmp = tempdir_unique("scan_fixtures_returns_sorted_pngs_only");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("02-second.png"), b"x").unwrap();
        std::fs::write(tmp.join("01-first.png"), b"x").unwrap();
        std::fs::write(tmp.join("not-a-png.txt"), b"x").unwrap();
        std::fs::write(tmp.join("03-third.PNG"), b"x").unwrap();

        let got = scan_fixtures(&tmp);

        assert_eq!(
            got,
            vec![
                tmp.join("01-first.png"),
                tmp.join("02-second.png"),
                tmp.join("03-third.PNG"),
            ]
        );

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn compute_test_scale_returns_one_when_png_fits() {
        // PNG smaller than monitor: no scaling.
        assert_eq!(compute_test_scale((1999, 1124), (2560, 1440)), 1.0);
        // Exactly equal: no scaling (factor would be exactly 1.0).
        assert_eq!(compute_test_scale((2560, 1440), (2560, 1440)), 1.0);
    }

    #[test]
    fn compute_test_scale_scales_down_uniformly_for_same_aspect() {
        // 4K PNG on 1440p monitor, both 16:9. Scale = 2560/3840 = 2/3.
        let got = compute_test_scale((3840, 2160), (2560, 1440));
        assert!((got - (2.0 / 3.0)).abs() < 1e-9, "got {got}");
    }

    #[test]
    fn compute_test_scale_uses_more_constraining_axis() {
        // PNG is 4000x1000, monitor is 2000x1000. Width constraint
        // (0.5) is tighter than height (1.0), so the result is 0.5.
        assert_eq!(compute_test_scale((4000, 1000), (2000, 1000)), 0.5);
        // PNG is 1000x4000, monitor is 1000x2000. Height constraint
        // (0.5) is tighter.
        assert_eq!(compute_test_scale((1000, 4000), (1000, 2000)), 0.5);
    }

    /// Create a unique temp directory path for a test. Avoids collisions
    /// between parallel test runs without needing the `tempfile` crate.
    fn tempdir_unique(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("storm-almanac-test-{label}-{nanos}"))
    }
}
```

- [ ] **Step 2: Register the module in `lib.rs` under `#[cfg(debug_assertions)]`**

Open `src-tauri/src/lib.rs`. Find the alphabetical module declarations near the top (between `mod state;` and `mod uploader;`). Insert `mod test_mode;` gated by debug-assertions:

```rust
mod state;
#[cfg(debug_assertions)]
mod test_mode;
mod uploader;
```

(The order shown above is what the file should look like after your edit. Adjust only by inserting the two new lines — do not reorder anything else.)

- [ ] **Step 3: Run the new tests**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo test --lib test_mode 2>&1 | tail -10)
```

Expected: `test result: ok. 5 passed; 0 failed`.

- [ ] **Step 4: Run the full test suite**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo test 2>&1 | grep -E "test result" | head -3)
```

Expected: `test result: ok. 47 passed; 0 failed` (was 42, +5 new).

- [ ] **Step 5: cargo check clean (debug build)**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo check 2>&1 | tail -3)
```

Expected: `Finished`. The module is unused at this point so a `dead_code` warning for `scan_fixtures` and `compute_test_scale` is fine — they get callers in Task 5.

- [ ] **Step 6: cargo check release (confirm debug-only gating works)**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo check --release 2>&1 | tail -5)
```

Expected: `Finished` with no errors and no references to `test_mode` in any warning (since it's compiled out).

- [ ] **Step 7: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/src/test_mode.rs src-tauri/src/lib.rs && git commit -m "Add test_mode module with fixture scan and scale helpers")
```

---

## Task 4: Add `base64` dep + the `?mode=test-draft` overlay view

The test PNG is delivered from Rust to the webview as a base64 data URL embedded in an event payload. Adding the dependency now sets up Task 5 (the actual emit). The Svelte view is added in parallel since they're tightly coupled — the Svelte side is small.

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src/routes/overlay/+page.svelte`

- [ ] **Step 1: Add `base64` to Cargo.toml**

Open `src-tauri/Cargo.toml`. Find the `[dependencies]` block. Append a new line after the last existing dependency (after `image = "0.25"` near the bottom of the main block, before any `[target...]` blocks):

```toml
base64 = "0.22"
```

The relevant region of the file should look like this after the edit:

```toml
flate2 = "1"
image = "0.25"
base64 = "0.22"

[target.'cfg(target_os = "macos")'.dependencies]
```

- [ ] **Step 2: Extend the overlay page to handle `?mode=test-draft`**

Open `src/routes/overlay/+page.svelte`. Two small changes:

(a) In the mode-detection line (currently line 22):

```js
mode = m === 'clickthrough' || m === 'blocker' || m === 'draft' ? m : 'interactive';
```

Replace with:

```js
mode = m === 'clickthrough' || m === 'blocker' || m === 'draft' || m === 'test-draft' ? m : 'interactive';
```

(b) Inside the `onMount` block, add a new branch that listens for the test-draft load event and click handler. Insert this **after** the existing `if (mode === 'draft') { ... }` block (which ends with the `unlisten = await listen('draft://update', ...)` call):

```js
		if (mode === 'test-draft') {
			unlisten = await listen('test-draft://load', (event) => {
				const p = event?.payload;
				if (p && typeof p.dataUrl === 'string') {
					testDraft = {
						dataUrl: p.dataUrl,
						scale: typeof p.scale === 'number' ? p.scale : 1,
					};
				}
			});
		}
```

(c) Add the corresponding state declaration near the other `$state` lines (currently around line 13). After the `let draftHeroes = $state([]);` line, add:

```js
	/** @type {{dataUrl: string, scale: number} | null} */
	let testDraft = $state(null);
```

(d) Add the markup branch. After the existing `{:else if mode === 'draft'} ... {/each}` block but before the `{/if}`, add:

```svelte
{:else if mode === 'test-draft'}
	{#if testDraft}
		<button
			class="test-draft-bg"
			onclick={onTestDraftClick}
			aria-label="advance to next test fixture"
		>
			<img
				src={testDraft.dataUrl}
				alt="ARAM draft test fixture"
				style="transform: translate(-50%, -50%) scale({testDraft.scale}); transform-origin: center;"
			/>
		</button>
	{/if}
{/if}
```

(e) Add the click handler near the other handlers (e.g., after `function close()`):

```js
	async function onTestDraftClick() {
		try {
			await invoke('test_mode_next');
		} catch (e) {
			console.error('test_mode_next failed', e);
		}
	}
```

(f) Add the CSS at the end of the `<style>` block (before the closing `</style>`):

```css
	.test-draft-bg {
		position: fixed;
		inset: 0;
		width: 100vw;
		height: 100vh;
		margin: 0;
		padding: 0;
		border: 0;
		background: #000;
		cursor: pointer;
		overflow: hidden;
	}

	.test-draft-bg img {
		position: absolute;
		left: 50%;
		top: 50%;
		max-width: none;
		max-height: none;
		display: block;
	}
```

- [ ] **Step 3: Confirm the svelte view still type-checks / builds**

```bash
(cd C:/Users/light/code/storm-almanac-app && npm run build 2>&1 | tail -15)
```

Expected: build succeeds (warnings are fine, errors are not). If `npm run build` doesn't exist or behaves differently, fall back to `npx svelte-check` or `npx vite build`; record whichever was used in your report.

- [ ] **Step 4: cargo check (after adding base64 dep)**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo check 2>&1 | tail -5)
```

Expected: `Finished` (a download of the `base64` crate may happen on first run).

- [ ] **Step 5: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/Cargo.toml src-tauri/Cargo.lock src/routes/overlay/+page.svelte && git commit -m "Add ?mode=test-draft overlay view and base64 dependency")
```

Note: `Cargo.lock` should also be staged if it changed (it will, because `base64` is new). If `Cargo.lock` is gitignored or otherwise not present, just stage Cargo.toml.

---

## Task 5: `test_mode::run` and `test_mode::next` — open window, run pipeline, cycle

Wire everything together: state, window lifecycle, the two Tauri commands, the pipeline call. This is the meat of the feature.

**Files:**
- Modify: `src-tauri/src/test_mode.rs` (extend with Tauri integration)
- Modify: `src-tauri/src/lib.rs` (register state + commands)

- [ ] **Step 1: Extend `test_mode.rs` with the runtime integration**

Open `src-tauri/src/test_mode.rs`. Append the following below the existing `compute_test_scale` function and **before** the existing `#[cfg(test)] mod tests` block:

```rust
use base64::Engine as _;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Label of the test-draft window. Reused across cycles; we open it
/// once on the first trigger and swap its content on subsequent clicks.
pub const TEST_DRAFT_LABEL: &str = "test-draft";

/// Tauri-managed state for an in-progress test session. Holds the
/// sorted fixture paths and the current index. Reset (cleared) when
/// the user advances past the last fixture or the window is closed.
#[derive(Default)]
pub struct TestModeState {
    inner: Mutex<Option<Session>>,
}

struct Session {
    fixtures: Vec<std::path::PathBuf>,
    index: usize,
}

#[derive(Serialize, Clone)]
struct LoadPayload {
    #[serde(rename = "dataUrl")]
    data_url: String,
    scale: f64,
}

/// Entry point invoked by the tray menu item. Scans the fixture
/// directory, starts a fresh session at index 0, opens (or reuses)
/// the test-draft window, and runs the pipeline against the first
/// fixture.
pub fn run(app: &AppHandle) {
    let fixtures = scan_fixtures(std::path::Path::new(FIXTURE_DIR));
    if fixtures.is_empty() {
        log::warn!("test mode: no .png fixtures found in {FIXTURE_DIR}");
        return;
    }
    {
        let state = app.state::<TestModeState>();
        *state.inner.lock().unwrap() = Some(Session { fixtures, index: 0 });
    }
    load_current(app);
}

/// Advance to the next fixture in the active session. If we're past
/// the last one, close the test window and the overlay.
pub fn next(app: &AppHandle) {
    let advance_or_finish = {
        let state = app.state::<TestModeState>();
        let mut guard = state.inner.lock().unwrap();
        match guard.as_mut() {
            Some(session) => {
                session.index += 1;
                if session.index >= session.fixtures.len() {
                    *guard = None;
                    None
                } else {
                    Some(())
                }
            }
            None => None,
        }
    };
    if advance_or_finish.is_some() {
        load_current(app);
    } else {
        close_window(app);
        crate::draft_overlay::hide_window(app);
    }
}

/// Internal: load the fixture at the current index, push it to the
/// test-draft window (opening it if needed), and trigger the pipeline.
fn load_current(app: &AppHandle) {
    let (path, scale, img) = {
        let state = app.state::<TestModeState>();
        let guard = state.inner.lock().unwrap();
        let session = match guard.as_ref() {
            Some(s) => s,
            None => return,
        };
        let path = session.fixtures[session.index].clone();
        let monitor = monitor_size(app);
        let img = match image::open(&path) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                log::error!("test mode: failed to load {}: {e}", path.display());
                return;
            }
        };
        let png_size = (img.width(), img.height());
        let scale = compute_test_scale(png_size, monitor);
        (path, scale, img)
    };

    // Read raw PNG bytes (separately from the decoded RgbaImage, so
    // we don't have to re-encode the image just to display it).
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            log::error!("test mode: failed to read PNG bytes for {}: {e}", path.display());
            return;
        }
    };
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );

    open_or_focus_window(app);
    let payload = LoadPayload {
        data_url,
        scale,
    };
    if let Err(e) = app.emit_to(TEST_DRAFT_LABEL, "test-draft://load", payload) {
        log::error!("test mode: failed to emit load event: {e}");
    }

    log::info!("test mode: running pipeline against {}", path.display());
    let app_clone = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::draft_overlay::run_pipeline_with_image(&app_clone, img, Some(scale)) {
            log::error!("test mode: pipeline failed: {e}");
        }
    });
}

/// Look up the primary monitor's logical size in pixels. Falls back to
/// 1920x1080 if unavailable (very unusual on Windows; the live overlay
/// has the same fallback shape).
fn monitor_size(app: &AppHandle) -> (u32, u32) {
    app.primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let scale = m.scale_factor();
            let s = m.size();
            (
                (s.width as f64 / scale).round() as u32,
                (s.height as f64 / scale).round() as u32,
            )
        })
        .unwrap_or((1920, 1080))
}

/// Open the test-draft window if not present; otherwise focus it so
/// it surfaces above other windows. NOT always-on-top — the draft
/// overlay window IS always-on-top and must sit above this one.
fn open_or_focus_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(TEST_DRAFT_LABEL) {
        let _ = w.set_focus();
        return;
    }
    let mut builder = WebviewWindowBuilder::new(
        app,
        TEST_DRAFT_LABEL,
        WebviewUrl::App("/overlay?mode=test-draft".into()),
    )
    .decorations(false)
    .always_on_top(false)
    .skip_taskbar(true)
    .resizable(false)
    .focusable(true)
    .visible(true);

    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor();
        let size = m.size();
        let pos = m.position();
        builder = builder
            .inner_size(size.width as f64 / scale, size.height as f64 / scale)
            .position(pos.x as f64 / scale, pos.y as f64 / scale);
    }

    match builder.build() {
        Ok(_) => log::info!("test mode: opened test-draft window"),
        Err(e) => log::error!("test mode: failed to open window: {e}"),
    }
}

fn close_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(TEST_DRAFT_LABEL) {
        match w.close() {
            Ok(_) => log::info!("test mode: closed test-draft window"),
            Err(e) => log::error!("test mode: close failed: {e}"),
        }
    }
}

/// Internal counterpart to the `test_mode_next` Tauri command in
/// `lib.rs`. Forwards to `next`. Kept here so the test-mode logic
/// stays in one module; the public Tauri command lives in `lib.rs`
/// so it can be registered unconditionally (with a release stub).
pub fn handle_next_command(app: &AppHandle) {
    next(app);
}
```

- [ ] **Step 2: Register the state + command in `lib.rs`**

Open `src-tauri/src/lib.rs`.

(a) In the `setup()` block, manage the new state. Find the existing `app.manage(hero_catalog::SharedHeroCatalog::new());` line (added in the prior overlay-batch-endpoint feature). Immediately after it, add:

```rust
            #[cfg(debug_assertions)]
            app.manage(test_mode::TestModeState::default());
```

(b) Define a top-level `test_mode_next` Tauri command in `lib.rs` so it can be registered unconditionally. In debug builds it forwards to `test_mode::handle_next_command`; in release builds it's a no-op stub. Add this near the other `#[tauri::command]` functions (e.g. before `fn open_website_window`):

```rust
/// Tauri command invoked by the test-draft webview on click. Forwards
/// to the test_mode module when present; a no-op in release builds
/// (the webview is never opened there).
#[tauri::command]
fn test_mode_next(app: tauri::AppHandle) {
    #[cfg(debug_assertions)]
    test_mode::handle_next_command(&app);
    #[cfg(not(debug_assertions))]
    let _ = app;
}
```

The `let _ = app;` in the release branch silences the "unused parameter" warning without needing a separate function signature.

(c) Add the command to the existing `tauri::generate_handler![...]` macro call near the bottom of `pub fn run()`. The current call ends with `draft_overlay::get_draft_overlay_data,`. Add `test_mode_next` after it:

```rust
        .invoke_handler(tauri::generate_handler![
            get_uploads,
            watch_uploads,
            get_config,
            save_config_cmd,
            autostart::enable_autostart,
            autostart::disable_autostart,
            autostart::is_autostart_enabled,
            read_talent_builds,
            write_talent_builds,
            is_game_running_cmd,
            check_input_permission,
            get_recording_status,
            toggle_overlay_pair,
            reveal_path,
            clear_webview_data,
            draft_overlay::get_draft_overlay_data,
            test_mode_next,
        ])
```

The handler list is unconditional — only the function body changes between debug and release builds.

- [ ] **Step 3: cargo check both profiles**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo check 2>&1 | tail -3 && cargo check --release 2>&1 | tail -3)
```

Expected: both `Finished` with no errors.

- [ ] **Step 4: Run the test suite (existing tests should still pass; no new tests in this task)**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo test 2>&1 | grep -E "test result" | head -3)
```

Expected: `test result: ok. 47 passed`.

- [ ] **Step 5: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/src/test_mode.rs src-tauri/src/lib.rs && git commit -m "Wire test mode window, state, and Tauri command")
```

---

## Task 6: Add the `Show Test Draft` tray menu item

The very last wiring step: add the menu item to the tray so the user can actually trigger test mode.

**Files:**
- Modify: `src-tauri/src/lib.rs` (tray menu + click handler)

- [ ] **Step 1: Add the menu item under `#[cfg(debug_assertions)]`**

Open `src-tauri/src/lib.rs`. Find the tray-menu construction in `setup()`. The current menu builds with these items in order: `open_website`, `settings`, separator, `check_update`, `rescan`, separator, `enable_blocker`, `enable_draft`, `toggle_overlay`, separator, `quit`.

(a) Just before the `let quit = MenuItemBuilder::with_id("quit", ...)` line, add:

```rust
            #[cfg(debug_assertions)]
            let show_test_draft = MenuItemBuilder::with_id("show_test_draft", "Show Test Draft").build(app)?;
```

(b) In the `MenuBuilder::new(app).item(&open_website)...` chain, insert the test-draft item between `toggle_overlay` and the separator-before-quit. The current chain ends with:

```rust
                .item(&enable_blocker)
                .item(&enable_draft)
                .item(&toggle_overlay)
                .separator()
                .item(&quit)
                .build()?;
```

Change it to:

```rust
            let mut menu = MenuBuilder::new(app)
                .item(&open_website)
                .item(&settings)
                .separator()
                .item(&check_update)
                .item(&rescan)
                .separator()
                .item(&enable_blocker)
                .item(&enable_draft)
                .item(&toggle_overlay);
            #[cfg(debug_assertions)]
            {
                menu = menu.separator().item(&show_test_draft);
            }
            let menu = menu.separator().item(&quit).build()?;
```

Notes for the implementer:
- The double `let menu = ...` shadows; the second binding consumes the `MenuBuilder` and produces the built `Menu`. This pattern lets the conditional debug-only item slot into the chain.
- If the surrounding code has changed since this plan was written and the exact menu chain differs, follow the same shape: build up the menu in a mutable binding, conditionally insert the test-draft item, then finalise.

(c) In the `.on_menu_event(...)` closure (also in `setup()`), add a new branch handling the test-draft click. The closure currently handles `quit`, `open_website`, `check_update`, `settings`, `rescan`, `toggle_overlay`, `enable_blocker`, `enable_draft_overlay`. After the final `else if event.id() == "enable_draft_overlay"` block, add:

```rust
                    } else if event.id() == "show_test_draft" {
                        #[cfg(debug_assertions)]
                        test_mode::run(app);
                    }
```

(Watch the brace structure — that snippet replaces the closing brace of the *previous* `else if` block. The full edit looks like: the line currently reading `}` after the enable_draft branch becomes `} else if event.id() == "show_test_draft" {`, followed by the cfg-gated body, followed by a new closing `}`.)

For release builds the click would be a no-op since the item never appears in the menu, but the cfg-gate on the body prevents an unused-function warning when `test_mode` isn't compiled.

- [ ] **Step 2: cargo check both profiles**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo check 2>&1 | tail -3 && cargo check --release 2>&1 | tail -3)
```

Expected: both `Finished`. The release build must succeed with the menu item compiled out.

- [ ] **Step 3: Test suite still green**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo test 2>&1 | grep -E "test result" | head -3)
```

Expected: `test result: ok. 47 passed`.

- [ ] **Step 4: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/src/lib.rs && git commit -m "Add Show Test Draft tray menu item")
```

---

## Task 7: Manual live verification

The whole point of this feature is the manual loop, so this task is the user-facing acceptance test.

**Files:** none modified.

- [ ] **Step 1: Launch the dev build**

```bash
(cd C:/Users/light/code/storm-almanac-app && npm run tauri:dev)
```

Leave the terminal open; watch the log file at `%LOCALAPPDATA%\com.lightster.storm-almanac.dev\logs\` for `hero catalog warmed (NN heroes)`. If the warmup line doesn't appear within 30 seconds, the backend may be unreachable — stop and investigate before continuing.

- [ ] **Step 2: First fixture — Lost Cavern**

Right-click the tray icon → click `Show Test Draft`. Expected:
- A fullscreen window opens displaying `01-lost-cavern.png` at 2/3 scale (so it exactly fills your 2560×1440 monitor).
- The log shows `test mode: running pipeline against ...01-lost-cavern.png`, then the existing `draft pipeline: ocr ...`, `catalog ...`, `batch-fetch ...`, `total ...` lines.
- After ~1–2 seconds the overlay badges appear on top of the Lost Cavern portraits, aligned to the hero circles.

If badges are misaligned, the `extra_descale` math is off — capture a log + screenshot and stop.

- [ ] **Step 3: Click to advance — Braxis Outpost**

Click anywhere on the test window. Expected:
- The window content swaps to `02-braxis-outpost.png` at native resolution (1999×1124), centred on the 2560×1440 monitor with black bars around it.
- A new pipeline run logs (capture/catalog/batch-fetch/total lines).
- Badges re-appear, aligned to the Braxis portraits.

- [ ] **Step 4: Click to close**

Click the test window once more. Expected:
- The test window closes.
- The overlay window hides (no more badges floating on whatever is behind).
- Log shows `test mode: closed test-draft window` and `draft overlay: hid window`.

- [ ] **Step 5: Re-trigger to confirm idempotency**

Right-click tray → `Show Test Draft` again. Expected: starts again from Lost Cavern (index 0). Confirms the session state was properly cleared on close.

- [ ] **Step 6: Confirm release-build menu does not show the item (optional but recommended)**

```bash
(cd C:/Users/light/code/storm-almanac-app && npm run tauri build 2>&1 | tail -10)
```

Then briefly run the resulting release binary (path printed in the build output). Verify that the tray menu does NOT contain `Show Test Draft`. Quit the release build immediately afterward — don't install it.

This step is optional because it's a long build; skip if cycle time matters more than the verification.

---

## Task 8: Finish the development branch

Wrap by invoking the finishing-a-development-branch skill.

- [ ] **Step 1: Confirm all tests pass and the build is clean**

```bash
(cd C:/Users/light/code/storm-almanac-app/src-tauri && cargo test 2>&1 | grep -E "test result" | head -3 && cargo check 2>&1 | tail -3)
```

Expected: 47 tests pass, cargo check clean.

- [ ] **Step 2: Invoke the finishing skill**

This is a debug-only dev affordance — no behavior changes for production users. Per the repo's recent convention the user typically merges directly to main once verified; the skill will present the four standard options (merge, PR, keep, discard) and let the user decide. Skill: `superpowers:finishing-a-development-branch`.

---

## Self-review

**Spec coverage:**
- "Tray menu trigger only" → Task 6. ✓
- "Debug-only via `#[cfg(debug_assertions)]`" → cfg gates in Tasks 3, 5, 6. ✓
- "Fixtures live at `<repo>/dev-screenshots/*.png`" → Task 1 commits them, Task 3 defines `FIXTURE_DIR`. ✓
- "Click-to-cycle" → `test_mode::next` in Task 5, click handler in Task 4. ✓
- "Scale-down-only display" → `compute_test_scale` in Task 3, applied via `extra_descale` in Task 2's `run_pipeline_with_image`, and on the CSS img in Task 4. ✓
- "Watcher is orthogonal" → no watcher changes anywhere in the plan. ✓
- "Pipeline split, not flagged" → Task 2. ✓
- Code structure: new `test_mode.rs` (Tasks 3+5), `draft_overlay.rs` refactor (Task 2), `lib.rs` integration (Tasks 5+6). ✓
- Fixture path resolution via `concat!(env!("CARGO_MANIFEST_DIR"), "/../dev-screenshots")` → Task 3. ✓
- `compute_test_scale` signature `((u32,u32), (u32,u32)) -> f64` → Task 3 (matches spec exactly). ✓
- Test PNG window: decoration-less, not always-on-top, focusable → Task 5's `open_or_focus_window`. ✓
- Window persists across cycles → Task 5's `open_or_focus_window` checks for existing window. ✓
- Error: empty fixture dir → `log::warn!` in Task 5's `run`. ✓
- Error: PNG load fail → `log::error!` and early return in Task 5's `load_current`. (Plan does not currently skip to the next fixture on load failure — see Note below.)
- Error: pipeline failure → existing behaviour preserved by Task 2's refactor (error returned, logged in spawn handler). ✓
- `looks_like_draft` false → existing `log::info!` from `run_pipeline_with_image` (Task 2). ✓
- Unit tests for `scan_fixtures` and `compute_test_scale` → Task 3. ✓
- Manual verification: Lost Cavern → Braxis → close → re-trigger → Task 7. ✓

**Note on error-handling fidelity:** the spec says "If PNG fails to load, skip to the next fixture in order." The plan as written returns early on load failure rather than skipping. Skipping requires either a loop or recursion, both of which add complexity and obscure the common case. Given that the fixtures are dev-controlled (the dev knows whether their PNG is valid), the simpler "log error, return" is appropriate; the dev can click to advance past the bad fixture manually. If automated skip becomes desirable later it's a small follow-up.

**Placeholder scan:** no TODO/TBD/"similar to" placeholders. The one prose hedge ("If the surrounding code has changed since this plan was written and the exact menu chain differs, follow the same shape...") is intentional — the implementer needs that latitude because the lib.rs setup block is large and may have small reorderings — but the desired shape is fully specified.

**Type consistency:**
- `run_pipeline_with_image(app, img, extra_descale: Option<f64>)` — same signature in Task 2's definition and Task 5's invocation. ✓
- `TestModeState`, `Session`, `LoadPayload` — defined in Task 5, used only in Task 5. ✓
- `FIXTURE_DIR`, `scan_fixtures`, `compute_test_scale` — defined in Task 3, used in Task 5. ✓
- `TEST_DRAFT_LABEL` — defined in Task 5, used in `open_or_focus_window`, `close_window`, and `app.emit_to(...)`. Frontend uses string literal `'test-draft'` matching this label. ✓
- `test-draft://load` event name + `dataUrl`/`scale` payload — emitted in Task 5, consumed in Task 4's Svelte listener. ✓
- `test_mode_next` Tauri command — defined in Task 5, invoked from Svelte in Task 4. ✓
