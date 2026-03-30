use crate::config::load_config;
use crate::input_recorder::{self, gzip_file, InputRecorder};
use crate::state::{RecordingStatus, SharedRecordingState};
use reqwest::multipart;
use std::path::Path;
use std::sync::Mutex;
use tauri::Manager;

pub fn start_game_session_polling(app: tauri::AppHandle) {
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut was_running = false;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));

        loop {
            interval.tick().await;

            let config = load_config(&app_clone);
            if !config.input_recording_enabled {
                // If recording was active but got disabled, stop it
                if was_running {
                    stop_and_upload(&app_clone).await;
                    was_running = false;
                }
                continue;
            }

            let is_running = tokio::task::spawn_blocking(crate::is_game_running)
                .await
                .unwrap_or(false);

            if is_running && !was_running {
                // Game just started
                start_recording(&app_clone);
                was_running = true;
            } else if !is_running && was_running {
                // Game just exited
                stop_and_upload(&app_clone).await;
                was_running = false;
            }
        }
    });
}

fn start_recording(app: &tauri::AppHandle) {
    if !input_recorder::check_accessibility_permission() {
        log::warn!("Input recording requires accessibility permission");
        return;
    }

    let session_uuid = uuid::Uuid::new_v4().to_string();
    let session_dir = app
        .path()
        .app_data_dir()
        .expect("failed to get app data dir");
    let _ = std::fs::create_dir_all(&session_dir);
    let session_path = session_dir.join(format!("{}_inputs.jsonl", session_uuid));

    match InputRecorder::new(&session_path) {
        Ok(recorder) => {
            log::info!(
                "Started input recording session {} at {:?}",
                session_uuid,
                session_path
            );

            let recording_state = app.state::<SharedRecordingState>();
            let mut state = recording_state.lock().unwrap();
            state.status = RecordingStatus::Recording;
            state.recording_session_uuid = Some(session_uuid);
            state.session_path = Some(session_path);

            // Store the recorder so it stays alive
            let recorder_holder = app.state::<RecorderHolder>();
            let mut holder = recorder_holder.lock().unwrap();
            *holder = Some(recorder);
        }
        Err(e) => {
            log::error!("Failed to start input recording: {}", e);
        }
    }
}

async fn stop_and_upload(app: &tauri::AppHandle) {
    // Stop the recorder
    let (session_uuid, session_path) = {
        let recorder_holder = app.state::<RecorderHolder>();
        let mut holder = recorder_holder.lock().unwrap();
        if let Some(ref mut recorder) = *holder {
            recorder.stop();
        }
        *holder = None;

        let recording_state = app.state::<SharedRecordingState>();
        let mut state = recording_state.lock().unwrap();
        let uuid = state.recording_session_uuid.take();
        let path = state.session_path.take();
        state.status = RecordingStatus::Uploading;
        (uuid, path)
    };

    if let (Some(uuid), Some(path)) = (session_uuid, session_path) {
        log::info!("Compressing and uploading session {}", uuid);
        upload_session_file(app, &uuid, &path).await;
    }

    let recording_state = app.state::<SharedRecordingState>();
    let mut state = recording_state.lock().unwrap();
    state.status = RecordingStatus::Idle;
}

async fn upload_session_file(_app: &tauri::AppHandle, session_uuid: &str, jsonl_path: &Path) {
    // Check the file has content
    let metadata = match std::fs::metadata(jsonl_path) {
        Ok(m) => m,
        Err(e) => {
            log::error!("Session file not found: {}", e);
            return;
        }
    };

    if metadata.len() == 0 {
        log::info!("Session file is empty, skipping upload");
        let _ = std::fs::remove_file(jsonl_path);
        return;
    }

    // Read first and last lines to get timestamps
    let (started_at, ended_at) = match read_session_timestamps(jsonl_path) {
        Some(ts) => ts,
        None => {
            log::error!("Failed to read timestamps from session file");
            return;
        }
    };

    // Gzip compress
    let gz_path = match gzip_file(jsonl_path) {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to gzip session file: {}", e);
            return;
        }
    };

    // Upload
    let url = format!("{}/api/input-sessions/upload", crate::API_URL);
    match do_upload_session(&url, session_uuid, &gz_path, started_at, ended_at).await {
        Ok(_) => {
            log::info!("Session {} uploaded successfully", session_uuid);
            let _ = std::fs::remove_file(jsonl_path);
            let _ = std::fs::remove_file(&gz_path);
        }
        Err(e) => {
            log::error!("Failed to upload session {}: {}", session_uuid, e);
            // Keep the .gz file for retry; remove the uncompressed version
            let _ = std::fs::remove_file(jsonl_path);
        }
    }
}

fn read_session_timestamps(path: &Path) -> Option<(u64, u64)> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut first_ts: Option<u64> = None;
    let mut last_ts: Option<u64> = None;

    for line in reader.lines() {
        let line = line.ok()?;
        if line.is_empty() {
            continue;
        }
        // Parse just the "t" field from each JSON line
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(t) = val.get("t").and_then(|v| v.as_u64()) {
                if first_ts.is_none() {
                    first_ts = Some(t);
                }
                last_ts = Some(t);
            }
        }
    }

    match (first_ts, last_ts) {
        (Some(f), Some(l)) => Some((f, l)),
        _ => None,
    }
}

async fn do_upload_session(
    url: &str,
    session_uuid: &str,
    gz_path: &Path,
    started_at: u64,
    ended_at: u64,
) -> Result<(), String> {
    let file_bytes = tokio::fs::read(gz_path)
        .await
        .map_err(|e| format!("Failed to read gz file: {}", e))?;

    let file_name = gz_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let part = multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("application/gzip")
        .map_err(|e| format!("MIME error: {}", e))?;

    let form = multipart::Form::new()
        .part("file", part)
        .text("recording_session_uuid", session_uuid.to_string())
        .text("started_at", started_at.to_string())
        .text("ended_at", ended_at.to_string());

    log::info!("[session-upload] POST {}", url);

    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header("Origin", "storm-almanac://")
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(format!("HTTP {}: {}", status, body))
    }
}

/// Retry uploading any leftover session files from previous crashes.
/// Handles both orphaned .jsonl files (not yet compressed) and .gz files (compressed but not uploaded).
pub async fn retry_pending_uploads(app: &tauri::AppHandle) {
    let session_dir = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(_) => return,
    };

    let entries = match std::fs::read_dir(&session_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    // Collect .jsonl files to process (compress + upload)
    let mut jsonl_files: Vec<std::path::PathBuf> = Vec::new();
    let mut gz_files: Vec<std::path::PathBuf> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();

        if name.ends_with("_inputs.jsonl.gz") {
            gz_files.push(path);
        } else if name.ends_with("_inputs.jsonl") {
            jsonl_files.push(path);
        }
    }

    // Process orphaned .jsonl files: compress then upload
    for jsonl_path in &jsonl_files {
        let gz_path = jsonl_path.with_extension("jsonl.gz");
        if gz_files.iter().any(|p| p == &gz_path) {
            continue; // .gz already exists, will be handled below
        }

        let session_uuid = extract_session_uuid(jsonl_path);
        if session_uuid.is_empty() {
            continue;
        }

        if let Some((started_at, ended_at)) = read_session_timestamps(jsonl_path) {
            match gzip_file(jsonl_path) {
                Ok(gz) => {
                    let url = format!("{}/api/input-sessions/upload", crate::API_URL);
                    match do_upload_session(&url, &session_uuid, &gz, started_at, ended_at).await {
                        Ok(_) => {
                            log::info!("Retried session {} uploaded successfully", session_uuid);
                            let _ = std::fs::remove_file(jsonl_path);
                            let _ = std::fs::remove_file(&gz);
                        }
                        Err(e) => {
                            log::warn!("Retry upload failed for session {}: {}", session_uuid, e);
                            let _ = std::fs::remove_file(jsonl_path);
                            // Keep .gz for next retry
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to compress session {}: {}", session_uuid, e);
                }
            }
        }
    }

    // Process .gz files that haven't been uploaded yet
    for gz_path in &gz_files {
        let session_uuid = extract_session_uuid(gz_path);
        if session_uuid.is_empty() {
            continue;
        }

        // Try to read timestamps from the companion .jsonl if it exists
        let jsonl_path = gz_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{}_inputs.jsonl", session_uuid));

        let timestamps = if jsonl_path.exists() {
            read_session_timestamps(&jsonl_path)
        } else {
            read_session_timestamps_from_gz(gz_path)
        };

        if let Some((started_at, ended_at)) = timestamps {
            let url = format!("{}/api/input-sessions/upload", crate::API_URL);
            match do_upload_session(&url, &session_uuid, gz_path, started_at, ended_at).await {
                Ok(_) => {
                    log::info!("Retried session {} uploaded successfully", session_uuid);
                    let _ = std::fs::remove_file(&jsonl_path);
                    let _ = std::fs::remove_file(gz_path);
                }
                Err(e) => {
                    log::warn!("Retry upload failed for session {}: {}", session_uuid, e);
                }
            }
        }
    }
}

fn extract_session_uuid(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    name.split("_inputs").next().unwrap_or_default().to_string()
}

fn read_session_timestamps_from_gz(gz_path: &Path) -> Option<(u64, u64)> {
    use flate2::read::GzDecoder;
    use std::io::{BufRead, BufReader};

    let file = std::fs::File::open(gz_path).ok()?;
    let decoder = GzDecoder::new(file);
    let reader = BufReader::new(decoder);

    let mut first_ts: Option<u64> = None;
    let mut last_ts: Option<u64> = None;

    for line in reader.lines() {
        let line = line.ok()?;
        if line.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(t) = val.get("t").and_then(|v| v.as_u64()) {
                if first_ts.is_none() {
                    first_ts = Some(t);
                }
                last_ts = Some(t);
            }
        }
    }

    match (first_ts, last_ts) {
        (Some(f), Some(l)) => Some((f, l)),
        _ => None,
    }
}

pub type RecorderHolder = Mutex<Option<InputRecorder>>;
