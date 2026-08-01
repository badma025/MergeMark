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

#[derive(Deserialize, Clone, Debug)]
pub struct DoclingResponse {
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

pub async fn extract_pdf(
    bytes: Vec<u8>,
    filename: &str,
    app: &AppHandle,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let part = multipart::Part::bytes(bytes)
        .file_name(filename.to_string())
        .mime_str("application/pdf")
        .map_err(|e| format!("Failed to create multipart body: {}", e))?;

    let form = multipart::Form::new().part("file", part);

    let base_url = std::env::var("DOCLING_SERVICE_URL")
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
        .map_err(|e| format!("Failed to send request to Docling: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if let Ok(json) = serde_json::from_str::<DoclingResponse>(&text) {
            if json.status == "error" {
                return Err(
                    json.message
                        .unwrap_or_else(|| "Unknown Docling error".to_string()),
                );
            }
        }
        return Err(format!("Docling API returned {}: {}", status, text));
    }

    let result: DoclingResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON from Docling: {}", e))?;

    if result.status != "success" {
        return Err(result.message.unwrap_or_else(|| "Unknown Docling error".to_string()));
    }

    let mut markdown = result
        .markdown
        .ok_or_else(|| "Docling returned success but no markdown field".to_string())?;

    if let Some(mut images) = result.images {
        if !images.is_empty() {
            let dir = diagrams_dir(app)?;
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("Failed to create diagrams directory: {}", e))?;

            // Sort images by filename length descending to prevent partial substring matches
            images.sort_by(|a, b| b.filename.len().cmp(&a.filename.len()));

            for img in images {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&img.base64)
                    .map_err(|e| format!("Failed to decode base64 image {}: {}", img.filename, e))?;

                let unique_filename = format!("{}_{}", uuid::Uuid::new_v4(), img.filename);
                let dest_path = dir.join(&unique_filename);

                std::fs::write(&dest_path, &decoded)
                    .map_err(|e| format!("Failed to write image {}: {}", dest_path.display(), e))?;

                let abs_path_str = dest_path.to_string_lossy().replace('\\', "/");
                markdown = markdown.replace(&img.filename, &abs_path_str);
            }
        }
    }

    Ok(markdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docling_response_deserialization_with_images() {
        let json = r#"{
            "status": "success",
            "markdown": "# Question 1\n![Diagram](figure1.png)",
            "images": [
                {
                    "filename": "figure1.png",
                    "base64": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
                }
            ]
        }"#;

        let resp: DoclingResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "success");
        assert!(resp.markdown.is_some());
        assert_eq!(resp.images.as_ref().unwrap().len(), 1);
        assert_eq!(resp.images.as_ref().unwrap()[0].filename, "figure1.png");
    }

    #[test]
    fn test_docling_response_deserialization_without_images() {
        let json = r#"{
            "status": "success",
            "markdown": "# Question 1"
        }"#;

        let resp: DoclingResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "success");
        assert!(resp.markdown.is_some());
        assert!(resp.images.is_none());
    }
}
