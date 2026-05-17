# ARAM Draft Overlay — Auto-Detection — Design

**Date:** 2026-05-16
**Status:** Approved design — ready for implementation planning

## Context

Storm Almanac ships an ARAM Draft Win-Rate Overlay (v0.1.41): the user presses
`Ctrl+Shift+D` during the HoTS "CHOOSE A HERO" screen and a transparent overlay
draws per-hero win rates. The original design deferred auto-detection of the
draft screen — hotkey only.

This design adds auto-detection so the overlay appears on its own when an ARAM
draft starts. ARAM drafts are short (15–30s), so detection must be quick.

## Goal

When an ARAM draft screen appears, the win-rate overlay shows automatically,
without the user pressing a hotkey. It hides when the draft screen goes away.

## Decisions (locked)

- **Watch window trigger:** the `replay.server.battlelobby` file appearing in
  the HoTS temp dir — the same signal the map blocker already uses. When it
  appears, a *draft watch window* arms for up to 2 minutes.
- **Watch window is HoTS-foreground-gated:** polling only runs while HoTS is
  the foreground window. No cost when the user is alt-tabbed away.
- **Cheap detection gate:** while armed, every **1 second**, capture the
  primary monitor and OCR only a **thin horizontal band** where the
  "CHOOSE A HERO" header appears (~10–30ms). This avoids a full-screen OCR on
  every poll.
- **Full pipeline runs once per draft:** when the cheap gate first matches, the
  existing capture → OCR → parse → win-rates pipeline runs **once**. The 15
  offered heroes are static for the draft, so there is no repeated full-screen
  OCR.
- **Dismiss debounce:** the overlay hides when the cheap gate fails to match
  for **2 consecutive polls**, guarding against momentary UI transitions.
- **No re-arm:** an ARAM draft happens once per game. After the draft is
  detected and then dismissed, the watch window closes and does not re-arm
  until the next `replay.server.battlelobby` appears.
- **Hotkey fallback retained:** `Ctrl+Shift+D` still manually runs the full
  pipeline and shows the overlay — unchanged.
- **Single tray toggle:** the existing "Enable Draft Overlay" toggle now also
  controls auto-detection. Enabling it arms auto-detection *and* keeps the
  hotkey live. No new tray item, no new config.

## Architecture

A new background component — the **draft watcher** — parallel to the map
blocker. It is armed by the existing battlelobby probe and drives the existing
draft overlay pipeline.

```
replay.server.battlelobby appears
  -> arm draft watch window (2-minute deadline)
  -> loop, every 1s, while HoTS is foreground:
       capture primary monitor
       OCR the header band only  (cheap gate)
       if "CHOOSE A HERO" matches:
         if draft not yet parsed -> run full pipeline once -> show overlay
         else -> ensure overlay shown
       else:
         count a miss; after 2 consecutive misses -> hide overlay, close window
  -> on HoTS regaining focus while armed: re-check immediately (see below)
  -> on 2-minute deadline with no draft -> close window
```

The full pipeline (`screen_capture` → `ocr` → `draft_parse` → `win_rates` →
overlay emit) is unchanged and is reused as-is. The watcher only decides *when*
to run it.

## Components

- **`draft_watcher`** (Rust, new module) — owns the watch-window state machine:
  armed/disarmed, the 2-minute deadline, the 1s poll loop, the consecutive-miss
  counter, and the "draft already parsed this window" flag. Calls the existing
  draft pipeline when the cheap gate trips.
- **Cheap header gate** — a function that captures the primary monitor, crops a
  thin horizontal band where the "CHOOSE A HERO" header is expected, OCRs just
  that band, and returns whether the header text matched. Reuses
  `screen_capture` and `ocr`; the crop keeps it ~10–30ms.
- **Battlelobby probe** (`battle_lobby_probe.rs`, existing) — gains a hook so
  that, in addition to its current map-fingerprint duties, a new
  `replay.server.battlelobby` appearing also arms the draft watcher.
- **Draft pipeline** (`draft_overlay.rs` + `screen_capture`, `ocr`,
  `draft_parse`, `win_rates`, existing) — unchanged. Invoked by the watcher.
- **Overlay UI** (`?mode=draft`, existing) — unchanged.
- **Tray / lifecycle wiring** (`lib.rs`, existing) — the "Enable Draft Overlay"
  toggle starts/stops the watcher in addition to registering the hotkey.

## Focus loss / regain

The draft overlay window closes when HoTS loses focus (existing behavior). So
alt-tabbing mid-draft removes the overlay. To make returning seamless, when
**HoTS regains focus while the watch window is armed**, the watcher re-checks
immediately instead of waiting up to 1s for the next tick:

- If the draft was **already OCR'd this window** — re-show the overlay
  directly. No re-capture, no re-OCR, no re-fetch; it is instant.
- If the draft was **not yet detected** — run the cheap gate right away, and
  the full pipeline if it matches.

When HoTS is not the foreground window, the poll loop is idle (no capture, no
OCR), so there is no cost while alt-tabbed.

## Data flow

1. `replay.server.battlelobby` appears -> draft watcher arms; 2-minute deadline
   set.
2. Every 1s while HoTS is foreground: capture primary monitor, OCR the header
   band.
3. Header matches, draft not yet parsed -> run the full pipeline once
   (capture → OCR → parse → win-rates) -> emit to the overlay -> show it.
4. Header matches, draft already parsed -> ensure the overlay is shown.
5. Header misses -> increment miss counter; at 2 consecutive misses -> hide the
   overlay and close the watch window.
6. HoTS regains focus while armed -> immediate re-check (see Focus section).
7. 2-minute deadline reached with no draft detected -> close the watch window.

## Error handling

- **Screen capture fails** (e.g. exclusive fullscreen) -> the poll is a no-op;
  log once per window, do not crash, do not spam. The hotkey path already
  surfaces a user-facing notice for this case.
- **Cheap-gate OCR returns nothing** -> treated as a miss; same debounce path.
- **Full pipeline fails after the gate matched** (OCR/parse/backend error) ->
  the existing pipeline error handling applies (render heroes with no numbers /
  brief notice); the watch window still closes normally on draft dismissal.
- **HoTS not running / battlelobby never appears** -> the watcher simply never
  arms; zero cost.
- **App started mid-draft** — the battlelobby file is likely already on disk,
  so no new arm event fires. This is an accepted v1 limitation: the user can
  press `Ctrl+Shift+D` to trigger the overlay manually. (The map blocker's
  existing "process existing files on startup" behavior may also arm the
  watcher; if so, the watch window covers it. Confirmed during implementation.)

## Testing

- **Unit:** the watch-window state machine — arming, the miss-counter debounce
  (1 miss does not dismiss, 2 consecutive do), the 2-minute deadline, and the
  "parse only once per window" flag. Driven by injected clock + injected
  gate-result inputs so it is pure and deterministic.
- **Cheap header gate:** exercised against a saved ARAM-draft screenshot (the
  fixture already used for the existing draft-parse tests) — assert the band
  crop + OCR matches the header; assert a non-draft screenshot does not.
- **Manual:** end-to-end in a live ARAM game — confirm the overlay appears
  within ~1–2s of the draft screen, survives an alt-tab out and back, and hides
  when the draft ends.

## Open investigation item

Verify live whether `replay.server.battlelobby` is written **before, as, or
after** the ARAM draft screen appears. The 2-minute window absorbs a trigger
that lands a beat early or a beat late — the next 1s poll still catches the
draft. Only if the file appears *much* later (after the match fully loads)
would a different trigger be needed; the cheap gate makes that fallback easy
(poll whenever HoTS is foreground). This is a quick live check, not a design
blocker.

## Out of scope for v1

- Re-arming the watcher for a second draft in the same game — ARAM has one
  draft per game.
- Auto-detecting a draft when the app is launched mid-draft — covered by the
  manual hotkey.
- Multi-monitor / window-tracked capture — unchanged from the original draft
  overlay design (full-screen mirror, HoTS on the primary monitor).
- Any backend changes.
