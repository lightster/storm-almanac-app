# Overlay Batch Endpoint — Design

**Date:** 2026-05-22
**Status:** Approved design — ready for implementation planning
**Spans:** `hotsds` (backend) and `storm-almanac-app` (overlay client)

## Context

The ARAM draft overlay's pipeline currently takes ~7s on a typical
draft. Stage-timing logs from a live game (commit `d5608ba`) revealed:

```
draft pipeline: capture 554ms, ocr 684ms (28 lines)
draft pipeline: base-fetch 437ms
draft pipeline: player[0] "zulu"      total  739ms (search 321ms, fetch 418ms)
draft pipeline: player[1] "Eragon"    total  652ms (search 301ms, fetch 350ms)
draft pipeline: player[2] "(self)"    total  352ms (search   0ms, fetch 352ms)
draft pipeline: player[3] "omegarojo" total 3588ms (search 136ms, fetch 3451ms)
draft pipeline: player[4] "Zen"       total  292ms (search 292ms, fetch   0ms)
draft pipeline: total 7362ms
```

~76% of total time is sequential per-player work — 5 × (`/api/players/search`
+ `/api/draft?player=…`), each call paying a fresh TLS handshake (no
`reqwest::Client` reuse). One player is also a 3.5-second outlier — most
likely a low-data battletag falling off the fast aggregation path in the
backend.

Since the previous-draft's payload is no longer shown during this window
(`70d6cd5`), the overlay sits **blank** for those ~7 seconds — correct
but unusable for the first half of a 30-second ARAM draft.

## Goal

Drop the pipeline's network phase from ~6 seconds of sequential work
(11 HTTP requests) to a single round-trip whose latency is bounded by
the slowest single per-player query. Expected end-state: total
pipeline ~1.7–2s.

## Decisions (locked)

- **Add two endpoints on `hotsds`, alongside the existing ones:**
  - `GET /api/overlay/heroes` — the canonical hero-name catalog.
  - `POST /api/overlay/draft` — overall + per-player win rates for a
    specific set of `(player, heroes)` slots, all in one request.
- **`/api/draft` and `/api/players/search` are unchanged.** The
  `/play/draft` web page keeps using them.
- **Server-side battletag resolution.** The overlay endpoint accepts
  raw OCR'd player names *or* battletags. The server fans out the name
  searches in parallel with the stats queries, so the 5 sequential
  `search_players` calls disappear.
- **Server-side fan-out.** The batch endpoint runs every per-player
  query and the global query as one `Promise.all`. Total wait is bounded
  by the slowest single query, not the sum.
- **Hero-filtered SQL.** Per-player queries include `ps.hero = ANY($heroes)`
  — only 3 heroes per player, not all of a player's history. Designed to
  let a `(display_battletag, hero)` index serve the query directly, so
  the low-data-player outlier collapses.
- **Existing per-player SQL semantics are preserved.** The overlay shows
  the same numbers as the `/play/draft` web page (last 6 months when
  ≥10 recent games on the hero, else last 10 games overall).
- **Client caches the hero catalog in memory at app start.** HoTS has not
  added a new hero in years; refresh on each app launch is plenty. If
  the startup fetch hasn't completed when the first draft fires, the
  pipeline blocks briefly on it (worst case ~50–100 ms).

## API design (hotsds, additive)

### `GET /api/overlay/heroes`

Returns the canonical hero name list — just the names, alphabetical.
The catalog endpoints today return roles, slugs, and aliases too; the
overlay client uses only the names, so the lean shape keeps the response
small (~1 KB) and makes the client's job trivial.

```http
GET /api/overlay/heroes
```

Response:

```json
{
  "heroes": ["Abathur", "Alarak", "Alexstrasza", "...", "Zul'jin"]
}
```

Errors: only 5xx (DB failure). The client treats a failure as "catalog
unavailable, try again later."

### `POST /api/overlay/draft`

Returns overall win rates for every hero mentioned in the request and
per-player win rates for each player's specific 3 heroes.

```http
POST /api/overlay/draft
Content-Type: application/json
```

```json
{
  "players": [
    { "name": "zulu",          "heroes": ["Stitches", "Kel'Thuzad", "Xul"] },
    { "name": "truffle",       "heroes": ["D.Va", "Kharazim", "Sonya"] },
    { "battletag": "Lightster#1234", "heroes": ["Whitemane", "Fenix", "Imperius"] },
    { "name": "Eragon",        "heroes": ["Malthael", "Xul", "Valeera"] },
    { "name": "Zen",           "heroes": ["Tychus", "Zagara", "Anub'arak"] }
  ]
}
```

Each element of `players` is an object with `heroes` (always present)
and at most one of `battletag` or `name`:

- `{ "battletag": "X#1234", "heroes": [...] }` — the client already has a
  battletag (typically the user's own, from config). Server uses it
  directly.
- `{ "name": "zulu", "heroes": [...] }` — the client only has the OCR'd
  name. Server runs the same `name` → battletag lookup `/api/players/search`
  does, and proceeds only on a *unique* case-insensitive match.
- `{ "heroes": [...] }` — no identifier. The server returns `null` in
  `players[i]` (still includes the heroes in `overall`). This is the v1
  client's shape for the self column when the user has not configured
  their battletag, or for any unresolvable OCR'd column where the client
  has already given up on resolution.

Response:

```json
{
  "overall": {
    "Whitemane": 46.6,
    "Stitches": 51.2,
    "...": 0.0
  },
  "players": [
    { "Stitches": { "win_rate": 60.0, "games": 12 }, "Kel'Thuzad": null, "Xul": { "win_rate": 55.0, "games": 4 } },
    { "D.Va": null, "Kharazim": null, "Sonya": null },
    { "Whitemane": { "win_rate": 60.0, "games": 10 }, "Fenix": { "win_rate": 40.0, "games": 5 }, "Imperius": { "win_rate": 43.0, "games": 7 } },
    null,
    { "Tychus": null, "Zagara": null, "Anub'arak": null }
  ]
}
```

- `overall`: union of every hero mentioned in any `players` entry; each
  value is the global ARAM win rate (rounded to one decimal place, as
  today).
- `players[i]`: position-aligned with the request's `players[i]`.
  - An object keyed by every hero the player was offered. Each value is
    `{ win_rate, games }` or `null` (the player has no games on that
    hero in this matching window).
  - `null` if the request slot supplied no identifier, or if a supplied
    `name` did not resolve to a unique battletag (today's
    `resolve_unique` semantics), or if the per-player query threw.

Errors:
- Malformed JSON → 400.
- DB failure on the global query → 500 (the whole response fails).
- DB failure on a single per-player query → that `players[i]` is `null`;
  the rest of the response succeeds; the failure is logged server-side.

## Client architecture (storm-almanac-app)

### New module: `hero_catalog`

A trivial in-memory cache, app-managed:

```rust
pub struct HeroCatalog {
    cache: tokio::sync::Mutex<Option<Vec<String>>>,
}
```

- `pub async fn ensure(app: &AppHandle) -> Result<Vec<String>, String>` —
  returns the cached list, or fetches `/api/overlay/heroes` and caches
  it. Idempotent: concurrent callers see one fetch in flight.
- Spawned at app start (`setup()`) as a fire-and-forget task to warm
  the cache before the first draft. On startup failure, the next
  `ensure` call retries.

### Pipeline changes

`run_pipeline_inner` (`draft_overlay.rs`) drops down to:

```
1. capture_primary_monitor()
2. ocr::recognize_lines(&img)
3. hero_catalog::ensure(app)  // instant if cache warm
4. parse_draft(&lines, &hero_list)
5. build batch request from the parsed draft
6. fetch_overlay_draft(&request)  // one POST, fan-out on the server
7. build DraftOverlayHero list, merge in overall + per-player rates
8. show_overlay(app, heroes)
```

Steps removed: `fetch_draft(None)` (catalog now from `hero_catalog`),
5 × `search_players` (server resolves names), 5 × `fetch_draft(Some(bt))`
(folded into the single batch call).

### New API client function

In `win_rates.rs` (or a new `overlay_api.rs` module — to be decided
during planning, depending on how clean the split feels):

```rust
pub async fn fetch_overlay_draft(
    req: &OverlayDraftRequest,
) -> Result<OverlayDraftResponse, String>
```

`OverlayDraftRequest` and `OverlayDraftResponse` are new serde structs
mirroring the JSON shapes above.

### Removed code

`search_players` and the `?player=` mode of `fetch_draft` become unused
in the overlay's pipeline. The deletion happens at the end of the
client-side change — the implementation plan will sequence it so the
old code is replaced, not orphaned.

### Logging

The existing per-stage timing instrumentation stays, with adjustments:
- `base-fetch` becomes `catalog` (the `hero_catalog::ensure` call, ~0
  ms when warm).
- The per-player loop becomes a single `batch-fetch` line.

### Error handling

- Catalog fetch fails (cold, no cache) → log error, abort this
  pipeline run. Watcher will tick again in 1 s; the overlay stays blank
  until a fetch succeeds.
- Batch fetch fails (network / 5xx) → log error, abort this pipeline
  run. Overlay stays blank.
- Batch fetch returns 200 with some `players[i] = null` → render
  overall only for that column's heroes, no personal numbers. Same
  graceful degradation as today.

## Backend architecture (hotsds)

Both endpoints are SvelteKit `+server.js` files; the project already
follows that pattern.

### `app/src/routes/api/overlay/heroes/+server.js`

```js
import { json } from '@sveltejs/kit';
import { query } from '$lib/server/db.js';

export async function GET() {
  const rows = await query('SELECT DISTINCT hero FROM heroes ORDER BY hero');
  return json({ heroes: rows.map(r => r.hero) });
}
```

### `app/src/routes/api/overlay/draft/+server.js`

For each request element with a `name` (not a `battletag`), resolve to a
unique battletag using the same logic as `resolve_unique` in
`win_rates.rs` — case-insensitive `name_part` match against the players
table; require exactly one match or `null`.

Then run, in parallel via `Promise.all`:
- One **global stats** query covering the union of all heroes mentioned.
- One **per-player stats** query per resolved slot, using the existing
  CTE-and-window-function shape from `/api/draft` *but* with the added
  `ps.hero = ANY($heroes::text[])` predicate to limit the scan to just
  the 3 heroes.

The per-player SQL pattern (preserving the existing 6-month + ≥10-game
semantics):

```sql
WITH hero_recent AS (
  SELECT ps.hero, COUNT(*) AS recent_games
  FROM player_stats_computed ps
  JOIN replays r USING (replay_id)
  WHERE ps.display_battletag = $1
    AND ps.hero = ANY($2::text[])
    AND r.game_mode = 'ARAM'
    AND r.played_at >= CURRENT_DATE - INTERVAL '6 months'
  GROUP BY ps.hero
),
ranked AS (
  SELECT
    ps.hero, ps.result, r.played_at,
    COALESCE(hr.recent_games, 0) AS recent_games,
    ROW_NUMBER() OVER (PARTITION BY ps.hero ORDER BY r.played_at DESC) AS rn
  FROM player_stats_computed ps
  JOIN replays r USING (replay_id)
  LEFT JOIN hero_recent hr ON ps.hero = hr.hero
  WHERE ps.display_battletag = $1
    AND ps.hero = ANY($2::text[])
    AND r.game_mode = 'ARAM'
)
SELECT
  hero,
  COUNT(*) AS games,
  ROUND(AVG(CASE WHEN result = 'win' THEN 1.0 ELSE 0.0 END) * 100, 1) AS win_rate
FROM ranked
WHERE (recent_games >= 10 AND played_at >= CURRENT_DATE - INTERVAL '6 months')
   OR (recent_games < 10 AND rn <= 10)
GROUP BY hero
```

The query shape is otherwise identical to today's `/api/draft?player=`
query, just with the extra hero filter. Same indexes apply.

### Concurrency model

```
Promise.all([
  globalStatsForHeroes(union_of_all_heroes),
  resolveAndQueryPlayer(players[0]),
  resolveAndQueryPlayer(players[1]),
  resolveAndQueryPlayer(players[2]),
  resolveAndQueryPlayer(players[3]),
  resolveAndQueryPlayer(players[4]),
])
```

`resolveAndQueryPlayer` is per-slot:
- If the slot supplied `battletag`, skip resolution.
- Else look up the unique battletag for `name`.
- If resolved, run the per-player SQL with `(battletag, heroes)`.
- Return `{ heroResults | null }`. Catching exceptions returns `null`
  so a single failure doesn't poison the response.

## Data flow

```
client                              server (hotsds)
  │
  ├── (warm at app start) ────────► GET /api/overlay/heroes
  │                          ◄──── { heroes: [...] }
  │
  │ (per draft)
  ├── capture + OCR
  ├── hero_catalog::ensure  (cache hit)
  ├── parse_draft
  ├── build OverlayDraftRequest
  └── POST /api/overlay/draft ────►
                                    Promise.all:
                                      global stats over union(heroes)
                                      for each player slot:
                                        resolve name -> battletag (if needed)
                                        per-player stats over (battletag, heroes)
                              ◄──── { overall, players: [...] }
  ├── merge into DraftOverlayHero[]
  └── show_overlay
```

## Testing

**Backend (`hotsds`):**
- Manual: hit both endpoints with `curl`; verify shape with a known
  battletag and 3 heroes vs. what `/api/draft?player=…` returns for
  parity on those 3 heroes.
- Whatever test layer hotsds already uses for the existing API routes
  — to be confirmed in planning. If none, this is consistent with the
  existing code's testing posture.

**Client (`storm-almanac-app`):**
- New unit tests for `OverlayDraftRequest`/`OverlayDraftResponse` serde
  round-trips against fixture JSON.
- New unit test for the merge step: given an `OverlayDraftResponse` and
  a parsed `Draft`, produce the right `DraftOverlayHero` list.
- `hero_catalog` cache behaviour: one-fetch-only-on-concurrent-callers,
  cache-hit-instant. May need test scaffolding; will be evaluated in
  planning.
- Integration / live: a real ARAM draft. Watch the new timing logs:
  expect `catalog 0ms`, `batch-fetch` somewhere in the 500–1500 ms range,
  total ~1.7–2 s.

## Open investigation item (load-bearing)

The whole performance argument rests on the assumption that
`ps.hero = ANY($heroes::text[])` lets Postgres serve the per-player
query from a `(display_battletag, hero)`-shaped index — and that
low-data players like `omegarojo` get fast queries when scoped to 3
heroes rather than their full history.

Before implementing the client side, verify against the dev database:
- `EXPLAIN ANALYZE` of the new per-player SQL for a high-data and a
  low-data battletag.
- Measure round-trip times for both via `curl`.

If the index plan doesn't materialize, the design still works (fan-out
parallelism alone collapses the sum to a max), but the outlier player
still dominates. That's worth knowing before measuring against the
target end-state of ~1.7–2 s.

## Rollout

Two phases, no coordination beyond ordering:

1. **Deploy `hotsds`.** Both new endpoints live. No existing
   functionality changes. Old desktop clients keep working.
2. **Ship `storm-almanac-app`.** New build uses the overlay endpoints.

Each phase is independently revertible.

## Out of scope for v1

- The `/play/draft` web page or `/api/draft` itself.
- The "CHOOSE A HERO" self-column name leak (tracked separately:
  storm-almanac-app issue #1).
- Adding any new overlay-displayed data (just preserving today's
  overall + personal win rates).
- OCR latency. ~1.2 s is acceptable; this design doesn't touch it.
- `reqwest::Client` reuse on the desktop side. With one HTTP request
  per pipeline instead of 11, the per-call TLS-handshake savings are
  negligible.
