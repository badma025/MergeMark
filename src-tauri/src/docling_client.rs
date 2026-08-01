use reqwest::multipart;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct DoclingResponse {
    pub status: String,
    pub markdown: Option<String>,
    pub message: Option<String>,
}

pub async fn extract_pdf(bytes: Vec<u8>, filename: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    
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
        // Docling API might return a 500 error or similar without the JSON structure,
        // or it might return the JSON structure. Let's try to parse it first.
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if let Ok(json) = serde_json::from_str::<DoclingResponse>(&text) {
            if json.status == "error" {
                return Err(json.message.unwrap_or_else(|| "Unknown Docling error".to_string()));
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

    result
        .markdown
        .ok_or_else(|| "Docling returned success but no markdown field".to_string())
}
