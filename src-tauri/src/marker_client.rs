use reqwest::multipart;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Deserialize, Debug, Clone)]
pub struct MarkerResponse {
    pub status: Option<String>,
    pub markdown: Option<String>,
    pub images: Option<HashMap<String, String>>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MarkerExtraction {
    pub markdown: String,
    pub images: HashMap<String, String>,
}

/// Resolves the Marker / Modal service endpoint URL from environment variables.
pub fn get_service_endpoint() -> String {
    let base_url = std::env::var("MARKER_SERVICE_URL")
        .or_else(|_| std::env::var("MODAL_SERVICE_URL"))
        .unwrap_or_else(|_| "https://badma025--marker-pdf-extraction-markerextractor-extract.modal.run".to_string());
    let base_url = base_url.trim().trim_end_matches('/').to_string();

    // If the base URL is already pointing to an endpoint or modal app root, format cleanly
    if base_url.ends_with("/extract") {
        base_url
    } else if base_url.contains(".modal.run") {
        // Modal FastAPI endpoints exposed on classes are typically available at root or /extract
        base_url
    } else {
        format!("{}/extract", base_url)
    }
}

/// Full extraction returning both extracted Markdown and Base64-encoded figures/diagrams.
pub async fn extract_pdf_full(bytes: Vec<u8>, filename: &str) -> Result<MarkerExtraction, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300)) // 300-second (5-minute) timeout buffer to prevent premature request cancellations
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let part = multipart::Part::bytes(bytes)
        .file_name(filename.to_string())
        .mime_str("application/pdf")
        .map_err(|e| format!("Failed to create multipart body: {e}"))?;

    let form = multipart::Form::new().part("file", part);
    let endpoint = get_service_endpoint();

    let response = client
        .post(&endpoint)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Failed to send request to Marker service at {endpoint}: {e}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .unwrap_or_default();

    if !status.is_success() {
        if let Ok(json) = serde_json::from_str::<MarkerResponse>(&text) {
            if let Some(msg) = json.message {
                return Err(format!("Marker service error ({status}): {msg}"));
            }
        }
        return Err(format!("Marker service returned error {status}: {text}"));
    }

    let result: MarkerResponse = serde_json::from_str(&text)
        .map_err(|e| format!("Failed to parse JSON response from Marker service: {e}. Raw response: {text}"))?;

    if let Some(status_str) = &result.status {
        if status_str.eq_ignore_ascii_case("error") {
            let msg = result.message.unwrap_or_else(|| "Unknown Marker extraction error".to_string());
            return Err(format!("Marker extraction failed: {msg}"));
        }
    }

    let markdown = result
        .markdown
        .ok_or_else(|| "Marker service returned success but no markdown field".to_string())?;

    let images = result.images.unwrap_or_default();

    Ok(MarkerExtraction { markdown, images })
}

/// Convenience function returning just the extracted Markdown string.
pub async fn extract_pdf(bytes: Vec<u8>, filename: &str) -> Result<String, String> {
    let extraction = extract_pdf_full(bytes, filename).await?;
    Ok(extraction.markdown)
}
