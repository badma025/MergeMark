// ── LLM client boundary ─────────────────────────────────────────────────────
//
// All HTTP to the model goes through `LlmClient` so the pipeline can be
// driven deterministically by `MockLlm` in tests — no network, no API key,
// no nondeterminism. Retry policy is defined ONCE here and applies to every
// call site (previously the question path, mark-scheme path, classifier, and
// tagger each had their own inconsistent handling).

#[derive(Debug, Clone)]
pub enum LlmError {
    /// request never got a usable HTTP response
    Network(String),
    /// a non-success HTTP status
    Http { status: u16, body: String },
    /// still rate-limited after the backoff budget
    RateLimited,
    /// response was 2xx but had no message content
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

/// Response format for structured outputs. Some providers (OpenAI, some
/// OpenRouter models) support JSON Schema via the `response_format`
/// parameter. Use `JsonSchema` to request strict schema-validated output.
#[derive(Debug, Clone)]
pub enum ResponseFormat {
    #[allow(dead_code)]
    JsonObject,
    JsonSchema { schema: serde_json::Value },
}

/// Vision image `detail` hint for OpenAI-style providers. `Low` sends a
/// single 512-px tile; `High` requests 768-px tiling for fine print. Gemini
/// and Anthropic ignore this field entirely — for those providers the real
/// cost lever is the image's pixel dimensions, so callers also cap size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDetail {
    Low,
    High,
}

impl ImageDetail {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImageDetail::Low => "low",
            ImageDetail::High => "high",
        }
    }
}

/// One chat completion call. The caller awaits the boxed future — this keeps
/// the trait object-safe without pulling in an extra crate.
pub trait LlmClient: Send + Sync {
    fn chat<'a>(
        &'a self,
        body: &'a serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, LlmError>> + Send + 'a>,
    >;
}

/// Build a standard OpenAI-compatible chat request body (json_object mode).
/// `images` are base64 page renders. Existing data URLs retain their MIME
/// type; legacy raw-base64 inputs are treated as WebP. `detail` applies to
/// every image in the call; callers should pass `ImageDetail::Low` only for
/// tightly-cropped single-question bands (OpenAI tiling savings) and
/// `ImageDetail::High` for any call that contains a full page.
pub fn chat_body<S: AsRef<str>>(
    model: &str,
    system: &str,
    images: &[S],
    detail: ImageDetail,
    text: Option<&str>,
    max_tokens: u32,
    response_format: Option<ResponseFormat>,
) -> serde_json::Value {
    let mut content: Vec<serde_json::Value> = Vec::new();
    if let Some(t) = text {
        content.push(serde_json::json!({ "type": "text", "text": t }));
    }
    for img in images {
        // Phase 0: mirror pipeline::is_sentinel_b64. Anything that isn't real
        // base64 JPEG must be dropped here so it never reaches the vision API
        // as a bogus image. We also accept legacy sentinels so old tests and
        // code paths don't accidentally ship "TEXT_ONLY" as an image.
        let t = img.as_ref().trim();
        if t.is_empty()
            || t == "__SKIP__"
            || t == "SKIP"
            || t == "__TEXT_ONLY__"
            || t == "TEXT_ONLY"
        {
            continue;
        }
        // Preserve the source MIME type when a data URL is supplied.
        // Legacy raw-base64 callers default to WebP.
        let image_url = if t.starts_with("data:image/") && t.contains(',') {
            t.to_string()
        } else {
            format!("data:image/webp;base64,{}", crate::geometry::strip_data_url(t))
        };
        // Phase 0: OpenAI-style vision APIs honour a "detail" hint. "high"
        // forces 768-px tiles and lets the model see fine detail (small
        // subscripts, axis labels, circuit symbols). Providers that don't
        // understand this field (Gemini, Anthropic) ignore it safely. API
        // images are capped at 768 px on the long edge by the render path,
        // so "high" maps to at most a few tiles and "low" to a single tile.
        content.push(serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": image_url,
                "detail": detail.as_str()
            }
        }));
    }
    let user_content = if content.is_empty() {
        serde_json::json!("")
    } else if content.len() == 1 && content[0]["type"] == "text" {
        serde_json::json!(content[0]["text"])
    } else {
        serde_json::json!(content)
    };

    let rf = match response_format {
        Some(ResponseFormat::JsonSchema { schema }) => serde_json::json!({
            "type": "json_schema",
            "json_schema": schema
        }),
        _ => serde_json::json!({ "type": "json_object" }),
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

    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user_content }
        ],
        "temperature": 0.1,
        "max_tokens": max_tokens,
        "response_format": rf,
        "reasoning": { "effort": reasoning_effort }
    })
}

/// Pull `choices[0].message.content` out of a chat completion response.
pub fn message_content(resp: &serde_json::Value) -> Result<String, LlmError> {
    resp["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            eprintln!(
                "[DIAGNOSTIC][LLM_SHAPE_ERROR] missing choices[0].message.content; raw response:\n{}",
                serde_json::to_string_pretty(resp)
                    .unwrap_or_else(|_| format!("<unserializable response: {:?}>", resp))
            );
            LlmError::BadShape("missing choices[0].message.content".to_string())
        })
}

/// Real token usage extracted from the API response `usage` block.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// Extract token usage from an OpenAI-compatible chat completion response.
/// Returns `TokenUsage::default()` if the usage block is missing.
#[allow(dead_code)]
pub fn usage_from_response(resp: &serde_json::Value) -> TokenUsage {
    let usage = &resp["usage"];
    if usage.is_null() {
        return TokenUsage::default();
    }
    TokenUsage {
        prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0),
        completion_tokens: usage["completion_tokens"].as_u64().unwrap_or(0),
        total_tokens: usage["total_tokens"].as_u64()
            .unwrap_or_else(|| {
                usage["prompt_tokens"].as_u64().unwrap_or(0)
                    + usage["completion_tokens"].as_u64().unwrap_or(0)
            }),
    }
}

// ── Real client ─────────────────────────────────────────────────────────────

/// Shared HTTP client with connection pooling. All `ReqwestLlm` instances
/// reuse the same underlying connection pool, so parallel API calls to the
/// same host avoid repeated TCP + TLS handshakes. The pool supports up to
/// 8 idle connections per host (matching typical BYOK parallelism) and
/// keeps them alive for 90 seconds.
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
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    std::time::Duration::from_millis(100 + (nanos as u64 % 401))
}

fn retry_after(response: &reqwest::Response) -> Option<std::time::Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
}

impl LlmClient for ReqwestLlm {
    fn chat<'a>(
        &'a self,
        body: &'a serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, LlmError>> + Send + 'a>,
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
                            Ok(body) => body,
                            Err(error) => {
                                eprintln!(
                                    "[LLM][BODY_READ_ERROR] status={} error={}",
                                    status, error
                                );
                                return Err(LlmError::BadShape(format!(
                                    "unable to read provider response body: {}",
                                    error
                                )));
                            }
                        };
                        let trimmed_body = body_text.trim();
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
                        if trimmed_body.is_empty() {
                            eprintln!(
                                "[LLM][EMPTY_BODY] WARN: LLM returned empty body. Check API provider for content filter flags or silent drops."
                            );
                            return Err(LlmError::BadShape(
                                "provider returned an empty response body".to_string(),
                            ));
                        }
                        let resp: serde_json::Value = match serde_json::from_str(&body_text) {
                            Ok(value) => value,
                            Err(error) => {
                                eprintln!(
                                    "[LLM][RESPONSE_JSON_ERROR] error={} raw_body:\n{}",
                                    error, body_text
                                );
                                return Err(LlmError::BadShape(format!(
                                    "invalid provider response JSON: {}",
                                    error
                                )));
                            }
                        };
                        // Empty-content guard: some Kilo-Gateway providers
                        // respond 200 but leave choices[0].message.content
                        // blank or whitespace-only. Retry up to the same
                        // budget used for rate-limit / network errors.
                        if message_content(&resp).is_err() {
                            attempt += 1;
                            if attempt > 3 {
                                return Err(LlmError::BadShape(
                                    "provider returned empty content after retries".to_string(),
                                ));
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(5) + retry_jitter())
                                .await;
                            continue;
                        }
                        return Ok(resp);
                    }
                    Err(e) => {
                        attempt += 1;
                        if attempt > 2 {
                            return Err(LlmError::Network(e.to_string()));
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(5) + retry_jitter()).await;
                    }
                }
            }
        })
    }
}

// ── Test double ─────────────────────────────────────────────────────────────

#[cfg(test)]
pub struct MockLlm {
    pub scripts: std::sync::Mutex<std::collections::VecDeque<Result<serde_json::Value, LlmError>>>,
    pub observed_bodies: std::sync::Mutex<Vec<serde_json::Value>>,
}

#[cfg(test)]
impl MockLlm {
    pub fn new(responses: Vec<Result<serde_json::Value, LlmError>>) -> Self {
        Self {
            scripts: std::sync::Mutex::new(responses.into()),
            observed_bodies: std::sync::Mutex::new(Vec::new()),
        }
    }
    #[allow(dead_code)]
    pub fn push(&self, r: Result<serde_json::Value, LlmError>) {
        self.scripts.lock().unwrap().push_back(r);
    }
    pub fn remaining(&self) -> usize {
        self.scripts.lock().unwrap().len()
    }
    pub fn bodies(&self) -> Vec<serde_json::Value> {
        self.observed_bodies.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl LlmClient for MockLlm {
    fn chat<'a>(
        &'a self,
        body: &'a serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, LlmError>> + Send + 'a>,
    > {
        self.observed_bodies.lock().unwrap().push(body.clone());
        let next = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(LlmError::BadShape("mock script exhausted".to_string())));
        Box::pin(async move { next })
    }
}

/// Wrap a plain string as a chat-completion-shaped response-value, handy in
/// tests: `ok_chat(json_string)` → the Value the real API would return.
#[cfg(test)]
pub fn ok_chat(content: &str) -> Result<serde_json::Value, LlmError> {
    Ok(serde_json::json!({
        "choices": [{ "message": { "content": content } }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_url(body: &serde_json::Value) -> &str {
        body["messages"][1]["content"][0]["image_url"]["url"]
            .as_str()
            .unwrap()
    }

    #[test]
    fn chat_body_preserves_png_data_url() {
        let images = ["data:image/png;base64,AAAA"];
        let body = chat_body("model", "system", &images, ImageDetail::High, None, 100, None);
        assert_eq!(image_url(&body), images[0]);
    }

    #[test]
    fn chat_body_preserves_jpeg_data_url() {
        let images = ["data:image/jpeg;base64,BBBB"];
        let body = chat_body("model", "system", &images, ImageDetail::High, None, 100, None);
        assert_eq!(image_url(&body), images[0]);
    }

    #[test]
    fn chat_body_preserves_webp_data_url() {
        let images = ["data:image/webp;base64,WWWW"];
        let body = chat_body("model", "system", &images, ImageDetail::High, None, 100, None);
        assert_eq!(image_url(&body), images[0]);
    }

    #[test]
    fn chat_body_defaults_raw_base64_to_webp() {
        let images = ["CCCC"];
        let body = chat_body("model", "system", &images, ImageDetail::High, None, 100, None);
        assert_eq!(image_url(&body), "data:image/webp;base64,CCCC");
    }

    #[test]
    fn chat_body_emits_low_detail() {
        let images = ["CCCC"];
        let body = chat_body("model", "system", &images, ImageDetail::Low, None, 100, None);
        assert_eq!(
            body["messages"][1]["content"][0]["image_url"]["detail"],
            "low"
        );
    }

    #[test]
    fn chat_body_emits_high_detail() {
        let images = ["CCCC"];
        let body = chat_body("model", "system", &images, ImageDetail::High, None, 100, None);
        assert_eq!(
            body["messages"][1]["content"][0]["image_url"]["detail"],
            "high"
        );
    }

    #[test]
    fn chat_body_sets_low_reasoning_for_3_7_flash() {
        let images = ["CCCC"];
        let body = chat_body("google/gemini-3.7-flash", "system", &images, ImageDetail::High, None, 100, None);
        assert_eq!(body["reasoning"]["effort"], "low");

        let body2 = chat_body("google/gemini-2.5-flash", "system", &images, ImageDetail::High, None, 100, None);
        assert_eq!(body2["reasoning"]["effort"], "none");
    }
}
