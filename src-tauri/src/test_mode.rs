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
