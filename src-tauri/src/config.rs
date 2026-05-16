use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tauri_plugin_store::StoreExt;

use crate::state::UploadEntry;

const STORE_FILE: &str = "storm-almanac.json";

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

fn default_watch_dir() -> String {
    if cfg!(target_os = "macos") {
        if let Some(home) = dirs::home_dir() {
            return home
                .join("Library/Application Support/Blizzard/Heroes of the Storm/Accounts")
                .to_string_lossy()
                .to_string();
        }
    } else if cfg!(target_os = "windows") {
        if let Some(docs) = dirs::document_dir() {
            return docs
                .join("Heroes of the Storm/Accounts")
                .to_string_lossy()
                .to_string();
        }
    }
    String::new()
}

pub fn load_config(app: &tauri::AppHandle) -> AppConfig {
    let store = app.store(STORE_FILE).expect("failed to open store");

    match store.get("config") {
        Some(val) => serde_json::from_value(val).unwrap_or_default(),
        None => AppConfig::default(),
    }
}

pub fn save_config(app: &tauri::AppHandle, config: &AppConfig) {
    let store = app.store(STORE_FILE).expect("failed to open store");

    let val = serde_json::to_value(config).expect("failed to serialize config");
    store.set("config", val);
    let _ = store.save();
}

pub fn load_history(app: &tauri::AppHandle) -> Vec<UploadEntry> {
    let store = app.store(STORE_FILE).expect("failed to open store");

    match store.get("history") {
        Some(val) => serde_json::from_value(val).unwrap_or_default(),
        None => Vec::new(),
    }
}

pub fn save_history(app: &tauri::AppHandle, entries: &[UploadEntry]) {
    let store = app.store(STORE_FILE).expect("failed to open store");

    let val = serde_json::to_value(entries).expect("failed to serialize history");
    store.set("history", val);
    let _ = store.save();
}

pub fn load_known_hashes(app: &tauri::AppHandle) -> HashSet<String> {
    let store = app.store(STORE_FILE).expect("failed to open store");

    match store.get("knownHashes") {
        Some(val) => serde_json::from_value(val).unwrap_or_default(),
        None => HashSet::new(),
    }
}

pub fn save_known_hashes(app: &tauri::AppHandle, hashes: &HashSet<String>) {
    let store = app.store(STORE_FILE).expect("failed to open store");

    let val = serde_json::to_value(hashes).expect("failed to serialize known hashes");
    store.set("knownHashes", val);
    let _ = store.save();
}

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
