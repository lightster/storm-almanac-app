# ARAM Overlay Badge Scaling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scale ARAM overlay badge text proportionally to the displayed portrait size with a 10px floor, and re-anchor each badge so its box grows outward into the column gap instead of toward the neighbour portrait.

**Architecture:** All changes live in `src/routes/overlay/+page.svelte`. The Rust pipeline already sends `DraftOverlayHero.circle.width`/`height` in CSS pixels, descaled for both monitor DPI (live) and the test-display scale (test mode), so a per-badge inline `font-size` computed from `h.circle.width` is the only signal needed — no Rust changes, no payload changes. Badge `padding` and `border-radius` move from fixed px to em so the box rides along with the font; the badges' inline `left`/`top` and CSS `transform` change so each badge's outer edge anchors to one side of the portrait and its box grows into the column gap.

**Tech Stack:** SvelteKit 5 (Svelte 5 runes + `{@const}` inside `{#each}`), Tauri 2, Rust (test runner only — no Rust changes here).

**Branch:** All commits go on the existing `aram-badge-scaling` feature branch (created during design). Do **not** commit to `main`.

**Spec:** `docs/superpowers/specs/2026-05-23-aram-overlay-badge-scaling-design.md`

---

## File Structure

Only one file is edited:

- **Modify:** `src/routes/overlay/+page.svelte`
  - `<script>` block: add two module-scope `const`s for the scaling constant and floor.
  - `{:else if mode === 'draft'}` template branch: add `{@const}` for per-iteration font size; change inline `left`/`top` math on both badge `<div>`s; add inline `font-size`.
  - `<style>` block: convert `.wr-badge` `padding`/`border-radius` from px to em; remove fixed `font-size`; change `transform` on `.wr-badge.overall` and `.wr-badge.personal`.

Nothing else changes. The 47 cargo tests in `src-tauri/` are untouched.

---

## Task 1: Convert badge padding and border-radius from px to em

This task introduces no visual change at the current 14px font. It's a no-op refactor that lets the box scale with the font in later tasks, so it can be verified by "looks identical to before."

**Files:**
- Modify: `src/routes/overlay/+page.svelte` (CSS block, `.wr-badge` rule around lines 486–499)

- [ ] **Step 1: Locate the `.wr-badge` rule**

Open `src/routes/overlay/+page.svelte` and find the rule:

```css
.wr-badge {
    position: absolute;
    font-family: 'DM Sans', system-ui, sans-serif;
    font-size: 14px;
    font-weight: 700;
    line-height: 1;
    padding: 3px 7px;
    border-radius: 6px;
    background: rgba(8, 10, 20, 0.82);
    border: 1px solid rgba(255, 255, 255, 0.16);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.55);
    pointer-events: none;
    white-space: nowrap;
}
```

- [ ] **Step 2: Convert `padding` and `border-radius` to em**

Change exactly two lines:

```css
    padding: 0.22em 0.5em;
    border-radius: 0.43em;
```

(Math: at 14px font, `0.22em ≈ 3px`, `0.5em = 7px`, `0.43em ≈ 6px`. Same look, em-relative.)

Leave `font-size: 14px;` alone for now — task 3 removes it.

- [ ] **Step 3: Visual sanity check via test mode**

From the repo root, in PowerShell:

```powershell
npm run tauri:dev
```

Once the app launches: tray icon → right-click → **Show Test Draft**. The first fixture (`01-lost-cavern.png`) opens with badges overlaid. Click the PNG once to cycle to the second fixture (`02-lost-cavern-small.png`); click again to close.

Expected: badges look **identical** to before this task — same size, same position, same padding. If anything looks visually different, the em math is off.

- [ ] **Step 4: Commit**

```powershell
git add src/routes/overlay/+page.svelte
git commit -m "Convert overlay badge padding and border-radius to em units

Replace fixed-px padding and border-radius on .wr-badge with em
values that reproduce the same proportions at the current 14px
font. No visual change at present, but a later step introduces
per-portrait font scaling and the em-based box needs to ride
along with it automatically."
```

---

## Task 2: Re-anchor badges so each box grows outward into the column gap

Each badge is currently centered on a point inside the portrait, so widening the text spills equally into the portrait AND into the neighbour portrait. This task moves the anchor to one outer corner per badge and removes the centering translate, so width grows only outward — into the larger column gap, not into a neighbour's portrait.

**Files:**
- Modify: `src/routes/overlay/+page.svelte` (template `{:else if mode === 'draft'}` branch around lines 262–278, CSS block around lines 500–505)

- [ ] **Step 1: Update overall-badge inline position**

In the `{:else if mode === 'draft'}` branch, locate:

```svelte
<div
    class="wr-badge overall {wrClass(h.win_rates.overall)}"
    style="left: {h.circle.x + h.circle.width * 0.15}px; top: {h.circle.y + h.circle.height * 0.15}px;"
>
    {fmt(h.win_rates.overall)}
</div>
```

Change the `style` attribute to:

```svelte
    style="left: {h.circle.x}px; top: {h.circle.y}px;"
```

- [ ] **Step 2: Update personal-badge inline position**

Locate (just below):

```svelte
<div
    class="wr-badge personal {wrClass(h.win_rates.player)}"
    style="left: {h.circle.x + h.circle.width * 0.85}px; top: {h.circle.y + h.circle.height * 0.15}px;"
>
```

Change the `style` attribute to:

```svelte
    style="left: {h.circle.x + h.circle.width}px; top: {h.circle.y}px;"
```

- [ ] **Step 3: Update the two `transform` CSS rules**

In the `<style>` block, locate:

```css
.wr-badge.overall {
    transform: translate(-50%, -50%);
}
.wr-badge.personal {
    transform: translate(-50%, -50%);
}
```

Replace with:

```css
.wr-badge.overall {
    transform: translate(-100%, 0);
}
.wr-badge.personal {
    transform: none;
}
```

(Overall: right edge of the badge box now sits at the portrait's left edge; box grows leftward. Personal: left edge sits at the portrait's right edge; box grows rightward. Vertical: top edge of each badge aligns with `circle.y`, the top of the portrait bounding box.)

- [ ] **Step 4: Visual verification**

```powershell
npm run tauri:dev
```

Tray → Show Test Draft. Cycle through both fixtures.

Expected:
- Each portrait has its overall badge sitting **to the left** of the portrait (right edge touching the left side of the portrait circle), and its personal badge sitting **to the right** of the portrait (left edge touching the right side of the portrait circle).
- Both badges align with the top of the portrait.
- Long player names extend further into the column gap, **not** toward the neighbouring portrait.
- Font is still 14px (this task did not change sizing).

If badges are visibly clipped or vertically misaligned, recheck the inline `left`/`top` math.

- [ ] **Step 5: Commit**

```powershell
git add src/routes/overlay/+page.svelte
git commit -m "Anchor draft overlay badges outward into the column gap

Each win-rate badge used to center on a point 15% in from one
side of the portrait, so a wider box spilled equally toward the
portrait and toward the neighbour portrait. Re-anchor the overall
badge so its right edge sits at the portrait's left side and
grows leftward into the column gap, and the personal badge so
its left edge sits at the portrait's right side and grows
rightward. Width now grows only into the gap, never over a
neighbour."
```

---

## Task 3: Scale badge font-size with portrait diameter, floored at 10px

With anchoring fixed, introduce the actual proportional sizing. Badge font is computed inline per-badge from `h.circle.width` (which the Rust pipeline already provides in CSS pixels, descaled for both monitor DPI and test-mode display scale), with a 10px floor for legibility.

**Files:**
- Modify: `src/routes/overlay/+page.svelte` (`<script>` block around lines 7–20, template `{:else if mode === 'draft'}` branch around lines 262–278, CSS block `.wr-badge` rule)

- [ ] **Step 1: Add tuning constants near the top of `<script>`**

In the `<script>` block, just after the imports (before the `let mode = $state('interactive')` line), add:

```svelte
	const BADGE_FONT_K = 0.16;
	const BADGE_FONT_FLOOR_PX = 10;
```

(`0.16` is a starting value. Step 4 below verifies the look and adjusts if needed.)

- [ ] **Step 2: Compute per-iteration font size and apply it inline**

In the `{:else if mode === 'draft'}` branch, the current `{#each}` looks like:

```svelte
{#each draftHeroes as h (h.hero + h.rect.x + h.rect.y)}
    <div
        class="wr-badge overall {wrClass(h.win_rates.overall)}"
        style="left: {h.circle.x}px; top: {h.circle.y}px;"
    >
        {fmt(h.win_rates.overall)}
    </div>
    {#if h.win_rates.player !== null}
        <div
            class="wr-badge personal {wrClass(h.win_rates.player)}"
            style="left: {h.circle.x + h.circle.width}px; top: {h.circle.y}px;"
        >
            {h.is_self ? 'you' : h.player_name}&nbsp;{fmt(h.win_rates.player)}
        </div>
    {/if}
{/each}
```

Replace it with:

```svelte
{#each draftHeroes as h (h.hero + h.rect.x + h.rect.y)}
    {@const fontSize = Math.max(BADGE_FONT_FLOOR_PX, h.circle.width * BADGE_FONT_K)}
    <div
        class="wr-badge overall {wrClass(h.win_rates.overall)}"
        style="left: {h.circle.x}px; top: {h.circle.y}px; font-size: {fontSize}px;"
    >
        {fmt(h.win_rates.overall)}
    </div>
    {#if h.win_rates.player !== null}
        <div
            class="wr-badge personal {wrClass(h.win_rates.player)}"
            style="left: {h.circle.x + h.circle.width}px; top: {h.circle.y}px; font-size: {fontSize}px;"
        >
            {h.is_self ? 'you' : h.player_name}&nbsp;{fmt(h.win_rates.player)}
        </div>
    {/if}
{/each}
```

(The only diffs are the added `{@const fontSize = ...}` line and `font-size: {fontSize}px;` appended to each badge's `style`.)

- [ ] **Step 3: Remove the fixed `font-size` from the `.wr-badge` CSS rule**

In the `<style>` block, locate the `.wr-badge` rule and delete the line:

```css
    font-size: 14px;
```

The remaining rule should look like:

```css
.wr-badge {
    position: absolute;
    font-family: 'DM Sans', system-ui, sans-serif;
    font-weight: 700;
    line-height: 1;
    padding: 0.22em 0.5em;
    border-radius: 0.43em;
    background: rgba(8, 10, 20, 0.82);
    border: 1px solid rgba(255, 255, 255, 0.16);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.55);
    pointer-events: none;
    white-space: nowrap;
}
```

- [ ] **Step 4: Verify scaling looks right on the 4K fixture, then on the small fixture**

```powershell
npm run tauri:dev
```

Tray → Show Test Draft. The first fixture (`01-lost-cavern.png`, 3840×2160) renders at the game's native resolution on whatever monitor the dev runs on.

Expected on 4K fixture:
- Badge text size looks roughly the same as the pre-change 14px look (within a few px). The em-relative padding scales along with it so the box proportions stay clean.
- No neighbour-portrait overlap even on long player names.

If the badges look noticeably too large or too small on the 4K fixture:
- Open the browser devtools on the overlay window (right-click the overlay if it's interactable, or temporarily change `mode === 'draft'` to `'interactive'` in the URL for inspection — and revert before committing). Inspect a rendered badge's computed `font-size`. The displayed `h.circle.width` is roughly that value divided by `BADGE_FONT_K` (= `0.16`).
- A faster path: temporarily add `console.log('circle.width', h.circle.width)` inside the `{@const}` (or just inside the `{#each}` body), reload, read the logged width, set `BADGE_FONT_K = 14 / <logged width>` rounded to two decimals. Remove the `console.log` before committing.

Click to cycle to `02-lost-cavern-small.png` (1999×1124).

Expected on small fixture:
- Badges are visibly smaller than on the 4K fixture but stay legible — the 10px floor stops them shrinking past readability.
- No neighbour-portrait overlap.

- [ ] **Step 5: Commit**

```powershell
git add src/routes/overlay/+page.svelte
git commit -m "Scale overlay badge font-size with portrait diameter

The badge text was a fixed 14px regardless of portrait size, so
it looked oversized on smaller displayed portraits (low-DPI
monitors and downscaled test fixtures). Compute font-size inline
per badge from h.circle.width (already in CSS pixels, already
adjusted for both monitor DPI and the test-mode display scale)
with a 10px floor for legibility on heavily downscaled fixtures.
The em-relative padding and border-radius added earlier scale
along with the font automatically."
```

---

## Task 4: Final verification

Confirm nothing on the Rust side broke, and that the end-to-end look matches what the spec calls for.

**Files:** none modified.

- [ ] **Step 1: Run cargo tests**

Use a PowerShell subshell so the working directory is not mutated (per the user's "never bare cd" rule):

```powershell
& { Set-Location src-tauri; cargo test }
```

Or, equivalently, with `--manifest-path` from the repo root:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected output (last summary line): `test result: ok. 47 passed; 0 failed`. If a test fails or the count differs from 47, stop and investigate — this change should not touch Rust at all.

- [ ] **Step 2: Final visual pass**

```powershell
npm run tauri:dev
```

Tray → Show Test Draft. Run through both fixtures once more.

Acceptance criteria from the spec:
- Badge sizes look proportional to the displayed portrait sizes on both fixtures.
- No badge spills onto a neighbour portrait, even with long player names.
- The 10px floor keeps small-fixture badges legible.

- [ ] **Step 3: Confirm git state is clean**

```powershell
git status
git log --oneline main..HEAD
```

Expected: working tree clean. `git log` shows three new commits on `aram-badge-scaling` since `main` (one per implementation task; the design-doc commit is the fourth, oldest).
