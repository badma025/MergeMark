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
/// type; legacy raw-base64 inputs are treated as JPEG.
pub fn chat_body<S: AsRef<str>>(
    model: &str,
    system: &str,
    images: &[S],
    text: Option<&str>,
    max_tokens: u32,
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
        // Preserve the source MIME type when a data URL is supplied. PDF
        // renders containing vector objects are PNGs; relabelling their bytes
        // as JPEG produces an invalid payload for strict vision providers.
        // Legacy raw-base64 callers still default to JPEG.
        let image_url = if t.starts_with("data:image/") && t.contains(',') {
            t.to_string()
        } else {
            format!("data:image/jpeg;base64,{}", crate::geometry::strip_data_url(t))
        };
        // Phase 0: OpenAI-style vision APIs honour a "detail" hint. "high"
        // forces 768-px tiles and lets the model see fine detail (small
        // subscripts, axis labels, circuit symbols). Providers that don't
        // understand this field (Gemini, Anthropic) ignore it safely. At
        // our new ~200 DPI render the 2048-px long edge maps cleanly onto
        // two high-detail tiles.
        content.push(serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": image_url,
                "detail": "high"
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

    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user_content }
        ],
        "temperature": 0.1,
        "max_tokens": max_tokens,
        "response_format": { "type": "json_object" }
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
        let body = chat_body("model", "system", &images, None, 100);
        assert_eq!(image_url(&body), images[0]);
    }

    #[test]
    fn chat_body_preserves_jpeg_data_url() {
        let images = ["data:image/jpeg;base64,BBBB"];
        let body = chat_body("model", "system", &images, None, 100);
        assert_eq!(image_url(&body), images[0]);
    }

    #[test]
    fn chat_body_defaults_raw_base64_to_jpeg() {
        let images = ["CCCC"];
        let body = chat_body("model", "system", &images, None, 100);
        assert_eq!(image_url(&body), "data:image/jpeg;base64,CCCC");
    }
}
