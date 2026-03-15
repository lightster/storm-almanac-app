mod config;
mod state;
mod uploader;
mod watcher;

use config::{load_config, load_history, save_config, AppConfig};
use state::{AppState, SharedState, UploadEntry};
use std::collections::VecDeque;
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};
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

fn position_near_tray(window: &tauri::WebviewWindow) {
    // move_window can panic if tray position hasn't been tracked yet
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        window.move_window(Position::TrayCenter)
    }));
    if result.is_err() {
        let _ = window.move_window(Position::Center);
    }
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
        .setup(|app| {
            // Hide dock icon on macOS
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            // Load persisted history
            let history = load_history(app.handle());
            let mut app_state = AppState::default();
            app_state.uploads = VecDeque::from(history);

            app.manage(Mutex::new(app_state));

            // Build tray icon
            let check_update = MenuItemBuilder::with_id("check_update", "Check for Updates").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit Storm Uploader").build(app)?;
            let menu = MenuBuilder::new(app).item(&check_update).separator().item(&quit).build()?;

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
                    } else if event.id() == "check_update" {
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            check_for_updates(handle).await;
                        });
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

            // Auto-check for updates after a short delay
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                check_for_updates(handle).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_uploads,
            get_config,
            save_config_cmd,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                let _ = _app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                toggle_window(_app);
            }
        });
}
