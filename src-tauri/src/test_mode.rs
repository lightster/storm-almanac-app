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
