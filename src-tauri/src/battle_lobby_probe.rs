// Watches %TEMP% (recursively) for HoTS's replay.server.battlelobby file,
// which HoTS writes at game start under
// %TEMP%\Heroes of the Storm\TempWriteReplay*\. We fingerprint the set of
// .s2ma cache hashes it references — a stable, per-battleground identifier —
// and use that as the lookup key for per-map blocker rects.
//
// As a debug aid the watcher also copies each file it sees to
// <app_log_dir>/battlelobby-dumps/ and logs a hash-extraction summary, so
// if extraction ever breaks we have evidence to inspect.

use notify::{EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use tauri::Manager;

const HASH_LEN: usize = 64;

pub fn start(app: tauri::AppHandle) {
    let dumps_dir = match dumps_dir(&app) {
        Some(p) => p,
        None => {
            log::error!("battlelobby probe: could not resolve app log dir");
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dumps_dir) {
        log::error!(
            "battlelobby probe: failed to create dumps dir {:?}: {}",
            dumps_dir,
            e
        );
        return;
    }

    let watch_dirs = candidate_watch_dirs();
    log::info!(
        "battlelobby probe: dumps -> {:?}, watching {} dir(s) recursively:",
        dumps_dir,
        watch_dirs.len()
    );
    for d in &watch_dirs {
        log::info!("  - {:?}", d);
    }

    // Pick up the battlelobby file if HoTS is already mid-game at startup.
    if let Some(hots_dir) = hots_temp_dir() {
        process_existing_files(&app, &hots_dir, &dumps_dir);
    }

    let app_clone = app.clone();
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(
            move |res: Result<notify::Event, notify::Error>| {
                let _ = tx.send(res);
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                log::error!("battlelobby probe: watcher init failed: {}", e);
                return;
            }
        };

        let mut watched_any = false;
        for dir in &watch_dirs {
            match watcher.watch(dir, RecursiveMode::Recursive) {
                Ok(()) => {
                    log::info!("battlelobby probe: watching {:?}", dir);
                    watched_any = true;
                }
                Err(e) => {
                    log::warn!(
                        "battlelobby probe: failed to watch {:?}: {}",
                        dir,
                        e
                    );
                }
            }
        }
        if !watched_any {
            log::error!("battlelobby probe: no directories could be watched, giving up");
            return;
        }

        while let Ok(res) = rx.recv() {
            let event = match res {
                Ok(ev) => ev,
                Err(e) => {
                    log::warn!("battlelobby probe: watch error: {}", e);
                    continue;
                }
            };
            if !matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_)
            ) {
                continue;
            }
            for path in event.paths {
                if is_battlelobby_path(&path) {
                    handle_file(&app_clone, &path, &dumps_dir);
                }
            }
        }
    });
}

/// Directory to watch for the battlelobby file. HoTS writes it under
/// %TEMP%\Heroes of the Storm\TempWriteReplay*\, so we watch the temp root
/// recursively — that way the file is caught wherever HoTS stages it, even
/// if that subtree doesn't exist yet when the probe starts.
fn candidate_watch_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![std::env::temp_dir()];
    dirs.retain(|d| d.is_dir());
    dirs
}

/// The HoTS replay-staging subtree under %TEMP%, if it exists right now.
fn hots_temp_dir() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("Heroes of the Storm");
    dir.is_dir().then_some(dir)
}

fn process_existing_files(app: &tauri::AppHandle, dir: &Path, dumps_dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            process_existing_files(app, &path, dumps_dir);
        } else if is_battlelobby_path(&path) {
            log::info!("battlelobby probe: existing file at startup: {:?}", path);
            handle_file(app, &path, dumps_dir);
        }
    }
}

fn is_battlelobby_path(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()) == Some("battlelobby")
}

fn handle_file(app: &tauri::AppHandle, path: &Path, dumps_dir: &Path) {
    log::info!("battlelobby probe: detected {:?}", path);

    // Brief delay so HoTS has time to finish writing.
    std::thread::sleep(std::time::Duration::from_millis(500));

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            log::error!("battlelobby probe: read {:?} failed: {}", path, e);
            return;
        }
    };
    log::info!(
        "battlelobby probe: {:?} size = {} bytes",
        path,
        bytes.len()
    );

    let hashes = extract_all_map_hashes(&bytes);
    log::info!(
        "battlelobby probe: {} .s2ma cache refs in {:?}",
        hashes.len(),
        path
    );
    for h in &hashes {
        log::info!("  s2ma: {}", h);
    }
    let hash = compute_map_fingerprint(&hashes);
    match &hash {
        Some(h) => log::info!("battlelobby probe: map fingerprint = {}", h),
        None => log::warn!("battlelobby probe: no .s2ma hashes in {:?}", path),
    }

    // Save a copy with a stamped name so we can compare across games if
    // anything goes weird.
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let original_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let dump_path = dumps_dir.join(format!("{}-{}.bin", stamp, original_name));
    if let Err(e) = std::fs::write(&dump_path, &bytes) {
        log::error!("battlelobby probe: dump save failed: {}", e);
    }

    if let Some(hash) = hash {
        crate::on_active_map_changed(app, hash);
    }
}

/// Pull every 64-hex-char .s2ma cache hash out of the file. HoTS writes
/// paths like `C:\...\Cache\1f\1b\<hash>.s2ma`; each game lobby contains
/// a handful of these for the active battleground + shared launcher
/// assets + lobby template, etc.
fn extract_all_map_hashes(bytes: &[u8]) -> Vec<String> {
    let needle = b".s2ma";
    let mut results = Vec::new();
    let mut from = 0;
    while from + needle.len() <= bytes.len() {
        let rel = match bytes[from..].windows(needle.len()).position(|w| w == needle) {
            Some(r) => r,
            None => break,
        };
        let pos = from + rel;
        if pos >= HASH_LEN {
            let candidate = &bytes[pos - HASH_LEN..pos];
            if candidate.iter().all(|b| b.is_ascii_hexdigit()) {
                if let Ok(s) = std::str::from_utf8(candidate) {
                    results.push(s.to_ascii_lowercase());
                }
            }
        }
        from = pos + needle.len();
    }
    results
}

/// Combine all the .s2ma hashes from a lobby into a single fingerprint
/// keyed off the unordered set. Same battleground -> same asset set ->
/// same fingerprint, regardless of position in the file.
fn compute_map_fingerprint(hashes: &[String]) -> Option<String> {
    if hashes.is_empty() {
        return None;
    }
    let mut sorted: Vec<&str> = hashes.iter().map(|s| s.as_str()).collect();
    sorted.sort_unstable();
    sorted.dedup();

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for h in &sorted {
        hasher.update(h.as_bytes());
        hasher.update(b",");
    }
    Some(hex::encode(hasher.finalize()))
}

fn dumps_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_log_dir()
        .ok()
        .map(|d| d.join("battlelobby-dumps"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H1: &str = "1f1b228ddb1f72205cbfd444055287100b0f39959be816548162e4081ea85511";
    const H2: &str = "1ad94c126eb98a4c1de7b8e6f8aaa153dba52b09d3825e0c89dbc5828db70f76";

    fn build(hashes: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for h in hashes {
            out.extend_from_slice(b"prefix\\");
            out.extend_from_slice(h.as_bytes());
            out.extend_from_slice(b".s2ma ");
        }
        out
    }

    #[test]
    fn extracts_all_hashes() {
        let sample = build(&[H1, H2]);
        let found = extract_all_map_hashes(&sample);
        assert_eq!(found, vec![H1.to_string(), H2.to_string()]);
    }

    #[test]
    fn returns_empty_when_no_s2ma_present() {
        assert!(extract_all_map_hashes(b"some random bytes with no map ref").is_empty());
    }

    #[test]
    fn fingerprint_order_independent() {
        let a = compute_map_fingerprint(&[H1.to_string(), H2.to_string()]).unwrap();
        let b = compute_map_fingerprint(&[H2.to_string(), H1.to_string()]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_changes_with_different_set() {
        let a = compute_map_fingerprint(&[H1.to_string(), H2.to_string()]).unwrap();
        let b = compute_map_fingerprint(&[H1.to_string()]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_none_for_empty() {
        assert!(compute_map_fingerprint(&[]).is_none());
    }
}
