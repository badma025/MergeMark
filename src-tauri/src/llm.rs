// ── LLM client boundary ─────────────────────────────────────────────────────
//
// All HTTP to the model goes through `LlmClient` so the pipeline can be
// driven deterministically by `MockLlm` in tests — no network, no API key,
// no nondeterminism. Retry policy is defined ONCE here and applies to every
// call site (question path, mark-scheme path, taxonomy generation, etc.).

use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub enum LlmError {
    /// request never got a usable HTTP response
    Network(String),
    /// a non-success HTTP status
    Http { status: u16, body: String },
    /// still rate-limited after the backoff budget was exhausted
    RateLimited,
    /// response was 2xx but had no usable message content
    BadShape(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Network(e) => write!(f, "network error: {e}"),
            LlmError::Http { status, body } => {
                let snippet: String = body.chars().take(300).collect();
                write!(f, "API error {status}: {snippet}")
            }
            LlmError::RateLimited => write!(f, "rate limited (429) after retries"),
            LlmError::BadShape(e) => write!(f, "unexpected response shape: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    #[allow(dead_code)]
    pub model: String,
    pub timeout: std::time::Duration,
}

/// Structured-output mode. Some providers (OpenAI, some OpenRouter models)
/// support JSON Schema via the `response_format` parameter.
#[derive(Debug, Clone)]
pub enum ResponseFormat {
    #[allow(dead_code)]
    JsonObject,
    JsonSchema { schema: Value },
}

/// Vision-detail hint for image inputs, mirroring the OpenAI/Gemini API.
///
/// * `Low`  — the provider downscales the image to a single ~512 px tile
///            and bills a flat ~85-255 tokens. Used for the structure pass,
///            which only needs to locate question headings and y-bands.
/// * `High` — the image is tiled at 768 px (2048 px long edge) for fine
///            detail (subscripts, Greek letters, circuit symbols). Used for
///            the extraction/transcription pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDetail {
    Low,
    High,
}

impl ImageDetail {
    fn as_str(self) -> &'static str {
        match self {
            ImageDetail::Low => "low",
            ImageDetail::High => "high",
        }
    }
}

/// One chat completion call. The returned tuple carries both the raw JSON
/// response and the provider-reported token usage (zeroed when the provider
/// omits a `usage` block). The boxed future keeps the trait object-safe.
pub trait LlmClient: Send + Sync {
    fn chat<'a>(
        &'a self,
        body: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, LlmError>> + Send + 'a>,
    >;

    /// Same as `chat`, but also returns the parsed `usage` block so callers
    /// can record exact prompt/completion token counts for cost accounting.
    fn chat_usage<'a>(
        &'a self,
        body: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(Value, TokenUsage), LlmError>> + Send + 'a>,
    >;
}

/// Build the system-message portion of a chat body. When `cache` is true the
/// single text block is annotated with an Anthropic/OpenRouter
/// `cache_control: { type: "ephemeral" }` marker; providers that don't
/// understand the marker ignore it safely. The large, static pipeline
/// prompts are repeated dozens of times per import, so caching the prefix
/// cuts the billed input cost by 50-90 % on supporting providers.
fn system_message(system: &str, cache: bool) -> Value {
    if cache {
        json!([
            {
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" }
            }
        ])
    } else {
        json!(system)
    }
}

/// Build a standard OpenAI-compatible chat request body (JSON mode by
/// default). `images` are base64 page renders; existing data URLs keep
/// their MIME type, while bare base64 is treated as WebP (the format the
/// pipeline emits after downscaling).
///
/// * `image_detail` controls per-image vision-token billing — use `Low` for
///   structural/banding calls and `High` for the transcription pass.
/// * `cache` enables prompt-prefix caching for the system message.
#[allow(clippy::too_many_arguments)]
pub fn chat_body<S: AsRef<str>>(
    model: &str,
    system: &str,
    images: &[S],
    text: Option<&str>,
    max_tokens: u32,
    response_format: Option<ResponseFormat>,
    image_detail: ImageDetail,
    cache: bool,
) -> Value {
    let mut content: Vec<Value> = Vec::new();
    if let Some(t) = text {
        content.push(json!({ "type": "text", "text": t }));
    }
    // Mirror pipeline sentinel values: anything that isn't a real base64
    // image is dropped here so we never ship bogus data to the vision API.
    for img in images {
        let t = img.as_ref().trim();
        if t.is_empty()
            || t == "__SKIP__"
            || t == "SKIP"
            || t == "__TEXT_ONLY__"
            || t == "TEXT_ONLY"
        {
            continue;
        }
        let image_url = if t.starts_with("data:image/") && t.contains(',') {
            t.to_string()
        } else {
            format!("data:image/webp;base64,{}", crate::geometry::strip_data_url(t))
        };
        // `detail` is honoured by OpenAI/Gemini-style vision APIs. Anthropic
        // and other providers ignore the field safely. "low" bills a flat
        // thumbnail tile; "high" tiles a 2048 px long edge at 768 px.
        content.push(json!({
            "type": "image_url",
            "image_url": {
                "url": image_url,
                "detail": image_detail.as_str()
            }
        }));
    }
    let user_content = if content.is_empty() {
        json!("")
    } else if content.len() == 1 && content[0]["type"] == "text" {
        json!(content[0]["text"])
    } else {
        json!(content)
    };

    let rf = match response_format {
        Some(ResponseFormat::JsonSchema { schema }) => json!({
            "type": "json_schema",
            "json_schema": schema
        }),
        _ => json!({ "type": "json_object" }),
    };

    let m_lower = model.to_lowercase();
    let reasoning_effort = if m_lower.contains("3.7-flash")
        || m_lower.contains("3.7_flash")
        || (m_lower.contains("3.7") && m_lower.contains("flash"))
    {
        "low"
    } else {
        "none"
    };

    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_message(system, cache) },
            { "role": "user", "content": user_content }
        ],
        "temperature": 0.1,
        "max_tokens": max_tokens,
        "response_format": rf,
        "reasoning": { "effort": reasoning_effort }
    })
}

/// Build a TEXT-ONLY request body for repairing a failed response. Instead
/// of re-sending the page images (which dominate token cost), this replays
/// the original user request, the model's bad output, and a precise repair
/// instruction as a multi-turn conversation. The model already saw the
/// images on attempt 1, so structural/JSON/bbox errors can be corrected
/// without any vision tokens — typically a 5-15x reduction in repair
/// prompt cost. `response_format`/reasoning match the original call.
pub fn chat_body_repair(
    model: &str,
    system: &str,
    original_user_text: &str,
    assistant_response: &str,
    repair_instruction: &str,
    max_tokens: u32,
    response_format: Option<ResponseFormat>,
    cache: bool,
) -> Value {
    let rf = match response_format {
        Some(ResponseFormat::JsonSchema { schema }) => json!({
            "type": "json_schema",
            "json_schema": schema
        }),
        _ => json!({ "type": "json_object" }),
    };

    let m_lower = model.to_lowercase();
    let reasoning_effort = if m_lower.contains("3.7-flash")
        || m_lower.contains("3.7_flash")
        || (m_lower.contains("3.7") && m_lower.contains("flash"))
    {
        "low"
    } else {
        "none"
    };

    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_message(system, cache) },
            { "role": "user", "content": original_user_text },
            { "role": "assistant", "content": assistant_response },
            { "role": "user", "content": repair_instruction }
        ],
        "temperature": 0.1,
        "max_tokens": max_tokens,
        "response_format": rf,
        "reasoning": { "effort": reasoning_effort }
    })
}

/// Pull `choices[0].message.content` out of a chat-completion response.
pub fn message_content(resp: &Value) -> Result<String, LlmError> {
    resp["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            eprintln!(
                "[DIAGNOSTIC][LLM_SHAPE_ERROR] missing choices[0].message.content; raw response:\n{}",
                serde_json::to_string_pretty(resp)
                    .unwrap_or_else(|_| format!("<unserializable response: {resp:?}>"))
            );
            LlmError::BadShape("missing choices[0].message.content".to_string())
        })
}

/// Real token counts reported by the provider in the `usage` block. These
/// are the authoritative numbers for cost accounting — prompt_tokens
/// already includes vision image tiles for OpenAI/Gemini/Anthropic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// Tokens served from a prompt cache (discounted by providers).
    pub cached_tokens: u64,
}

/// Parse an OpenAI-compatible `usage` block, tolerating the field-name
/// variants used by OpenRouter/Anthropic (`input_tokens`/`output_tokens`)
/// and Anthropic's `cache_read_input_tokens`.
pub fn usage_from_response(resp: &Value) -> TokenUsage {
    let usage = &resp["usage"];
    if usage.is_null() {
        return TokenUsage::default();
    }
    let prompt = usage["prompt_tokens"]
        .as_u64()
        .or_else(|| usage["input_tokens"].as_u64())
        .unwrap_or(0);
    let completion = usage["completion_tokens"]
        .as_u64()
        .or_else(|| usage["output_tokens"].as_u64())
        .unwrap_or(0);
    let total = usage["total_tokens"]
        .as_u64()
        .unwrap_or(prompt + completion);
    // Anthropic-through-OpenRouter surfaces cached tokens under
    // `prompt_tokens_details.cached_tokens`; some builds expose
    // `cache_read_input_tokens` at the top level.
    let cached = usage["prompt_tokens_details"]["cached_tokens"]
        .as_u64()
        .or_else(|| usage["cache_read_input_tokens"].as_u64())
        .unwrap_or(0);
    TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cached_tokens: cached,
    }
}

// ── Real HTTP client ────────────────────────────────────────────────────────

/// Shared connection-pool client. Every `ReqwestLlm` reuses the same pool
/// so parallel calls to the same host avoid repeated TCP/TLS handshakes;
/// up to 8 idle connections are kept alive for 90 s to match the default
/// BYOK parallelism.
fn shared_http_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(8)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_keepalive(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build shared HTTP client")
    })
}

pub struct ReqwestLlm {
    client: reqwest::Client,
    config: LlmConfig,
}

impl ReqwestLlm {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            client: shared_http_client().clone(),
            config,
        }
    }
}

fn retry_jitter() -> std::time::Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    std::time::Duration::from_millis(100 + (nanos as u64 % 401))
}

fn retry_after(response: &reqwest::Response) -> Option<std::time::Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
}

impl LlmClient for ReqwestLlm {
    fn chat<'a>(
        &'a self,
        body: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, LlmError>> + Send + 'a>,
    > {
        Box::pin(async move { Ok(self.chat_usage(body).await?.0) })
    }

    fn chat_usage<'a>(
        &'a self,
        body: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(Value, TokenUsage), LlmError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let url = format!(
                "{}/chat/completions",
                self.config.base_url.trim_end_matches('/')
            );
            let mut attempt: u32 = 0;
            loop {
                let res = self
                    .client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.config.api_key))
                    .timeout(self.config.timeout)
                    .json(body)
                    .send()
                    .await;

                match res {
                    Ok(r) => {
                        let status = r.status();
                        let provider_delay = retry_after(&r);
                        let body_text = match r.text().await {
                            Ok(b) => b,
                            Err(error) => {
                                eprintln!(
                                    "[LLM][BODY_READ_ERROR] status={} error={}",
                                    status, error
                                );
                                return Err(LlmError::BadShape(format!(
                                    "unable to read provider response body: {error}"
                                )));
                            }
                        };
                        if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                            || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                        {
                            eprintln!(
                                "[LLM][RETRYABLE_HTTP] status={} raw_body:\n{}",
                                status, body_text
                            );
                            attempt += 1;
                            if attempt > 3 {
                                return Err(LlmError::RateLimited);
                            }
                            let backoff = provider_delay.unwrap_or_else(|| {
                                std::time::Duration::from_secs(10 * (1 << (attempt - 1)))
                            });
                            tokio::time::sleep(backoff + retry_jitter()).await;
                            continue;
                        }
                        if !status.is_success() {
                            eprintln!(
                                "[LLM][HTTP_ERROR] status={} raw_body:\n{}",
                                status, body_text
                            );
                            return Err(LlmError::Http {
                                status: status.as_u16(),
                                body: body_text,
                            });
                        }
                        if body_text.trim().is_empty() {
                            eprintln!(
                                "[LLM][EMPTY_BODY] WARN: LLM returned empty body — check provider content-filter flags or silent drops."
                            );
                            return Err(LlmError::BadShape(
                                "provider returned an empty response body".to_string(),
                            ));
                        }
                        let resp: Value = match serde_json::from_str(&body_text) {
                            Ok(v) => v,
                            Err(error) => {
                                eprintln!(
                                    "[LLM][RESPONSE_JSON_ERROR] error={} raw_body:\n{}",
                                    error, body_text
                                );
                                return Err(LlmError::BadShape(format!(
                                    "invalid provider response JSON: {error}"
                                )));
                            }
                        };
                        // Empty-content guard: some gateways return 200 but
                        // leave choices[0].message.content blank. Retry with
                        // the same budget used for 429/network errors.
                        if message_content(&resp).is_err() {
                            attempt += 1;
                            if attempt > 3 {
                                return Err(LlmError::BadShape(
                                    "provider returned empty content after retries".to_string(),
                                ));
                            }
                            tokio::time::sleep(
                                std::time::Duration::from_secs(5) + retry_jitter(),
                            )
                            .await;
                            continue;
                        }
                        let usage = usage_from_response(&resp);
                        return Ok((resp, usage));
                    }
                    Err(e) => {
                        attempt += 1;
                        if attempt > 2 {
                            return Err(LlmError::Network(e.to_string()));
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(5) + retry_jitter())
                            .await;
                    }
                }
            }
        })
    }
}

// ── Test double ─────────────────────────────────────────────────────────────

#[cfg(test)]
pub struct MockLlm {
    pub scripts: std::sync::Mutex<std::collections::VecDeque<Result<Value, LlmError>>>,
    pub observed_bodies: std::sync::Mutex<Vec<Value>>,
}

#[cfg(test)]
impl MockLlm {
    pub fn new(responses: Vec<Result<Value, LlmError>>) -> Self {
        Self {
            scripts: std::sync::Mutex::new(responses.into()),
            observed_bodies: std::sync::Mutex::new(Vec::new()),
        }
    }

    #[allow(dead_code)]
    pub fn push(&self, r: Result<Value, LlmError>) {
        self.scripts.lock().unwrap().push_back(r);
    }

    pub fn remaining(&self) -> usize {
        self.scripts.lock().unwrap().len()
    }

    pub fn bodies(&self) -> Vec<Value> {
        self.observed_bodies.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl LlmClient for MockLlm {
    fn chat<'a>(
        &'a self,
        body: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, LlmError>> + Send + 'a>,
    > {
        Box::pin(async move { Ok(self.chat_usage(body).await?.0) })
    }

    fn chat_usage<'a>(
        &'a self,
        body: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(Value, TokenUsage), LlmError>> + Send + 'a>,
    > {
        self.observed_bodies.lock().unwrap().push(body.clone());
        let next = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(LlmError::BadShape(
                "mock script exhausted".to_string(),
            )));
        Box::pin(async move {
            // Tests don't model real token counts; report zero and let the
            // pipeline's image-aware estimate fill in if needed.
            Ok((next?, TokenUsage::default()))
        })
    }
}

/// Wrap a string as a chat-completion response value — convenient in tests.
#[cfg(test)]
pub fn ok_chat(content: &str) -> Result<Value, LlmError> {
    Ok(json!({
        "choices": [{ "message": { "content": content } }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_url(body: &Value) -> &str {
        body["messages"][1]["content"][0]["image_url"]["url"]
            .as_str()
            .unwrap()
    }

    fn image_detail(body: &Value) -> &str {
        body["messages"][1]["content"][0]["image_url"]["detail"]
            .as_str()
            .unwrap()
    }

    #[test]
    fn chat_body_preserves_png_data_url() {
        let images = ["data:image/png;base64,AAAA"];
        let body = chat_body(
            "model", "system", &images, None, 100, None,
            ImageDetail::High, false,
        );
        assert_eq!(image_url(&body), images[0]);
    }

    #[test]
    fn chat_body_preserves_jpeg_data_url() {
        let images = ["data:image/jpeg;base64,BBBB"];
        let body = chat_body(
            "model", "system", &images, None, 100, None,
            ImageDetail::High, false,
        );
        assert_eq!(image_url(&body), images[0]);
    }

    #[test]
    fn chat_body_preserves_webp_data_url() {
        let images = ["data:image/webp;base64,WWWW"];
        let body = chat_body(
            "model", "system", &images, None, 100, None,
            ImageDetail::High, false,
        );
        assert_eq!(image_url(&body), images[0]);
    }

    #[test]
    fn chat_body_defaults_raw_base64_to_webp() {
        let images = ["CCCC"];
        let body = chat_body(
            "model", "system", &images, None, 100, None,
            ImageDetail::High, false,
        );
        assert_eq!(image_url(&body), "data:image/webp;base64,CCCC");
    }

    #[test]
    fn chat_body_sets_low_reasoning_for_3_7_flash() {
        let images = ["CCCC"];
        let body = chat_body(
            "google/gemini-3.7-flash", "system", &images, None, 100, None,
            ImageDetail::High, false,
        );
        assert_eq!(body["reasoning"]["effort"], "low");

        let body2 = chat_body(
            "google/gemini-2.5-flash", "system", &images, None, 100, None,
            ImageDetail::High, false,
        );
        assert_eq!(body2["reasoning"]["effort"], "none");
    }

    #[test]
    fn chat_body_applies_low_image_detail() {
        let images = ["data:image/jpeg;base64,BBBB"];
        let body = chat_body(
            "model", "system", &images, None, 100, None,
            ImageDetail::Low, false,
        );
        assert_eq!(image_detail(&body), "low");
    }

    #[test]
    fn chat_body_high_detail_is_used_for_transcription() {
        let images = ["data:image/jpeg;base64,BBBB"];
        let body = chat_body(
            "model", "system", &images, None, 100, None,
            ImageDetail::High, false,
        );
        assert_eq!(image_detail(&body), "high");
    }

    #[test]
    fn chat_body_cache_flag_marks_system_message() {
        let body = chat_body(
            "anthropic/claude-3.5-sonnet", "SYS", &[] as &[&str], None, 100, None,
            ImageDetail::High, true,
        );
        // With caching on, the system message becomes a content array whose
        // sole text block carries the ephemeral cache_control marker.
        let sys = &body["messages"][0]["content"];
        assert!(sys.is_array(), "cached system content must be an array");
        assert_eq!(sys[0]["type"], "text");
        assert_eq!(sys[0]["text"], "SYS");
        assert_eq!(sys[0]["cache_control"]["type"], "ephemeral");

        // Without caching it stays a plain string.
        let body2 = chat_body(
            "anthropic/claude-3.5-sonnet", "SYS", &[] as &[&str], None, 100, None,
            ImageDetail::High, false,
        );
        assert_eq!(body2["messages"][0]["content"], "SYS");
    }

    #[test]
    fn chat_body_repair_builds_four_turn_text_only() {
        let body = chat_body_repair(
            "google/gemini-2.5-flash",
            "SYS",
            "ORIGINAL USER REQUEST",
            "BAD MODEL OUTPUT",
            "FIX THIS PLEASE",
            4096,
            None,
            true,
        );
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4, "repair body has 4 turns");
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "ORIGINAL USER REQUEST");
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[2]["content"], "BAD MODEL OUTPUT");
        assert_eq!(msgs[3]["role"], "user");
        assert_eq!(msgs[3]["content"], "FIX THIS PLEASE");
        // No images parameter in a text-only repair.
        assert!(body.get("images").is_none() || body["messages"][1].get("images").is_none());
        assert_eq!(body["max_tokens"], 4096);
    }

    #[test]
    fn usage_from_response_reads_standard_block() {
        let resp = json!({
            "usage": { "prompt_tokens": 1200, "completion_tokens": 340, "total_tokens": 1540 }
        });
        let u = usage_from_response(&resp);
        assert_eq!(u.prompt_tokens, 1200);
        assert_eq!(u.completion_tokens, 340);
        assert_eq!(u.total_tokens, 1540);
    }

    #[test]
    fn usage_from_response_handles_anthropic_variants_and_cache() {
        let resp = json!({
            "usage": {
                "input_tokens": 900,
                "output_tokens": 100,
                "cache_read_input_tokens": 700
            }
        });
        let u = usage_from_response(&resp);
        assert_eq!(u.prompt_tokens, 900);
        assert_eq!(u.completion_tokens, 100);
        assert_eq!(u.cached_tokens, 700);
    }

    #[test]
    fn usage_from_response_defaults_when_missing() {
        let resp = json!({ "choices": [] });
        let u = usage_from_response(&resp);
        assert_eq!(u, TokenUsage::default());
    }
}
