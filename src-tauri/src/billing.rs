// ── Hybrid billing model (Arena.ai gateway + BYOK) ───────────────────────────
//
// The free tier is funded by the developer: the first FREE_UPLOAD_LIMIT (= 3)
// successful extractions route through the Arena.ai Enterprise API Gateway
// using the developer's embedded API key. The free tier is capped by a local
// SQLite counter (`usage_config.free_uploads_used`) that only ever ticks up
// on a 200 OK from the gateway.
//
// Beyond the cap, the user must supply a personal LLM key in `usage_config`.
// When that key is present the command bypasses the gateway entirely and
// calls the upstream provider directly using the user's key.
//
// Two safety nets are enforced in the calling command BEFORE any of the
// network code in this file runs:
//   1. A per-app Mutex that rejects concurrent invocations with 429.
//   2. A 60 000-character pre-flight cap on the extracted PDF text.
//
// This file owns only the route decision and the actual HTTP wiring.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// Hard pre-flight cap on the extracted PDF text, in characters. Anything
/// larger than this is dropped locally — the gateway never sees it.
pub const MAX_PDF_TEXT_CHARS: usize = 60_000;

/// Hard upper bound on the model output. Sent both as a JSON body field and
/// (in the gateway case) as the `X-Arena-Max-Tokens` header so we are
/// protected against runaway completions even if the gateway strips one
/// signal but not the other.
pub const MAX_OUTPUT_TOKENS: u32 = 15_000;

/// Hard request timeout for the reqwest client. 45 seconds — anything longer
/// would block the UI for too long and most LLM calls finish well under
/// this when capped at 15k tokens.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// Developer-side defaults. The developer's Arena.ai gateway key is embedded
/// at build time via the `MERGEMARK_ARENA_API_KEY` env var. If the env var
/// is absent we fall back to a development placeholder string so the binary
/// still links and runs in `cargo test` environments; calls will fail with
/// 401, which is the correct behaviour for a missing key.
pub const ARENA_GATEWAY_URL: &str = "https://api.arena.ai/v1";
pub const ARENA_MODEL: &str = "arena-pro";

fn arena_developer_key() -> &'static str {
    // The key is baked in at compile time. Using a `static` rather than
    // `include_str!` keeps the constant initialised lazily only on first
    // use, which is the right behaviour for the free-tier codepath that
    // will short-circuit before we ever need it.
    option_env!("MERGEMARK_ARENA_API_KEY").unwrap_or("dev-arena-key-not-set")
}

// ── Route decision ───────────────────────────────────────────────────────────

/// Which transport will service the next extraction request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BillingRoute {
    /// Free tier: the Arena.ai gateway with the developer's embedded key.
    /// `remaining_after` is the value the counter will hold AFTER this
    /// successful 200 OK (used for the "X of N used" UI hint).
    FreeTier { remaining_after: i64 },
    /// BYOK: user's personal LLM key, going direct to the upstream provider.
    Byok,
    /// Free tier is exhausted AND no BYOK key is on file. The command
    /// surfaces this to the React side as a structured error so the UI
    /// can show a "please supply your own key" prompt.
    NeedsByok,
}

/// Decide the route based on the live `free_uploads_used` and the stored
/// BYOK key. `free_uploads_used` is read just before this is called so
/// the counter is fresh.
pub fn pick_route(free_uploads_used: i64, byok_key_present: bool) -> BillingRoute {
    // BYOK has priority over the free tier. If the user has stored a
    // personal key we never charge the free counter, even if the user
    // happens to still have free uploads available.
    if byok_key_present {
        return BillingRoute::Byok;
    }
    if free_uploads_used < crate::db::FREE_UPLOAD_LIMIT {
        let remaining_after = crate::db::FREE_UPLOAD_LIMIT - (free_uploads_used + 1);
        return BillingRoute::FreeTier { remaining_after };
    }
    BillingRoute::NeedsByok
}

// ── Error / response payloads ────────────────────────────────────────────────

/// Wire-shape error that the Tauri command returns to the React frontend.
/// React inspects `code` to switch between:
///   * `payload_too_large`        → "Document is too large"
///   * `too_many_requests`        → "Already running, please wait"
///   * `needs_byok`               → show BYOK prompt
///   * `upstream_*`               → bubble the provider's message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BillingError {
    pub code: String,
    pub message: String,
    /// Best-effort human-friendly hint shown in the toast / modal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Underlying HTTP status if there was one (so the React side can
    /// distinguish 401/403/429/5xx without parsing the message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
    /// How many free uploads the user has used — included on `needs_byok`
    /// so the modal can show "You have used 3 of 3 free uploads".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_uploads_used: Option<i64>,
}

impl BillingError {
    pub fn payload_too_large(len: usize) -> Self {
        Self {
            code: "payload_too_large".into(),
            message: format!(
                "Extracted text is {} characters; the pre-flight cap is {}.",
                len, MAX_PDF_TEXT_CHARS
            ),
            hint: Some(
                "Try splitting the PDF, or remove image-only pages before uploading."
                    .into(),
            ),
            upstream_status: None,
            free_uploads_used: None,
        }
    }

    pub fn too_many_requests() -> Self {
        Self {
            code: "too_many_requests".into(),
            message: "Another extraction is already in progress.".into(),
            hint: Some("Please wait for the current job to finish.".into()),
            upstream_status: None,
            free_uploads_used: None,
        }
    }

    pub fn needs_byok(free_uploads_used: i64) -> Self {
        Self {
            code: "needs_byok".into(),
            message: format!(
                "You have used all {} free uploads. Please add your own API key to continue.",
                crate::db::FREE_UPLOAD_LIMIT
            ),
            hint: Some(
                "Open Settings → API Key. Your key is stored only on this device."
                    .into(),
            ),
            upstream_status: None,
            free_uploads_used: Some(free_uploads_used),
        }
    }

    pub fn upstream(status: u16, body: &str) -> Self {
        let snippet: String = body.chars().take(400).collect();
        Self {
            code: match status {
                401 | 403 => "upstream_unauthorized",
                429 => "upstream_rate_limited",
                s if s >= 500 => "upstream_unavailable",
                _ => "upstream_error",
            }
            .into(),
            message: format!("Upstream returned {}.", status),
            hint: Some(snippet),
            upstream_status: Some(status),
            free_uploads_used: None,
        }
    }

    pub fn network(detail: &str) -> Self {
        Self {
            code: "network".into(),
            message: "Could not reach the LLM provider.".into(),
            hint: Some(detail.chars().take(400).collect()),
            upstream_status: None,
            free_uploads_used: None,
        }
    }
}

// ── The actual transport calls ───────────────────────────────────────────────
//
// Both `call_arena_gateway` and `call_byok_direct` share the same JSON
// body shape (chat/completions). The differences are only the URL, the
// Authorization header, the `X-Arena-*` enforcement headers, and the
// decision of when (if ever) to bump the free-tier counter.

/// Build the shared reqwest client. Hard 45s timeout, no auto-redirect
/// (LLM providers don't redirect), and a sensible user agent so the
/// provider logs identify us.
fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("MergeMark/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("reqwest client must build with the hard-coded timeouts")
}

/// Common body for both transports. The caller decides which model string
/// and which transport to send it to. The `max_tokens` cap is what saves
/// us from runaway bills even if the provider ignores the `X-Arena-Max-Tokens`
/// header.
fn build_chat_body(model: &str, system: &str, user_text: &str) -> Value {
    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user",   "content": user_text }
        ],
        "temperature": 0.1,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "response_format": { "type": "json_object" },
        // Extra defence-in-depth: most providers also honour this field
        // by name and the Arena.ai gateway passes it through.
        "max_output_tokens": MAX_OUTPUT_TOKENS,
    })
}

/// Send the request through the Arena.ai gateway. On a 200 OK the caller
/// must increment `free_uploads_used`; we do NOT touch the DB here so the
/// caller can decide what to do on a 5xx (don't burn a free credit).
///
/// The gateway-specific protections:
///   * `X-Arena-Max-Tokens: 15000`         — gateway enforces the cap
///   * `X-Arena-User-Tier: free`          — gateway picks the free quota path
///   * `X-Arena-Enforce-Limits: strict`   — gateway rejects oversize bodies
///   * Hard 45s timeout                    — reqwest-level kill switch
pub async fn call_arena_gateway(
    model: &str,
    system: &str,
    user_text: &str,
) -> Result<Value, BillingError> {
    let client = build_client();
    let url = format!("{}/chat/completions", ARENA_GATEWAY_URL.trim_end_matches('/'));

    let body = build_chat_body(model, system, user_text);

    let res = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", arena_developer_key()))
        .header("Content-Type", "application/json")
        .header("X-Arena-Max-Tokens", MAX_OUTPUT_TOKENS.to_string())
        .header("X-Arena-User-Tier", "free")
        .header("X-Arena-Enforce-Limits", "strict")
        .timeout(REQUEST_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|e| BillingError::network(&e.to_string()))?;

    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(BillingError::upstream(status.as_u16(), &text));
    }
    res.json::<Value>()
        .await
        .map_err(|e| BillingError::network(&format!("bad response shape: {e}")))
}

/// Send the request directly to the user's chosen LLM provider, using
/// their stored BYOK key. There is no token cap enforced server-side here
/// (we don't control the provider), but the `max_tokens` field in the
/// body is still sent so OpenAI-compatible providers honour it.
pub async fn call_byok_direct(
    byok_base_url: &str,
    byok_api_key: &str,
    model: &str,
    system: &str,
    user_text: &str,
) -> Result<Value, BillingError> {
    let client = build_client();
    let url = format!(
        "{}/chat/completions",
        byok_base_url.trim_end_matches('/')
    );

    let body = build_chat_body(model, system, user_text);

    let res = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", byok_api_key))
        .header("Content-Type", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|e| BillingError::network(&e.to_string()))?;

    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(BillingError::upstream(status.as_u16(), &text));
    }
    res.json::<Value>()
        .await
        .map_err(|e| BillingError::network(&format!("bad response shape: {e}")))
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_bypasses_free_when_byok_present() {
        // Even with 0 free uploads used, a stored BYOK key should win.
        let r = pick_route(0, true);
        assert_eq!(r, BillingRoute::Byok);
    }

    #[test]
    fn route_uses_free_when_quota_remaining() {
        for used in 0..crate::db::FREE_UPLOAD_LIMIT {
            let r = pick_route(used, false);
            match r {
                BillingRoute::FreeTier { remaining_after } => {
                    assert_eq!(remaining_after, crate::db::FREE_UPLOAD_LIMIT - (used + 1));
                }
                other => panic!("expected FreeTier at used={used}, got {other:?}"),
            }
        }
    }

    #[test]
    fn route_blocks_when_quota_exhausted() {
        let r = pick_route(crate::db::FREE_UPLOAD_LIMIT, false);
        assert_eq!(r, BillingRoute::NeedsByok);
        let r = pick_route(crate::db::FREE_UPLOAD_LIMIT + 1, false);
        assert_eq!(r, BillingRoute::NeedsByok);
    }

    #[test]
    fn byok_rescues_an_exhausted_quota() {
        // If the user is out of free credits but supplied a key, route
        // through BYOK rather than blocking.
        let r = pick_route(crate::db::FREE_UPLOAD_LIMIT, true);
        assert_eq!(r, BillingRoute::Byok);
    }

    #[test]
    fn pre_flight_cap_constant_is_60k() {
        assert_eq!(MAX_PDF_TEXT_CHARS, 60_000);
    }

    #[test]
    fn max_output_tokens_constant_is_15k() {
        assert_eq!(MAX_OUTPUT_TOKENS, 15_000);
    }

    #[test]
    fn request_timeout_is_45s() {
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(45));
    }
}
