# ARAM Overlay Badge Scaling — Design

**Date:** 2026-05-23
**Status:** Approved design — ready for implementation planning
**Scope:** `storm-almanac-app` only (Tauri 2 + Rust + SvelteKit, Windows)

## Context

The ARAM draft overlay renders two win-rate badges per portrait: an
overall badge (top-left of the portrait) and an optional personal
badge (top-right). Both use a fixed 14px CSS font and are positioned
in `src/routes/overlay/+page.svelte` (`mode === 'draft'` branch). The
badge boxes are anchored at points 15%/85% across the portrait's
bounding box and centered on those points via
`transform: translate(-50%, -50%)`.

Two related problems with this:

1. **Fixed font, variable portrait size.** Badge text stays 14px
   regardless of how large the displayed portrait actually is.
   At native game resolution that reads as a reasonable badge.
   On a lower-DPI monitor — or, equivalently, when test mode displays
   a downscaled fixture — the portraits are smaller in CSS pixels but
   the badge text stays 14px, so the badge dominates the portrait
   visually and occludes hero art.
2. **Center-anchored boxes grow outward.** Because both badges are
   centered on their anchor points, a wider badge (longer player name,
   larger font) extends equally toward and away from the portrait
   center. The half that extends away spills *into the adjacent
   portrait's space*, not into empty area. Bigger font + longer
   name = neighbor-portrait collisions.

The `circle.width` and `circle.height` fields the Rust pipeline
already sends in `DraftOverlayHero` are exactly what we need to fix
sizing: they're the portrait's estimated diameter in CSS pixels,
descaled for monitor DPI in production and for the test-display scale
in test mode. So "displayed portrait diameter" is already on the
wire — we just have not been using it.

## Goal

- Badge font size scales proportionally to the displayed portrait
  size, so badges visually occupy roughly the same fraction of the
  portrait at any DPI or test-fixture scale.
- Badges have a minimum legible font size on heavily-downscaled
  fixtures so they remain readable.
- Badge boxes grow into the column gap rather than toward neighbor
  portraits, since the gap between draft columns has more room than
  the portrait itself.

Out of scope: vertical badge layout changes beyond what falls out of
the anchoring change; any Rust-side payload changes; any new tests
infrastructure.

## Decisions (locked)

- **Scale source: payload `circle.width`.** The overlay reads
  `h.circle.width` (already in CSS pixels, already DPI/test-scale
  adjusted) and computes `font-size` inline. No new payload fields,
  no extra plumbing, no Rust changes.
- **Font only, em-based padding.** Only the inline `font-size`
  changes per-badge. The badge's `padding` and `border-radius` move
  from fixed px values to `em` so the box rides along with the font
  automatically. Font-weight and the 1px border stay fixed — at the
  10px floor a sub-pixel border would alias worse than a slightly
  proportionally thicker one.
- **Floor at 10px, no max cap.** `font-size = max(10px, circle.width *
  k)`. Below ~10px the digits become hard to read at arm's length.
  No upper cap — native game resolution is the largest realistic
  portrait we see, and a cap would just be unused complexity.
- **Tuning constant `k` lives as a `const` in the Svelte file.**
  Initial value back-calculated during implementation by measuring
  `circle.width` on a native-resolution-equivalent fixture and
  setting `k = 14 / measured_width` so today's look is preserved at
  native res. Likely lands near `0.16`, but the implementation step
  will measure and tune. No runtime configuration.
- **Anchor outward into the column gap.** Overall badge: right edge
  anchored at the portrait's left side (`circle.x`); box grows
  leftward into the gap. Personal badge: left edge anchored at the
  portrait's right side (`circle.x + circle.width`); box grows
  rightward into the gap. Both badges sit at the top of the portrait
  bounding box (`circle.y`). This change makes "badge box bigger
  than gap" the only collision risk and removes neighbor-portrait
  overlap.
- **No automated tests added.** The change is purely presentational
  Svelte/CSS. The 47 cargo tests are unaffected and must continue to
  pass. Manual verification through the existing test-mode harness
  (4K + 1999×1124 fixtures) is the acceptance signal.

## Implementation outline

All changes in `src/routes/overlay/+page.svelte`.

**Sizing.** In the `<script>` block, declare a single constant near
the top:

```svelte
const BADGE_FONT_K = /* tuned during implementation, ~0.16 */;
const BADGE_FONT_FLOOR_PX = 10;
```

In the `mode === 'draft'` template branch, compute the per-badge
font size with `{@const}` inside each `{#each}` iteration:

```svelte
{@const fontSize = Math.max(BADGE_FONT_FLOOR_PX, h.circle.width * BADGE_FONT_K)}
```

Apply `font-size: {fontSize}px;` in each badge's inline `style`
attribute, alongside the existing `left`/`top` math.

**Anchoring.** Replace the existing inline positions and `transform`
rules:

- Overall badge inline: `left: {h.circle.x}px; top: {h.circle.y}px;
  font-size: {fontSize}px;`
- Overall badge CSS: `transform: translate(-100%, 0);` (replaces
  `translate(-50%, -50%)`)
- Personal badge inline: `left: {h.circle.x + h.circle.width}px;
  top: {h.circle.y}px; font-size: {fontSize}px;`
- Personal badge CSS: `transform: none;` (replaces
  `translate(-50%, -50%)`)

**Padding/radius.** In the `.wr-badge` rule, convert:

- `padding: 3px 7px` → `padding: 0.22em 0.5em` (reproduces the
  3px/7px proportions at 14px font; values may be nudged visually
  if they look off at the extremes).
- `border-radius: 6px` → `border-radius: 0.43em` (reproduces 6px
  at 14px font).

`line-height: 1` stays as-is.

**End-column edge case.** The leftmost column's overall badge will
extend left of its portrait; the rightmost column's personal badge
will extend right. In practice the draft is centered on screen with
substantial side margin, so neither clips the screen. No code
defense needed; verification is by visual inspection of test
fixtures.

## Verification

Manual, via the existing test-mode harness:

- `npm run tauri:dev`
- Tray → Show Test Draft
- Inspect badges over the 4K `01-lost-cavern.png` fixture: badge
  sizes look proportional to the (native-equivalent) portrait sizes;
  no neighbor-portrait overlap.
- Click to advance to `02-lost-cavern-small.png` (1999×1124):
  badges scale down proportionally; floor keeps them legible at
  ~10px; no neighbor-portrait overlap.

Automated: `cargo test` from `src-tauri/` must still report 47
passing tests, unchanged. No new tests added — there is no Svelte
test infrastructure in the repo today, and standing one up is
out of scope for this change.

## Non-goals

- Settings UI for badge scaling. Once `k` is tuned the value is
  static; user configurability is not warranted.
- Restructuring badge content (e.g. truncating long player names,
  splitting personal-badge into label + percent). The anchoring
  change makes long names safe to display in full because the box
  grows into the gap; truncation can wait until we see a real case
  where the gap is too narrow.
- Vertical positioning beyond aligning to the top of the portrait
  bounding box. If visual review shows the badges sit too high or
  too low, an em-based vertical offset will be added during
  implementation; it is not a separate design decision.
- Any Rust-side change. `circle.width` already carries the signal
  we need.
