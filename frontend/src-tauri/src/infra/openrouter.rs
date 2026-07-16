//! Shared OpenRouter structured-response client.

use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChatMessage {
    role: String,
    content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenRouterResponse {
    pub data: Value,
    pub usage: TokenUsage,
    pub model: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OpenRouterError {
    #[error("LLM request cancelled")]
    Cancelled,
    #[error("LLM request timed out after {0}ms")]
    Timeout(u128),
    #[error("OpenRouter request failed with HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("OpenRouter request failed: {0}")]
    Network(String),
    #[error("OpenRouter response did not include choices: {0}")]
    MissingChoices(String),
    #[error("OpenRouter returned empty message content after retry ({0})")]
    EmptyContent(String),
    #[error("LLM returned non-JSON content: {0}")]
    NonJson(String),
    #[error("LLM returned malformed JSON{finish_reason}: {message}")]
    MalformedJson {
        finish_reason: String,
        message: String,
    },
}

#[derive(Clone)]
pub struct OpenRouterClient {
    http: Client,
    api_key: String,
    model: String,
    base_url: String,
    temperature: f64,
    max_tokens: u32,
    timeout: Duration,
    retry_delays: Vec<Duration>,
}

#[derive(Clone, Copy)]
struct CompletionRequest<'a> {
    messages: &'a [ChatMessage],
    schema_name: &'a str,
    schema: &'a Value,
    max_tokens: u32,
    disable_reasoning: bool,
}

impl OpenRouterClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::at(api_key, model, OPENROUTER_BASE)
    }

    pub fn at(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            http: Client::builder()
                .build()
                .expect("reqwest RustLS client configuration is valid"),
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            temperature: 0.7,
            max_tokens: 1024,
            timeout: DEFAULT_TIMEOUT,
            retry_delays: vec![Duration::from_millis(250), Duration::from_millis(500)],
        }
    }

    pub fn with_generation(mut self, temperature: f64, max_tokens: u32) -> Self {
        self.temperature = temperature;
        self.max_tokens = max_tokens;
        self
    }

    #[cfg(test)]
    fn with_retry_delays(mut self, delays: Vec<u64>) -> Self {
        self.retry_delays = delays.into_iter().map(Duration::from_millis).collect();
        self
    }

    pub async fn complete_json(
        &self,
        messages: Vec<ChatMessage>,
        schema_name: &str,
        schema: Value,
        cancelled: &AtomicBool,
    ) -> Result<OpenRouterResponse, OpenRouterError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(OpenRouterError::Cancelled);
        }
        let deadline = Instant::now() + self.timeout;
        let mut payload = None;
        let mut parse_error = None;

        for repair_attempt in 0..2 {
            let max_tokens = if repair_attempt > 0
                && finish_reason(payload.as_ref()).as_deref() == Some("length")
            {
                self.max_tokens.saturating_mul(2)
            } else {
                self.max_tokens
            };
            let response = self
                .post_with_retries(
                    CompletionRequest {
                        messages: &messages,
                        schema_name,
                        schema: &schema,
                        max_tokens,
                        disable_reasoning: repair_attempt > 0,
                    },
                    cancelled,
                    deadline,
                )
                .await?;
            let content = response
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let current = &response;
            if current
                .get("choices")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
            {
                let preview = current.to_string().chars().take(200).collect();
                return Err(OpenRouterError::MissingChoices(preview));
            }
            if content.is_empty() {
                if repair_attempt == 0 {
                    payload = Some(response);
                    continue;
                }
                return Err(OpenRouterError::EmptyContent(response_detail(
                    current,
                    &self.model,
                )));
            }

            let trimmed = content.trim();
            if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
                if let Some(extracted) = extract_json_object(trimmed) {
                    if let Ok(data) = serde_json::from_str(extracted) {
                        return Ok(build_response(current, data, &self.model));
                    }
                }
                return Err(OpenRouterError::NonJson(
                    trimmed.chars().take(120).collect(),
                ));
            }
            match serde_json::from_str(trimmed) {
                Ok(data) => return Ok(build_response(current, data, &self.model)),
                Err(error) => {
                    parse_error = Some(error.to_string());
                    payload = Some(response);
                }
            }
        }

        let finish_reason = finish_reason(payload.as_ref())
            .map(|reason| format!(" (finish_reason={reason})"))
            .unwrap_or_default();
        Err(OpenRouterError::MalformedJson {
            finish_reason,
            message: parse_error.unwrap_or_else(|| "unknown parse error".into()),
        })
    }

    async fn post_with_retries(
        &self,
        request: CompletionRequest<'_>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> Result<Value, OpenRouterError> {
        let attempts = self.retry_delays.len() + 1;
        let mut last_error = None;
        for attempt in 0..attempts {
            let mut retry_delay = self.retry_delays.get(attempt).copied().unwrap_or_default();
            if cancelled.load(Ordering::Acquire) {
                return Err(OpenRouterError::Cancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(OpenRouterError::Timeout(self.timeout.as_millis()));
            }
            match tokio::time::timeout(
                remaining,
                self.post(
                    request.messages,
                    request.schema_name,
                    request.schema,
                    request.max_tokens,
                    request.disable_reasoning,
                    cancelled,
                ),
            )
            .await
            {
                Err(_) => return Err(OpenRouterError::Timeout(self.timeout.as_millis())),
                Ok(Ok(response)) => {
                    let status = response.status();
                    let body =
                        read_response_text(response, cancelled, deadline, self.timeout.as_millis())
                            .await?;
                    if status.is_success() {
                        return serde_json::from_str(&body)
                            .map_err(|error| OpenRouterError::Network(error.to_string()));
                    }
                    let provider_error =
                        status == StatusCode::BAD_REQUEST && body.contains("provider_name");
                    let retryable = is_retryable(status) || provider_error;
                    let error = OpenRouterError::Http {
                        status: status.as_u16(),
                        body,
                    };
                    if !retryable || attempt + 1 == attempts {
                        return Err(error);
                    }
                    if provider_error {
                        retry_delay = retry_delay.saturating_mul(4);
                    }
                    last_error = Some(error);
                }
                Ok(Err(OpenRouterError::Cancelled)) => return Err(OpenRouterError::Cancelled),
                Ok(Err(error)) => {
                    if attempt + 1 == attempts {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            }
            if Instant::now()
                .checked_add(retry_delay)
                .is_none_or(|after_delay| after_delay >= deadline)
            {
                return Err(last_error.unwrap_or_else(|| {
                    OpenRouterError::Network("retry deadline exhausted".into())
                }));
            }
            cancellable_sleep(retry_delay, cancelled).await?;
        }
        Err(last_error.unwrap_or_else(|| OpenRouterError::Network("no response".into())))
    }

    async fn post(
        &self,
        messages: &[ChatMessage],
        schema_name: &str,
        schema: &Value,
        max_tokens: u32,
        disable_reasoning: bool,
        cancelled: &AtomicBool,
    ) -> Result<reqwest::Response, OpenRouterError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(OpenRouterError::Cancelled);
        }
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": self.temperature,
            "max_tokens": max_tokens,
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": schema_name, "schema": schema }
            }
        });
        if disable_reasoning {
            body["reasoning"] = json!({ "enabled": false });
        }
        let request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send();
        tokio::pin!(request);
        loop {
            tokio::select! {
                response = &mut request => {
                    return response.map_err(|error| OpenRouterError::Network(error.to_string()));
                }
                () = tokio::time::sleep(Duration::from_millis(10)) => {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(OpenRouterError::Cancelled);
                    }
                }
            }
        }
    }
}

async fn read_response_text(
    response: reqwest::Response,
    cancelled: &AtomicBool,
    deadline: Instant,
    timeout_millis: u128,
) -> Result<String, OpenRouterError> {
    let body = response.text();
    tokio::pin!(body);
    loop {
        tokio::select! {
            result = &mut body => {
                return result.map_err(|error| OpenRouterError::Network(error.to_string()));
            }
            () = tokio::time::sleep(Duration::from_millis(10)) => {
                if cancelled.load(Ordering::Acquire) {
                    return Err(OpenRouterError::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(OpenRouterError::Timeout(timeout_millis));
                }
            }
        }
    }
}

fn is_retryable(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

async fn cancellable_sleep(
    duration: Duration,
    cancelled: &AtomicBool,
) -> Result<(), OpenRouterError> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Acquire) {
            return Err(OpenRouterError::Cancelled);
        }
        tokio::time::sleep(Duration::from_millis(10).min(deadline - Instant::now())).await;
    }
    Ok(())
}

fn finish_reason(payload: Option<&Value>) -> Option<String> {
    payload?
        .pointer("/choices/0/finish_reason")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn response_detail(payload: &Value, fallback_model: &str) -> String {
    format!(
        "model={} finish_reason={} completion_tokens={} reasoning_tokens={}",
        payload
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(fallback_model),
        finish_reason(Some(payload)).unwrap_or_else(|| "?".into()),
        payload
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .map_or_else(|| "?".into(), |value| value.to_string()),
        payload
            .pointer("/usage/reasoning_tokens")
            .and_then(Value::as_u64)
            .map_or_else(|| "?".into(), |value| value.to_string()),
    )
}

fn extract_json_object(content: &str) -> Option<&str> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    (end > start).then_some(&content[start..=end])
}

fn build_response(payload: &Value, data: Value, fallback_model: &str) -> OpenRouterResponse {
    OpenRouterResponse {
        data,
        usage: TokenUsage {
            prompt_tokens: payload
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            completion_tokens: payload
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            total_tokens: payload
                .pointer("/usage/total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
        model: payload
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(fallback_model)
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[tokio::test]
    async fn sends_schema_request_and_parses_json_message_content() {
        let server = TestServer::start(vec![TestResponse::json(json!({
            "model": "test/model",
            "usage": { "prompt_tokens": 4, "completion_tokens": 3, "total_tokens": 7 },
            "choices": [{
                "finish_reason": "stop",
                "message": { "content": "{\"tracks\":[]}" }
            }]
        }))]);
        let client = OpenRouterClient::at("secret", "test/model", server.base_url());

        let response = client
            .complete_json(
                vec![ChatMessage::user("audit")],
                "AuditResponse",
                json!({ "type": "object" }),
                &AtomicBool::new(false),
            )
            .await
            .unwrap();

        assert_eq!(response.data, json!({ "tracks": [] }));
        assert_eq!(response.usage.total_tokens, 7);
        let request = server.request(0);
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer secret")
        );
        assert_eq!(request.body["model"], "test/model");
        assert_eq!(
            request.body["response_format"]["json_schema"]["name"],
            "AuditResponse"
        );
    }

    #[tokio::test]
    async fn retries_retryable_status_and_fails_loud_on_non_json_content() {
        let server = TestServer::start(vec![
            TestResponse::status(429, "busy"),
            TestResponse::json(json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": "not json" } }]
            })),
        ]);
        let client = OpenRouterClient::at("secret", "test/model", server.base_url())
            .with_retry_delays(vec![0, 0]);

        let error = client
            .complete_json(
                vec![ChatMessage::user("audit")],
                "AuditResponse",
                json!({ "type": "object" }),
                &AtomicBool::new(false),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("non-JSON content"));
        assert_eq!(server.request_count(), 2);
    }

    #[tokio::test]
    async fn cancellation_before_request_makes_no_network_call() {
        let server = TestServer::start(vec![]);
        let client = OpenRouterClient::at("secret", "test/model", server.base_url());
        let cancelled = AtomicBool::new(true);

        let error = client
            .complete_json(
                vec![ChatMessage::user("audit")],
                "AuditResponse",
                json!({ "type": "object" }),
                &cancelled,
            )
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "LLM request cancelled");
        assert_eq!(server.request_count(), 0);
    }

    #[tokio::test]
    async fn malformed_truncated_json_repairs_once_with_reasoning_disabled() {
        let server = TestServer::start(vec![
            TestResponse::json(json!({
                "choices": [{ "finish_reason": "length", "message": { "content": "{\"tracks\":" } }]
            })),
            TestResponse::json(json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": "{\"tracks\":[]}" } }]
            })),
        ]);
        let client = OpenRouterClient::at("secret", "test/model", server.base_url())
            .with_retry_delays(vec![0, 0]);

        let response = client
            .complete_json(
                vec![ChatMessage::user("audit")],
                "AuditResponse",
                json!({ "type": "object" }),
                &AtomicBool::new(false),
            )
            .await
            .unwrap();

        assert_eq!(response.data, json!({ "tracks": [] }));
        assert_eq!(server.request_count(), 2);
        let repair = server.request(1);
        assert_eq!(repair.body["max_tokens"], 2048);
        assert_eq!(repair.body["reasoning"], json!({ "enabled": false }));
    }

    #[derive(Clone)]
    struct CapturedRequest {
        headers: HashMap<String, String>,
        body: Value,
    }

    struct TestResponse {
        status: u16,
        body: String,
    }

    impl TestResponse {
        fn json(body: Value) -> Self {
            Self {
                status: 200,
                body: body.to_string(),
            }
        }

        fn status(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.to_string(),
            }
        }
    }

    struct TestServer {
        address: String,
        requests: Arc<Mutex<Vec<CapturedRequest>>>,
        stopped: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn start(responses: Vec<TestResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stopped = Arc::new(AtomicBool::new(false));
            let server_requests = Arc::clone(&requests);
            let server_stopped = Arc::clone(&stopped);
            let mut responses = VecDeque::from(responses);
            let thread = thread::spawn(move || {
                while !server_stopped.load(Ordering::Acquire) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    };
                    if server_stopped.load(Ordering::Acquire) {
                        break;
                    }
                    stream.set_nonblocking(false).unwrap();
                    let mut bytes = Vec::new();
                    let mut buffer = [0_u8; 4096];
                    loop {
                        let count = stream.read(&mut buffer).unwrap_or(0);
                        if count == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&buffer[..count]);
                        if request_is_complete(&bytes) {
                            break;
                        }
                    }
                    server_requests.lock().unwrap().push(parse_request(&bytes));
                    let response = responses
                        .pop_front()
                        .unwrap_or_else(|| TestResponse::status(500, "no response"));
                    let reason = if response.status == 200 {
                        "OK"
                    } else {
                        "Error"
                    };
                    let reply = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.status,
                        reason,
                        response.body.len(),
                        response.body
                    );
                    stream.write_all(reply.as_bytes()).unwrap();
                }
            });
            Self {
                address,
                requests,
                stopped,
                thread: Some(thread),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.address)
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }

        fn request(&self, index: usize) -> CapturedRequest {
            self.requests.lock().unwrap()[index].clone()
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            let _ = std::net::TcpStream::connect(&self.address);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn request_is_complete(bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(bytes);
        let Some(header_end) = text.find("\r\n\r\n") else {
            return false;
        };
        let content_length = text[..header_end]
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        bytes.len() >= header_end + 4 + content_length
    }

    fn parse_request(bytes: &[u8]) -> CapturedRequest {
        let text = String::from_utf8_lossy(bytes);
        let (headers, body) = text.split_once("\r\n\r\n").unwrap();
        let headers = headers
            .lines()
            .skip(1)
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.to_ascii_lowercase(), value.trim().to_string()))
            })
            .collect();
        CapturedRequest {
            headers,
            body: serde_json::from_str(body).unwrap(),
        }
    }
}
