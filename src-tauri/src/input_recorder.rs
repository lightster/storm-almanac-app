use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize)]
struct InputEvent {
    t: u64,
    #[serde(rename = "type")]
    event_type: &'static str,
    key: String,
    raw: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

fn system_time_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn event_type_str(event_type: &rdev::EventType) -> Option<&'static str> {
    match event_type {
        rdev::EventType::KeyPress(_) => Some("kd"),
        rdev::EventType::KeyRelease(_) => Some("ku"),
        rdev::EventType::ButtonPress(_) => Some("bd"),
        rdev::EventType::ButtonRelease(_) => Some("bu"),
        _ => None,
    }
}

fn key_name(event_type: &rdev::EventType) -> String {
    match event_type {
        rdev::EventType::KeyPress(key) | rdev::EventType::KeyRelease(key) => {
            format!("{:?}", key)
        }
        rdev::EventType::ButtonPress(btn) | rdev::EventType::ButtonRelease(btn) => {
            format!("{:?}", btn)
        }
        _ => String::new(),
    }
}

fn raw_code(event_type: &rdev::EventType) -> u32 {
    match event_type {
        rdev::EventType::KeyPress(key) | rdev::EventType::KeyRelease(key) => {
            if let rdev::Key::Unknown(code) = key {
                *code as u32
            } else {
                0
            }
        }
        rdev::EventType::ButtonPress(btn) | rdev::EventType::ButtonRelease(btn) => {
            match btn {
                rdev::Button::Left => 1,
                rdev::Button::Right => 2,
                rdev::Button::Middle => 3,
                rdev::Button::Unknown(code) => *code as u32,
            }
        }
        _ => 0,
    }
}

pub struct InputRecorder {
    recording: Arc<AtomicBool>,
    writer_handle: Option<std::thread::JoinHandle<()>>,
}

impl InputRecorder {
    pub fn new(session_path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(session_path)?;
        let writer = BufWriter::new(file);

        let recording = Arc::new(AtomicBool::new(true));

        let (tx, rx) = mpsc::channel::<rdev::Event>();

        // Writer thread: receives events from channel, writes JSONL
        let recording_clone = Arc::clone(&recording);
        let writer_handle = std::thread::spawn(move || {
            Self::writer_loop(rx, writer, recording_clone);
        });

        // Listener thread: captures global input events
        let recording_for_listener = Arc::clone(&recording);
        std::thread::spawn(move || {
            let _ = rdev::listen(move |event| {
                if !recording_for_listener.load(Ordering::Relaxed) {
                    return;
                }
                if event_type_str(&event.event_type).is_some() {
                    let _ = tx.send(event);
                }
            });
        });

        Ok(Self {
            recording,
            writer_handle: Some(writer_handle),
        })
    }

    fn writer_loop(
        rx: mpsc::Receiver<rdev::Event>,
        mut writer: BufWriter<File>,
        recording: Arc<AtomicBool>,
    ) {
        let mut event_count = 0u64;
        let flush_interval = 100; // flush every 100 events

        while recording.load(Ordering::Relaxed) {
            match rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(event) => {
                    if let Some(type_str) = event_type_str(&event.event_type) {
                        let input_event = InputEvent {
                            t: system_time_millis(event.time),
                            event_type: type_str,
                            key: key_name(&event.event_type),
                            raw: raw_code(&event.event_type),
                            name: event.name,
                        };
                        if let Ok(json) = serde_json::to_string(&input_event) {
                            let _ = writeln!(writer, "{}", json);
                            event_count += 1;
                            if event_count % flush_interval == 0 {
                                let _ = writer.flush();
                            }
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Periodic flush even when idle
                    let _ = writer.flush();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }

        // Final flush
        let _ = writer.flush();
    }

    pub fn stop(&mut self) {
        self.recording.store(false, Ordering::SeqCst);
        // The writer thread will exit after the flag is set and the next recv timeout
        if let Some(handle) = self.writer_handle.take() {
            let _ = handle.join();
        }
    }

}

pub fn gzip_file(input_path: &Path) -> std::io::Result<PathBuf> {
    let output_path = input_path.with_extension("jsonl.gz");
    let input_file = File::open(input_path)?;
    let output_file = File::create(&output_path)?;
    let mut encoder = GzEncoder::new(output_file, Compression::default());
    std::io::copy(&mut std::io::BufReader::new(input_file), &mut encoder)?;
    encoder.finish()?;
    Ok(output_path)
}

#[cfg(target_os = "macos")]
pub fn check_accessibility_permission() -> bool {
    macos_accessibility_client::accessibility::application_is_trusted_with_prompt()
}

#[cfg(not(target_os = "macos"))]
pub fn check_accessibility_permission() -> bool {
    true
}
