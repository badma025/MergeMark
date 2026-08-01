use base64::Engine as _;
use reqwest::multipart;
use serde::Deserialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Deserialize, Clone, Debug)]
pub struct ExtractedImage {
    pub filename: String,
    pub base64: String,
}

#[derive(Deserialize)]
pub struct MinerUResponse {
    pub status: String,
    pub markdown: Option<String>,
    pub message: Option<String>,
    #[serde(default)]
    pub images: Option<Vec<ExtractedImage>>,
}

fn diagrams_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("diagrams"))
        .map_err(|e| format!("Failed to resolve app data directory: {}", e))
}

pub async fn extract_pdf(bytes: Vec<u8>, filename: &str, app: &AppHandle) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
    
    let part = multipart::Part::bytes(bytes)
        .file_name(filename.to_string())
        .mime_str("application/pdf")
        .map_err(|e| format!("Failed to create multipart body: {}", e))?;
        
    let form = multipart::Form::new().part("file", part);

    let base_url = std::env::var("MODAL_SERVICE_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());
    let base_url = base_url.trim_end_matches('/');
    let endpoint = if base_url.ends_with("/extract") {
        base_url.to_string()
    } else {
        format!("{}/extract", base_url)
    };

    let response = client
        .post(&endpoint)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Failed to send request to MinerU: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if let Ok(json) = serde_json::from_str::<MinerUResponse>(&text) {
            if json.status == "error" {
                return Err(json.message.unwrap_or_else(|| "Unknown MinerU error".to_string()));
            }
        }
        return Err(format!("MinerU API returned {}: {}", status, text));
    }

    let result: MinerUResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON from MinerU: {}", e))?;

    if result.status != "success" {
        return Err(result.message.unwrap_or_else(|| "Unknown MinerU error".to_string()));
    }

    let mut markdown = result
        .markdown
        .ok_or_else(|| "MinerU returned success but no markdown field".to_string())?;

    // Process images
    if let Some(images) = result.images {
        let dir = diagrams_dir(app)?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create diagrams directory: {}", e))?;

        for img in images {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&img.base64)
                .map_err(|e| format!("Failed to decode base64 image {}: {}", img.filename, e))?;
            
            let file_path = dir.join(&img.filename);
            std::fs::write(&file_path, &bytes)
                .map_err(|e| format!("Failed to write image {}: {}", img.filename, e))?;
                
            // Replace MinerU output references (e.g. ![...](figure.png)) to absolute paths
            let abs_path_str = file_path.to_string_lossy().replace("\\", "/");
            markdown = markdown.replace(&img.filename, &abs_path_str);
        }
    }

    Ok(markdown)
}
