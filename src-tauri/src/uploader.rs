use reqwest::multipart;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct UploadResponse {
    pub status: String,
    pub sha256: Option<String>,
    pub error: Option<String>,
}

pub async fn upload_file(
    url: &str,
    file_path: &str,
    _sha256: &str,
) -> Result<UploadResponse, String> {
    let path = Path::new(file_path);
    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let file_bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let part = multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("application/octet-stream")
        .map_err(|e| format!("Failed to set MIME type: {}", e))?;

    let form = multipart::Form::new().part("file", part);

    eprintln!("[upload] POST {} (file: {})", url, file_path);

    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header("Origin", "storm-almanac://")
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            eprintln!("[upload] Network error: {}", e);
            format!("Network error: {}", e)
        })?;

    let status_code = response.status();
    eprintln!("[upload] Response status: {}", status_code);

    if status_code == 429 {
        return Err("Rate limited (429). Will retry later.".to_string());
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    eprintln!("[upload] Response body: {}", body);

    if status_code.is_success() {
        serde_json::from_str::<UploadResponse>(&body)
            .map_err(|e| format!("Failed to parse response: {} (body: {})", e, body))
    } else {
        if let Ok(parsed) = serde_json::from_str::<UploadResponse>(&body) {
            if let Some(error) = parsed.error {
                return Err(error);
            }
        }
        Err(format!("HTTP {}: {}", status_code, body))
    }
}
