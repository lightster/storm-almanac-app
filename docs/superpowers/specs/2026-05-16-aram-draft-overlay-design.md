# ARAM Draft Win-Rate Overlay — Design

**Date:** 2026-05-16
**Status:** Approved design — ready for implementation planning

## Context

Storm Almanac is a tray-only desktop companion for Heroes of the Storm
(Tauri 2 + SvelteKit + Rust, Windows). It already has a transparent
always-on-top overlay system (the map blocker).

In a HoTS ARAM game, the hero-select screen ("CHOOSE A HERO") offers each
player 3 random heroes to pick from. Seeing win rates for those options at
draft time would help the player choose.

Prior investigation established that the offered-hero pool is **not recoverable
from any file on disk**: `replay.server.battlelobby`, `replay.details`,
`replay.initData`, and `replay.attributes.events` all record only *outcomes*
(final picks), never the *options*. This was confirmed empirically — a hero
offered but not picked (Butcher) was absent from every file. The offered heroes
exist only transiently on screen during the draft.

Therefore the only viable data source is the **draft screen itself**. The hero
names are rendered as clean white UI text under each portrait, which makes OCR
a tractable approach. A captured ARAM select screen shows the player's whole
team — 5 players × 3 hero choices = 15 hero names, plus the 5 player names.

## Goal

When the ARAM hero-select screen is showing, the player presses a hotkey and a
transparent overlay draws a win rate next to each of the 15 offered heroes.

## Decisions (locked)

- **Trigger:** hotkey only for v1 (`Ctrl+Shift+D`). Auto-detection of the
  "CHOOSE A HERO" screen is deferred.
- **Display:** win rate anchored next to each hero on the draft screen, for all
  15 heroes — positioned using the bounding boxes OCR returns per text line.
- **Capture model — "full-screen mirror" (Approach A):** capture the entire
  primary monitor; the overlay is one full-screen transparent click-through
  window; OCR boxes are screen pixels and map 1:1 to overlay coordinates.
  Assumes HoTS runs in Windowed Fullscreen on the primary monitor — already a
  requirement for any Storm Almanac overlay.
- **Win-rate source:** the existing `hots.lightster.ninja` backend, endpoint
  `GET /api/draft` (public, no auth). It returns per-player, per-hero ARAM win
  rates plus a global baseline.
- **Each anchored label shows both:** the **overall** ARAM win rate for the
  hero (always available) and the **column player's personal** win rate on
  that hero (when their data exists).
- **OCR engine:** `Windows.Media.Ocr` (built into Windows; the app already
  depends on the `windows` crate, so no new dependency).

## Architecture

A new feature parallel to the map blocker: a tray toggle ("Enable Draft
Overlay") that registers the hotkey, plus the hotkey handler. Pressing the
hotkey during an ARAM draft runs a one-shot pipeline:

```
hotkey
  -> capture primary monitor (bitmap)
  -> OCR (lines with bounding boxes)
  -> parse: group into 5 player columns, 3 heroes each
  -> fuzzy-match hero text to canonical hero names
  -> resolve each player name to a battletag; fetch GET /api/draft per player
  -> merge: each hero gets { rect, overall_wr, player_wr? }
  -> emit to a transparent click-through overlay window
  -> overlay (Svelte) draws each win rate at its rect
```

The overlay dismisses on hotkey re-press or when HoTS loses focus.

## Components

Each is independently understandable and testable.

- **`screen_capture`** (Rust) — captures the primary monitor to an in-memory
  bitmap, via BitBlt through the `windows` crate.
- **`ocr`** (Rust) — bitmap -> `Vec<OcrLine { text, rect }>` via
  `Windows.Media.Ocr`.
- **`draft_parse`** (Rust, **pure function**) — `Vec<OcrLine>` ->
  `Draft { players: [{ name, rect, heroes: [{ hero, rect }] }] }`. Groups text
  into player columns by x-position; fuzzy-matches each hero label to the
  canonical hero list. Pure and fixture-testable.
- **`win_rates`** (Rust) — resolves each OCR'd player name to a backend
  battletag, calls `GET /api/draft?player=<battletag>`, caches responses per
  session. The `heroes` array returned by `/api/draft` doubles as the canonical
  hero list (names + `search_aliases` + `slug`) used by `draft_parse`.
- **Overlay UI** — a new mode `?mode=draft` in
  `src/routes/overlay/+page.svelte`. Receives the merged draft data via a Tauri
  event and renders anchored win-rate labels.
- **Config** — a stored setting for the *user's own* player battletag, so the
  user's own column always resolves reliably regardless of name-matching.
- **Window / tray / hotkey wiring** in `src-tauri/src/lib.rs`, reusing the map
  blocker's patterns (`WebviewWindowBuilder`, `global_shortcut`, tray
  `CheckMenuItem`, `capabilities/overlay.json`).

## Data flow

1. Hotkey pressed (feature enabled) -> pipeline starts.
2. `screen_capture` grabs the primary monitor.
3. `ocr` returns text lines with bounding boxes.
4. `draft_parse` groups lines into 5 player columns (by x), pairs each column's
   player name with its 3 hero labels, fuzzy-matches hero text to canonical
   names. Output carries each hero's screen `rect`.
5. `win_rates` resolves the 5 player names and fetches `GET /api/draft` for
   each (parallel, cached). Each response yields `globalStats` (overall ARAM
   win rate per hero) and `playerStats` (that player's per-hero ARAM win rate).
6. Merge: every hero -> `{ hero, rect, overall_wr, player_wr? }`.
7. Emit a Tauri event to the `?mode=draft` overlay window.
8. The overlay draws each label at its `rect`.

## Anchored label

At each hero's bounding box, the overlay shows:

- **Overall ARAM win rate** — always present.
- **Column player's personal ARAM win rate** — shown when that player resolved
  to a battletag and has games on the hero; otherwise omitted.

Color coding mirrors the `/play/draft` page (green ≥ 55%, red < 45%, neutral
between).

## Trigger & lifecycle

- Tray toggle "Enable Draft Overlay" registers/unregisters the `Ctrl+Shift+D`
  hotkey and persists the enabled state, mirroring the map blocker.
- Hotkey press runs the pipeline once and shows the overlay.
- The overlay window is transparent, click-through, always-on-top, and covers
  the full primary monitor.
- It dismisses on hotkey re-press or when HoTS loses focus.

## Error handling

- OCR finds no "CHOOSE A HERO" header or no player columns -> brief
  "no draft detected" notice; do nothing.
- A hero label will not fuzzy-match the hero list -> skip that anchor.
- A player name will not resolve to a battletag, or the player has no ARAM
  games on a hero -> show the overall win rate only, omit the personal number.
- `GET /api/draft` fails -> render heroes with no numbers plus a small error
  indicator; never crash.
- Screen capture fails (e.g. exclusive fullscreen) -> notice instructing the
  user to switch HoTS to Windowed Fullscreen.

## Testing

- **Unit:** `draft_parse` — column grouping and hero fuzzy-matching against
  fixture OCR output, including tricky names ("Li-Ming", "The Butcher") and
  simulated OCR typos.
- **Integration:** feed a saved ARAM-draft screenshot through `ocr` ->
  `draft_parse` and assert the 5 players / 15 heroes are recovered.
- **Manual:** end-to-end in a live ARAM game.
- `screen_capture` and `ocr` wrap OS APIs and are exercised by the integration
  test rather than unit-tested in isolation.

## Open item

Player-name -> battletag resolution is the soft spot. The draft screen shows
name-parts (e.g. `Togo99`); `GET /api/draft` expects a unique
`display_battletag`. The exact behavior of the backend's player-search/alias
facility (`GET /api/players/search` or equivalent) must be confirmed during
implementation planning. The user's own identity is pinned by the config
setting and does not depend on this; teammates that do not resolve simply fall
back to the overall win rate, which the user has accepted.

## Out of scope for v1

- Auto-detection of the draft screen (periodic OCR) — hotkey only for now.
- Window-tracked overlay / multi-monitor support (Approach B) — full-screen
  mirror assumes HoTS on the primary monitor in Windowed Fullscreen.
- Any backend changes — `/api/draft` is used as-is. A future batch endpoint for
  multiple players is possible but not required.
- The enemy team's draft — it is not shown on screen, so only the player's own
  team (15 heroes) is in scope.
