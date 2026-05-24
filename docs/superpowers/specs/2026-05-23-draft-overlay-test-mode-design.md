# Draft Overlay Test Mode — Design

**Date:** 2026-05-23
**Status:** Approved design — ready for implementation planning
**Scope:** `storm-almanac-app` only (Tauri 2 + Rust + SvelteKit, Windows)

## Context

The ARAM draft overlay's pipeline runs end-to-end only when the developer
is actually in an ARAM draft. That makes iteration painful: every
change to OCR parsing, badge positioning, or the batch-fetch path
requires queueing a real game and waiting for the draft screen to
appear (~5 minutes per cycle, and the draft window itself is only
~30 seconds long).

The pipeline is well-isolated — `capture_primary_monitor` → `recognize_lines`
→ `looks_like_draft` → `hero_catalog::ensure` → `parse_draft` →
`build_overlay_request` → `fetch_overlay_draft` → `build_overlay_heroes`
→ `show_overlay`. The only step that requires a live game is the very
first one (`capture_primary_monitor`). Substituting a saved screenshot
for the live capture exercises everything else against real OCR,
real network, real window rendering.

## Goal

A debug-only "Show Test Draft" tray menu item that, on click:

- Opens a fullscreen window displaying a saved ARAM draft PNG.
- Runs the draft pipeline against that PNG instead of the screen.
- Draws the existing overlay's win-rate badges on top of the PNG.
- Cycles through additional fixture PNGs on subsequent clicks.

End result: the dev sees the same screenshot + overlay output they
would see in-game, with no game required.

## Decisions (locked)

- **Tray menu trigger only.** New item `Show Test Draft` under the
  existing tray menu. No hotkey, no settings UI. Debug-builds only —
  the item does not appear in release builds.
- **Debug-only via `#[cfg(debug_assertions)]`.** Production users never
  see the item; the test PNGs never ship in the bundled release.
- **Fixtures live at `<repo>/dev-screenshots/*.png`,** discovered by
  scanning the directory at trigger time and sorting alphabetically.
  Filename convention `NN-<map-name>.png` (e.g. `01-lost-cavern.png`)
  keeps order predictable.
- **Click-to-cycle.** Clicking the test PNG window advances to the next
  fixture. After the last one, click closes both windows.
- **Scale-down-only display.** If the PNG is larger than the primary
  monitor in either dimension, scale uniformly to fit. Otherwise
  render at native size, centred on the monitor with black bars around.
  OCR-derived coordinates are scaled by the same factor so badges
  always land on the displayed pixels.
- **Watcher is orthogonal.** The draft watcher only arms when HoTS
  writes a battlelobby file; in dev (no game running) it never arms.
  Test mode does not interact with it.
- **Pipeline split, not flagged.** Today's `run_pipeline_inner` is
  factored into capture + the-rest, so test mode reuses the OCR-onward
  code path verbatim by passing in a pre-loaded image.

## Architecture

### Code structure

- **New module** `src-tauri/src/test_mode.rs`, gated by
  `#[cfg(debug_assertions)]`. Owns the fixture-scan, scale-compute, and
  PNG-display-window logic.
- **Refactor in** `src-tauri/src/draft_overlay.rs`: split today's
  `run_pipeline_inner` into:
  - `run_pipeline_inner(app)` — captures the screen, then calls
    `run_pipeline_with_image(app, img, None)`.
  - `run_pipeline_with_image(app, img, extra_descale: Option<f64>)` —
    everything from OCR through `show_overlay`. The `extra_descale`
    factor multiplies the existing DPI descale when set (test mode
    passes `Some(test_scale)`; the real pipeline passes `None`).
- **Tray-menu insertion in** `src-tauri/src/lib.rs`: a single
  `#[cfg(debug_assertions)]` block adding the menu item and routing
  its click event to `test_mode::run(app, 0)`.

### Fixture path resolution

The test PNGs live in the repo at `dev-screenshots/`, which is outside
the Tauri bundle. Resolved at compile time:

```rust
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../dev-screenshots");
```

`CARGO_MANIFEST_DIR` expands to `<repo>/src-tauri`, so this resolves to
`<repo>/dev-screenshots`. The dev machine never moves, so a compile-time
absolute path is fine.

### Scale computation

```rust
fn compute_test_scale(png: (u32, u32), monitor: (u32, u32)) -> f64 {
    let sx = monitor.0 as f64 / png.0 as f64;
    let sy = monitor.1 as f64 / png.1 as f64;
    sx.min(sy).min(1.0)
}
```

Returned value is multiplied into the existing `descale` step in
`build_overlay_heroes`. When test mode is off, no extra scaling is
applied.

### Test PNG window

A new Tauri window (label `test-draft`) created on first trigger:

- **Fullscreen** at the primary monitor's full logical size.
- **Decoration-less**, **not always-on-top** (so the existing overlay
  window sits above it).
- **Focusable** (so clicks land on it). The existing overlay's
  click-through behaviour means clicks pass through it to the PNG.
- **Content:** a tiny inline HTML page (or a new SvelteKit route
  `/test-draft`) that:
  - Renders the current PNG via a `data:` URL or asset URL, scaled and
    centred per `test_scale`.
  - Listens for `click` and emits a Tauri event `test-draft://next`.

The window persists across cycles; only its content is swapped when
the user clicks to advance.

## Data flow

```
[tray: Show Test Draft]
   │
   ▼
test_mode::run(app, index=0)
   │
   ├── scan dev-screenshots/  → sorted [01-lost-cavern.png, 02-braxis-outpost.png, ...]
   ├── load PNGs[index]       (image::open)
   ├── compute test_scale     (scale-down-only)
   ├── open window (first call) or swap its content (subsequent calls)
   │       (label test-draft, fullscreen, focusable)
   └── draft_overlay::run_pipeline_with_image(app, img, Some(test_scale))
           │
           ├── ocr::recognize_lines
           ├── looks_like_draft
           ├── hero_catalog::ensure
           ├── parse_draft
           ├── build_overlay_request    (self col uses configured player_battletag)
           ├── fetch_overlay_draft       (real HTTP)
           ├── build_overlay_heroes      (rect.descale(dpi_scale * test_scale))
           └── show_overlay              (existing transparent overlay window)

[user clicks test window]
   │
   ├── index+1 < PNGs.len()  → test_mode::run(app, index+1)
   └── else                    → close test window + draft_overlay::hide_window
```

## Error handling

- **`dev-screenshots/` missing or empty** → `log::warn!` with the path
  scanned. No windows open. Tray click is a silent no-op beyond the log.
- **PNG fails to load** (corrupt, unsupported format) → `log::error!`
  with the filename, skip to the next fixture in order. If all fail,
  return without opening anything.
- **Pipeline failure inside `run_pipeline_with_image`** (OCR error,
  catalog fetch fail, batch fetch fail) → existing behaviour applies:
  the error is logged, the overlay stays blank, the test window stays
  visible so the user can click to advance.
- **`looks_like_draft` returns false** for the test PNG → log a test-
  mode-specific warning so the cause is clear (vs. the live pipeline,
  where "no draft detected" is normal). Leave overlay blank; user
  clicks to advance or close.

## Testing

- **Unit tests** in `test_mode.rs`:
  - `scan_fixtures(dir)` returns the alphabetised list of `.png` files,
    or an empty list when the directory is missing.
  - `compute_test_scale((png_w, png_h), (mon_w, mon_h))` returns the
    correct scale for cases: PNG smaller than monitor (returns 1.0),
    PNG larger and same aspect (returns monitor_w/png_w), PNG larger
    and different aspect (returns the more constraining factor).
- **Manual verification** (the iteration loop the feature provides):
  - Trigger tray item → `01-lost-cavern.png` (3840×2160) fills the
    2560×1440 monitor, badges align with portraits.
  - Click → `02-braxis-outpost.png` (1999×1124) appears centred with
    black bars, badges still align.
  - Click → both windows close.
- **No automated end-to-end test.** Mocking the real OCR + HTTP path
  would be both expensive and not exercise the thing we care about
  (which is precisely the real pipeline). The manual loop is the test.

## Out of scope for v1

- Multiple monitor support beyond the primary monitor (consistent with
  the production overlay, which is also primary-monitor only).
- A way to add or rotate fixtures from the running app (drop a PNG in
  `dev-screenshots/` and trigger again — that's the workflow).
- A non-debug "demo mode" for end users to verify their install. If
  desired later, lift the `#[cfg(debug_assertions)]` gate and bundle
  the PNGs into the app resources.
- Recording new fixtures via the app. Use the OS screenshot tool.

## Fixtures included in this branch

- `dev-screenshots/01-lost-cavern.png` — 3840×2160 4K Lost Cavern,
  clean draft, no overlay badges baked in. Primary fixture; exercises
  the scale-down path on a 2560×1440 monitor.
- `dev-screenshots/02-braxis-outpost.png` — 1999×1124 Braxis Outpost,
  carried over from the prior session's cached screenshots. Smaller
  than a 2560×1440 monitor; exercises the render-as-is + black-bars
  path.
