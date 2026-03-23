use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use tokio::time::sleep;
use uuid::Uuid;

use crate::config::{load_config, save_history, save_known_hashes};
use crate::state::{SharedState, UploadChannels, UploadEntry, UploadSemaphore, UploadStatus};
use crate::uploader;

pub fn start_watcher(app: &AppHandle) {
    let config = load_config(app);
    let watch_dir = PathBuf::from(&config.watch_dir);

    if !watch_dir.exists() {
        log::warn!("Watch directory does not exist: {}", config.watch_dir);
        return;
    }

    let app_handle = app.clone();
    let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();

    // Spawn the upload consumer
    let consumer_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        upload_consumer(consumer_handle, rx).await;
    });

    // Spawn the retry loop
    let retry_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        retry_loop(retry_handle).await;
    });

    // Start the file system watcher in a separate thread
    let tx_clone = tx.clone();
    let watch_dir_clone = watch_dir.clone();
    std::thread::spawn(move || {
        let watch_dir = watch_dir_clone;
        let (notify_tx, notify_rx) = std::sync::mpsc::channel();

        let mut debouncer = new_debouncer(Duration::from_secs(2), notify_tx)
            .expect("failed to create debouncer");

        debouncer
            .watcher()
            .watch(&watch_dir, notify::RecursiveMode::Recursive)
            .expect("failed to watch directory");

        app_handle
            .state::<SharedState>()
            .lock()
            .unwrap()
            .watcher_running = true;

        log::info!("Watching directory: {}", watch_dir.display());

        loop {
            match notify_rx.recv() {
                Ok(Ok(events)) => {
                    for event in events {
                        if event.kind == DebouncedEventKind::Any && is_replay_file(&event.path) {
                            let _ = tx_clone.send(event.path);
                        }
                    }
                }
                Ok(Err(e)) => {
                    log::error!("Watch error: {:?}", e);
                }
                Err(_) => break,
            }
        }
    });

    // Startup scan
    let scan_handle = app.clone();
    let scan_tx = tx;
    let scan_dir = watch_dir;
    tauri::async_runtime::spawn(async move {
        startup_scan(scan_handle, scan_tx, &scan_dir).await;
    });
}

fn is_replay_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext == "StormReplay")
        .unwrap_or(false)
}

async fn wait_for_stable(path: &Path) -> bool {
    for _ in 0..60 {
        let size1 = tokio::fs::metadata(path).await.map(|m| m.len()).ok();
        sleep(Duration::from_millis(500)).await;
        let size2 = tokio::fs::metadata(path).await.map(|m| m.len()).ok();

        match (size1, size2) {
            (Some(s1), Some(s2)) if s1 == s2 && s1 > 0 => return true,
            (None, _) | (_, None) => return false,
            _ => continue,
        }
    }
    false
}

async fn compute_sha256(path: &Path) -> Result<String, std::io::Error> {
    let bytes = tokio::fs::read(path).await?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

async fn process_file(app: &AppHandle, path: PathBuf) {
    if !wait_for_stable(&path).await {
        log::warn!("File not stable, skipping: {}", path.display());
        return;
    }

    let sha256 = match compute_sha256(&path).await {
        Ok(hash) => hash,
        Err(e) => {
            log::error!("Failed to hash {}: {}", path.display(), e);
            return;
        }
    };

    // Client-side dedup
    {
        let state = app.state::<SharedState>();
        let state = state.lock().unwrap();
        if state.has_sha256(&sha256) {
            log::info!("Skipping duplicate: {}", path.display());
            return;
        }
    }

    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let entry = UploadEntry {
        id: Uuid::new_v4().to_string(),
        file_name,
        file_path: path.to_string_lossy().to_string(),
        status: UploadStatus::Pending,
        sha256: Some(sha256),
        error: None,
        created_at: chrono::Utc::now(),
        retry_count: 0,
    };

    {
        let state = app.state::<SharedState>();
        let mut state = state.lock().unwrap();
        state.add_entry(entry.clone());
    }
    persist_and_emit(app);

    do_upload(app, &entry.id).await;
}

pub async fn do_upload(app: &AppHandle, entry_id: &str) {
    let semaphore = app.state::<UploadSemaphore>();
    let _permit = semaphore.acquire().await.expect("semaphore closed");

    let (file_path, sha256) = {
        let state = app.state::<SharedState>();
        let mut state = state.lock().unwrap();
        let entry = state.update_entry(entry_id, |e| {
            e.status = UploadStatus::Uploading;
        });
        match entry {
            Some(e) => (e.file_path.clone(), e.sha256.clone().unwrap_or_default()),
            None => return,
        }
    };
    persist_and_emit(app);

    let config = load_config(app);
    let url = format!("{}/api/replays/upload", config.api_url);

    match uploader::upload_file(&url, &file_path, &sha256).await {
        Ok(response) => {
            let state = app.state::<SharedState>();
            let mut state = state.lock().unwrap();
            state.update_entry(entry_id, |e| {
                match response.status.as_str() {
                    "queued" => e.status = UploadStatus::Queued,
                    "duplicate" => e.status = UploadStatus::Duplicate,
                    _ => {
                        e.status = UploadStatus::Error;
                        e.error = Some(format!("Unexpected status: {}", response.status));
                    }
                }
                if let Some(sha) = &response.sha256 {
                    e.sha256 = Some(sha.clone());
                }
            });
        }
        Err(e) => {
            let state = app.state::<SharedState>();
            let mut state = state.lock().unwrap();
            state.update_entry(entry_id, |entry| {
                entry.status = UploadStatus::Error;
                entry.error = Some(e.message);
                if e.retryable {
                    entry.retry_count += 1;
                } else {
                    entry.retry_count = 5;
                }
            });
        }
    }
    persist_and_emit(app);
}

async fn upload_consumer(app: AppHandle, mut rx: mpsc::UnboundedReceiver<PathBuf>) {
    while let Some(path) = rx.recv().await {
        process_file(&app, path).await;
    }
}

async fn startup_scan(app: AppHandle, tx: mpsc::UnboundedSender<PathBuf>, watch_dir: &Path) {
    sleep(Duration::from_secs(2)).await;

    let mut replay_files = Vec::new();
    collect_replay_files(watch_dir, &mut replay_files);
    replay_files.sort_by(|a, b| {
        let time_a = std::fs::metadata(a).and_then(|m| m.modified()).ok();
        let time_b = std::fs::metadata(b).and_then(|m| m.modified()).ok();
        time_b.cmp(&time_a)
    });

    log::info!("Startup scan found {} replay files", replay_files.len());

    for path in replay_files {
        let sha256 = match compute_sha256(&path).await {
            Ok(hash) => hash,
            Err(_) => continue,
        };

        let already_known = {
            let state = app.state::<SharedState>();
            let state = state.lock().unwrap();
            state.has_sha256(&sha256)
        };

        if !already_known {
            let _ = tx.send(path);
            sleep(Duration::from_secs(1)).await;
        }
    }
}

fn collect_replay_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_replay_files(&path, files);
            } else if is_replay_file(&path) {
                files.push(path);
            }
        }
    }
}

async fn retry_loop(app: AppHandle) {
    loop {
        sleep(Duration::from_secs(30)).await;

        let retryable: Vec<UploadEntry> = {
            let state = app.state::<SharedState>();
            let state = state.lock().unwrap();
            state.get_retryable()
        };

        for entry in retryable {
            let backoff_secs = 60u64 * (1 << entry.retry_count.min(4));
            let age = chrono::Utc::now()
                .signed_duration_since(entry.created_at)
                .num_seconds() as u64;

            if age >= backoff_secs {
                log::info!(
                    "Retrying upload: {} (attempt {})",
                    entry.file_name,
                    entry.retry_count + 1
                );
                do_upload(&app, &entry.id).await;
            }
        }
    }
}

pub fn rescan(app: &AppHandle) {
    let config = load_config(app);
    let watch_dir = PathBuf::from(&config.watch_dir);

    if !watch_dir.exists() {
        log::warn!("Watch directory does not exist: {}", config.watch_dir);
        return;
    }

    {
        let state = app.state::<SharedState>();
        let mut state = state.lock().unwrap();
        state.uploads.clear();
        state.known_hashes.clear();
    }
    persist_and_emit(app);

    let app_handle = app.clone();
    let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();

    tauri::async_runtime::spawn(async move {
        upload_consumer(app_handle, rx).await;
    });

    let scan_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        startup_scan(scan_handle, tx, &watch_dir).await;
    });
}

pub fn persist_and_emit(app: &AppHandle) {
    let (entries, known_hashes) = {
        let state = app.state::<SharedState>();
        let state = state.lock().unwrap();
        (
            state.uploads.iter().cloned().collect::<Vec<UploadEntry>>(),
            state.known_hashes.clone(),
        )
    };

    save_history(app, &entries);
    save_known_hashes(app, &known_hashes);
    let _ = app.emit("upload-changed", &entries);

    let channels = app.state::<UploadChannels>();
    let mut chans = channels.lock().unwrap();
    chans.retain(|ch| ch.send(entries.clone()).is_ok());
}
