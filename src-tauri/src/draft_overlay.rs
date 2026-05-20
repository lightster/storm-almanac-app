//! ARAM draft overlay orchestration: pipeline, window, hotkey.

use crate::draft_types::{DraftOverlayHero, DraftOverlayPayload, HeroWinRates};
use crate::win_rates;
use std::collections::HashMap;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const DRAFT_LABEL: &str = "overlay-draft";

/// Estimate the vertical pitch (px) between a column's hero rows.
fn column_pitch(heroes: &[crate::draft_types::DraftHero]) -> f64 {
    if heroes.len() < 2 {
        return heroes.first().map(|h| h.rect.height * 6.0).unwrap_or(120.0);
    }
    let mut gaps: Vec<f64> = heroes
        .windows(2)
        .map(|w| (w[1].rect.y - w[0].rect.y).abs())
        .collect();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
    gaps[gaps.len() / 2]
}

/// Estimate a hero's portrait-circle box from its name rect and the column
/// pitch. HoTS draws the circle centred just above the name label.
fn estimate_circle(name: &crate::draft_types::Rect, pitch: f64) -> crate::draft_types::Rect {
    let diameter = pitch * 0.72;
    let center_x = name.x + name.width / 2.0;
    let bottom = name.y - pitch * 0.05;
    crate::draft_types::Rect {
        x: center_x - diameter / 2.0,
        y: bottom - diameter,
        width: diameter,
        height: diameter,
    }
}

/// Holds the most recent draft payload for the overlay window to pull on mount.
pub type SharedDraftPayload = std::sync::Mutex<Option<DraftOverlayPayload>>;

/// Create the overlay window if it does not exist yet, then persist it. It is
/// built **visible**, mirroring the map blocker: the WebView2 initialises
/// while the window is on screen, so it paints correctly. A window created
/// hidden shows a blank webview the first time it is revealed. After creation
/// the window is only ever shown/hidden, never rebuilt.
pub fn ensure_window(app: &tauri::AppHandle) {
    if app.get_webview_window(DRAFT_LABEL).is_some() {
        return;
    }
    let mut builder = WebviewWindowBuilder::new(
        app,
        DRAFT_LABEL,
        WebviewUrl::App("/overlay?mode=draft".into()),
    )
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .resizable(false)
    .focused(false)
    .focusable(false)
    .visible(true);

    // Size + position the window to cover the primary monitor explicitly.
    // `.maximized(true)` is avoided: applying the maximized state activates
    // the window on Windows, which would steal foreground from HoTS.
    match app.primary_monitor() {
        Ok(Some(m)) => {
            let scale = m.scale_factor();
            let size = m.size();
            let pos = m.position();
            builder = builder
                .inner_size(size.width as f64 / scale, size.height as f64 / scale)
                .position(pos.x as f64 / scale, pos.y as f64 / scale);
        }
        _ => {
            log::warn!("draft overlay: primary monitor unavailable at window creation");
        }
    }

    match builder.build() {
        Ok(_) => log::info!("Created draft overlay window"),
        Err(e) => log::error!("Failed to create draft overlay window: {e}"),
    }
}

/// Hide the overlay window. It persists, hidden, for the next draft.
pub fn hide_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(DRAFT_LABEL) {
        match w.hide() {
            Ok(_) => log::info!("draft overlay: hid window"),
            Err(e) => log::error!("draft overlay: hide failed: {e}"),
        }
    }
}

/// Re-show the overlay window if it exists — used to restore it after a
/// focus-loss hide while a draft is still on screen.
pub fn reveal(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window(DRAFT_LABEL) {
        if let Err(e) = w.show() {
            log::error!("draft overlay: reveal failed: {e}");
        }
    }
}

/// Store the payload, push it to the overlay window, and reveal the window.
fn show_overlay(app: &tauri::AppHandle, heroes: Vec<DraftOverlayHero>) -> Result<(), String> {
    let payload = DraftOverlayPayload { heroes };
    {
        let state = app.state::<SharedDraftPayload>();
        *state.lock().unwrap() = Some(payload.clone());
    }
    ensure_window(app);
    let _ = app.emit_to(DRAFT_LABEL, "draft://update", payload);
    if let Some(w) = app.get_webview_window(DRAFT_LABEL) {
        w.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Drop the previous draft's payload and blank the overlay. Called when a new
/// pipeline run begins: capture + OCR + network take several seconds, during
/// which the watcher may reveal the window — without this, it would show the
/// *previous* draft's win rates pinned to the new draft's (identically laid
/// out) portraits.
fn clear_payload(app: &tauri::AppHandle) {
    {
        let state = app.state::<SharedDraftPayload>();
        *state.lock().unwrap() = None;
    }
    let _ = app.emit_to(
        DRAFT_LABEL,
        "draft://update",
        DraftOverlayPayload { heroes: Vec::new() },
    );
}

/// Run the full pipeline once: capture -> OCR -> parse -> win rates -> emit.
/// Runs on a background thread; never blocks the caller.
pub fn run_pipeline(app: tauri::AppHandle) {
    clear_payload(&app);
    std::thread::spawn(move || {
        if let Err(e) = run_pipeline_inner(&app) {
            log::error!("draft overlay pipeline failed: {e}");
        }
    });
}

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

    let t = std::time::Instant::now();
    let base = tauri::async_runtime::block_on(win_rates::fetch_draft(None))?;
    let base_fetch_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: base-fetch {base_fetch_ms}ms");
    let base_table = win_rates::build_table(&base);

    let draft = crate::draft_parse::parse_draft(&lines, &base_table.hero_names);
    if draft.players.is_empty() {
        log::info!("draft overlay: parsed no player columns");
        return Ok(());
    }

    let own_battletag = crate::config::load_config(app).player_battletag;

    // Physical-pixel OCR rects -> logical pixels for the overlay window.
    let scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);

    let mut heroes: Vec<DraftOverlayHero> = Vec::new();
    for (idx, player) in draft.players.iter().enumerate() {
        let player_start = std::time::Instant::now();

        // Resolve this column's player to a backend battletag.
        let mut search_ms: u128 = 0;
        let battletag: Option<String> = if player.is_self {
            if own_battletag.is_empty() {
                None
            } else {
                Some(own_battletag.clone())
            }
        } else {
            let t = std::time::Instant::now();
            let result =
                match tauri::async_runtime::block_on(win_rates::search_players(&player.name)) {
                    Ok(cands) => win_rates::resolve_unique(&player.name, &cands),
                    Err(e) => {
                        log::warn!("player search failed for {}: {e}", player.name);
                        None
                    }
                };
            search_ms = t.elapsed().as_millis();
            result
        };

        // Fetch that player's per-hero ARAM win rates, if resolved.
        let mut fetch_ms: u128 = 0;
        let player_table: Option<HashMap<String, (f64, u32)>> = match battletag {
            Some(bt) => {
                let t = std::time::Instant::now();
                let result =
                    match tauri::async_runtime::block_on(win_rates::fetch_draft(Some(&bt))) {
                        Ok(resp) => Some(win_rates::build_table(&resp).player),
                        Err(e) => {
                            log::warn!("draft fetch failed for {bt}: {e}");
                            None
                        }
                    };
                fetch_ms = t.elapsed().as_millis();
                result
            }
            None => None,
        };

        log::info!(
            "draft pipeline: player[{idx}] {:?} total {}ms (search {}ms, fetch {}ms)",
            player.name,
            player_start.elapsed().as_millis(),
            search_ms,
            fetch_ms,
        );

        let pitch = column_pitch(&player.heroes);
        for hero in &player.heroes {
            let player_wr = player_table.as_ref().and_then(|t| t.get(&hero.hero).copied());
            heroes.push(DraftOverlayHero {
                hero: hero.hero.clone(),
                rect: hero.rect.descale(scale),
                circle: estimate_circle(&hero.rect, pitch).descale(scale),
                player_name: player.name.clone(),
                is_self: player.is_self,
                win_rates: HeroWinRates {
                    overall: base_table.overall.get(&hero.hero).copied(),
                    player: player_wr.map(|(wr, _)| wr),
                    player_games: player_wr.map(|(_, g)| g),
                },
            });
        }
    }

    log::info!(
        "draft pipeline: total {}ms",
        pipeline_start.elapsed().as_millis()
    );
    show_overlay(app, heroes)
}

/// The overlay window calls this on mount to fetch the latest draft data.
#[tauri::command]
pub fn get_draft_overlay_data(
    state: tauri::State<'_, SharedDraftPayload>,
) -> Option<DraftOverlayPayload> {
    state.lock().unwrap().clone()
}

/// Hotkey handler: if the overlay is showing, hide it; else run the pipeline.
pub fn toggle(app: &tauri::AppHandle) {
    let showing = app
        .get_webview_window(DRAFT_LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if showing {
        hide_window(app);
    } else {
        run_pipeline(app.clone());
    }
}
