//! ARAM draft overlay orchestration: pipeline, window, hotkey.

use crate::draft_types::{DraftOverlayHero, DraftOverlayPayload, HeroWinRates};
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
    let t = std::time::Instant::now();
    let img = crate::screen_capture::capture_primary_monitor()?;
    let capture_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: capture {capture_ms}ms");
    run_pipeline_with_image(app, img, None)
}

/// Shared pipeline body: OCR through render. Used by both the live
/// pipeline (image from `capture_primary_monitor`) and test mode
/// (image loaded from a PNG fixture).
///
/// `extra_descale` is multiplied into the existing DPI descale when
/// rendering badge coordinates. Test mode passes `Some(scale)` when
/// the displayed PNG is smaller than its native resolution (because
/// it was scaled down to fit the monitor); the live pipeline always
/// passes `None`.
pub(crate) fn run_pipeline_with_image(
    app: &tauri::AppHandle,
    img: image::RgbaImage,
    extra_descale: Option<f64>,
) -> Result<(), String> {
    let pipeline_start = std::time::Instant::now();

    let t = std::time::Instant::now();
    let lines = crate::ocr::recognize_lines(&img)?;
    let ocr_ms = t.elapsed().as_millis();
    log::info!("draft pipeline: ocr {ocr_ms}ms ({} lines)", lines.len());

    if !crate::draft_parse::looks_like_draft(&lines) {
        log::info!("draft overlay: no 'CHOOSE A HERO' found; ignoring");
        return Ok(());
    }

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

    let dpi_scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let combined_scale = dpi_scale / extra_descale.unwrap_or(1.0);

    let heroes = build_overlay_heroes(&draft, &resp, combined_scale);

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
