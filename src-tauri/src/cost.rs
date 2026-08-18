// ── OpenRouter Real Cost & Spend API Integration ──────────────────────────
//
// Directly queries OpenRouter's official endpoints:
// 1. `GET https://openrouter.ai/api/v1/auth/key` -> Live key usage (USD spent), limits, and label.
// 2. `GET https://openrouter.ai/api/v1/generation?id={gen_id}` -> Exact generation cost & token audit.
// 3. `GET https://openrouter.ai/api/v1/credits` -> Account credit balance & usage.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const OPENROUTER_AUTH_KEY_URL: &str = "https://openrouter.ai/api/v1/auth/key";
pub const OPENROUTER_GENERATION_URL: &str = "https://openrouter.ai/api/v1/generation";
pub const OPENROUTER_CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRouterKeyInfo {
    pub label: String,
    pub usage_usd: f64,
    pub limit_usd: Option<f64>,
    pub is_free_tier: bool,
    pub rate_limit_requests: u32,
    pub rate_limit_interval: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationCostDetails {
    pub id: String,
    pub model: String,
    pub total_cost_usd: f64,
    pub tokens_prompt: u64,
    pub tokens_completion: u64,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRouterCredits {
    pub total_credits: f64,
    pub total_usage: f64,
}

/// Calculate estimated or exact cost based on model and token counts.
/// Rates are USD per 1 M tokens, approximating OpenRouter list prices as
/// of mid-2025. These are only used when the provider omits a real `usage`
/// block (local models / tests) — the command layer prefers the
/// provider-reported counts accumulated in `ImportReport`.
///
/// Vision image tokens are already included in the provider's
/// `prompt_tokens` for OpenAI/Gemini/Anthropic, so no separate image
/// surcharge is applied here.
pub fn calculate_cost(model: &str, prompt_tokens: u64, completion_tokens: u64) -> f64 {
    let m = model.to_lowercase();
    let (in_rate, out_rate) = if m.contains("gemini-2.5-flash") || m.contains("gemini-2.0-flash") {
        // Gemini 2.5/2.0 Flash — the default free-tier model
        (0.30, 2.50)
    } else if m.contains("gemini-flash") || m.contains("gemini-1.5-flash") {
        (0.075, 0.30)
    } else if m.contains("gemini-2.5-pro") || m.contains("gemini-pro") {
        (1.25, 10.00)
    } else if m.contains("deepseek-r1") {
        (0.80, 2.40)
    } else if m.contains("deepseek-chat") || m.contains("deepseek-v3") {
        (0.14, 0.28)
    } else if m.contains("deepseek") {
        (0.14, 0.28)
    } else if m.contains("gpt-4o-mini") {
        (0.15, 0.60)
    } else if m.contains("gpt-4o") {
        (2.50, 10.00)
    } else if m.contains("claude-3-5-haiku") || m.contains("claude-haiku") {
        (0.80, 4.00)
    } else if m.contains("claude-3-7-sonnet") || m.contains("claude-3-5-sonnet") || m.contains("claude-sonnet") {
        (3.00, 15.00)
    } else if m.contains("o1") || m.contains("o3") {
        (15.00, 60.00)
    } else {
        // Unknown model — assume near-flash pricing as a conservative lower bound
        (0.30, 2.50)
    };

    let in_cost = (prompt_tokens as f64 / 1_000_000.0) * in_rate;
    let out_cost = (completion_tokens as f64 / 1_000_000.0) * out_rate;
    in_cost + out_cost
}

/// Fetch live key information and total USD spend directly from OpenRouter
pub async fn fetch_openrouter_key_info(api_key: &str) -> Result<OpenRouterKeyInfo, String> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let res = client
        .get(OPENROUTER_AUTH_KEY_URL)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| format!("Network error querying OpenRouter: {}", e))?;

    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(format!("OpenRouter returned HTTP {}: {}", status, text));
    }

    let val: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenRouter response: {}", e))?;

    let data = &val["data"];
    if data.is_null() {
        return Err("Invalid response from OpenRouter: missing 'data' field".to_string());
    }

    let label = data["label"].as_str().unwrap_or("API Key").to_string();
    let usage_usd = data["usage"].as_f64().unwrap_or(0.0);
    let limit_usd = data["limit"].as_f64();
    let is_free_tier = data["is_free_tier"].as_bool().unwrap_or(false);
    let rate_limit_requests = data["rate_limit"]["requests"].as_u64().unwrap_or(0) as u32;
    let rate_limit_interval = data["rate_limit"]["interval"].as_str().unwrap_or("").to_string();

    Ok(OpenRouterKeyInfo {
        label,
        usage_usd,
        limit_usd,
        is_free_tier,
        rate_limit_requests,
        rate_limit_interval,
    })
}

/// Query OpenRouter for exact cost breakdown of a single generation ID (e.g. `gen-xxxx`)
pub async fn fetch_openrouter_generation_cost(
    generation_id: &str,
    api_key: &str,
) -> Result<GenerationCostDetails, String> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let url = format!("{}?id={}", OPENROUTER_GENERATION_URL, generation_id.trim());

    let res = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| format!("Network error querying OpenRouter generation: {}", e))?;

    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(format!("OpenRouter generation query returned HTTP {}: {}", status, text));
    }

    let val: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenRouter generation response: {}", e))?;

    let data = &val["data"];
    let id = data["id"].as_str().unwrap_or(generation_id).to_string();
    let model = data["model"].as_str().unwrap_or("unknown").to_string();
    let total_cost_usd = data["total_cost"].as_f64().unwrap_or(0.0);
    let tokens_prompt = data["tokens_prompt"].as_u64().unwrap_or(0);
    let tokens_completion = data["tokens_completion"].as_u64().unwrap_or(0);
    let created_at = data["created_at"].as_str().map(|s| s.to_string());

    Ok(GenerationCostDetails {
        id,
        model,
        total_cost_usd,
        tokens_prompt,
        tokens_completion,
        created_at,
    })
}

/// Fetch credit balance and usage
pub async fn fetch_openrouter_credits(api_key: &str) -> Result<OpenRouterCredits, String> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let res = client
        .get(OPENROUTER_CREDITS_URL)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| format!("Network error querying OpenRouter credits: {}", e))?;

    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(format!("OpenRouter credits returned HTTP {}: {}", status, text));
    }

    let val: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenRouter credits response: {}", e))?;

    let data = &val["data"];
    let total_credits = data["total_credits"].as_f64().unwrap_or(0.0);
    let total_usage = data["total_usage"].as_f64().unwrap_or(0.0);

    Ok(OpenRouterCredits {
        total_credits,
        total_usage,
    })
}
