# ARAM Draft Win-Rate Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a hotkey-triggered transparent overlay that OCRs the HoTS ARAM hero-select screen and draws each offered hero's overall + personal ARAM win rate anchored next to it.

**Architecture:** A hotkey press runs a one-shot Rust pipeline — capture the primary monitor, OCR it with `Windows.Media.Ocr`, parse 5 player columns × 3 heroes, fuzzy-match hero names, fetch per-player ARAM win rates from the existing public `GET /api/draft` endpoint, then emit the merged data to a full-screen transparent click-through overlay window that draws labels at the OCR bounding boxes.

**Tech Stack:** Rust + Tauri 2, the `windows` crate (BitBlt capture + WinRT OCR), `reqwest` (HTTP), SvelteKit (overlay UI). Windows-only feature.

**Spec:** `docs/superpowers/specs/2026-05-16-aram-draft-overlay-design.md`

---

## File structure

New Rust modules under `src-tauri/src/`:

- `draft_types.rs` — shared plain data types (`Rect`, `OcrLine`, `Draft`, `DraftPlayer`, `DraftHero`, `HeroWinRates`, `DraftOverlayHero`, `DraftOverlayPayload`). No logic, so every other module can depend on it without cycles.
- `draft_parse.rs` — **pure logic**: `match_hero()` (fuzzy hero-name matching) and `parse_draft()` (OCR lines → `Draft`). Fully unit-tested.
- `win_rates.rs` — `/api/draft` HTTP client, JSON parsing, player→battletag resolution, per-session cache.
- `screen_capture.rs` — primary-monitor capture via GDI BitBlt → `image::RgbaImage`.
- `ocr.rs` — `image::RgbaImage` → `Vec<OcrLine>` via `Windows.Media.Ocr`.
- `draft_overlay.rs` — orchestration: runs the pipeline, manages the overlay window, holds enabled-state, hotkey handler.

Modified:

- `src-tauri/Cargo.toml` — add `windows` crate features.
- `src-tauri/src/lib.rs` — declare modules, tray item, hotkey registration, wire `draft_overlay`.
- `src-tauri/src/config.rs` — add `player_battletag` config field.
- `src-tauri/capabilities/overlay.json` — add the new window label.
- `src/routes/overlay/+page.svelte` — add `?mode=draft` rendering.

Heroes/win rates come from `GET {API_URL}/api/draft` (defined at `lib.rs:75`). The `heroes` array in that response is the canonical hero list (fields `hero`, `search_aliases`, `slug`) used for fuzzy-matching.

---

## Task 1: Add windows-crate features and module declarations

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs:1-9`

- [ ] **Step 1: Add windows features to Cargo.toml**

Find the `windows` dependency in `src-tauri/Cargo.toml` (currently `windows = "0.59"` or `windows = { version = "0.59" }`). Replace it with the explicit feature set the capture and OCR modules need:

```toml
windows = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_UI_WindowsAndMessaging",
    "Foundation",
    "Globalization",
    "Graphics_Imaging",
    "Media_Ocr",
    "Storage_Streams",
] }
```

- [ ] **Step 2: Declare the new modules**

In `src-tauri/src/lib.rs`, the module declarations block is lines 1-9. Add the six new modules, keeping alphabetical order:

```rust
mod autostart;
mod battle_lobby_probe;
mod config;
mod draft_overlay;
mod draft_parse;
mod draft_types;
mod game_focus;
mod game_session;
mod input_recorder;
mod ocr;
mod screen_capture;
mod state;
mod uploader;
mod watcher;
mod win_rates;
```

- [ ] **Step 3: Create empty module files so the project compiles**

Create these files each containing only a doc comment, so `cargo check` passes until later tasks fill them in:

`src-tauri/src/draft_types.rs`:
```rust
//! Shared plain data types for the ARAM draft overlay pipeline.
```

`src-tauri/src/draft_parse.rs`:
```rust
//! Pure parsing logic: OCR lines -> structured draft.
```

`src-tauri/src/win_rates.rs`:
```rust
//! Win-rate API client for the ARAM draft overlay.
```

`src-tauri/src/screen_capture.rs`:
```rust
//! Primary-monitor screen capture (Windows GDI).
```

`src-tauri/src/ocr.rs`:
```rust
//! OCR via Windows.Media.Ocr.
```

`src-tauri/src/draft_overlay.rs`:
```rust
//! ARAM draft overlay orchestration: pipeline, window, hotkey.
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: PASS (warnings about unused modules are fine).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/lib.rs src-tauri/src/draft_types.rs src-tauri/src/draft_parse.rs src-tauri/src/win_rates.rs src-tauri/src/screen_capture.rs src-tauri/src/ocr.rs src-tauri/src/draft_overlay.rs
git commit -m "Scaffold ARAM draft overlay modules"
```

---

## Task 2: Shared draft types

**Files:**
- Modify: `src-tauri/src/draft_types.rs`

- [ ] **Step 1: Write the types**

Replace the contents of `src-tauri/src/draft_types.rs` with:

```rust
//! Shared plain data types for the ARAM draft overlay pipeline.

use serde::Serialize;

/// A rectangle in primary-monitor screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// Horizontal centre of the rect.
    pub fn center_x(&self) -> f64 {
        self.x + self.width / 2.0
    }
    /// Vertical centre of the rect.
    pub fn center_y(&self) -> f64 {
        self.y + self.height / 2.0
    }
}

/// One line of recognized text with its on-screen bounding box.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub rect: Rect,
}

/// A hero offered to a player, with the bounding box of its name label.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftHero {
    /// Canonical hero name (e.g. "Li-Ming").
    pub hero: String,
    pub rect: Rect,
}

/// One player column in the draft: a name and (up to) 3 hero choices.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftPlayer {
    /// Player name as OCR'd from the screen (no battletag discriminator).
    pub name: String,
    pub name_rect: Rect,
    pub heroes: Vec<DraftHero>,
}

/// The parsed ARAM draft screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Draft {
    pub players: Vec<DraftPlayer>,
}

/// Win rates for a single hero (overall ARAM + a specific player's).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct HeroWinRates {
    pub overall: Option<f64>,
    pub player: Option<f64>,
    pub player_games: Option<u32>,
}

/// A hero ready to draw: its box plus its win rates.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftOverlayHero {
    pub hero: String,
    pub rect: Rect,
    pub win_rates: HeroWinRates,
}

/// The full payload emitted to the overlay window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftOverlayPayload {
    pub heroes: Vec<DraftOverlayHero>,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/draft_types.rs
git commit -m "Add shared types for the draft overlay pipeline"
```

---

## Task 3: Hero fuzzy-matching

`match_hero` maps an OCR'd hero label (possibly with typos) to a canonical hero name from the backend hero list. OCR is reliable for clean game text, so light fuzziness suffices: case-insensitive exact match, then case-insensitive match with non-alphanumerics stripped (handles "Li-Ming" vs "LiMing", "D.Va" vs "DVa"), then a small Levenshtein tolerance for single-character OCR errors.

**Files:**
- Modify: `src-tauri/src/draft_parse.rs`
- Test: same file, `#[cfg(test)]` module (matches the repo pattern in `battle_lobby_probe.rs`).

- [ ] **Step 1: Write the failing tests**

Put this in `src-tauri/src/draft_parse.rs`:

```rust
//! Pure parsing logic: OCR lines -> structured draft.

/// Normalize a name for comparison: lowercase, keep only ASCII alphanumerics.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Levenshtein edit distance between two byte-equal-length-agnostic strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Match an OCR'd hero label against the canonical hero list.
/// Returns the canonical hero name, or `None` if nothing matches well enough.
pub fn match_hero(ocr_text: &str, hero_list: &[String]) -> Option<String> {
    let target = normalize(ocr_text);
    if target.is_empty() {
        return None;
    }
    // Exact normalized match.
    if let Some(h) = hero_list.iter().find(|h| normalize(h) == target) {
        return Some(h.clone());
    }
    // Closest within an edit distance of 1 (single OCR error).
    let mut best: Option<(usize, &String)> = None;
    for h in hero_list {
        let d = levenshtein(&normalize(h), &target);
        if d <= 1 && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, h));
        }
    }
    best.map(|(_, h)| h.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heroes() -> Vec<String> {
        ["Nazeebo", "Li-Ming", "The Butcher", "D.Va", "Genji", "Nova"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn exact_match() {
        assert_eq!(match_hero("Genji", &heroes()), Some("Genji".to_string()));
    }

    #[test]
    fn punctuation_insensitive() {
        assert_eq!(match_hero("LiMing", &heroes()), Some("Li-Ming".to_string()));
        assert_eq!(match_hero("DVa", &heroes()), Some("D.Va".to_string()));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(match_hero("NAZEEBO", &heroes()), Some("Nazeebo".to_string()));
    }

    #[test]
    fn single_char_ocr_error() {
        // OCR misread 'b' as 'h'.
        assert_eq!(match_hero("Nazeeho", &heroes()), Some("Nazeebo".to_string()));
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(match_hero("Xyzzy", &heroes()), None);
        assert_eq!(match_hero("", &heroes()), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml draft_parse::tests::`
Expected: PASS (all 5). The implementation is included above; if any test fails, fix the implementation, not the test.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/draft_parse.rs
git commit -m "Add hero fuzzy-matching for the draft overlay"
```

---

## Task 4: Draft parsing (column grouping)

`parse_draft` turns `Vec<OcrLine>` into a `Draft`. The ARAM select screen has 5 player columns; each column has a player-name line near the top and 3 hero-name lines below it. Algorithm:

1. Keep only lines whose text `match_hero`es a hero — these are hero labels.
2. Cluster hero labels into columns by `center_x()` proximity (gap-based clustering).
3. For each column, the player name is the non-hero line directly above the topmost hero whose `center_x()` is nearest the column.
4. Sort columns left-to-right; within a column sort heroes top-to-bottom.

**Files:**
- Modify: `src-tauri/src/draft_parse.rs`
- Test: same file's `#[cfg(test)]` module.

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `src-tauri/src/draft_parse.rs`:

```rust
    use crate::draft_types::{OcrLine, Rect};

    fn line(text: &str, x: f64, y: f64) -> OcrLine {
        OcrLine {
            text: text.to_string(),
            rect: Rect { x, y, width: 80.0, height: 18.0 },
        }
    }

    #[test]
    fn parses_two_columns_with_names_and_heroes() {
        // Two player columns; each: a name line, then 3 hero lines below it.
        let lines = vec![
            line("Togo99", 100.0, 200.0),
            line("Nazeebo", 100.0, 260.0),
            line("Genji", 100.0, 320.0),
            line("Nova", 100.0, 380.0),
            line("Serverus", 400.0, 200.0),
            line("Li-Ming", 400.0, 260.0),
            line("D.Va", 400.0, 320.0),
            line("The Butcher", 400.0, 380.0),
            line("CHOOSE A HERO", 250.0, 50.0), // noise: not a hero, not in a column
        ];
        let draft = parse_draft(&lines, &heroes());
        assert_eq!(draft.players.len(), 2);
        assert_eq!(draft.players[0].name, "Togo99");
        assert_eq!(draft.players[0].heroes.len(), 3);
        assert_eq!(draft.players[0].heroes[0].hero, "Nazeebo");
        assert_eq!(draft.players[1].name, "Serverus");
        assert_eq!(draft.players[1].heroes[2].hero, "The Butcher");
    }

    #[test]
    fn columns_sorted_left_to_right() {
        let lines = vec![
            line("Right", 500.0, 200.0),
            line("Genji", 500.0, 260.0),
            line("Left", 100.0, 200.0),
            line("Nova", 100.0, 260.0),
        ];
        let draft = parse_draft(&lines, &heroes());
        assert_eq!(draft.players[0].name, "Left");
        assert_eq!(draft.players[1].name, "Right");
    }

    #[test]
    fn empty_input_yields_no_players() {
        assert_eq!(parse_draft(&[], &heroes()).players.len(), 0);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml draft_parse::tests::parses`
Expected: FAIL — `parse_draft` not found.

- [ ] **Step 3: Implement `parse_draft`**

Add to `src-tauri/src/draft_parse.rs` (before the `#[cfg(test)]` block), and add `use crate::draft_types::{Draft, DraftHero, DraftPlayer, OcrLine};` at the top of the file:

```rust
/// Maximum horizontal gap (px) between hero labels considered the same column.
const COLUMN_GAP: f64 = 140.0;

/// Parse OCR lines into a structured draft.
pub fn parse_draft(lines: &[OcrLine], hero_list: &[String]) -> Draft {
    // 1. Identify hero labels.
    struct HeroLabel<'a> {
        hero: String,
        line: &'a OcrLine,
    }
    let mut hero_labels: Vec<HeroLabel> = lines
        .iter()
        .filter_map(|l| match_hero(&l.text, hero_list).map(|hero| HeroLabel { hero, line: l }))
        .collect();
    if hero_labels.is_empty() {
        return Draft { players: vec![] };
    }

    // 2. Cluster hero labels into columns by center_x proximity.
    hero_labels.sort_by(|a, b| {
        a.line.rect.center_x().partial_cmp(&b.line.rect.center_x()).unwrap()
    });
    let mut columns: Vec<Vec<&HeroLabel>> = vec![vec![&hero_labels[0]]];
    for label in &hero_labels[1..] {
        let last_col = columns.last_mut().unwrap();
        let last_x = last_col.last().unwrap().line.rect.center_x();
        if (label.line.rect.center_x() - last_x).abs() <= COLUMN_GAP {
            last_col.push(label);
        } else {
            columns.push(vec![label]);
        }
    }

    // 3. Build a player per column.
    let mut players: Vec<DraftPlayer> = Vec::new();
    for col in columns {
        let col_x = col.iter().map(|l| l.line.rect.center_x()).sum::<f64>() / col.len() as f64;
        let top_hero_y = col
            .iter()
            .map(|l| l.line.rect.y)
            .fold(f64::INFINITY, f64::min);

        // Player name: the non-hero line nearest this column's x, above the heroes.
        let name_line = lines
            .iter()
            .filter(|l| match_hero(&l.text, hero_list).is_none())
            .filter(|l| l.rect.y < top_hero_y)
            .filter(|l| (l.rect.center_x() - col_x).abs() <= COLUMN_GAP)
            .min_by(|a, b| {
                (top_hero_y - a.rect.y)
                    .partial_cmp(&(top_hero_y - b.rect.y))
                    .unwrap()
            });

        let (name, name_rect) = match name_line {
            Some(l) => (l.text.clone(), l.rect),
            None => continue, // no name found near this column -> skip it
        };

        let mut heroes: Vec<DraftHero> = col
            .iter()
            .map(|l| DraftHero { hero: l.hero.clone(), rect: l.line.rect })
            .collect();
        heroes.sort_by(|a, b| a.rect.y.partial_cmp(&b.rect.y).unwrap());

        players.push(DraftPlayer { name, name_rect, heroes });
    }

    players.sort_by(|a, b| a.name_rect.center_x().partial_cmp(&b.name_rect.center_x()).unwrap());
    Draft { players }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml draft_parse::tests::`
Expected: PASS (all 8 — the 5 from Task 3 plus 3 here).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/draft_parse.rs
git commit -m "Add ARAM draft column-parsing logic"
```

---

## Task 5: Win-rate API client

`win_rates` fetches `GET {API_URL}/api/draft?player=<battletag>` and exposes win rates per hero. The response has `heroes` (canonical hero list), `globalStats` (overall ARAM win rate per hero), and `playerStats` (the player's per-hero ARAM win rate + game count). Player resolution:

- The **user's own** name resolves via the configured battletag (`AppConfig.player_battletag`, added in Task 6). If an OCR'd name's part before `#` case-insensitively equals the configured battletag's name-part, use the configured battletag directly.
- **Other** players: `GET {API_URL}/api/players/search?q=<name>` returns `[{ "battletag": "Name#1234" }, ...]`. Use it only if exactly one result's name-part case-insensitively equals the OCR'd name; otherwise that player gets no personal win rate.

**Files:**
- Modify: `src-tauri/src/win_rates.rs`
- Test: same file's `#[cfg(test)]` module (JSON-parsing logic is pure and tested; HTTP is exercised manually in Task 10).

- [ ] **Step 1: Write the failing tests for the pure parsing logic**

Put this in `src-tauri/src/win_rates.rs`:

```rust
//! Win-rate API client for the ARAM draft overlay.

use serde::Deserialize;
use std::collections::HashMap;

/// Raw shape of `GET /api/draft`.
#[derive(Debug, Deserialize)]
pub struct DraftApiResponse {
    pub heroes: Vec<HeroEntry>,
    #[serde(rename = "globalStats")]
    pub global_stats: Vec<GlobalStat>,
    #[serde(rename = "playerStats")]
    pub player_stats: Vec<PlayerStat>,
}

#[derive(Debug, Deserialize)]
pub struct HeroEntry {
    pub hero: String,
}

#[derive(Debug, Deserialize)]
pub struct GlobalStat {
    pub hero: String,
    pub global_win_rate: f64,
}

#[derive(Debug, Deserialize)]
pub struct PlayerStat {
    pub hero: String,
    pub win_rate: f64,
    pub games: u32,
}

/// Indexed win rates derived from a `DraftApiResponse`.
pub struct WinRateTable {
    pub hero_names: Vec<String>,
    pub overall: HashMap<String, f64>,
    pub player: HashMap<String, (f64, u32)>,
}

/// Build lookup tables from a raw API response.
pub fn build_table(resp: &DraftApiResponse) -> WinRateTable {
    WinRateTable {
        hero_names: resp.heroes.iter().map(|h| h.hero.clone()).collect(),
        overall: resp
            .global_stats
            .iter()
            .map(|g| (g.hero.clone(), g.global_win_rate))
            .collect(),
        player: resp
            .player_stats
            .iter()
            .map(|p| (p.hero.clone(), (p.win_rate, p.games)))
            .collect(),
    }
}

/// The name part of a battletag, before the `#` discriminator.
pub fn battletag_name_part(battletag: &str) -> &str {
    battletag.split('#').next().unwrap_or(battletag)
}

/// From a `/api/players/search` result list, pick the unique battletag whose
/// name-part exactly (case-insensitively) equals `ocr_name`. `None` if zero or
/// more than one matches.
pub fn resolve_unique(ocr_name: &str, candidates: &[String]) -> Option<String> {
    let matches: Vec<&String> = candidates
        .iter()
        .filter(|bt| battletag_name_part(bt).eq_ignore_ascii_case(ocr_name))
        .collect();
    if matches.len() == 1 {
        Some(matches[0].clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_indexes_response() {
        let json = r#"{
            "heroes": [{"hero": "Nazeebo"}, {"hero": "Genji"}],
            "globalStats": [{"hero": "Nazeebo", "global_win_rate": 52.3}],
            "playerStats": [{"hero": "Nazeebo", "win_rate": 61.0, "games": 12}]
        }"#;
        let resp: DraftApiResponse = serde_json::from_str(json).unwrap();
        let table = build_table(&resp);
        assert_eq!(table.hero_names, vec!["Nazeebo", "Genji"]);
        assert_eq!(table.overall.get("Nazeebo"), Some(&52.3));
        assert_eq!(table.player.get("Nazeebo"), Some(&(61.0, 12)));
        assert_eq!(table.player.get("Genji"), None);
    }

    #[test]
    fn battletag_name_part_splits_on_hash() {
        assert_eq!(battletag_name_part("Togo99#1234"), "Togo99");
        assert_eq!(battletag_name_part("NoDiscriminator"), "NoDiscriminator");
    }

    #[test]
    fn resolve_unique_requires_exactly_one() {
        let one = vec!["Togo99#1234".to_string()];
        assert_eq!(resolve_unique("togo99", &one), Some("Togo99#1234".to_string()));

        let ambiguous = vec!["Togo99#1234".to_string(), "Togo99#5678".to_string()];
        assert_eq!(resolve_unique("Togo99", &ambiguous), None);

        let substring_noise = vec!["NotTogo99#1".to_string()];
        assert_eq!(resolve_unique("Togo99", &substring_noise), None);

        assert_eq!(resolve_unique("Togo99", &[]), None);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml win_rates::tests::`
Expected: PASS (all 3).

- [ ] **Step 3: Add the async HTTP fetch functions**

Append to `src-tauri/src/win_rates.rs` (not unit-tested — verified end-to-end in Task 10). Query parameters use `reqwest`'s `.query()` builder, so no URL-encoding crate is needed (`reqwest` is already a dependency):

```rust
/// Fetch `/api/draft` for an optional player battletag. Pass `None` to fetch
/// just the hero list + global stats (no player param).
pub async fn fetch_draft(player: Option<&str>) -> Result<DraftApiResponse, String> {
    let url = format!("{}/api/draft", crate::API_URL);
    let mut req = reqwest::Client::new().get(&url);
    if let Some(bt) = player {
        req = req.query(&[("player", bt)]);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("draft request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("draft request HTTP {}", resp.status()));
    }
    resp.json::<DraftApiResponse>()
        .await
        .map_err(|e| format!("draft response parse failed: {e}"))
}

/// Search for battletags by plain name via `/api/players/search`.
pub async fn search_players(name: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct SearchRow {
        battletag: String,
    }
    let url = format!("{}/api/players/search", crate::API_URL);
    let resp = reqwest::Client::new()
        .get(&url)
        .query(&[("q", name)])
        .send()
        .await
        .map_err(|e| format!("player search failed: {e}"))?;
    let rows: Vec<SearchRow> = resp
        .json()
        .await
        .map_err(|e| format!("player search parse failed: {e}"))?;
    Ok(rows.into_iter().map(|r| r.battletag).collect())
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/win_rates.rs
git commit -m "Add win-rate API client for the draft overlay"
```

---

## Task 6: Add `player_battletag` config field

**Files:**
- Modify: `src-tauri/src/config.rs:9-29`

- [ ] **Step 1: Add the field to `AppConfig`**

In `src-tauri/src/config.rs`, the struct is at lines 11-18 and `Default` at 20-29. Add two fields — `player_battletag` (the user's identity for personal win rates) and `draft_overlay_enabled` (the tray-toggle persisted flag, mirroring `input_recording_enabled`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub watch_dir: String,
    pub autostart: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default)]
    pub input_recording_enabled: bool,
    /// The user's own full battletag ("Name#1234"), used to look up their
    /// personal ARAM win rates in the draft overlay.
    #[serde(default)]
    pub player_battletag: String,
    /// Whether the ARAM draft overlay hotkey is active.
    #[serde(default)]
    pub draft_overlay_enabled: bool,
}
```

And in `Default`:

```rust
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            watch_dir: default_watch_dir(),
            autostart: false,
            start_minimized: false,
            input_recording_enabled: false,
            player_battletag: String::new(),
            draft_overlay_enabled: false,
        }
    }
}
```

- [ ] **Step 2: Write a test for the default and round-trip**

Add a `#[cfg(test)]` module at the end of `src-tauri/src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_battletag_defaults_empty() {
        assert_eq!(AppConfig::default().player_battletag, "");
    }

    #[test]
    fn player_battletag_survives_round_trip() {
        let mut c = AppConfig::default();
        c.player_battletag = "lightster#1173".to_string();
        let json = serde_json::to_value(&c).unwrap();
        let back: AppConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back.player_battletag, "lightster#1173");
    }

    #[test]
    fn missing_battletag_in_stored_json_deserializes_to_empty() {
        // Configs saved before this field existed must still load.
        let old = serde_json::json!({
            "watchDir": "/x", "autostart": false,
            "startMinimized": false, "inputRecordingEnabled": false
        });
        let c: AppConfig = serde_json::from_value(old).unwrap();
        assert_eq!(c.player_battletag, "");
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml config::tests::`
Expected: PASS (all 3). `#[serde(default)]` makes the third test pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/config.rs
git commit -m "Add player_battletag config field"
```

NOTE for later: the Settings UI (`src/routes/settings/`) should get an input bound to `playerBattletag`. That is a small follow-up and is covered in Task 11's manual checklist; it is not required for the pipeline to function (the field can be set by editing the store), so it is intentionally not its own pipeline task.

---

## Task 7: Screen capture

Capture the primary monitor into an `image::RgbaImage`. Uses GDI: `GetDC(None)` → `CreateCompatibleDC` → `CreateCompatibleBitmap` → `BitBlt` → `GetDIBits`. `image` is already a dependency (`image v0.25` is in the lock file).

This wraps OS APIs and is verified by running it, not by a unit test.

**Files:**
- Modify: `src-tauri/src/screen_capture.rs`

- [ ] **Step 1: Implement capture**

Replace `src-tauri/src/screen_capture.rs` with:

```rust
//! Primary-monitor screen capture (Windows GDI).

use image::RgbaImage;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

/// Capture the primary monitor as an RGBA image (screen-pixel dimensions).
pub fn capture_primary_monitor() -> Result<RgbaImage, String> {
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return Err("could not read screen dimensions".into());
        }

        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err("GetDC failed".into());
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        let old = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

        let blt = BitBlt(mem_dc, 0, 0, width, height, screen_dc, 0, 0, SRCCOPY);

        let mut buf = vec![0u8; (width * height * 4) as usize];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative => top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let scanlines = GetDIBits(
            mem_dc,
            bitmap,
            0,
            height as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );

        // Clean up GDI objects regardless of success.
        SelectObject(mem_dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        if blt.is_err() || scanlines == 0 {
            return Err("BitBlt/GetDIBits failed".into());
        }

        // GDI gives BGRA; swap B and R for RGBA.
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        RgbaImage::from_raw(width as u32, height as u32, buf)
            .ok_or_else(|| "image buffer size mismatch".into())
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: PASS. If the `windows` crate's exact symbol paths differ for v0.59 (e.g. `BI_RGB` typing, `HGDIOBJ` construction), fix the imports/conversions until it compiles — the structure stays the same.

- [ ] **Step 3: Add a temporary manual-verification entry point**

Add this `#[tauri::command]` to `src-tauri/src/screen_capture.rs` so capture can be triggered and inspected during development:

```rust
/// DEV-ONLY: capture the screen and save it to the temp dir for inspection.
#[tauri::command]
pub fn dev_capture_screen() -> Result<String, String> {
    let img = capture_primary_monitor()?;
    let path = std::env::temp_dir().join("storm-almanac-capture.png");
    img.save(&path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}
```

Register it in `lib.rs`'s `invoke_handler` (find `tauri::generate_handler!` and add `screen_capture::dev_capture_screen`).

- [ ] **Step 4: Verify capture works**

Run `npm run tauri:dev`. From the app (devtools console of any window, or a temporary button) invoke `dev_capture_screen`. Open the returned PNG path.
Expected: the PNG shows the current screen contents, correct colors, correct dimensions.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/screen_capture.rs src-tauri/src/lib.rs
git commit -m "Add primary-monitor screen capture"
```

---

## Task 8: OCR via Windows.Media.Ocr

Convert an `RgbaImage` to a WinRT `SoftwareBitmap`, run `OcrEngine`, and return `Vec<OcrLine>` (one per recognized line, with the union bounding box of its words).

**Files:**
- Modify: `src-tauri/src/ocr.rs`

- [ ] **Step 1: Implement OCR**

Replace `src-tauri/src/ocr.rs` with:

```rust
//! OCR via Windows.Media.Ocr.

use crate::draft_types::{OcrLine, Rect};
use image::RgbaImage;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

/// Run OCR over an image, returning one entry per recognized text line.
pub fn recognize_lines(img: &RgbaImage) -> Result<Vec<OcrLine>, String> {
    let (w, h) = (img.width(), img.height());

    // Build a BGRA8 SoftwareBitmap from the RGBA buffer.
    let writer = DataWriter::new().map_err(|e| e.to_string())?;
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for px in img.pixels() {
        bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]); // RGBA -> BGRA
    }
    writer.WriteBytes(&bgra).map_err(|e| e.to_string())?;
    let buffer = writer.DetachBuffer().map_err(|e| e.to_string())?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        w as i32,
        h as i32,
        windows::Graphics::Imaging::BitmapAlphaMode::Premultiplied,
    )
    .map_err(|e| e.to_string())?;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| e.to_string())?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for line in result.Lines().map_err(|e| e.to_string())? {
        let text = line.Text().map_err(|e| e.to_string())?.to_string();
        // Union of word bounding rects.
        let mut x0 = f64::INFINITY;
        let mut y0 = f64::INFINITY;
        let mut x1 = f64::NEG_INFINITY;
        let mut y1 = f64::NEG_INFINITY;
        for word in line.Words().map_err(|e| e.to_string())? {
            let r = word.BoundingRect().map_err(|e| e.to_string())?;
            x0 = x0.min(r.X as f64);
            y0 = y0.min(r.Y as f64);
            x1 = x1.max((r.X + r.Width) as f64);
            y1 = y1.max((r.Y + r.Height) as f64);
        }
        if x0.is_finite() {
            out.push(OcrLine {
                text,
                rect: Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 },
            });
        }
    }
    Ok(out)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: PASS. The WinRT symbol names (`SoftwareBitmap::CreateCopyWithAlphaFromBuffer`, `BitmapAlphaMode`, the `.get()` blocking call on `IAsyncOperation`) may need small adjustments for `windows` 0.59 — fix until it compiles, keeping the RGBA→BGRA→SoftwareBitmap→OcrEngine→Lines flow.

- [ ] **Step 3: Add a temporary manual-verification command**

Add to `src-tauri/src/ocr.rs`:

```rust
/// DEV-ONLY: capture the screen, OCR it, and return every recognized line.
#[tauri::command]
pub fn dev_ocr_screen() -> Result<Vec<(String, f64, f64)>, String> {
    let img = crate::screen_capture::capture_primary_monitor()?;
    let lines = recognize_lines(&img)?;
    Ok(lines
        .into_iter()
        .map(|l| (l.text, l.rect.x, l.rect.y))
        .collect())
}
```

Register `ocr::dev_ocr_screen` in `lib.rs`'s `invoke_handler`.

- [ ] **Step 4: Verify OCR works against a real ARAM draft**

With HoTS at an ARAM hero-select screen (or a saved screenshot of one set as the desktop background full-screen), run `npm run tauri:dev` and invoke `dev_ocr_screen`.
Expected: the returned list contains the 5 player names and 15 hero names with plausible x/y coordinates. Spot-check that "CHOOSE A HERO" and hero names appear.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/ocr.rs src-tauri/src/lib.rs
git commit -m "Add Windows.Media.Ocr text recognition"
```

---

## Task 9: Overlay UI — `?mode=draft`

Add a `draft` mode to the overlay route that listens for a `draft://update` event and draws each hero's win rates at its bounding box.

**Files:**
- Modify: `src/routes/overlay/+page.svelte`
- Modify: `src-tauri/capabilities/overlay.json:5`

- [ ] **Step 1: Add the window label to capabilities**

In `src-tauri/capabilities/overlay.json`, line 5 lists window labels. Add `"overlay-draft"`:

```json
  "windows": ["overlay-interactive", "overlay-clickthrough", "overlay-blocker", "overlay-draft"],
```

- [ ] **Step 2: Add draft state and listener to the overlay script**

In `src/routes/overlay/+page.svelte`, in the `<script>` block, add draft state alongside the existing state declarations (after line 15):

```js
	/** @type {{ hero: string, rect: {x:number,y:number,width:number,height:number}, win_rates: {overall: number|null, player: number|null, player_games: number|null} }[]} */
	let draftHeroes = $state([]);
```

In `onMount`, extend the mode parsing (line 19) to accept `draft`, and add a listener:

```js
		const m = page.url.searchParams.get('mode');
		mode = m === 'clickthrough' || m === 'blocker' || m === 'draft' ? m : 'interactive';

		if (mode === 'clickthrough' || mode === 'draft') {
			try {
				await getCurrentWindow().setIgnoreCursorEvents(true);
			} catch (e) {
				console.error('setIgnoreCursorEvents failed', e);
			}
		}

		if (mode === 'draft') {
			unlisten = await listen('draft://update', (event) => {
				const payload = event?.payload;
				if (payload && Array.isArray(payload.heroes)) {
					draftHeroes = payload.heroes;
				}
			});
		}
```

(Keep the existing `clickthrough` and `blocker` branches; the snippet above replaces the `clickthrough`-only `setIgnoreCursorEvents` block so `draft` is click-through too.)

- [ ] **Step 3: Add the draft render branch**

In the markup, add a `draft` branch. Change the final `{:else}` (line 113, the blocker branch) into `{:else if mode === 'blocker'}`, and add before the closing `{/if}`:

```svelte
{:else if mode === 'draft'}
	{#each draftHeroes as h (h.hero + h.rect.x + h.rect.y)}
		<div
			class="draft-label"
			style="left: {h.rect.x}px; top: {h.rect.y + h.rect.height + 2}px;"
		>
			<span class="wr overall {wrClass(h.win_rates.overall)}">
				{fmt(h.win_rates.overall)}
			</span>
			{#if h.win_rates.player !== null}
				<span class="wr player {wrClass(h.win_rates.player)}">
					you {fmt(h.win_rates.player)}
				</span>
			{/if}
		</div>
	{/each}
{/if}
```

Add these helpers to the `<script>` block:

```js
	/** @param {number|null} wr */
	function fmt(wr) {
		return wr === null ? '—' : wr.toFixed(0) + '%';
	}
	/** @param {number|null} wr */
	function wrClass(wr) {
		if (wr === null) return 'neutral';
		if (wr >= 55) return 'good';
		if (wr < 45) return 'bad';
		return 'neutral';
	}
```

- [ ] **Step 4: Add styles**

Add to the `<style>` block:

```css
	.draft-label {
		position: absolute;
		display: flex;
		gap: 4px;
		font-family: 'DM Sans', system-ui, sans-serif;
		font-size: 12px;
		font-weight: 700;
		pointer-events: none;
		white-space: nowrap;
	}
	.wr {
		padding: 1px 5px;
		border-radius: 5px;
		background: rgba(0, 0, 0, 0.8);
	}
	.wr.good { color: #4ade80; }
	.wr.bad { color: #f87171; }
	.wr.neutral { color: #e5e7eb; }
	.wr.player { background: rgba(20, 30, 60, 0.9); }
```

- [ ] **Step 5: Verify the frontend builds**

Run: `npm run build`
Expected: PASS (SvelteKit build succeeds, no type errors).

- [ ] **Step 6: Commit**

```bash
git add src/routes/overlay/+page.svelte src-tauri/capabilities/overlay.json
git commit -m "Add draft mode to the overlay UI"
```

---

## Task 10: Pipeline orchestration, window, tray, hotkey

`draft_overlay` ties everything together: enabled-state, hotkey registration, the capture→OCR→parse→win-rate→emit pipeline, and the overlay window. This mirrors the map blocker's structure in `lib.rs` (`open_blocker_window`, `register_blocker_hotkey`, `set_blocker_enabled`, the tray `CheckMenuItem`).

**Files:**
- Modify: `src-tauri/src/draft_overlay.rs`
- Modify: `src-tauri/src/lib.rs` (constants ~line 90, tray ~956-1025, setup ~1052)

- [ ] **Step 1: Implement the `draft_overlay` module**

Replace `src-tauri/src/draft_overlay.rs` with:

```rust
//! ARAM draft overlay orchestration: pipeline, window, hotkey.

use crate::draft_types::{DraftOverlayHero, DraftOverlayPayload, HeroWinRates};
use crate::win_rates;
use std::collections::HashMap;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const DRAFT_LABEL: &str = "overlay-draft";

/// Open the full-screen transparent click-through overlay window.
fn open_window(app: &tauri::AppHandle) {
    if app.get_webview_window(DRAFT_LABEL).is_some() {
        return;
    }
    let result = WebviewWindowBuilder::new(
        app,
        DRAFT_LABEL,
        WebviewUrl::App("/overlay?mode=draft".into()),
    )
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false)
    .focusable(false)
    .maximized(true)
    .visible(true)
    .build();
    match result {
        Ok(_) => log::info!("Opened draft overlay window"),
        Err(e) => log::error!("Failed to open draft overlay window: {e}"),
    }
}

/// Close the overlay window if open.
pub fn close_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(DRAFT_LABEL) {
        let _ = w.close();
    }
}

/// Run the full pipeline once: capture -> OCR -> parse -> win rates -> emit.
/// Runs on a background thread; never blocks the caller.
pub fn run_pipeline(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        if let Err(e) = run_pipeline_inner(&app) {
            log::error!("draft overlay pipeline failed: {e}");
        }
    });
}

fn run_pipeline_inner(app: &tauri::AppHandle) -> Result<(), String> {
    // 1. Capture + OCR.
    let img = crate::screen_capture::capture_primary_monitor()?;
    let lines = crate::ocr::recognize_lines(&img)?;

    // 2. Sanity check: must look like a draft screen.
    let looks_like_draft = lines
        .iter()
        .any(|l| l.text.to_uppercase().contains("CHOOSE A HERO"));
    if !looks_like_draft {
        log::info!("draft overlay: no 'CHOOSE A HERO' found; ignoring");
        return Ok(());
    }

    // 3. Hero list + global stats from one player-less /api/draft call.
    let base = tauri::async_runtime::block_on(win_rates::fetch_draft(None))?;
    let base_table = win_rates::build_table(&base);

    // 4. Parse columns.
    let draft = crate::draft_parse::parse_draft(&lines, &base_table.hero_names);
    if draft.players.is_empty() {
        log::info!("draft overlay: parsed no player columns");
        return Ok(());
    }

    // 5. Resolve each player and fetch personal win rates.
    let own_battletag = {
        let cfg = crate::config::load_config(app);
        cfg.player_battletag
    };
    let own_name_part = win_rates::battletag_name_part(&own_battletag).to_string();

    // hero -> personal (win_rate, games), per resolved player.
    let mut personal: HashMap<String, HashMap<String, (f64, u32)>> = HashMap::new();
    for player in &draft.players {
        let battletag = if !own_name_part.is_empty()
            && player.name.eq_ignore_ascii_case(&own_name_part)
        {
            Some(own_battletag.clone())
        } else {
            match tauri::async_runtime::block_on(win_rates::search_players(&player.name)) {
                Ok(cands) => win_rates::resolve_unique(&player.name, &cands),
                Err(e) => {
                    log::warn!("player search failed for {}: {e}", player.name);
                    None
                }
            }
        };
        if let Some(bt) = battletag {
            match tauri::async_runtime::block_on(win_rates::fetch_draft(Some(&bt))) {
                Ok(resp) => {
                    personal.insert(player.name.clone(), win_rates::build_table(&resp).player);
                }
                Err(e) => log::warn!("draft fetch failed for {bt}: {e}"),
            }
        }
    }

    // 6. Merge into the overlay payload.
    let mut heroes: Vec<DraftOverlayHero> = Vec::new();
    for player in &draft.players {
        let player_table = personal.get(&player.name);
        for hero in &player.heroes {
            let player_wr = player_table.and_then(|t| t.get(&hero.hero).copied());
            heroes.push(DraftOverlayHero {
                hero: hero.hero.clone(),
                rect: hero.rect,
                win_rates: HeroWinRates {
                    overall: base_table.overall.get(&hero.hero).copied(),
                    player: player_wr.map(|(wr, _)| wr),
                    player_games: player_wr.map(|(_, g)| g),
                },
            });
        }
    }

    // 7. Show the overlay and emit.
    let app2 = app.clone();
    app.run_on_main_thread(move || open_window(&app2))
        .map_err(|e| e.to_string())?;
    // Give the window a moment to mount its event listener.
    std::thread::sleep(std::time::Duration::from_millis(250));
    app.emit_to(DRAFT_LABEL, "draft://update", DraftOverlayPayload { heroes })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Hotkey handler: if the overlay is open, dismiss it; else run the pipeline.
pub fn toggle(app: &tauri::AppHandle) {
    if app.get_webview_window(DRAFT_LABEL).is_some() {
        close_window(app);
    } else {
        run_pipeline(app.clone());
    }
}
```

- [ ] **Step 2: Add constants and hotkey registration in `lib.rs`**

In `src-tauri/src/lib.rs`, after the `BLOCKER_HOTKEY` constant (line 90), add:

```rust
const DRAFT_OVERLAY_HOTKEY: &str = "CmdOrCtrl+Shift+D";
```

Add a hotkey registration function near `register_blocker_hotkey` (around line 571), modeled on it:

```rust
fn register_draft_overlay_hotkey(app: &tauri::AppHandle) {
    let shortcut: Shortcut = match DRAFT_OVERLAY_HOTKEY.parse() {
        Ok(s) => s,
        Err(e) => {
            log::error!("invalid draft overlay hotkey: {e}");
            return;
        }
    };
    if app.global_shortcut().is_registered(shortcut.clone()) {
        return;
    }
    let app_handle = app.clone();
    let res = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                let ah = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    draft_overlay::toggle(&ah);
                });
            }
        });
    match res {
        Ok(_) => log::info!("registered draft overlay hotkey {DRAFT_OVERLAY_HOTKEY}"),
        Err(e) => log::error!("register draft overlay hotkey failed: {e}"),
    }
}
```

In the same area, add an unregister function and an enable/disable function (these mirror `unregister_blocker_hotkey` / `set_blocker_enabled`):

```rust
fn unregister_draft_overlay_hotkey(app: &tauri::AppHandle) {
    let shortcut: Shortcut = match DRAFT_OVERLAY_HOTKEY.parse() {
        Ok(s) => s,
        Err(_) => return,
    };
    if app.global_shortcut().is_registered(shortcut.clone()) {
        let _ = app.global_shortcut().unregister(shortcut);
    }
}

fn set_draft_overlay_enabled(app: &tauri::AppHandle, enabled: bool) {
    let mut cfg = load_config(app);
    cfg.draft_overlay_enabled = enabled;
    save_config(app, &cfg);
    if enabled {
        register_draft_overlay_hotkey(app);
    } else {
        unregister_draft_overlay_hotkey(app);
        draft_overlay::close_window(app);
    }
    log::info!("draft overlay enabled={enabled}");
}
```

- [ ] **Step 3: Add the "Enable Draft Overlay" tray item**

The tray is built in `lib.rs` around lines 950-1038. Make four edits, mirroring the existing `enable_blocker` `CheckMenuItem`:

(a) After the `enable_blocker` item is built (~line 958), build the draft item; its checked state comes from config:

```rust
            let draft_enabled_at_startup = load_config(app.handle()).draft_overlay_enabled;
            let enable_draft =
                CheckMenuItemBuilder::with_id("enable_draft_overlay", "Enable Draft Overlay")
                    .checked(draft_enabled_at_startup)
                    .build(app)?;
```

(b) Add it to the `MenuBuilder` chain (~lines 960-971), right after `.item(&enable_blocker)`:

```rust
                .item(&enable_blocker)
                .item(&enable_draft)
```

(c) Clone it for the menu-event closure, next to `let enable_blocker_menu = enable_blocker.clone();` (~line 987):

```rust
            let enable_draft_menu = enable_draft.clone();
```

(d) Add a handler branch in the `on_menu_event` closure, after the `enable_blocker` branch (~lines 1019-1025):

```rust
                    } else if event.id() == "enable_draft_overlay" {
                        let cfg = load_config(app);
                        let new_enabled = !cfg.draft_overlay_enabled;
                        set_draft_overlay_enabled(app, new_enabled);
                        let _ = enable_draft_menu.set_checked(new_enabled);
                    }
```

- [ ] **Step 4: Register the hotkey at startup, and dismiss the overlay on focus loss**

(a) In the `setup` closure, near where `register_blocker_hotkey` is called (~lines 1056-1058):

```rust
            if load_config(app.handle()).draft_overlay_enabled {
                register_draft_overlay_hotkey(app.handle());
            }
```

(b) The overlay must also dismiss when HoTS loses focus. `handle_focus_change(app, focused)` in `lib.rs` already runs on every game-focus change. Add this near the top of its body so losing focus closes the draft overlay:

```rust
    if !focused {
        draft_overlay::close_window(app);
    }
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/draft_overlay.rs src-tauri/src/lib.rs
git commit -m "Wire ARAM draft overlay pipeline, tray toggle, and hotkey"
```

---

## Task 11: End-to-end verification and cleanup

**Files:**
- Modify: `src-tauri/src/screen_capture.rs`, `src-tauri/src/ocr.rs`, `src-tauri/src/lib.rs` (remove dev commands)
- Modify: `src/routes/settings/+page.svelte` (add the battletag input)

- [ ] **Step 1: Add the battletag field to Settings UI**

In the settings route (`src/routes/settings/+page.svelte` — confirm the path with `ls src/routes/settings`), add a text input bound to the config's `playerBattletag` field, following the existing inputs' pattern (the page already loads/saves `AppConfig` via the `get_config`/`save_config_cmd` commands). Label it "Your BattleTag (Name#1234)".

- [ ] **Step 2: Full manual test in a live ARAM**

Run `npm run tauri:dev`. Set your battletag in Settings and enable "Enable Draft Overlay" from the tray menu. Queue an ARAM. At the "CHOOSE A HERO" screen, press `Ctrl+Shift+D`.
Expected:
- The overlay appears within ~1-2 seconds.
- Each of the 15 heroes has an overall win% beside it.
- Your own column's heroes also show a "you NN%" personal figure.
- Pressing `Ctrl+Shift+D` again dismisses the overlay.
- Pressing it on a non-draft screen logs "no 'CHOOSE A HERO' found" and shows nothing.

Check the dev log (`%LOCALAPPDATA%\com.lightster.storm-almanac.dev\logs\`) for the pipeline log lines if anything is missing.

- [ ] **Step 3: Remove the dev-only commands**

Delete `dev_capture_screen` from `screen_capture.rs` and `dev_ocr_screen` from `ocr.rs`, and remove both from the `tauri::generate_handler!` list in `lib.rs`. They were scaffolding for Tasks 7-8.

- [ ] **Step 4: Verify everything still compiles and tests pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS (all `draft_parse`, `win_rates`, `config` tests).
Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/screen_capture.rs src-tauri/src/ocr.rs src-tauri/src/lib.rs src/routes/settings/+page.svelte
git commit -m "Add battletag setting and remove draft overlay dev scaffolding"
```

---

## Notes & known limitations (v1)

- Hotkey-only trigger; no auto-detection of the draft screen (deferred per spec).
- Full-screen mirror capture assumes HoTS in Windowed Fullscreen on the primary monitor.
- Teammate personal win rates appear only when an OCR'd name resolves to exactly one battletag whose name-part matches — usually only the configured user. Everyone else shows overall-only, as the spec accepts.
- `screen_capture` and `ocr` wrap OS/WinRT APIs; the exact `windows` 0.59 symbol paths may need small fixes to compile. The integration verification is the manual ARAM test in Task 11.
