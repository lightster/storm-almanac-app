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

use config::{load_config, load_history, load_known_hashes, save_known_hashes, save_config, AppConfig};
use serde::{Deserialize, Serialize};
use state::{
    AppState, RecordingStatus, SharedRecordingState, SharedState, UploadChannels, UploadEntry,
    UploadSemaphore,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{
    image::Image,
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder,
};
#[cfg(target_os = "macos")]
use tauri::menu::SubmenuBuilder;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_store::StoreExt;
use tauri_plugin_updater::UpdaterExt;

#[tauri::command]
fn get_uploads(state: tauri::State<'_, SharedState>) -> Vec<UploadEntry> {
    let state = state.lock().unwrap();
    state.uploads.iter().cloned().collect()
}

#[tauri::command]
fn watch_uploads(
    state: tauri::State<'_, SharedState>,
    channels: tauri::State<'_, UploadChannels>,
    on_event: tauri::ipc::Channel<Vec<UploadEntry>>,
) {
    let entries: Vec<UploadEntry> = {
        let state = state.lock().unwrap();
        state.uploads.iter().cloned().collect()
    };
    let _ = on_event.send(entries);

    let mut chans = channels.lock().unwrap();
    chans.push(on_event);
}

#[tauri::command]
fn get_config(app: tauri::AppHandle) -> AppConfig {
    load_config(&app)
}

#[tauri::command]
fn save_config_cmd(app: tauri::AppHandle, config: AppConfig) {
    save_config(&app, &config);
}

const SETTINGS_LABEL: &str = "settings";
const SETTINGS_WIDTH: f64 = 400.0;
const SETTINGS_HEIGHT: f64 = 400.0;

const WEBSITE_LABEL: &str = "website";
const WEBSITE_URL: &str = match option_env!("STORM_WEBSITE_URL") {
    Some(url) => url,
    None => "https://hots.lightster.ninja",
};
pub const API_URL: &str = match option_env!("STORM_API_URL") {
    Some(url) => url,
    None => "https://hots.lightster.ninja",
};
const WEBSITE_WIDTH: f64 = 1024.0;
const WEBSITE_HEIGHT: f64 = 768.0;

const OVERLAY_INTERACTIVE_LABEL: &str = "overlay-interactive";
const OVERLAY_CLICKTHROUGH_LABEL: &str = "overlay-clickthrough";
const OVERLAY_WIDTH: f64 = 280.0;
const OVERLAY_HEIGHT: f64 = 140.0;

const BLOCKER_LABEL: &str = "overlay-blocker";
const BLOCKER_STORE_FILE: &str = "storm-almanac.json";
const BLOCKER_STORE_KEY: &str = "map_blocker";
const BLOCKER_HOTKEY: &str = "CmdOrCtrl+Shift+B";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum BlockerVisualMode {
    Blocking,
    Interactable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlockerRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlockerSettings {
    enabled: bool,
    // Default geometry — used until a per-map rect is loaded for the
    // active battleground.
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    // Per-map rect overrides, keyed by the .s2ma cache hash extracted
    // from HoTS's battlelobby file.
    #[serde(default)]
    per_map: std::collections::HashMap<String, BlockerRect>,
}

impl Default for BlockerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            x: 50.0,
            y: 600.0,
            width: 250.0,
            height: 200.0,
            per_map: std::collections::HashMap::new(),
        }
    }
}

struct BlockerState {
    settings: BlockerSettings,
    mode: BlockerVisualMode,
    /// Hash of the currently active battleground (from the most recent
    /// battlelobby read). When set, geometry changes are saved under this
    /// key in `settings.per_map`.
    active_map_hash: Option<String>,
    /// Whether HoTS is currently in a match. Set when a battlelobby file is
    /// written (match starting), cleared when a .StormReplay is written
    /// (match ended). The blocker only shows in Blocking mode while true,
    /// so it doesn't cover the menu / draft / post-match voting screen.
    in_game: bool,
}

impl BlockerState {
    fn new(settings: BlockerSettings) -> Self {
        Self {
            settings,
            mode: BlockerVisualMode::Blocking,
            active_map_hash: None,
            in_game: false,
        }
    }
}

type SharedBlockerState = Mutex<BlockerState>;

async fn check_for_updates(
    app: tauri::AppHandle,
    menu_item: tauri::menu::MenuItem<tauri::Wry>,
    update_available: Arc<AtomicBool>,
) {
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            log::error!("Failed to create updater: {}", e);
            return;
        }
    };
    match updater.check().await {
        Ok(Some(update)) => {
            let _ = menu_item.set_text(format!("Update to v{}", update.version));
            update_available.store(true, Ordering::SeqCst);
        }
        Ok(None) => {}
        Err(e) => {
            log::error!("Update check failed: {}", e);
        }
    }
}

async fn install_update(app: tauri::AppHandle) {
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            log::error!("Failed to create updater: {}", e);
            return;
        }
    };
    match updater.check().await {
        Ok(Some(update)) => {
            match update.download_and_install(|_, _| {}, || {}).await {
                Ok(()) => {
                    app.restart();
                }
                Err(e) => {
                    log::error!("Update install failed: {}", e);
                }
            }
        }
        Ok(None) => {
            log::info!("No update available");
        }
        Err(e) => {
            log::error!("Update check failed: {}", e);
        }
    }
}

fn open_website_window(app: &tauri::AppHandle, path: Option<&str>) {
    let full_url: String = match path {
        Some(p) => format!("{}{}", WEBSITE_URL, p),
        None => WEBSITE_URL.to_string(),
    };

    if let Some(window) = app.get_webview_window(WEBSITE_LABEL) {
        if path.is_some() {
            let url: tauri::Url = full_url.parse().unwrap();
            let _ = window.navigate(url);
        }
        let _ = window.set_focus();
        return;
    }

    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    let url = WebviewUrl::External(full_url.parse().unwrap());
    let window = WebviewWindowBuilder::new(app, WEBSITE_LABEL, url)
        .title("Storm Almanac — Website")
        .inner_size(WEBSITE_WIDTH, WEBSITE_HEIGHT)
        .resizable(true)
        .decorations(true)
        .skip_taskbar(false)
        .visible(true)
        .build();

    if let Ok(win) = window {
        let _ = win.set_focus();
        let app_handle = app.clone();
        win.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                #[cfg(target_os = "macos")]
                let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
        });
    }
}

fn open_settings_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.set_focus();
        return;
    }

    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    let window = WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        WebviewUrl::App("/settings".into()),
    )
    .title("Settings")
    .inner_size(SETTINGS_WIDTH, SETTINGS_HEIGHT)
    .resizable(false)
    .decorations(true)
    .skip_taskbar(false)
    .visible(true)
    .build();

    if let Ok(win) = window {
        let _ = win.set_focus();
        let app_handle = app.clone();
        win.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                #[cfg(target_os = "macos")]
                {
                    // Only revert to Accessory if the website window isn't open
                    if app_handle.get_webview_window(WEBSITE_LABEL).is_none() {
                        let _ =
                            app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                    }
                }
            }
        });
    }
}

fn open_overlay_window(app: &tauri::AppHandle, label: &str, url: &str, x: f64, y: f64) {
    if app.get_webview_window(label).is_some() {
        return;
    }

    let result = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
        .position(x, y)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .visible(true)
        .build();

    match result {
        Ok(_) => log::info!("Opened overlay window {}", label),
        Err(e) => log::error!("Failed to open overlay window {}: {}", label, e),
    }
}

fn open_overlay_pair(app: &tauri::AppHandle) {
    open_overlay_window(
        app,
        OVERLAY_INTERACTIVE_LABEL,
        "/overlay?mode=interactive",
        200.0,
        200.0,
    );
    open_overlay_window(
        app,
        OVERLAY_CLICKTHROUGH_LABEL,
        "/overlay?mode=clickthrough",
        520.0,
        200.0,
    );
}

fn close_overlay_pair(app: &tauri::AppHandle) {
    for label in [OVERLAY_INTERACTIVE_LABEL, OVERLAY_CLICKTHROUGH_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            match window.close() {
                Ok(_) => log::info!("Closed overlay window {}", label),
                Err(e) => log::error!("Failed to close overlay {}: {}", label, e),
            }
        }
    }
}

fn load_blocker_settings(app: &tauri::AppHandle) -> BlockerSettings {
    app.store(BLOCKER_STORE_FILE)
        .ok()
        .and_then(|s| s.get(BLOCKER_STORE_KEY))
        .and_then(|v| serde_json::from_value::<BlockerSettings>(v).ok())
        .unwrap_or_default()
}

fn save_blocker_settings(app: &tauri::AppHandle, settings: &BlockerSettings) {
    if let Ok(store) = app.store(BLOCKER_STORE_FILE) {
        if let Ok(val) = serde_json::to_value(settings) {
            store.set(BLOCKER_STORE_KEY, val);
            let _ = store.save();
        }
    }
}

fn open_blocker_window(app: &tauri::AppHandle, visible_now: bool) {
    if app.get_webview_window(BLOCKER_LABEL).is_some() {
        return;
    }

    let (x, y, w, h) = {
        let state = app.state::<SharedBlockerState>();
        let s = state.lock().unwrap();
        (
            s.settings.x,
            s.settings.y,
            s.settings.width,
            s.settings.height,
        )
    };

    let result = WebviewWindowBuilder::new(
        app,
        BLOCKER_LABEL,
        WebviewUrl::App("/overlay?mode=blocker".into()),
    )
    .inner_size(w, h)
    .position(x, y)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false)
    .focusable(false)
    .visible(visible_now)
    .build();

    let window = match result {
        Ok(w) => {
            log::info!("Opened blocker window (visible={})", visible_now);
            w
        }
        Err(e) => {
            log::error!("Failed to open blocker window: {}", e);
            return;
        }
    };

    let app_handle = app.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Moved(pos) => {
            let scale = app_handle
                .get_webview_window(BLOCKER_LABEL)
                .and_then(|w| w.scale_factor().ok())
                .unwrap_or(1.0);
            let state = app_handle.state::<SharedBlockerState>();
            let mut s = state.lock().unwrap();
            s.settings.x = pos.x as f64 / scale;
            s.settings.y = pos.y as f64 / scale;
            persist_per_map_geometry(&mut s);
            let snapshot = s.settings.clone();
            drop(s);
            save_blocker_settings(&app_handle, &snapshot);
        }
        tauri::WindowEvent::Resized(size) => {
            let scale = app_handle
                .get_webview_window(BLOCKER_LABEL)
                .and_then(|w| w.scale_factor().ok())
                .unwrap_or(1.0);
            let state = app_handle.state::<SharedBlockerState>();
            let mut s = state.lock().unwrap();
            s.settings.width = size.width as f64 / scale;
            s.settings.height = size.height as f64 / scale;
            persist_per_map_geometry(&mut s);
            let snapshot = s.settings.clone();
            drop(s);
            save_blocker_settings(&app_handle, &snapshot);
        }
        _ => {}
    });
}

/// If a battleground is active, mirror the current default geometry into
/// the per-map table under that hash so a subsequent map switch can
/// restore it.
fn persist_per_map_geometry(s: &mut BlockerState) {
    if let Some(hash) = s.active_map_hash.clone() {
        let rect = BlockerRect {
            x: s.settings.x,
            y: s.settings.y,
            width: s.settings.width,
            height: s.settings.height,
        };
        s.settings.per_map.insert(hash, rect);
    }
}

/// Called by the battlelobby probe when it detects a new active map. If we
/// have a saved rect for that hash, resize/reposition the blocker to it.
/// Either way, store the hash so subsequent geometry changes save under it.
pub fn on_active_map_changed(app: &tauri::AppHandle, hash: String) {
    let target_rect = {
        let state = app.state::<SharedBlockerState>();
        let mut s = state.lock().unwrap();
        let unchanged = s.active_map_hash.as_deref() == Some(hash.as_str());
        if unchanged {
            return;
        }
        s.active_map_hash = Some(hash.clone());
        s.settings.per_map.get(&hash).cloned()
    };

    log::info!(
        "active map changed -> {} (saved rect: {})",
        hash,
        if target_rect.is_some() { "yes" } else { "no" }
    );

    let Some(rect) = target_rect else {
        return;
    };

    {
        let state = app.state::<SharedBlockerState>();
        let mut s = state.lock().unwrap();
        s.settings.x = rect.x;
        s.settings.y = rect.y;
        s.settings.width = rect.width;
        s.settings.height = rect.height;
        let snapshot = s.settings.clone();
        drop(s);
        save_blocker_settings(app, &snapshot);
    }

    if let Some(window) = app.get_webview_window(BLOCKER_LABEL) {
        let pos = tauri::LogicalPosition::new(rect.x, rect.y);
        let size = tauri::LogicalSize::new(rect.width, rect.height);
        if let Err(e) = window.set_position(pos) {
            log::error!("blocker set_position failed: {}", e);
        }
        if let Err(e) = window.set_size(size) {
            log::error!("blocker set_size failed: {}", e);
        }
    }
}

fn close_blocker_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(BLOCKER_LABEL) {
        match window.close() {
            Ok(_) => log::info!("Closed blocker window"),
            Err(e) => log::error!("Failed to close blocker: {}", e),
        }
    }
}

fn apply_blocker_visual_mode(app: &tauri::AppHandle, mode: BlockerVisualMode) {
    let Some(window) = app.get_webview_window(BLOCKER_LABEL) else {
        return;
    };

    let interactable = mode == BlockerVisualMode::Interactable;

    // The window stays focusable=false and decorations=false in BOTH modes —
    // OS chrome would force focus to transfer when the user clicks the title
    // bar, which would kick HoTS out of foreground. Drag and resize in
    // interactable mode are handled by HTML hit-zones calling
    // window.startDragging() / startResizeDragging(...), which work fine on
    // non-focusable windows (hit-test based, not focus based).
    //
    // We DO toggle resizable so startResizeDragging is allowed by the OS.
    if let Err(e) = window.set_resizable(interactable) {
        log::error!("set_resizable({}) failed: {}", interactable, e);
    }
    if let Err(e) = window.show() {
        log::error!("blocker show failed: {}", e);
    }

    let payload = match mode {
        BlockerVisualMode::Blocking => "blocking",
        BlockerVisualMode::Interactable => "interactable",
    };
    if let Err(e) = app.emit_to(BLOCKER_LABEL, "blocker://mode-changed", payload) {
        log::error!("emit blocker mode failed: {}", e);
    }
}

fn toggle_blocker_mode(app: &tauri::AppHandle) {
    let new_mode = {
        let state = app.state::<SharedBlockerState>();
        let mut s = state.lock().unwrap();
        if !s.settings.enabled {
            return;
        }
        s.mode = match s.mode {
            BlockerVisualMode::Blocking => BlockerVisualMode::Interactable,
            BlockerVisualMode::Interactable => BlockerVisualMode::Blocking,
        };
        s.mode
    };

    if app.get_webview_window(BLOCKER_LABEL).is_none() {
        open_blocker_window(app, true);
    }

    apply_blocker_visual_mode(app, new_mode);

    if new_mode == BlockerVisualMode::Blocking {
        // Blocking mode is gated on focus + an active match.
        refresh_blocker_visibility(app);
    }

    log::info!("blocker mode toggled to {:?}", new_mode);
}

fn register_blocker_hotkey(app: &tauri::AppHandle) {
    let shortcut: Shortcut = match BLOCKER_HOTKEY.parse() {
        Ok(s) => s,
        Err(e) => {
            log::error!("invalid blocker hotkey '{}': {}", BLOCKER_HOTKEY, e);
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
                    toggle_blocker_mode(&ah);
                });
            }
        });
    if let Err(e) = res {
        log::error!("register blocker hotkey failed: {}", e);
    } else {
        log::info!("registered blocker hotkey {}", BLOCKER_HOTKEY);
    }
}

fn unregister_blocker_hotkey(app: &tauri::AppHandle) {
    let shortcut: Shortcut = match BLOCKER_HOTKEY.parse() {
        Ok(s) => s,
        Err(_) => return,
    };
    if app.global_shortcut().is_registered(shortcut.clone()) {
        let _ = app.global_shortcut().unregister(shortcut);
        log::info!("unregistered blocker hotkey");
    }
}

fn set_blocker_enabled(app: &tauri::AppHandle, enabled: bool) {
    let snapshot = {
        let state = app.state::<SharedBlockerState>();
        let mut s = state.lock().unwrap();
        s.settings.enabled = enabled;
        s.mode = BlockerVisualMode::Blocking;
        s.settings.clone()
    };
    save_blocker_settings(app, &snapshot);

    if enabled {
        register_blocker_hotkey(app);
        refresh_blocker_visibility(app);
    } else {
        unregister_blocker_hotkey(app);
        close_blocker_window(app);
    }
    log::info!("blocker enabled={}", enabled);
}

/// Show the blocker only when it's enabled, in Blocking mode, HoTS is the
/// focused window, and a match is actually in progress. Interactable mode is
/// left alone — it stays visible so it can be repositioned at any time.
fn refresh_blocker_visibility(app: &tauri::AppHandle) {
    let (enabled, mode, in_game) = {
        let state = app.state::<SharedBlockerState>();
        let s = state.lock().unwrap();
        (s.settings.enabled, s.mode, s.in_game)
    };
    if !enabled || mode == BlockerVisualMode::Interactable {
        return;
    }
    if in_game && game_focus::is_focused(app) {
        if app.get_webview_window(BLOCKER_LABEL).is_none() {
            open_blocker_window(app, true);
            apply_blocker_visual_mode(app, BlockerVisualMode::Blocking);
        } else if let Some(w) = app.get_webview_window(BLOCKER_LABEL) {
            let _ = w.show();
        }
    } else if let Some(w) = app.get_webview_window(BLOCKER_LABEL) {
        let _ = w.hide();
    }
}

fn handle_focus_change(app: &tauri::AppHandle, focused: bool) {
    if !focused && !is_game_running() {
        // HoTS has fully exited — clear the in-game flag so the blocker won't
        // reappear over the menu when the game is relaunched.
        let state = app.state::<SharedBlockerState>();
        state.lock().unwrap().in_game = false;
    }
    refresh_blocker_visibility(app);
}

/// Called when HoTS writes a battlelobby file — a match is starting.
pub fn on_game_started(app: &tauri::AppHandle) {
    {
        let state = app.state::<SharedBlockerState>();
        let mut s = state.lock().unwrap();
        if s.in_game {
            return;
        }
        s.in_game = true;
    }
    log::info!("game session: match started");
    refresh_blocker_visibility(app);
}

/// Called when HoTS writes a .StormReplay file — the match has ended.
pub fn on_game_ended(app: &tauri::AppHandle) {
    {
        let state = app.state::<SharedBlockerState>();
        let mut s = state.lock().unwrap();
        if !s.in_game {
            return;
        }
        s.in_game = false;
    }
    log::info!("game session: match ended");
    refresh_blocker_visibility(app);
}

fn deep_link_path(url: &str) -> Option<String> {
    url.strip_prefix("storm-almanac://")
        .filter(|rest| !rest.is_empty())
        .map(|rest| format!("/{}", rest))
}

fn handle_deep_link(app: &tauri::AppHandle, url: &str) {
    let path = deep_link_path(url);
    let handle = app.clone();
    // Spawn a thread to break free from the app delegate callback, then
    // dispatch window creation back to the main thread.
    std::thread::spawn(move || {
        let h = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            open_website_window(&h, path.as_deref());
        });
    });
}

pub(crate) fn is_game_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("pgrep")
            .args(["-f", "Heroes.app/Contents/MacOS/Heroes"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("tasklist")
            .args(["/NH", "/FI", "IMAGENAME eq HeroesOfTheStorm_x64.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("HeroesOfTheStorm"))
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

#[tauri::command]
fn check_input_permission() -> bool {
    input_recorder::check_accessibility_permission()
}

#[tauri::command]
fn get_recording_status(
    recording_state: tauri::State<'_, SharedRecordingState>,
) -> RecordingStatus {
    let state = recording_state.lock().unwrap();
    state.status.clone()
}

#[tauri::command]
async fn is_game_running_cmd() -> bool {
    tokio::task::spawn_blocking(is_game_running)
        .await
        .unwrap_or(false)
}

fn find_talent_builds_path(watch_dir: &str) -> Option<PathBuf> {
    let accounts_dir = std::path::Path::new(watch_dir);
    let entries = std::fs::read_dir(accounts_dir).ok()?;

    let mut best_path: Option<PathBuf> = None;
    let mut best_modified = std::time::SystemTime::UNIX_EPOCH;
    let mut first_subdir: Option<PathBuf> = None;

    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let candidate = entry.path().join("TalentBuilds.txt");
        if first_subdir.is_none() {
            first_subdir = Some(entry.path());
        }
        if candidate.exists() {
            let modified = std::fs::metadata(&candidate)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if best_path.is_none() || modified > best_modified {
                best_path = Some(candidate);
                best_modified = modified;
            }
        }
    }

    best_path.or_else(|| first_subdir.map(|d| d.join("TalentBuilds.txt")))
}

#[tauri::command]
fn read_talent_builds(app: tauri::AppHandle) -> String {
    let config = load_config(&app);
    let Some(path) = find_talent_builds_path(&config.watch_dir) else {
        return String::new();
    };
    std::fs::read_to_string(&path).unwrap_or_default()
}

#[tauri::command]
fn write_talent_builds(app: tauri::AppHandle, contents: String) -> Result<(), String> {
    let config = load_config(&app);
    let path = find_talent_builds_path(&config.watch_dir)
        .ok_or_else(|| "No account directory found".to_string())?;
    let backup_path = path.with_file_name("TalentBuilds-pre-StormAlmanac.txt");
    if !backup_path.exists() && path.exists() {
        std::fs::copy(&path, &backup_path).map_err(|e| e.to_string())?;
    }

    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_path(path: String) {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(&path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path))
            .spawn();
    }
}

#[tauri::command]
async fn clear_webview_data(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(WEBSITE_LABEL) {
        window
            .clear_all_browsing_data()
            .map_err(|e| format!("Failed to clear browsing data: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
fn toggle_overlay_pair(app: tauri::AppHandle) -> Result<(), String> {
    let any_open = app.get_webview_window(OVERLAY_INTERACTIVE_LABEL).is_some()
        || app.get_webview_window(OVERLAY_CLICKTHROUGH_LABEL).is_some();
    if any_open {
        close_overlay_pair(&app);
    } else {
        open_overlay_pair(&app);
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .max_file_size(1_000_000) // 1MB per log file
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .level(log::LevelFilter::Info)
                // The notify crate logs every filesystem event; left at Info it
                // floods the log and rotates out the probe's own output.
                .level_for("notify", log::LevelFilter::Warn)
                .build(),
        )
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // On Windows, deep link URLs arrive as args to the second instance.
            // On macOS, deep links come through the event listener instead.
            for arg in &args {
                if arg.starts_with("storm-almanac://") {
                    handle_deep_link(app, arg);
                    return;
                }
            }
            // No deep link — just bring the app to the foreground
            open_website_window(app, None);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // Hide dock icon on macOS
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            // Load persisted history and known hashes
            let history = load_history(app.handle());
            let mut known_hashes = load_known_hashes(app.handle());

            // Seed known_hashes from history entries (migration for first launch after update)
            for entry in &history {
                if let Some(sha256) = &entry.sha256 {
                    known_hashes.insert(sha256.clone());
                }
            }
            save_known_hashes(app.handle(), &known_hashes);

            let mut app_state = AppState::default();
            app_state.uploads = VecDeque::from(history);
            app_state.known_hashes = known_hashes;

            // Reset interrupted uploads so the retry loop picks them up.
            // Also reset retryable failures (including pre-upgrade entries where
            // `retryable` is None) so transient server outages don't leave replays
            // stuck in Error after the server recovers.
            for entry in app_state.uploads.iter_mut() {
                if entry.status == state::UploadStatus::Uploading
                    || entry.status == state::UploadStatus::Pending
                {
                    entry.status = state::UploadStatus::Error;
                    entry.error = Some("Interrupted by app restart".to_string());
                    entry.retry_count = 0;
                    entry.retryable = Some(true);
                    entry.last_attempt_at = None;
                } else if entry.status == state::UploadStatus::Error
                    && entry.retryable != Some(false)
                {
                    entry.retry_count = 0;
                    entry.last_attempt_at = None;
                }
            }

            app.manage(Mutex::new(app_state));
            app.manage(UploadSemaphore::new(5));
            app.manage(UploadChannels::default());
            app.manage(SharedRecordingState::default());
            app.manage(game_session::RecorderHolder::default());

            // Map blocker state — load persisted settings before tray builds
            // (the tray's "Enable Map Blocker" item reflects the saved flag).
            let blocker_settings = load_blocker_settings(app.handle());
            let blocker_enabled_at_startup = blocker_settings.enabled;
            app.manage(SharedBlockerState::new(BlockerState::new(blocker_settings)));

            // Foreground-window poller — also app.manage()s the FocusFlag.
            let mut focus_rx = game_focus::init(app.handle());

            // Build tray icon
            let open_website = MenuItemBuilder::with_id("open_website", "Open Website").build(app)?;
            let settings = MenuItemBuilder::with_id("settings", "Settings").build(app)?;
            let check_update = MenuItemBuilder::with_id("check_update", "Check for Updates").build(app)?;
            let rescan = MenuItemBuilder::with_id("rescan", "Re-upload All Replays").build(app)?;
            let toggle_overlay =
                MenuItemBuilder::with_id("toggle_overlay", "Toggle Overlay POC").build(app)?;
            let enable_blocker = CheckMenuItemBuilder::with_id("enable_blocker", "Enable Map Blocker")
                .checked(blocker_enabled_at_startup)
                .build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit Storm Almanac").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&open_website)
                .item(&settings)
                .separator()
                .item(&check_update)
                .item(&rescan)
                .separator()
                .item(&enable_blocker)
                .item(&toggle_overlay)
                .separator()
                .item(&quit)
                .build()?;

            #[cfg(target_os = "macos")]
            let (tray_icon, is_template) = (
                Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?,
                true,
            );
            #[cfg(not(target_os = "macos"))]
            let (tray_icon, is_template) = (
                Image::from_bytes(include_bytes!("../icons/32x32.png"))?,
                false,
            );

            let update_available = Arc::new(AtomicBool::new(false));
            let update_flag_menu = update_available.clone();
            let check_update_menu = check_update.clone();
            let enable_blocker_menu = enable_blocker.clone();

            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(is_template)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("Storm Almanac")
                .on_menu_event(move |app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    } else if event.id() == "open_website" {
                        open_website_window(app, None);
                    } else if event.id() == "check_update" {
                        let handle = app.clone();
                        if update_flag_menu.load(Ordering::SeqCst) {
                            tauri::async_runtime::spawn(async move {
                                install_update(handle).await;
                            });
                        } else {
                            let item = check_update_menu.clone();
                            let flag = update_flag_menu.clone();
                            tauri::async_runtime::spawn(async move {
                                check_for_updates(handle, item, flag).await;
                            });
                        }
                    } else if event.id() == "settings" {
                        open_settings_window(app);
                    } else if event.id() == "rescan" {
                        watcher::rescan(app);
                    } else if event.id() == "toggle_overlay" {
                        let _ = toggle_overlay_pair(app.clone());
                    } else if event.id() == "enable_blocker" {
                        let state = app.state::<SharedBlockerState>();
                        let currently_enabled = state.lock().unwrap().settings.enabled;
                        let new_enabled = !currently_enabled;
                        set_blocker_enabled(app, new_enabled);
                        let _ = enable_blocker_menu.set_checked(new_enabled);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        open_website_window(tray.app_handle(), None);
                    }
                })
                .build(app)?;

            // macOS app menu with Settings shortcut (Cmd+,)
            #[cfg(target_os = "macos")]
            {
                let app_submenu = SubmenuBuilder::new(app, "Storm Almanac")
                    .about(None)
                    .separator()
                    .item(
                        &MenuItemBuilder::with_id("app_settings", "Settings")
                            .accelerator("CmdOrCtrl+,")
                            .build(app)?,
                    )
                    .separator()
                    .hide()
                    .hide_others()
                    .show_all()
                    .separator()
                    .quit()
                    .build()?;
                let file_submenu = SubmenuBuilder::new(app, "File")
                    .close_window()
                    .build()?;
                let edit_submenu = SubmenuBuilder::new(app, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;
                let app_menu = MenuBuilder::new(app)
                    .item(&app_submenu)
                    .item(&file_submenu)
                    .item(&edit_submenu)
                    .build()?;
                app.set_menu(app_menu)?;

                app.on_menu_event(move |app, event| {
                    if event.id() == "app_settings" {
                        open_settings_window(app);
                    }
                });
            }

            // Diagnostic probe — watches %TEMP% for *.battlelobby files and
            // dumps them with a printable-strings index so we can figure out
            // where map names live in the binary. Remove once that's known.
            battle_lobby_probe::start(app.handle().clone());

            // Map blocker: register hotkey if it was enabled at last shutdown,
            // and spawn a subscriber that reacts to game-focus changes.
            if blocker_enabled_at_startup {
                register_blocker_hotkey(app.handle());
            }
            let focus_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while focus_rx.changed().await.is_ok() {
                    let focused = *focus_rx.borrow();
                    handle_focus_change(&focus_app, focused);
                }
            });

            // Start file watcher
            watcher::start_watcher(app.handle());

            // Start game session polling for input recording
            game_session::start_game_session_polling(app.handle().clone());

            // Retry any pending session file uploads from previous crashes
            let retry_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                game_session::retry_pending_uploads(&retry_app).await;
            });

            // Open website window on startup unless start_minimized is set
            let startup_config = load_config(app.handle());
            if !startup_config.start_minimized {
                open_website_window(app.handle(), None);
            }

            // Handle deep link that launched the app (e.g. storm-almanac://builds)
            if let Ok(urls) = app.deep_link().get_current() {
                if let Some(url) = urls.and_then(|u| u.into_iter().next()) {
                    handle_deep_link(app.handle(), url.as_str());
                }
            }

            // Handle deep link events while the app is already running
            let deep_link_handle = app.handle().clone();
            app.handle().listen("deep-link://new-url", move |event| {
                if let Ok(urls) = serde_json::from_str::<Vec<String>>(event.payload()) {
                    if let Some(url_str) = urls.first() {
                        handle_deep_link(&deep_link_handle, url_str);
                    }
                }
            });

            // Periodically check for updates
            let handle = app.handle().clone();
            let check_update_periodic = check_update.clone();
            let update_flag_periodic = update_available.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                loop {
                    check_for_updates(
                        handle.clone(),
                        check_update_periodic.clone(),
                        update_flag_periodic.clone(),
                    )
                    .await;
                    tokio::time::sleep(std::time::Duration::from_secs(6 * 60 * 60)).await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_uploads,
            watch_uploads,
            get_config,
            save_config_cmd,
            autostart::enable_autostart,
            autostart::disable_autostart,
            autostart::is_autostart_enabled,
            read_talent_builds,
            write_talent_builds,
            is_game_running_cmd,
            check_input_permission,
            get_recording_status,
            toggle_overlay_pair,
            reveal_path,
            clear_webview_data,
            screen_capture::dev_capture_screen,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            match event {
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    open_website_window(_app, None);
                }
                tauri::RunEvent::ExitRequested { api, code, .. } => {
                    // Prevent exit when triggered by last window closing (code
                    // is None). Allow explicit app.exit() calls (code is Some).
                    if code.is_none() {
                        api.prevent_exit();
                    }
                }
                _ => {}
            }
        });
}
