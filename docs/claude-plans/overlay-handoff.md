# Overlay / Map Blocker — Windows Dev Handoff

> **Purpose of this file:** hand off the in-progress overlay work to a Claude
> Code session running on a Windows dev machine, so the feature can be tested
> against a real HoTS install with direct access to debug logs (no
> deploy-and-wait loop). If you're a fresh Claude session on Windows: read
> this top to bottom, then say what you found in the logs.

## Project

**Storm Almanac** — desktop companion for Heroes of the Storm (Tauri 2 +
SvelteKit + Rust). Tray-only app. Repo dir is `storm-uploader`; GitHub remote
is `lightster/storm-almanac-app`. Auto-publishes releases; existing installs
auto-update.

## What we're building

A **map blocker overlay**: a transparent, always-on-top window the user
positions over the HoTS minimap so accidental clicks on the map don't issue
move/camera commands. It's just a window in front of the game catching the
click — no input injection, no hooks, no DLL injection (deliberately, to stay
clear of anti-cheat risk).

Design docs (read these for full rationale):
- `docs/claude-plans/overlay-poc.md` — the initial transparent-window POC
- `docs/claude-plans/map-blocker-overlay.md` — the map blocker feature design

## Shipped so far (all on `main`, released)

| Version | What |
|---|---|
| v0.1.32 | Overlay POC — interactive + click-through demo windows (tray: "Toggle Overlay POC") |
| v0.1.33 | Map blocker — focus-aware show/hide, `Ctrl+Shift+B` mode toggle, persisted geometry |
| v0.1.34 | Fix: blocker no longer steals focus from HoTS (`focusable(false)` + HTML drag/resize handles instead of OS chrome) |
| v0.1.35 | Suppress WebView2 native context menu on right-clicks in the blocker |
| v0.1.36 | Battlelobby probe (diagnostic file watcher) |
| v0.1.37 | Per-map auto-detect — keys the blocker rect off the HoTS map |
| v0.1.38 | Broadened the battlelobby watch set beyond `%TEMP%` |
| v0.1.39 | Map identity = SHA-256 fingerprint of the full `.s2ma` hash set (first-hash-only was constant across maps) |

## How the map blocker works now

1. **Foreground detection** (`src-tauri/src/game_focus.rs`): polls every 500ms.
   On Windows, `GetForegroundWindow` + `GetWindowThreadProcessId`, compared to
   `HeroesOfTheStorm_x64.exe`'s PID. Publishes focus changes.
2. **Show/hide**: when the blocker is enabled (tray: "Enable Map Blocker") and
   HoTS becomes the foreground window, the blocker shows; when HoTS loses
   focus, it hides.
3. **Mode toggle** (`Ctrl+Shift+B`): switches between *blocking* (transparent,
   locked, eats clicks, faint dashed border, red flash on absorbed click) and
   *interactable* (drag the interior to move, edge handles to resize).
4. **Per-map geometry** (`src-tauri/src/battle_lobby_probe.rs`): watches for
   HoTS's `*.battlelobby` file, extracts every `.s2ma` cache hash, SHA-256s the
   sorted set into a per-map fingerprint. The blocker rect is persisted per
   fingerprint — resize once per map, and it auto-restores on future games.

## Open questions — what to verify on Windows

These are the reasons this handoff exists. We've been shipping blind (testing
only via released builds). On Windows with a real HoTS install + dev logs:

1. **Is the `.battlelobby` file detected at all?** v0.1.38 broadened the watch
   dirs. Check the startup log for `battlelobby probe: watching "<dir>"` lines
   and confirm one of them is where HoTS actually writes the file. The file
   was previously seen at `~/Library/CloudStorage/OneDrive-Personal/` on macOS
   (OneDrive-synced) — on Windows it may be in `%TEMP%`, Documents, or a
   OneDrive-redirected Documents folder.
2. **Do different maps produce different fingerprints?** Play 2+ different
   battlegrounds, compare the `battlelobby probe: map fingerprint = <hash>`
   log lines. They must differ. If they don't, capture the per-game
   `s2ma:` hash lists from the log — the fingerprinting strategy needs rework.
3. **Does per-map auto-resize work end-to-end?** Play a map, resize the
   blocker, play it again later, confirm it self-restores. Look for
   `active map changed -> <fp> (saved rect: yes)`.
4. **Does focus-aware show/hide feel right?** ~500ms lag is expected.

Also worth knowing: HoTS must be in **Windowed Fullscreen**, not exclusive
fullscreen — the overlay can't layer over a true-fullscreen swap chain.

## Key files

- `src-tauri/src/lib.rs` — blocker state (`BlockerSettings`, `BlockerState`,
  `BlockerVisualMode`), window helpers (`open_blocker_window`,
  `apply_blocker_visual_mode`, `toggle_blocker_mode`, `on_active_map_changed`,
  `handle_focus_change`), tray menu entries, hotkey registration, focus-poller
  subscriber.
- `src-tauri/src/game_focus.rs` — foreground-window poller.
- `src-tauri/src/battle_lobby_probe.rs` — battlelobby file watcher + `.s2ma`
  hash extraction + map fingerprint. **Still in debug mode** — it dumps every
  battlelobby file to disk and logs all hashes. Trim this down once detection
  is confirmed working (see Cleanup below).
- `src/routes/overlay/+page.svelte` — overlay UI; one route, three modes via
  `?mode=interactive|clickthrough|blocker`.
- `src-tauri/capabilities/overlay.json` — permissions for the overlay windows.

## Windows dev environment

Prerequisites:
- **Node.js** (`^24`, see `package.json` engines)
- **Rust** via [rustup](https://rustup.rs/)
- **MSVC C++ build tools** — Rust's Windows toolchain needs the MSVC linker.
  Install "Desktop development with C++" from the Visual Studio Build Tools.
- **WebView2 runtime** — preinstalled on Windows 11 and recent Windows 10.
- **Tauri CLI**: `cargo install tauri-cli --version "^2"`
- Optional: `rustup component add rust-analyzer` (the repo's
  `.claude/settings.json` enables the rust-analyzer Claude Code plugin).

Then:
```
npm install
npm run tauri:dev
```

## Dev workflow — the whole point of moving to Windows

`npm run tauri:dev` runs the real app locally. **Test changes directly — no
commit / push / bump / wait-for-CI / reinstall loop needed.** Edit code, let
`tauri:dev` hot-reload (frontend) or rebuild (Rust), interact with HoTS.

Only use the release pipeline when you actually want to ship a build.

### IMPORTANT: dev build uses a different identifier

`npm run tauri:dev` runs `tauri dev --config src-tauri/tauri.dev.conf.json`,
which overrides the bundle identifier to **`com.lightster.storm-almanac.dev`**
(note the `.dev`). So in dev mode, logs / dumps / the settings store live
under that identifier — *not* the release `com.lightster.storm-almanac`.

## Where the debug logs are (Windows)

App logs (via `tauri-plugin-log`) and battlelobby dumps go under the app log
directory:

- **Dev build** (`npm run tauri:dev`):
  `%LOCALAPPDATA%\com.lightster.storm-almanac.dev\logs\`
- **Installed release build**:
  `%LOCALAPPDATA%\com.lightster.storm-almanac\logs\`

Inside that `logs\` dir:
- the rolling log file (the `battlelobby probe:`, `active map changed ->`,
  `game focus changed:` lines are here)
- `battlelobby-dumps\` — raw copies of every `.battlelobby` file the probe saw,
  timestamped, for inspection

(`ls` the dir to get exact filenames — don't assume.)

The persisted settings store (`storm-almanac.json`, holds blocker geometry +
`per_map` fingerprint→rect table) lives under the app config dir for the
matching identifier.

## Release workflow (only when shipping)

- This repo commits directly to `main` (no feature branches — owner's
  preference for this repo).
- The bump workflow handles version + tag + triggering the cross-platform
  release build:
  `gh -R lightster/storm-almanac-app workflow run bump-version.yml -f bump=patch`
- Because the bump workflow pushes a "Bump version" commit to `main`, before
  pushing local work: `git stash && git pull --rebase origin main && git stash pop`.
- Releases auto-publish; installs auto-update.

## Cleanup TODO (once detection is confirmed)

`battle_lobby_probe.rs` is currently a debug-grade probe: it copies every
battlelobby file to `battlelobby-dumps\` and logs every individual `.s2ma`
hash. Once map detection + per-map auto-resize are confirmed working on
Windows, trim it: drop the disk dumps (or gate behind a debug flag), and
reduce the per-hash logging to just the final fingerprint.
