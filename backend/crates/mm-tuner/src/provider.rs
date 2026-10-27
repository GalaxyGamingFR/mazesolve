//! Chat providers: the real Meta Model API client and a scripted mock.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rand::Rng;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::TunerError;

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub system: String,
    pub user: String,
    /// `(schema name, strict JSON schema)`.
    pub json_schema: Option<(String, Value)>,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    pub content: String,
    pub usage: Usage,
    pub model: String,
    pub latency_ms: u64,
}

#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, TunerError>;
}

// ---------------------------------------------------------------- Muse

#[derive(Clone)]
pub struct MuseConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: String,
    pub timeout_s: u64,
    pub max_tries: u32,
    pub base_backoff_ms: u64,
}

impl MuseConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        MuseConfig {
            api_key: api_key.into(),
            base_url: "https://api.meta.ai/v1".into(),
            model: "muse-spark-1.3".into(),
            reasoning_effort: "medium".into(),
            timeout_s: 120,
            max_tries: 4,
            base_backoff_ms: 500,
        }
    }

    /// `None` when `META_MODEL_API_KEY` is unset or blank.
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("META_MODEL_API_KEY").ok()?;
        if key.trim().is_empty() {
            return None;
        }
        let mut c = MuseConfig::new(key.trim());
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        if let Some(v) = env("MUSE_BASE_URL") {
            c.base_url = v.trim().trim_end_matches('/').to_string();
        }
        if let Some(v) = env("MUSE_MODEL") {
            c.model = v.trim().to_string();
        }
        if let Some(v) = env("MUSE_REASONING_EFFORT") {
            c.reasoning_effort = v.trim().to_string();
        }
        if let Some(v) = env("MUSE_TIMEOUT_S").and_then(|v| v.trim().parse().ok()) {
            c.timeout_s = v;
        }
        Some(c)
    }
}

impl fmt::Debug for MuseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MuseConfig")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("timeout_s", &self.timeout_s)
            .field("max_tries", &self.max_tries)
            .field("base_backoff_ms", &self.base_backoff_ms)
            .finish()
    }
}

pub struct MuseProvider {
    cfg: MuseConfig,
    client: reqwest::Client,
}

impl fmt::Debug for MuseProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MuseProvider")
            .field("cfg", &self.cfg)
            .finish()
    }
}

struct Failure {
    err: TunerError,
    retryable: bool,
    retry_after: Option<Duration>,
}

/// Delay before retry number `attempt` (0-based). A server `retry-after` wins when it
/// is longer than the computed backoff; both are capped at 60 s. `jitter` in `[0,1]`
/// scales the exponential part into `[0.5, 1.0]` of its nominal value.
pub fn backoff_delay(
    attempt: u32,
    base_ms: u64,
    retry_after: Option<Duration>,
    jitter: f64,
) -> Duration {
    let exp = base_ms.saturating_mul(1u64 << attempt.min(10));
    let jittered = (exp as f64 * (0.5 + 0.5 * jitter.clamp(0.0, 1.0))) as u64;
    let d = Duration::from_millis(jittered);
    let d = retry_after.map_or(d, |ra| ra.max(d));
    d.min(Duration::from_secs(60))
}

fn parse_retry_after(v: &str) -> Option<Duration> {
    v.trim()
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64)
}

fn retryable_status(code: u16) -> bool {
    code == 429 || (500..600).contains(&code)
}

fn fatal(err: TunerError) -> Failure {
    Failure {
        err,
        retryable: false,
        retry_after: None,
    }
}

fn map_reqwest(e: reqwest::Error) -> Failure {
    let err = if e.is_timeout() {
        TunerError::Timeout
    } else {
        TunerError::Upstream {
            status: 0,
            message: e.without_url().to_string(),
        }
    };
    Failure {
        err,
        retryable: true,
        retry_after: None,
    }
}

fn jitter() -> f64 {
    rand::thread_rng().gen::<f64>()
}

impl MuseProvider {
    pub fn new(cfg: MuseConfig) -> Result<Self, TunerError> {
        if cfg.api_key.trim().is_empty() {
            return Err(TunerError::NotConfigured);
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_s.max(1)))
            .build()
            .map_err(|e| TunerError::Upstream {
                status: 0,
                message: format!("http client: {}", e.without_url()),
            })?;
        Ok(MuseProvider { cfg, client })
    }

    pub fn config(&self) -> &MuseConfig {
        &self.cfg
    }

    fn body(&self, req: &ChatRequest) -> Value {
        let mut body = json!({
            "model": self.cfg.model,
            "messages": [
                {"role": "system", "content": req.system},
                {"role": "user", "content": req.user},
            ],
            "max_completion_tokens": req.max_tokens,
            "temperature": req.temperature,
            "reasoning_effort": self.cfg.reasoning_effort,
        });
        if let Some((name, schema)) = &req.json_schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": name, "schema": schema, "strict": true},
            });
        }
        body
    }

    async fn attempt(&self, body: &Value) -> Result<ChatResponse, Failure> {
        let url = format!(
            "{}/chat/completions",
            self.cfg.base_url.trim_end_matches('/')
        );
        let started = Instant::now();
        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.cfg.api_key)
            .json(body)
            .send()
            .await
            .map_err(map_reqwest)?;
        let status = resp.status();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after);
        let text = resp.text().await.map_err(map_reqwest)?;
        if !status.is_success() {
            return Err(Failure {
                err: TunerError::Upstream {
                    status: status.as_u16(),
                    message: text.chars().take(300).collect(),
                },
                retryable: retryable_status(status.as_u16()),
                retry_after,
            });
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|e| fatal(TunerError::Parse(e.to_string())))?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                fatal(TunerError::Parse(
                    "response has no choices[0].message.content".into(),
                ))
            })?
            .to_string();
        Ok(ChatResponse {
            content,
            usage: Usage {
                prompt_tokens: v["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                completion_tokens: v["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            },
            model: v["model"].as_str().unwrap_or(&self.cfg.model).to_string(),
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[async_trait]
impl ChatProvider for MuseProvider {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, TunerError> {
        let body = self.body(&req);
        let started = Instant::now();
        let tries = self.cfg.max_tries.max(1);
        for attempt in 0..tries {
            match self.attempt(&body).await {
                Ok(mut r) => {
                    r.latency_ms = started.elapsed().as_millis() as u64;
                    return Ok(r);
                }
                Err(f) if f.retryable && attempt + 1 < tries => {
                    let d =
                        backoff_delay(attempt, self.cfg.base_backoff_ms, f.retry_after, jitter());
                    tokio::time::sleep(d).await;
                }
                Err(f) => return Err(f.err),
            }
        }
        Err(TunerError::Timeout)
    }
}

// ---------------------------------------------------------------- Mock

type MockFn = Box<dyn Fn(&ChatRequest) -> Result<String, TunerError> + Send + Sync>;

/// Scripted provider for tests: a queue of replies, or a closure. Records requests.
pub struct MockProvider {
    queue: Mutex<VecDeque<String>>,
    handler: Option<MockFn>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl MockProvider {
    pub fn scripted<I, S>(replies: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        MockProvider {
            queue: Mutex::new(replies.into_iter().map(Into::into).collect()),
            handler: None,
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn from_fn(
        f: impl Fn(&ChatRequest) -> Result<String, TunerError> + Send + Sync + 'static,
    ) -> Self {
        MockProvider {
            queue: Mutex::new(VecDeque::new()),
            handler: Some(Box::new(f)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn call_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatProvider for MockProvider {
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, TunerError> {
        let content = match &self.handler {
            Some(h) => h(&req)?,
            None => self
                .queue
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| TunerError::Upstream {
                    status: 0,
                    message: "mock provider exhausted".into(),
                })?,
        };
        let usage = Usage {
            prompt_tokens: ((req.system.len() + req.user.len()) / 4) as u64,
            completion_tokens: (content.len() / 4) as u64,
        };
        self.requests.lock().unwrap().push(req);
        Ok(ChatResponse {
            content,
            usage,
            model: "mock".into(),
            latency_ms: 1,
        })
    }
}

// ---------------------------------------------------------------- helpers

/// `Some` when `META_MODEL_API_KEY` is set.
pub fn provider_from_env() -> Option<Arc<dyn ChatProvider>> {
    let p = MuseProvider::new(MuseConfig::from_env()?).ok()?;
    Some(Arc::new(p))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConnectionInfo {
    pub model: String,
    pub latency_ms: u64,
}

/// Cheapest possible round trip, for a "test connection" button.
pub async fn test_connection(p: &dyn ChatProvider) -> Result<ConnectionInfo, TunerError> {
    let r = p
        .complete(ChatRequest {
            system: "You are a connectivity probe.".into(),
            user: "Reply with the single word OK.".into(),
            json_schema: None,
            max_tokens: 64,
            temperature: 0.0,
        })
        .await?;
    Ok(ConnectionInfo {
        model: r.model,
        latency_ms: r.latency_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn debug_never_shows_key() {
        let cfg = MuseConfig::new("sk-SECRET-123");
        let s = format!("{cfg:?}");
        assert!(!s.contains("SECRET"));
        let p = MuseProvider::new(cfg).unwrap();
        let s = format!("{p:?}");
        assert!(!s.contains("SECRET"));
        assert!(s.contains("redacted"));
    }

    #[test]
    fn blank_key_is_not_configured() {
        assert!(matches!(
            MuseProvider::new(MuseConfig::new("  ")),
            Err(TunerError::NotConfigured)
        ));
    }

    #[test]
    fn backoff_is_exponential_jittered_and_honours_retry_after() {
        let lo = backoff_delay(2, 500, None, 0.0);
        let hi = backoff_delay(2, 500, None, 1.0);
        assert_eq!(lo, Duration::from_millis(1000));
        assert_eq!(hi, Duration::from_millis(2000));
        assert!(backoff_delay(3, 500, None, 0.5) > backoff_delay(1, 500, None, 0.5));
        let ra = Some(Duration::from_secs(7));
        assert_eq!(backoff_delay(0, 500, ra, 0.5), Duration::from_secs(7));
        assert_eq!(
            backoff_delay(0, 500, Some(Duration::from_secs(999)), 0.5),
            Duration::from_secs(60)
        );
    }

    /// Serves canned HTTP responses in order; returns the base URL and hit counter.
    async fn mock_server(
        responses: Vec<(u16, &'static str, String)>,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        tokio::spawn(async move {
            for (status, extra, body) in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let cl = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + cl {
                            break;
                        }
                    }
                }
                h.fetch_add(1, Ordering::SeqCst);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}/v1"), hits)
    }

    fn ok_body() -> String {
        json!({
            "model": "muse-spark-1.3",
            "choices": [{"message": {"role": "assistant", "content": "OK"}}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 1}
        })
        .to_string()
    }

    fn test_cfg(url: String) -> MuseConfig {
        let mut c = MuseConfig::new("sk-test");
        c.base_url = url;
        c.base_backoff_ms = 5;
        c
    }

    #[tokio::test]
    async fn retries_429_then_succeeds() {
        let (url, hits) = mock_server(vec![
            (429, "retry-after: 0\r\n", "{\"error\":\"slow\"}".into()),
            (200, "", ok_body()),
        ])
        .await;
        let p = MuseProvider::new(test_cfg(url)).unwrap();
        let info = test_connection(&p).await.unwrap();
        assert_eq!(info.model, "muse-spark-1.3");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn non_retryable_status_fails_fast_and_exhausted_retries_report_upstream() {
        let (url, hits) = mock_server(vec![(401, "", "{\"error\":\"bad key\"}".into())]).await;
        let p = MuseProvider::new(test_cfg(url)).unwrap();
        let e = test_connection(&p).await.unwrap_err();
        assert!(matches!(e, TunerError::Upstream { status: 401, .. }));
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        let (url, hits) = mock_server(vec![(503, "", "x".into()); 4]).await;
        let p = MuseProvider::new(test_cfg(url)).unwrap();
        let e = test_connection(&p).await.unwrap_err();
        assert!(matches!(e, TunerError::Upstream { status: 503, .. }));
        assert_eq!(hits.load(Ordering::SeqCst), 4);
    }
}
