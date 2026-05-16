//! ARAM draft overlay orchestration: pipeline, window, hotkey.

use crate::draft_types::{DraftOverlayHero, DraftOverlayPayload, HeroWinRates};
use crate::win_rates;
use std::collections::HashMap;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub const DRAFT_LABEL: &str = "overlay-draft";

/// Holds the most recent draft payload for the overlay window to pull on mount.
pub type SharedDraftPayload = std::sync::Mutex<Option<DraftOverlayPayload>>;

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
    let img = crate::screen_capture::capture_primary_monitor()?;
    let lines = crate::ocr::recognize_lines(&img)?;

    let looks_like_draft = lines
        .iter()
        .any(|l| l.text.to_uppercase().contains("CHOOSE A HERO"));
    if !looks_like_draft {
        log::info!("draft overlay: no 'CHOOSE A HERO' found; ignoring");
        return Ok(());
    }

    let base = tauri::async_runtime::block_on(win_rates::fetch_draft(None))?;
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
    for player in &draft.players {
        // Resolve this column's player to a backend battletag.
        let battletag: Option<String> = if player.is_self {
            if own_battletag.is_empty() {
                None
            } else {
                Some(own_battletag.clone())
            }
        } else {
            match tauri::async_runtime::block_on(win_rates::search_players(&player.name)) {
                Ok(cands) => win_rates::resolve_unique(&player.name, &cands),
                Err(e) => {
                    log::warn!("player search failed for {}: {e}", player.name);
                    None
                }
            }
        };

        // Fetch that player's per-hero ARAM win rates, if resolved.
        let player_table: Option<HashMap<String, (f64, u32)>> = match battletag {
            Some(bt) => match tauri::async_runtime::block_on(win_rates::fetch_draft(Some(&bt))) {
                Ok(resp) => Some(win_rates::build_table(&resp).player),
                Err(e) => {
                    log::warn!("draft fetch failed for {bt}: {e}");
                    None
                }
            },
            None => None,
        };

        for hero in &player.heroes {
            let player_wr = player_table.as_ref().and_then(|t| t.get(&hero.hero).copied());
            heroes.push(DraftOverlayHero {
                hero: hero.hero.clone(),
                rect: hero.rect.descale(scale),
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

    // Store the payload first, then open the window. The overlay pulls the
    // data via the get_draft_overlay_data command once its UI has mounted,
    // so there is no timing race against webview load.
    {
        let state = app.state::<SharedDraftPayload>();
        *state.lock().unwrap() = Some(DraftOverlayPayload { heroes });
    }
    let app2 = app.clone();
    app.run_on_main_thread(move || open_window(&app2))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The overlay window calls this on mount to fetch the latest draft data.
#[tauri::command]
pub fn get_draft_overlay_data(
    state: tauri::State<'_, SharedDraftPayload>,
) -> Option<DraftOverlayPayload> {
    state.lock().unwrap().clone()
}

/// Hotkey handler: if the overlay is open, dismiss it; else run the pipeline.
pub fn toggle(app: &tauri::AppHandle) {
    if app.get_webview_window(DRAFT_LABEL).is_some() {
        close_window(app);
    } else {
        run_pipeline(app.clone());
    }
}
