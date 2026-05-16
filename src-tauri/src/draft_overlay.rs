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

    let own_battletag = {
        let cfg = crate::config::load_config(app);
        cfg.player_battletag
    };
    let own_name_part = win_rates::battletag_name_part(&own_battletag).to_string();

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

    let app2 = app.clone();
    app.run_on_main_thread(move || open_window(&app2))
        .map_err(|e| e.to_string())?;
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
