mod autostart;
mod config;
mod state;
mod uploader;
mod watcher;

use config::{load_config, load_history, load_known_hashes, save_known_hashes, save_config, AppConfig};
use state::{AppState, SharedState, UploadEntry, UploadSemaphore};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_positioner::{Position, WindowExt};
use tauri_plugin_updater::UpdaterExt;

#[tauri::command]
fn get_uploads(state: tauri::State<'_, SharedState>) -> Vec<UploadEntry> {
    let state = state.lock().unwrap();
    state.uploads.iter().cloned().collect()
}

#[tauri::command]
fn get_config(app: tauri::AppHandle) -> AppConfig {
    load_config(&app)
}

#[tauri::command]
fn save_config_cmd(app: tauri::AppHandle, config: AppConfig) {
    save_config(&app, &config);
}

const WINDOW_LABEL: &str = "main";
const WINDOW_WIDTH: f64 = 360.0;
const WINDOW_HEIGHT: f64 = 480.0;

const WEBSITE_LABEL: &str = "website";
const WEBSITE_URL: &str = match option_env!("STORM_WEBSITE_URL") {
    Some(url) => url,
    None => "https://hots.lightster.ninja",
};
const WEBSITE_WIDTH: f64 = 1024.0;
const WEBSITE_HEIGHT: f64 = 768.0;

fn position_near_tray(window: &tauri::WebviewWindow) {
    // move_window panics if current_monitor() returns None (e.g. after
    // prolonged display sleep). catch_unwind guards against this, but we
    // cannot fall back to Position::Center since it hits the same code path.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        window.move_window(Position::TrayCenter)
    }));
}

async fn check_for_updates(app: tauri::AppHandle) {
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            log::error!("Failed to create updater: {}", e);
            return;
        }
    };
    match updater.check().await {
        Ok(Some(update)) => {
            let _ = app.emit("update-available", &update.version);
        }
        Ok(None) => {}
        Err(e) => {
            log::error!("Update check failed: {}", e);
        }
    }
}

fn toggle_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            position_near_tray(&window);
            let _ = window.show();
            let _ = window.set_focus();
        }
    } else {
        let window = WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::default())
            .title("Storm Uploader")
            .inner_size(WINDOW_WIDTH, WINDOW_HEIGHT)
            .resizable(false)
            .decorations(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .visible(false)
            .build();

        if let Ok(win) = window {
            position_near_tray(&win);
            let _ = win.show();
            let _ = win.set_focus();

            let win_clone = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::Focused(false) = event {
                    let _ = win_clone.hide();
                }
            });
        }
    }
}

fn open_website_window(app: &tauri::AppHandle, path: Option<&str>) {
    let full_url: String = match path {
        Some(p) => format!("{}{}", WEBSITE_URL, p),
        None => WEBSITE_URL.to_string(),
    };

    if let Some(window) = app.get_webview_window(WEBSITE_LABEL) {
        let url: tauri::Url = full_url.parse().unwrap();
        let _ = window.navigate(url);
        let _ = window.set_focus();
        return;
    }

    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    let url = WebviewUrl::External(full_url.parse().unwrap());
    let window = WebviewWindowBuilder::new(app, WEBSITE_LABEL, url)
        .title("Storm Uploader — Website")
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

fn is_game_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("pgrep")
            .args(["-f", "Heroes of the Storm"])
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
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_overlay() -> Result<(), String> {
    log::info!("load_overlay stub called");
    Ok(())
}

#[tauri::command]
fn open_uploads(app: tauri::AppHandle) {
    toggle_window(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_positioner::init())
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
            toggle_window(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
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

            app.manage(Mutex::new(app_state));
            app.manage(UploadSemaphore::new(5));

            // Build tray icon
            let open_website = MenuItemBuilder::with_id("open_website", "Open Website").build(app)?;
            let check_update = MenuItemBuilder::with_id("check_update", "Check for Updates").build(app)?;
            let rescan = MenuItemBuilder::with_id("rescan", "Re-upload All Replays").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit Storm Uploader").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&open_website)
                .separator()
                .item(&check_update)
                .item(&rescan)
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

            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(is_template)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("Storm Uploader")
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    } else if event.id() == "open_website" {
                        open_website_window(app, None);
                    } else if event.id() == "check_update" {
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            check_for_updates(handle).await;
                        });
                    } else if event.id() == "rescan" {
                        watcher::rescan(app);
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
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // Start file watcher
            watcher::start_watcher(app.handle());

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
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                loop {
                    check_for_updates(handle.clone()).await;
                    tokio::time::sleep(std::time::Duration::from_secs(6 * 60 * 60)).await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_uploads,
            get_config,
            save_config_cmd,
            autostart::enable_autostart,
            autostart::disable_autostart,
            autostart::is_autostart_enabled,
            read_talent_builds,
            write_talent_builds,
            is_game_running_cmd,
            load_overlay,
            open_uploads,
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
