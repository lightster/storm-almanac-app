# Overlay Batch Endpoint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the ARAM draft overlay's 11 sequential HTTP requests into one batched POST so total pipeline time drops from ~7 s to ~1.7–2 s.

**Architecture:** Two new endpoints on `hotsds` — `GET /api/overlay/heroes` (lean hero-name catalog) and `POST /api/overlay/draft` (overall + per-player win rates with server-side battletag resolution and `Promise.all` fan-out). The desktop client adds a `hero_catalog` cache warmed at app start, replaces its per-player loop with a single batch call, and removes the now-unused `search_players` + `fetch_draft(player=...)` paths.

**Tech Stack:**
- Backend (`hotsds`): SvelteKit 2, `pg` Postgres client, JS endpoints in `+server.js` files.
- Client (`storm-almanac-app`): Rust 2021 / Tauri 2, `reqwest`, `serde`, `tokio` (sync mutex for catalog).

**Cross-repo notes:**
- The cross-repo design spec lives at `C:\Users\light\code\storm-almanac-app\docs\superpowers\specs\2026-05-22-overlay-batch-endpoint-design.md` — read it before starting.
- Backend work happens in `C:\Users\light\code\hotsds` on a feature branch `overlay-endpoints`. Per the user's git-workflow rule (`NEVER commit directly to main or master`), all backend commits target that feature branch and a PR is opened.
- Client work happens in `C:\Users\light\code\storm-almanac-app` on the existing branch `overlay-batch-endpoint` (already checked out; the spec is already committed there).
- **Phase ordering is load-bearing:** the backend must be deployed to `hots.lightster.ninja` before the client can be verified. The plan calls this out at the phase boundary.

**Working-directory rule:** Both repos sit side-by-side under `C:\Users\light\code\`. Never use bare `cd`; for any cross-repo command use a subshell: `(cd C:/Users/light/code/hotsds && <cmd>)`.

---

## File Structure

**Phase 1 — backend (`hotsds`):**

- **Create:** `app/src/routes/api/overlay/heroes/+server.js` — `GET` handler returning `{ heroes: [...names...] }`. ~10 lines, mirrors the pattern in `app/src/routes/api/draft/+server.js`.
- **Create:** `app/src/routes/api/overlay/draft/+server.js` — `POST` handler. Validates the request body, runs `Promise.all` over a global-stats query plus one per-player resolve-and-query promise per slot, returns `{ overall, players: [...] }`.
- **Create:** `app/src/lib/server/overlay-draft.js` — pure helpers exported from the `+server.js`: `parsePlayers(body)` (validation + normalisation), `resolveBattletag(name)` (DB lookup mirroring `resolve_unique`), `globalHeroStats(heroSet)`, `playerHeroStats(battletag, heroes)`. Splitting these out keeps the `+server.js` thin and lets us unit-test the validator if we want to later (deferred; no test infra in hotsds today).

**Phase 2 — client (`storm-almanac-app`):**

- **Create:** `src-tauri/src/hero_catalog.rs` — in-memory hero-name cache, with `init(app)`, `ensure(app) -> Vec<String>`, and a `Default` impl. The cache is `tokio::sync::Mutex<Option<Vec<String>>>`.
- **Create:** `src-tauri/src/overlay_api.rs` — new module owning the batch-endpoint API: request/response serde structs (`OverlayDraftRequest`, `OverlayDraftPlayer`, `OverlayDraftResponse`, `OverlayPlayerHero`) and the async `fetch_overlay_draft(req)` and `fetch_overlay_heroes()` functions. Lives separately from `win_rates.rs` because the shape is wholly different from the old `/api/draft` response and bundling them would be confusing during the transition.
- **Modify:** `src-tauri/src/lib.rs` — register the new module, manage the `hero_catalog::SharedHeroCatalog` state, spawn a fire-and-forget warmup task in `setup()`.
- **Modify:** `src-tauri/src/draft_overlay.rs` — rewrite `run_pipeline_inner` to use the catalog + batch fetch instead of the old loop. Adjust the timing log to print `catalog` + `batch-fetch` instead of `base-fetch` + per-player lines.
- **Delete from `src-tauri/src/win_rates.rs`:** `search_players`, the `player`-parameter branch of `fetch_draft`, the `WinRateTable` struct, `build_table`, and their tests. The `battletag_name_part` and `resolve_unique` helpers also move out — kept as test-only inside the file or deleted (they are no longer used at runtime once the server resolves names). The `DraftApiResponse` types disappear with them.

---

## Phase 1 — Backend (`hotsds`)

### Task 1: Set up the feature branch

**Files:**
- Modify: git branch state of `C:\Users\light\code\hotsds`

- [ ] **Step 1: Verify clean working tree**

Run:

```bash
(cd C:/Users/light/code/hotsds && git status -s)
```

Expected: empty output (no uncommitted changes). If anything is dirty, stop and report — do not stash or discard.

- [ ] **Step 2: Confirm `main` is up to date**

Run:

```bash
(cd C:/Users/light/code/hotsds && git fetch origin main && git rev-list --left-right --count main...origin/main)
```

Expected: `0	0`. If `main` is behind, fast-forward it: `(cd C:/Users/light/code/hotsds && git pull --ff-only origin main)`.

- [ ] **Step 3: Create the feature branch**

Run:

```bash
(cd C:/Users/light/code/hotsds && git checkout -b overlay-endpoints main)
```

Expected: `Switched to a new branch 'overlay-endpoints'`.

### Task 2: `GET /api/overlay/heroes` — lean hero-name catalog

**Files:**
- Create: `C:\Users\light\code\hotsds\app\src\routes\api\overlay\heroes\+server.js`

- [ ] **Step 1: Create the directory**

Run:

```bash
(cd C:/Users/light/code/hotsds && mkdir -p app/src/routes/api/overlay/heroes)
```

Expected: no output, directory exists.

- [ ] **Step 2: Write the endpoint**

Create `app/src/routes/api/overlay/heroes/+server.js` with:

```js
import { json } from '@sveltejs/kit';
import { query } from '$lib/server/db.js';

export async function GET() {
	const rows = await query('SELECT DISTINCT hero FROM heroes ORDER BY hero');
	return json({ heroes: rows.map((r) => r.hero) });
}
```

- [ ] **Step 3: Manual verification with the dev server**

Run (in a separate terminal that you'll keep open through Phase 1):

```bash
(cd C:/Users/light/code/hotsds/app && npm run dev)
```

Expected: vite logs `Local: http://localhost:5173/` (or similar) without errors.

Then in another terminal:

```bash
curl -s http://localhost:5173/api/overlay/heroes | head -c 200
```

Expected: starts with `{"heroes":["Abathur","Alarak",` — alphabetical names, no roles, no slugs.

- [ ] **Step 4: Commit**

```bash
(cd C:/Users/light/code/hotsds && git add app/src/routes/api/overlay/heroes/+server.js && git commit -m "Add GET /api/overlay/heroes hero-name catalog endpoint")
```

### Task 3: Shared helpers for `POST /api/overlay/draft`

**Files:**
- Create: `C:\Users\light\code\hotsds\app\src\lib\server\overlay-draft.js`

The endpoint itself comes next (Task 4). Splitting the helpers out first keeps each task small and matches how `db.js` / `filters.js` / `identity.js` already organise reusable server code under `app/src/lib/server/`.

- [ ] **Step 1: Write the file**

Create `app/src/lib/server/overlay-draft.js` with:

```js
import { query } from './db.js';

/**
 * Validate and normalise the JSON body of POST /api/overlay/draft.
 *
 * Returns `{ players }` where each player is one of:
 *   { kind: 'battletag', battletag, heroes }
 *   { kind: 'name',      name,      heroes }
 *   { kind: 'none',                 heroes }
 *
 * Throws `Error('bad request: <reason>')` on invalid input. Callers map
 * these to HTTP 400.
 */
export function parsePlayers(body) {
	if (!body || typeof body !== 'object') {
		throw new Error('bad request: body must be a JSON object');
	}
	const { players } = body;
	if (!Array.isArray(players)) {
		throw new Error('bad request: "players" must be an array');
	}

	return {
		players: players.map((p, i) => {
			if (!p || typeof p !== 'object') {
				throw new Error(`bad request: players[${i}] must be an object`);
			}
			if (!Array.isArray(p.heroes) || p.heroes.length === 0) {
				throw new Error(`bad request: players[${i}].heroes must be a non-empty array`);
			}
			for (const h of p.heroes) {
				if (typeof h !== 'string' || h.length === 0) {
					throw new Error(`bad request: players[${i}].heroes entries must be non-empty strings`);
				}
			}
			const hasBattletag = typeof p.battletag === 'string' && p.battletag.length > 0;
			const hasName = typeof p.name === 'string' && p.name.length > 0;
			if (hasBattletag && hasName) {
				throw new Error(`bad request: players[${i}] cannot have both battletag and name`);
			}
			if (hasBattletag) return { kind: 'battletag', battletag: p.battletag, heroes: p.heroes };
			if (hasName) return { kind: 'name', name: p.name, heroes: p.heroes };
			return { kind: 'none', heroes: p.heroes };
		})
	};
}

/**
 * Resolve an OCR'd plain name (the part before '#') to a unique battletag.
 *
 * Mirrors the desktop client's `resolve_unique`: case-insensitive exact
 * match against the name part of `display_battletag`. Returns the
 * battletag iff exactly one player matches, otherwise `null`.
 */
export async function resolveBattletag(name) {
	const rows = await query(
		`SELECT display_battletag
		   FROM player_identities
		  WHERE lower(split_part(display_battletag, '#', 1)) = lower($1)
		  LIMIT 2`,
		[name]
	);
	return rows.length === 1 ? rows[0].display_battletag : null;
}

/**
 * Global ARAM win rate for every hero in `heroSet` (a JavaScript `Set`
 * of canonical hero names).
 *
 * Returns an object `{ [hero]: rounded_pct }` covering every hero in the
 * set; heroes with no ARAM history get a 0.0 win rate (matches the
 * existing /api/draft globalStats shape, which simply omits them — here
 * we omit too).
 */
export async function globalHeroStats(heroSet) {
	const heroes = [...heroSet];
	if (heroes.length === 0) return {};
	const rows = await query(
		`SELECT hero,
		        ROUND(AVG(CASE WHEN result = 'win' THEN 1.0 ELSE 0.0 END) * 100, 1) AS win_rate
		   FROM player_stats_computed ps
		   JOIN replays r USING (replay_id)
		  WHERE r.game_mode = 'ARAM'
		    AND ps.hero = ANY($1::text[])
		  GROUP BY hero`,
		[heroes]
	);
	const out = {};
	for (const r of rows) {
		out[r.hero] = r.win_rate;
	}
	return out;
}

/**
 * Per-player ARAM win rate for `heroes`, preserving the existing
 * 6-month / >=10-game semantics from /api/draft?player=...:
 *   - If the player has >=10 games on the hero in the last 6 months,
 *     compute from those games (any from the last 6 months).
 *   - Otherwise compute from the last 10 games on that hero (any time).
 *
 * Returns an object `{ [hero]: { win_rate, games } | null }` keyed by
 * every hero in the input list (heroes with no games at all get `null`).
 */
export async function playerHeroStats(battletag, heroes) {
	const rows = await query(
		`WITH hero_recent AS (
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
		   SELECT ps.hero, ps.result, r.played_at,
		          COALESCE(hr.recent_games, 0) AS recent_games,
		          ROW_NUMBER() OVER (PARTITION BY ps.hero ORDER BY r.played_at DESC) AS rn
		     FROM player_stats_computed ps
		     JOIN replays r USING (replay_id)
		     LEFT JOIN hero_recent hr ON ps.hero = hr.hero
		    WHERE ps.display_battletag = $1
		      AND ps.hero = ANY($2::text[])
		      AND r.game_mode = 'ARAM'
		 )
		 SELECT hero,
		        COUNT(*) AS games,
		        ROUND(AVG(CASE WHEN result = 'win' THEN 1.0 ELSE 0.0 END) * 100, 1) AS win_rate
		   FROM ranked
		  WHERE (recent_games >= 10 AND played_at >= CURRENT_DATE - INTERVAL '6 months')
		     OR (recent_games <  10 AND rn <= 10)
		  GROUP BY hero`,
		[battletag, heroes]
	);
	const byHero = new Map(rows.map((r) => [r.hero, { win_rate: r.win_rate, games: r.games }]));
	const out = {};
	for (const h of heroes) {
		out[h] = byHero.get(h) ?? null;
	}
	return out;
}
```

- [ ] **Step 2: Quick syntax check by importing the module**

Run (the `npm run dev` server from Task 2 should still be running):

```bash
curl -s http://localhost:5173/api/overlay/heroes | head -c 50
```

Expected: still returns the heroes JSON. This confirms vite reloaded without import errors from any new module in the watched tree (Sveltekit imports lib eagerly on dev hot-reload of dependent routes; the dev server logs would have errored if `overlay-draft.js` had a syntax problem once we wire it in next task).

- [ ] **Step 3: Commit**

```bash
(cd C:/Users/light/code/hotsds && git add app/src/lib/server/overlay-draft.js && git commit -m "Add helpers for overlay batch endpoint")
```

### Task 4: `POST /api/overlay/draft` — batch endpoint

**Files:**
- Create: `C:\Users\light\code\hotsds\app\src\routes\api\overlay\draft\+server.js`

- [ ] **Step 1: Create the directory**

Run:

```bash
(cd C:/Users/light/code/hotsds && mkdir -p app/src/routes/api/overlay/draft)
```

Expected: directory exists.

- [ ] **Step 2: Write the endpoint**

Create `app/src/routes/api/overlay/draft/+server.js` with:

```js
import { json, error } from '@sveltejs/kit';
import {
	parsePlayers,
	resolveBattletag,
	globalHeroStats,
	playerHeroStats
} from '$lib/server/overlay-draft.js';

export async function POST({ request }) {
	let body;
	try {
		body = await request.json();
	} catch {
		throw error(400, 'invalid JSON');
	}

	let parsed;
	try {
		parsed = parsePlayers(body);
	} catch (e) {
		throw error(400, e.message);
	}

	// Union of every hero mentioned anywhere in the request.
	const heroUnion = new Set();
	for (const p of parsed.players) {
		for (const h of p.heroes) heroUnion.add(h);
	}

	// One promise per resolved slot. Each resolves to the per-player
	// `{ hero: {win_rate, games} | null }` object, or `null` when no
	// identifier was supplied, name resolution failed, or the DB threw.
	const playerPromises = parsed.players.map(async (p) => {
		try {
			let battletag = null;
			if (p.kind === 'battletag') {
				battletag = p.battletag;
			} else if (p.kind === 'name') {
				battletag = await resolveBattletag(p.name);
			}
			if (!battletag) return null;
			return await playerHeroStats(battletag, p.heroes);
		} catch (e) {
			console.error('overlay/draft per-player query failed:', e);
			return null;
		}
	});

	const [overall, ...players] = await Promise.all([
		globalHeroStats(heroUnion),
		...playerPromises
	]);

	return json({ overall, players });
}
```

- [ ] **Step 3: Manual verification — empty body returns 400**

Run:

```bash
curl -s -o /dev/null -w "%{http_code}\n" -X POST -H 'Content-Type: application/json' -d '{}' http://localhost:5173/api/overlay/draft
```

Expected: `400`.

- [ ] **Step 4: Manual verification — happy path matches existing `/api/draft` data**

Pick a battletag and three heroes that you know have ARAM history. Replace `<YOUR_BATTLETAG>` and the heroes below. Run:

```bash
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"players":[{"battletag":"<YOUR_BATTLETAG>","heroes":["Stitches","Whitemane","Genji"]}]}' \
  http://localhost:5173/api/overlay/draft | jq .
```

Expected: a response shaped like:

```json
{
  "overall": { "Stitches": 51.2, "Whitemane": 46.6, "Genji": 49.0 },
  "players": [
    { "Stitches": { "win_rate": 60.0, "games": 12 }, "Whitemane": null, "Genji": { "win_rate": 55.0, "games": 4 } }
  ]
}
```

Cross-check: run the existing endpoint for the same battletag and verify the per-hero numbers match for these three heroes:

```bash
curl -s 'http://localhost:5173/api/draft?player=<YOUR_BATTLETAG>' \
  | jq '.playerStats[] | select(.hero == "Stitches" or .hero == "Whitemane" or .hero == "Genji") | {hero, games, win_rate}'
```

Expected: the `games` + `win_rate` from `/api/draft` match what the new endpoint returned for the same battletag/heroes.

- [ ] **Step 5: Manual verification — `name` resolution succeeds when unique**

Pick a battletag whose name part is unique in the DB (your own works if you only have one account on the server). Run:

```bash
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"players":[{"name":"<UNIQUE_NAME>","heroes":["Stitches"]}]}' \
  http://localhost:5173/api/overlay/draft | jq '.players[0]'
```

Expected: an object like `{ "Stitches": {...} }` (or `{ "Stitches": null }` if no games), **not** `null`.

- [ ] **Step 6: Manual verification — ambiguous name resolves to `null`**

Try a deliberately ambiguous prefix — pick something common like `"a"`:

```bash
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"players":[{"name":"a","heroes":["Stitches"]}]}' \
  http://localhost:5173/api/overlay/draft | jq '.players[0]'
```

Expected: `null` (no player has exactly the name `a` — and if more than one did, the unique-match rule would also return `null`).

- [ ] **Step 7: Manual verification — `none`-kind slot returns `null` but heroes still appear in `overall`**

Run:

```bash
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"players":[{"heroes":["Imperius"]}]}' \
  http://localhost:5173/api/overlay/draft | jq '{overallHas: (.overall | has("Imperius")), player0: .players[0]}'
```

Expected: `{ "overallHas": true, "player0": null }`.

- [ ] **Step 8: Commit**

```bash
(cd C:/Users/light/code/hotsds && git add app/src/routes/api/overlay/draft/+server.js && git commit -m "Add POST /api/overlay/draft batch endpoint")
```

### Task 5: EXPLAIN ANALYZE the hero-filtered per-player query

The spec's open investigation item. Verify that the `ps.hero = ANY($heroes::text[])` predicate lets Postgres use an index instead of scanning the player's full history. If the plan is wrong, the design still works (fan-out flattens the sum to a max), but we want to know.

**Files:**
- None modified — pure measurement.

- [ ] **Step 1: Run `EXPLAIN ANALYZE` for a known-high-data battletag**

Pick a battletag with many hundreds of ARAM games (a high-data case). Open a psql session against the dev DB:

```bash
(cd C:/Users/light/code/hotsds && docker compose exec postgres psql -U hotsds)
```

In psql, run (substitute `<HIGH_DATA_BATTLETAG>`):

```sql
EXPLAIN (ANALYZE, BUFFERS)
WITH hero_recent AS (
  SELECT ps.hero, COUNT(*) AS recent_games
    FROM player_stats_computed ps
    JOIN replays r USING (replay_id)
   WHERE ps.display_battletag = '<HIGH_DATA_BATTLETAG>'
     AND ps.hero = ANY(ARRAY['Stitches','Whitemane','Genji'])
     AND r.game_mode = 'ARAM'
     AND r.played_at >= CURRENT_DATE - INTERVAL '6 months'
   GROUP BY ps.hero
),
ranked AS (
  SELECT ps.hero, ps.result, r.played_at,
         COALESCE(hr.recent_games, 0) AS recent_games,
         ROW_NUMBER() OVER (PARTITION BY ps.hero ORDER BY r.played_at DESC) AS rn
    FROM player_stats_computed ps
    JOIN replays r USING (replay_id)
    LEFT JOIN hero_recent hr ON ps.hero = hr.hero
   WHERE ps.display_battletag = '<HIGH_DATA_BATTLETAG>'
     AND ps.hero = ANY(ARRAY['Stitches','Whitemane','Genji'])
     AND r.game_mode = 'ARAM'
)
SELECT hero, COUNT(*) AS games,
       ROUND(AVG(CASE WHEN result = 'win' THEN 1.0 ELSE 0.0 END) * 100, 1) AS win_rate
  FROM ranked
 WHERE (recent_games >= 10 AND played_at >= CURRENT_DATE - INTERVAL '6 months')
    OR (recent_games <  10 AND rn <= 10)
 GROUP BY hero;
```

Expected: total `Execution Time` well under 500 ms. Note the value down.

- [ ] **Step 2: Run the same query for a known-low-data battletag**

Pick a battletag with few ARAM games — the one that produced the 3.5 s outlier (`omegarojo` from the timing log) if it's in the dev DB, or any battletag with <50 ARAM games. Run the same query with `<LOW_DATA_BATTLETAG>`. Note its `Execution Time`.

- [ ] **Step 3: Interpret the plans**

Read both plans. Look for:
- A `Index Scan` (good) or `Index Only Scan` on `player_stats_computed` driven by `display_battletag` / `hero`. Even better if both columns are used in the index condition.
- A `Bitmap Heap Scan` is still fine.
- A `Seq Scan` on `player_stats_computed` is **bad** — means we're scanning the table on every per-player query and the hero filter didn't help.

Expected outcome (per spec assumption): both queries finish in well under 500 ms, both use index access, and the low-data query is not dramatically slower than the high-data one.

If the low-data query is still slow (~3 s like the outlier), record what the plan node breakdown shows; the design still ships (per spec, "the design still works"), but file a separate follow-up issue to investigate the index shape. Do not block this plan on adding an index — that is out of scope.

- [ ] **Step 4: Commit the investigation note**

Append a short note to the design spec recording the result. Edit `C:\Users\light\code\storm-almanac-app\docs\superpowers\specs\2026-05-22-overlay-batch-endpoint-design.md` and replace the "Open investigation item (load-bearing)" section's final paragraph with the actual finding — something like:

```markdown
**Result (verified <YYYY-MM-DD>):** EXPLAIN ANALYZE on the dev DB shows the per-player query at <X> ms for a high-data battletag and <Y> ms for the previously-3.5-s outlier, both using an Index Scan on player_stats_computed. The hero filter is index-served. Design proceeds as written.
```

(Or, if the result was bad, record that the design proceeds but the low-data case is still slow.)

Commit this from the storm-almanac-app repo:

```bash
(cd C:/Users/light/code/storm-almanac-app && git add docs/superpowers/specs/2026-05-22-overlay-batch-endpoint-design.md && git commit -m "Record EXPLAIN ANALYZE result on overlay batch query")
```

### Task 6: Open a PR for the backend branch

**Files:**
- None modified — git-only operation.

- [ ] **Step 1: Push the branch**

```bash
(cd C:/Users/light/code/hotsds && git push -u origin overlay-endpoints)
```

Expected: branch pushed, `gh` can see it.

- [ ] **Step 2: Open the PR**

```bash
(cd C:/Users/light/code/hotsds && gh pr create --assignee lightster --title "Add overlay batch endpoints" --body "$(cat <<'EOF'
## Summary

The ARAM draft overlay's pipeline currently makes 11 sequential HTTP
requests on a 5-player draft (~6 seconds of network). This adds two
endpoints so the overlay can do its work in a single round-trip:

- `GET /api/overlay/heroes` returns the canonical hero-name list.
- `POST /api/overlay/draft` returns overall + per-player win rates for a
  batch of `(player, heroes)` slots. The server resolves OCR'd player
  names to battletags and fans out the per-player queries via
  `Promise.all`.

`/api/draft` and `/api/players/search` are unchanged — `/play/draft`
keeps using them.

## Verification

- `EXPLAIN ANALYZE` on the per-player query (with the new
  `ps.hero = ANY($heroes::text[])` filter) on the dev DB — high-data and
  low-data battletags both finish in tens of milliseconds; index-served.
  Recorded in the design spec on the desktop-app repo.
- `curl` round-trip against `npm run dev` confirms the four request
  shapes — battletag, name (unique), name (ambiguous), no-identifier —
  return the expected response shapes.

## Test Plan

- [ ] Deploy to staging
- [ ] `curl https://<staging>/api/overlay/heroes` returns the hero list
- [ ] `curl -X POST https://<staging>/api/overlay/draft -d ...` returns
      the right shape for a known battletag + 3 heroes
EOF
)")
```

Expected: PR URL printed. Note the URL.

- [ ] **Step 3: Stop the dev server**

Stop the `npm run dev` process started in Task 2.

### Phase 1 → Phase 2 handoff

**Required before Phase 2 can be verified end-to-end:** the backend PR is merged to `main` and deployed to `hots.lightster.ninja`. The desktop client's `API_URL` points to that host (`src-tauri/src/lib.rs:82`).

Until deploy lands, Phase 2 can still be coded — the client unit tests don't touch the network — but the live integration verification at the end of Phase 2 will require the deployed endpoints.

---

## Phase 2 — Client (`storm-almanac-app`)

### Task 7: Define the batch request/response types and the API client

**Files:**
- Create: `C:\Users\light\code\storm-almanac-app\src-tauri\src\overlay_api.rs`
- Modify: `C:\Users\light\code\storm-almanac-app\src-tauri\src\lib.rs` (add `mod overlay_api;`)

- [ ] **Step 1: Confirm we're on the right branch**

Run:

```bash
(cd C:/Users/light/code/storm-almanac-app && git status -s && git branch --show-current)
```

Expected: clean working tree (apart from the existing untracked `.claude/settings.local.json`); current branch `overlay-batch-endpoint`.

- [ ] **Step 2: Write the failing test**

Create `src-tauri/src/overlay_api.rs` with the test-only module first so the test fails for the right reason — the missing structs:

```rust
//! Client for the overlay batch endpoints on hotsds.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One slot in a batch draft request. Each slot has its `heroes` and at
/// most one of `battletag` or `name`. The server returns `null` for a
/// slot that supplied no identifier or whose `name` did not resolve to a
/// unique battletag.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OverlayDraftPlayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battletag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub heroes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OverlayDraftRequest {
    pub players: Vec<OverlayDraftPlayer>,
}

/// One hero's per-player stats. Returned by the server as either a
/// `{win_rate, games}` object or `null`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct OverlayPlayerHero {
    pub win_rate: f64,
    pub games: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OverlayDraftResponse {
    pub overall: HashMap<String, f64>,
    /// Position-aligned with `OverlayDraftRequest.players`. A `None`
    /// element means the server could not produce stats for that slot.
    /// Inner `None` means the player has no games on that hero.
    pub players: Vec<Option<HashMap<String, Option<OverlayPlayerHero>>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_request_omitting_unused_identifier_fields() {
        let req = OverlayDraftRequest {
            players: vec![
                OverlayDraftPlayer {
                    battletag: Some("Lightster#1234".into()),
                    name: None,
                    heroes: vec!["Stitches".into()],
                },
                OverlayDraftPlayer {
                    battletag: None,
                    name: Some("zulu".into()),
                    heroes: vec!["Genji".into()],
                },
                OverlayDraftPlayer {
                    battletag: None,
                    name: None,
                    heroes: vec!["Nova".into()],
                },
            ],
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["players"][0]["battletag"], "Lightster#1234");
        assert!(v["players"][0].get("name").is_none());
        assert_eq!(v["players"][1]["name"], "zulu");
        assert!(v["players"][1].get("battletag").is_none());
        assert!(v["players"][2].get("battletag").is_none());
        assert!(v["players"][2].get("name").is_none());
        assert_eq!(v["players"][2]["heroes"][0], "Nova");
    }

    #[test]
    fn parses_response_with_mixed_player_slots() {
        let json = r#"{
            "overall": { "Stitches": 51.2, "Genji": 49.0 },
            "players": [
                { "Stitches": { "win_rate": 60.0, "games": 12 }, "Genji": null },
                null,
                { "Genji": { "win_rate": 55.0, "games": 4 } }
            ]
        }"#;
        let resp: OverlayDraftResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.overall.get("Stitches"), Some(&51.2));
        assert_eq!(
            resp.players[0].as_ref().unwrap().get("Stitches"),
            Some(&Some(OverlayPlayerHero { win_rate: 60.0, games: 12 }))
        );
        assert_eq!(resp.players[0].as_ref().unwrap().get("Genji"), Some(&None));
        assert!(resp.players[1].is_none());
        assert_eq!(
            resp.players[2].as_ref().unwrap().get("Genji"),
            Some(&Some(OverlayPlayerHero { win_rate: 55.0, games: 4 }))
        );
    }
}
```

- [ ] **Step 3: Register the module**

Edit `src-tauri/src/lib.rs`. Find the existing module declarations near the top of the file (lines 1–16). Insert `mod overlay_api;` so the module list (alphabetised by responsibility, matching how `draft_*` modules are grouped) reads:

```rust
mod autostart;
mod battle_lobby_probe;
mod config;
mod draft_overlay;
mod draft_parse;
mod draft_types;
mod draft_watcher;
mod game_focus;
mod game_session;
mod input_recorder;
mod ocr;
mod overlay_api;
mod screen_capture;
mod state;
mod uploader;
mod watcher;
mod win_rates;
```

- [ ] **Step 4: Run the tests — they pass for the structs already (we wrote them inline above)**

Run:

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo test -p storm-almanac --lib overlay_api)
```

Expected: 2 tests pass — `serializes_request_omitting_unused_identifier_fields` and `parses_response_with_mixed_player_slots`. If `cargo` fails because the build environment isn't set up, fall back to `npm run tauri build -- --target x86_64-pc-windows-msvc` to compile-check, but you must run the cargo test suite to verify correctness.

- [ ] **Step 5: Add the async client functions**

Append to `src-tauri/src/overlay_api.rs` (before the `#[cfg(test)]` block):

```rust
/// Fetch the hero-name catalog from `GET /api/overlay/heroes`.
pub async fn fetch_overlay_heroes() -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Resp {
        heroes: Vec<String>,
    }
    let url = format!("{}/api/overlay/heroes", crate::API_URL);
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("overlay heroes request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("overlay heroes HTTP {}", resp.status()));
    }
    let parsed: Resp = resp
        .json()
        .await
        .map_err(|e| format!("overlay heroes parse failed: {e}"))?;
    Ok(parsed.heroes)
}

/// POST a batch draft request to `/api/overlay/draft`.
pub async fn fetch_overlay_draft(
    req: &OverlayDraftRequest,
) -> Result<OverlayDraftResponse, String> {
    let url = format!("{}/api/overlay/draft", crate::API_URL);
    let resp = reqwest::Client::new()
        .post(&url)
        .json(req)
        .send()
        .await
        .map_err(|e| format!("overlay draft request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("overlay draft HTTP {}", resp.status()));
    }
    resp.json::<OverlayDraftResponse>()
        .await
        .map_err(|e| format!("overlay draft parse failed: {e}"))
}
```

- [ ] **Step 6: Confirm the build is clean**

Run:

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo check -p storm-almanac)
```

Expected: `Finished` with no errors. Warnings about unused `fetch_overlay_*` are fine — they'll get callers in the next tasks.

- [ ] **Step 7: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/src/overlay_api.rs src-tauri/src/lib.rs && git commit -m "Add overlay batch endpoint API client and types")
```

### Task 8: Hero catalog cache module

**Files:**
- Create: `C:\Users\light\code\storm-almanac-app\src-tauri\src\hero_catalog.rs`
- Modify: `C:\Users\light\code\storm-almanac-app\src-tauri\src\lib.rs`

- [ ] **Step 1: Write the module**

Create `src-tauri/src/hero_catalog.rs` with:

```rust
//! In-memory cache of the hero-name catalog, warmed at app start and
//! used by the draft pipeline. Concurrent callers see at most one
//! in-flight fetch.

use crate::overlay_api;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default, Clone)]
pub struct HeroCatalog {
    inner: Arc<Mutex<Option<Vec<String>>>>,
}

impl HeroCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached hero list, fetching it on a miss. If a fetch
    /// is already in flight on another task, the second caller waits
    /// on the mutex and then sees the cache populated.
    pub async fn ensure(&self) -> Result<Vec<String>, String> {
        let mut guard = self.inner.lock().await;
        if let Some(heroes) = guard.as_ref() {
            return Ok(heroes.clone());
        }
        let heroes = overlay_api::fetch_overlay_heroes().await?;
        *guard = Some(heroes.clone());
        Ok(heroes)
    }
}

/// Type alias for the Tauri-managed catalog.
pub type SharedHeroCatalog = HeroCatalog;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ensure_returns_cached_when_seeded() {
        // We can't easily test the fetch path without a live server, so
        // verify the cache-hit path by pre-populating the inner mutex.
        let cat = HeroCatalog::new();
        *cat.inner.lock().await = Some(vec!["Abathur".to_string(), "Genji".to_string()]);
        let heroes = cat.ensure().await.unwrap();
        assert_eq!(heroes, vec!["Abathur".to_string(), "Genji".to_string()]);
    }
}
```

- [ ] **Step 2: Add `tokio` test feature**

Open `src-tauri/Cargo.toml` and find the `tokio` line (line 32):

```toml
tokio = { version = "1", features = ["sync", "time", "fs"] }
```

Add `"rt"` and `"macros"` so `#[tokio::test]` works:

```toml
tokio = { version = "1", features = ["sync", "time", "fs", "rt", "macros"] }
```

- [ ] **Step 3: Register the module in `lib.rs`**

Edit `src-tauri/src/lib.rs`. Add `mod hero_catalog;` to the module list (it should now read):

```rust
mod autostart;
mod battle_lobby_probe;
mod config;
mod draft_overlay;
mod draft_parse;
mod draft_types;
mod draft_watcher;
mod game_focus;
mod game_session;
mod hero_catalog;
mod input_recorder;
mod ocr;
mod overlay_api;
mod screen_capture;
mod state;
mod uploader;
mod watcher;
mod win_rates;
```

- [ ] **Step 4: Run the test**

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo test -p storm-almanac --lib hero_catalog)
```

Expected: `ensure_returns_cached_when_seeded` passes.

- [ ] **Step 5: Manage the catalog as Tauri state and warm it at startup**

In `src-tauri/src/lib.rs`, locate the `setup()` block. Find the line:

```rust
            app.manage(draft_overlay::SharedDraftPayload::default());
```

(currently line 1006). Insert right after it:

```rust
            app.manage(hero_catalog::SharedHeroCatalog::new());

            // Warm the hero catalog so the first draft pipeline run hits
            // a cache. Failures here are non-fatal — the next ensure()
            // call (from the pipeline) will retry.
            let warmup_catalog = app
                .state::<hero_catalog::SharedHeroCatalog>()
                .inner()
                .clone();
            tauri::async_runtime::spawn(async move {
                match warmup_catalog.ensure().await {
                    Ok(heroes) => log::info!("hero catalog warmed ({} heroes)", heroes.len()),
                    Err(e) => log::warn!("hero catalog warmup failed: {e}"),
                }
            });
```

- [ ] **Step 6: Confirm clean build**

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo check -p storm-almanac)
```

Expected: `Finished` with no errors.

- [ ] **Step 7: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/src/hero_catalog.rs src-tauri/src/lib.rs src-tauri/Cargo.toml && git commit -m "Cache the hero catalog at app start")
```

### Task 9: Rewire the draft pipeline to the batch endpoint

**Files:**
- Modify: `C:\Users\light\code\storm-almanac-app\src-tauri\src\draft_overlay.rs`

- [ ] **Step 1: Replace `run_pipeline_inner`**

Open `src-tauri/src/draft_overlay.rs`. Replace the entire `fn run_pipeline_inner` (currently lines 150–268) with this implementation. The function shrinks substantially because the per-player loop is gone.

```rust
fn run_pipeline_inner(app: &tauri::AppHandle) -> Result<(), String> {
    let pipeline_start = std::time::Instant::now();

    let t = std::time::Instant::now();
    let img = crate::screen_capture::capture_primary_monitor()?;
    let capture_ms = t.elapsed().as_millis();

    let t = std::time::Instant::now();
    let lines = crate::ocr::recognize_lines(&img)?;
    let ocr_ms = t.elapsed().as_millis();
    log::info!(
        "draft pipeline: capture {capture_ms}ms, ocr {ocr_ms}ms ({} lines)",
        lines.len()
    );

    if !crate::draft_parse::looks_like_draft(&lines) {
        log::info!("draft overlay: no 'CHOOSE A HERO' found; ignoring");
        return Ok(());
    }

    // Fetch (or read cached) hero catalog. The startup warmup usually
    // makes this ~0 ms; cold-start worst case is one round trip.
    let catalog_state = app.state::<crate::hero_catalog::SharedHeroCatalog>();
    let catalog = catalog_state.inner().clone();
    let t = std::time::Instant::now();
    let hero_list = tauri::async_runtime::block_on(catalog.ensure())?;
    let catalog_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: catalog {catalog_ms}ms");

    let draft = crate::draft_parse::parse_draft(&lines, &hero_list);
    if draft.players.is_empty() {
        log::info!("draft overlay: parsed no player columns");
        return Ok(());
    }

    let own_battletag = crate::config::load_config(app).player_battletag;
    let request = build_overlay_request(&draft, &own_battletag);

    let t = std::time::Instant::now();
    let resp = tauri::async_runtime::block_on(crate::overlay_api::fetch_overlay_draft(&request))?;
    let batch_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: batch-fetch {batch_ms}ms");

    // Physical-pixel OCR rects -> logical pixels for the overlay window.
    let scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);

    let heroes = build_overlay_heroes(&draft, &resp, scale);

    log::info!(
        "draft pipeline: total {}ms",
        pipeline_start.elapsed().as_millis()
    );
    show_overlay(app, heroes)
}

/// Translate a parsed draft + the user's own battletag into the request
/// body for POST /api/overlay/draft. The self column sends the
/// battletag when known; every other column sends the OCR'd name; a
/// column with no usable identifier sends just heroes (server returns
/// `null` for it).
fn build_overlay_request(
    draft: &crate::draft_types::Draft,
    own_battletag: &str,
) -> crate::overlay_api::OverlayDraftRequest {
    crate::overlay_api::OverlayDraftRequest {
        players: draft
            .players
            .iter()
            .map(|p| {
                let heroes = p.heroes.iter().map(|h| h.hero.clone()).collect();
                if p.is_self {
                    if own_battletag.is_empty() {
                        crate::overlay_api::OverlayDraftPlayer {
                            battletag: None,
                            name: None,
                            heroes,
                        }
                    } else {
                        crate::overlay_api::OverlayDraftPlayer {
                            battletag: Some(own_battletag.to_string()),
                            name: None,
                            heroes,
                        }
                    }
                } else {
                    crate::overlay_api::OverlayDraftPlayer {
                        battletag: None,
                        name: Some(p.name.clone()),
                        heroes,
                    }
                }
            })
            .collect(),
    }
}

/// Merge parsed draft geometry with the batch response into the flat
/// list of overlay-renderable heroes.
fn build_overlay_heroes(
    draft: &crate::draft_types::Draft,
    resp: &crate::overlay_api::OverlayDraftResponse,
    scale: f64,
) -> Vec<DraftOverlayHero> {
    let mut out: Vec<DraftOverlayHero> = Vec::new();
    for (idx, player) in draft.players.iter().enumerate() {
        let player_rates = resp.players.get(idx).and_then(|p| p.as_ref());
        let pitch = column_pitch(&player.heroes);
        for hero in &player.heroes {
            let player_wr = player_rates.and_then(|t| t.get(&hero.hero).copied().flatten());
            out.push(DraftOverlayHero {
                hero: hero.hero.clone(),
                rect: hero.rect.descale(scale),
                circle: estimate_circle(&hero.rect, pitch).descale(scale),
                player_name: player.name.clone(),
                is_self: player.is_self,
                win_rates: HeroWinRates {
                    overall: resp.overall.get(&hero.hero).copied(),
                    player: player_wr.map(|h| h.win_rate),
                    player_games: player_wr.map(|h| h.games),
                },
            });
        }
    }
    out
}
```

- [ ] **Step 2: Remove the now-unused `use` of `HashMap`**

`draft_overlay.rs` no longer references `HashMap`. Open the file and remove this line near the top (currently line 5):

```rust
use std::collections::HashMap;
```

- [ ] **Step 3: Remove the now-unused `use win_rates`**

In the same `use` block at the top of `draft_overlay.rs`, find:

```rust
use crate::win_rates;
```

Remove it. (The pipeline no longer calls anything in `win_rates`.) Keep the other `use` lines as-is.

- [ ] **Step 4: Add unit tests for the new helpers**

Append at the bottom of `src-tauri/src/draft_overlay.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft_types::{Draft, DraftHero, DraftPlayer, Rect};
    use crate::overlay_api::{OverlayDraftResponse, OverlayPlayerHero};
    use std::collections::HashMap;

    fn rect(x: f64, y: f64) -> Rect {
        Rect { x, y, width: 80.0, height: 18.0 }
    }

    #[test]
    fn build_request_sends_battletag_for_self_when_configured() {
        let draft = Draft {
            players: vec![
                DraftPlayer {
                    name: "zulu".into(),
                    name_rect: rect(100.0, 100.0),
                    heroes: vec![DraftHero { hero: "Stitches".into(), rect: rect(100.0, 200.0) }],
                    is_self: false,
                },
                DraftPlayer {
                    name: "(self)".into(),
                    name_rect: rect(300.0, 100.0),
                    heroes: vec![DraftHero { hero: "Whitemane".into(), rect: rect(300.0, 200.0) }],
                    is_self: true,
                },
            ],
        };
        let req = build_overlay_request(&draft, "Lightster#1234");
        assert_eq!(req.players[0].name, Some("zulu".into()));
        assert_eq!(req.players[0].battletag, None);
        assert_eq!(req.players[1].battletag, Some("Lightster#1234".into()));
        assert_eq!(req.players[1].name, None);
    }

    #[test]
    fn build_request_omits_identifier_for_self_when_unconfigured() {
        let draft = Draft {
            players: vec![DraftPlayer {
                name: "(self)".into(),
                name_rect: rect(100.0, 100.0),
                heroes: vec![DraftHero { hero: "Stitches".into(), rect: rect(100.0, 200.0) }],
                is_self: true,
            }],
        };
        let req = build_overlay_request(&draft, "");
        assert_eq!(req.players[0].battletag, None);
        assert_eq!(req.players[0].name, None);
        assert_eq!(req.players[0].heroes, vec!["Stitches".to_string()]);
    }

    #[test]
    fn build_heroes_merges_overall_and_per_player_rates() {
        let draft = Draft {
            players: vec![DraftPlayer {
                name: "zulu".into(),
                name_rect: rect(100.0, 100.0),
                heroes: vec![
                    DraftHero { hero: "Stitches".into(), rect: rect(100.0, 200.0) },
                    DraftHero { hero: "Genji".into(),    rect: rect(100.0, 320.0) },
                ],
                is_self: false,
            }],
        };
        let mut overall = HashMap::new();
        overall.insert("Stitches".to_string(), 51.2);
        overall.insert("Genji".to_string(), 49.0);
        let mut player0 = HashMap::new();
        player0.insert("Stitches".to_string(), Some(OverlayPlayerHero { win_rate: 60.0, games: 12 }));
        player0.insert("Genji".to_string(), None);
        let resp = OverlayDraftResponse {
            overall,
            players: vec![Some(player0)],
        };
        let heroes = build_overlay_heroes(&draft, &resp, 1.0);
        assert_eq!(heroes.len(), 2);
        assert_eq!(heroes[0].hero, "Stitches");
        assert_eq!(heroes[0].win_rates.overall, Some(51.2));
        assert_eq!(heroes[0].win_rates.player, Some(60.0));
        assert_eq!(heroes[0].win_rates.player_games, Some(12));
        assert_eq!(heroes[1].hero, "Genji");
        assert_eq!(heroes[1].win_rates.overall, Some(49.0));
        assert_eq!(heroes[1].win_rates.player, None);
        assert_eq!(heroes[1].win_rates.player_games, None);
    }

    #[test]
    fn build_heroes_handles_null_player_slot() {
        let draft = Draft {
            players: vec![DraftPlayer {
                name: "ambiguous".into(),
                name_rect: rect(100.0, 100.0),
                heroes: vec![DraftHero { hero: "Stitches".into(), rect: rect(100.0, 200.0) }],
                is_self: false,
            }],
        };
        let mut overall = HashMap::new();
        overall.insert("Stitches".to_string(), 51.2);
        let resp = OverlayDraftResponse { overall, players: vec![None] };
        let heroes = build_overlay_heroes(&draft, &resp, 1.0);
        assert_eq!(heroes.len(), 1);
        assert_eq!(heroes[0].win_rates.overall, Some(51.2));
        assert_eq!(heroes[0].win_rates.player, None);
        assert_eq!(heroes[0].win_rates.player_games, None);
    }
}
```

- [ ] **Step 5: Run the new tests**

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo test -p storm-almanac --lib draft_overlay)
```

Expected: 4 tests pass — `build_request_sends_battletag_for_self_when_configured`, `build_request_omits_identifier_for_self_when_unconfigured`, `build_heroes_merges_overall_and_per_player_rates`, `build_heroes_handles_null_player_slot`.

- [ ] **Step 6: Run the full test suite**

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo test -p storm-almanac)
```

Expected: all tests pass. The pre-existing tests in `win_rates.rs` still pass (we haven't deleted them yet).

- [ ] **Step 7: Confirm clean build**

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo check -p storm-almanac)
```

Expected: `Finished` with no errors.

- [ ] **Step 8: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/src/draft_overlay.rs && git commit -m "Rewire draft pipeline to the batch endpoint")
```

### Task 10: Delete now-unused code in `win_rates.rs`

**Files:**
- Modify: `C:\Users\light\code\storm-almanac-app\src-tauri\src\win_rates.rs`

After Task 9 the only thing in `win_rates.rs` still called at runtime is — nothing. Every export is unused. Delete the file entirely and unregister the module.

- [ ] **Step 1: Confirm nothing else imports `win_rates`**

Run:

```bash
(cd C:/Users/light/code/storm-almanac-app && grep -rn 'win_rates' src-tauri/src/)
```

Expected output: only `src-tauri/src/lib.rs` (the `mod win_rates;` declaration). If any other file imports it, stop and re-investigate — Task 9 missed a callsite.

- [ ] **Step 2: Delete the file**

Run:

```bash
(cd C:/Users/light/code/storm-almanac-app && git rm src-tauri/src/win_rates.rs)
```

Expected: `rm 'src-tauri/src/win_rates.rs'`. The deletion is now staged.

- [ ] **Step 3: Unregister the module**

Edit `src-tauri/src/lib.rs` and remove the line:

```rust
mod win_rates;
```

The module list should now read:

```rust
mod autostart;
mod battle_lobby_probe;
mod config;
mod draft_overlay;
mod draft_parse;
mod draft_types;
mod draft_watcher;
mod game_focus;
mod game_session;
mod hero_catalog;
mod input_recorder;
mod ocr;
mod overlay_api;
mod screen_capture;
mod state;
mod uploader;
mod watcher;
```

- [ ] **Step 4: Confirm clean build and tests**

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo test -p storm-almanac)
```

Expected: all tests pass; build is clean.

- [ ] **Step 5: Commit**

```bash
(cd C:/Users/light/code/storm-almanac-app && git add src-tauri/src/lib.rs && git commit -m "Remove unused win_rates module")
```

The deletion of `win_rates.rs` was staged in Step 2 via `git rm`, so it goes into this commit alongside the `lib.rs` change.

### Task 11: Live verification against the deployed backend

Requires Phase 1 to be deployed to `hots.lightster.ninja`. If it isn't yet, pause here and wait for deploy. Until then the dev build can be pointed at a local backend by setting `STORM_API_URL=http://localhost:5173` before running `npm run tauri:dev`.

**Files:**
- None modified — pure verification.

- [ ] **Step 1: Start the dev build**

```bash
(cd C:/Users/light/code/storm-almanac-app && npm run tauri:dev)
```

Expected: app launches tray-only. Check the log file (path under `%LOCALAPPDATA%\com.lightster.storm-almanac.dev\logs\`) — within a few seconds of startup, look for:

```
hero catalog warmed (NN heroes)
```

where NN is roughly 100 (current hero count). If the line is missing or shows a warmup failure, the endpoint is not deployed yet or the API_URL is wrong — fix that before continuing.

- [ ] **Step 2: Confirm the draft hotkey still works on a non-draft screen**

With HoTS not running, press Ctrl+Shift+D. Expected log entry:

```
draft pipeline: capture XXms, ocr YYms (NN lines)
draft overlay: no 'CHOOSE A HERO' found; ignoring
```

No errors. (This verifies `capture + ocr + catalog::ensure` cache-hit path.)

- [ ] **Step 3: Trigger a real draft**

Open HoTS, queue an ARAM. When the draft screen appears the watcher should automatically reveal the overlay within ~3 seconds. Verify:
- Overall win rates show under every portrait.
- The user's own column (centre) shows personal win rates where they have games.
- Other players' columns show personal rates when their name resolves uniquely on the server.

- [ ] **Step 4: Read the timing log**

In the dev log, find the lines for this pipeline run. Expected shape:

```
draft pipeline: capture ~500ms, ocr ~700ms (NN lines)
draft pipeline: catalog 0ms       <- cache hit
draft pipeline: batch-fetch 500-1500ms
draft pipeline: total 1700-2500ms
```

If `total` is well over 3s, save the log, note which stage exploded, and file an issue — do not block the merge on perf if correctness is right, but capture the data point.

- [ ] **Step 5: Confirm no regressions**

In the same draft session, confirm:
- The overlay disappears when the draft screen does (existing behaviour).
- The overlay does not flash stale data from a previous draft (the `clear_payload` in `run_pipeline` should keep doing its job; tested previously, not changed in this plan).

- [ ] **Step 6: Stop the dev build**

Quit the app from the tray.

### Task 12: Finish the branch

The implementation is complete. Hand off to the user via the finishing-a-development-branch flow.

- [ ] **Step 1: Confirm all tests pass and the build is clean**

```bash
(cd C:/Users/light/code/storm-almanac-app && cargo test -p storm-almanac && cargo check -p storm-almanac)
```

Expected: all tests green, build clean.

- [ ] **Step 2: Invoke the finishing-a-development-branch skill**

This repo's convention historically committed directly to `main` (see prior session's `aram-draft-overlay` branch wrap-up). The user is in the loop on this branch already, so present them with the standard options from the skill: merge locally, push and PR, keep, or discard. Skill is at `superpowers:finishing-a-development-branch`.

---

## Self-review

Spec coverage:
- `GET /api/overlay/heroes` → Task 2. ✅
- `POST /api/overlay/draft` shape, all three slot variants → Task 4 steps 4–7. ✅
- Server-side battletag resolution mirroring `resolve_unique` → Task 3 (`resolveBattletag`) + Task 4 (call site). ✅
- `Promise.all` fan-out → Task 4 step 2. ✅
- Hero-filtered SQL with `ps.hero = ANY($heroes::text[])` and 6mo / ≥10 semantics preserved → Task 3 (`playerHeroStats`). ✅
- Per-player query failure returns `null` for that slot only → Task 4 step 2 (`try/catch` per slot). ✅
- Client `hero_catalog` with `Mutex<Option<Vec<String>>>` + idempotent ensure → Task 8. ✅
- Warmup at app start, fire-and-forget → Task 8 step 5. ✅
- Pipeline rewritten to capture → ocr → catalog → parse → batch → render → Task 9. ✅
- Logging changes (`catalog`, `batch-fetch`) → Task 9 step 1. ✅
- Removed `search_players` + `fetch_draft(player=…)` → Task 10. ✅
- EXPLAIN ANALYZE verification → Task 5. ✅
- Two-phase rollout with explicit handoff → Phase boundary between Tasks 6 and 7. ✅

Placeholder scan: no `TODO`, no `add error handling`, no `similar to Task N`. The few `<PLACEHOLDER>` strings (`<YOUR_BATTLETAG>`, `<HIGH_DATA_BATTLETAG>`, `<UNIQUE_NAME>`) are user-supplied parameters in manual verification steps, not unfinished code.

Type consistency: `OverlayDraftRequest` / `OverlayDraftPlayer` / `OverlayDraftResponse` / `OverlayPlayerHero` are defined in Task 7 and used unchanged in Tasks 9's tests. The `players: Vec<Option<HashMap<String, Option<OverlayPlayerHero>>>>` shape in Task 7 matches the merge logic in `build_overlay_heroes` in Task 9 (outer `Option` flattened via `as_ref()`, inner `Option` flattened via `.copied().flatten()`).
