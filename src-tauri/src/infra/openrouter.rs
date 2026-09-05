//! Shared OpenRouter structured-response client.

use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tracing;

const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
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
    pub diagnostics: CompletionDiagnostics,
}

/// Allowlisted telemetry only: never retain response text or arbitrary metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CompletionDiagnostics {
    pub total_elapsed_ms: u128,
    pub attempts: Vec<AttemptDiagnostics>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AttemptDiagnostics {
    pub schema_repair: usize,
    pub max_tokens: u32,
    pub status: Option<u16>,
    pub headers_elapsed_ms: Option<u128>,
    pub body_read_ms: Option<u128>,
    pub total_elapsed_ms: u128,
    pub generation_id: Option<String>,
    pub resolved_model: Option<String>,
    pub selected_provider: Option<String>,
    pub provider_attempts: Option<u64>,
    pub provider_history: Vec<ProviderAttemptDiagnostics>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cache_status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderAttemptDiagnostics {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub status: Option<u64>,
}

// Identifiers and enum-like telemetry have bounded, single-line values.
fn diagnostic_label(value: Option<&str>) -> Option<String> {
    value
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 160
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_/.: ~".contains(c))
        })
        .map(str::to_owned)
}

impl AttemptDiagnostics {
    fn capture(&mut self, payload: &Value) {
        self.generation_id = diagnostic_label(payload.get("id").and_then(Value::as_str))
            .or(self.generation_id.take());
        self.resolved_model = diagnostic_label(payload.get("model").and_then(Value::as_str));
        self.finish_reason = diagnostic_label(finish_reason(Some(payload)).as_deref());
        self.prompt_tokens = payload
            .pointer("/usage/prompt_tokens")
            .or_else(|| payload.pointer("/usage/input_tokens"))
            .and_then(Value::as_u64);
        self.completion_tokens = payload
            .pointer("/usage/completion_tokens")
            .or_else(|| payload.pointer("/usage/output_tokens"))
            .and_then(Value::as_u64);
        self.reasoning_tokens = payload
            .pointer("/usage/completion_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64)
            .or_else(|| {
                payload
                    .pointer("/usage/reasoning_tokens")
                    .and_then(Value::as_u64)
            });
        self.provider_attempts = payload
            .pointer("/openrouter_metadata/attempt")
            .and_then(Value::as_u64);
        self.provider_history = payload
            .pointer("/openrouter_metadata/attempts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|v| v.is_object())
            .take(64)
            .map(|v| ProviderAttemptDiagnostics {
                provider: diagnostic_label(v.get("provider").and_then(Value::as_str)),
                model: diagnostic_label(v.get("model").and_then(Value::as_str)),
                status: v.get("status").and_then(Value::as_u64),
            })
            .collect();
        self.selected_provider = payload
            .pointer("/openrouter_metadata/endpoints/available")
            .and_then(Value::as_array)
            .and_then(|endpoints| {
                endpoints.iter().find(|endpoint| {
                    endpoint.get("selected").and_then(Value::as_bool) == Some(true)
                })
            })
            .and_then(|endpoint| {
                diagnostic_label(endpoint.get("provider").and_then(Value::as_str))
            });
    }
}

impl OpenRouterError {
    pub fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Timeout(_) => "timeout",
            Self::Http { .. } => "http_error",
            Self::Network(_) => "network_error",
            Self::MissingChoices(_) => "missing_choices",
            Self::EmptyContent(_) => "empty_content",
            Self::NonJson(_) => "non_json",
            Self::MalformedJson { .. } => "malformed_json",
        }
    }
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
    provider: ProviderKind,
    #[cfg(test)]
    pub(crate) test_policy: Option<TagCorrectionPolicy>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TagCorrectionPolicy {
    pub disable_reasoning: bool,
    pub performance_routing: bool,
    pub router_metadata: bool,
}

#[derive(Clone, Copy)]
struct CompletionRequest<'a> {
    messages: &'a [ChatMessage],
    schema_name: &'a str,
    schema: &'a Value,
    max_tokens: u32,
    disable_reasoning: bool,
    schema_repair: usize,
}

/// Which API format this client uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// OpenAI-compatible /chat/completions with Bearer auth.
    OpenAi,
    /// Anthropic /v1/messages with x-api-key auth and tool-use wrapping.
    Anthropic,
}

impl ProviderKind {
    pub fn from_provider_str(s: &str) -> Self {
        match s {
            "claude" | "anthropic" => ProviderKind::Anthropic,
            _ => ProviderKind::OpenAi,
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderKind::OpenAi => write!(f, "openai"),
            ProviderKind::Anthropic => write!(f, "claude"),
        }
    }
}

/// Resolve the base URL for a provider.
/// When `base_url` is set (non-empty) it takes priority, otherwise the
/// provider default is returned.  Returns `None` for unresolvable providers
/// so the caller can error explicitly.
pub fn resolve_base_url(provider: &ProviderKind, base_url: &str) -> &'static str {
    if !base_url.is_empty() {
        return ""; // caller should use base_url directly — see LlmEndpoint
    }
    match provider {
        ProviderKind::OpenAi => "https://openrouter.ai/api/v1",
        ProviderKind::Anthropic => "https://api.anthropic.com/v1",
    }
}

/// Default base URLs per provider.
/// Used when no explicit base_url is configured.
pub fn base_url_for_provider(provider: &str) -> &'static str {
    match provider {
        "openai" => "https://api.openai.com/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "claude" | "anthropic" => "https://api.anthropic.com/v1",
        "opencode_go" => "http://localhost:8080/v1",
        "opencode_zen" => "http://localhost:7070/v1",
        _ => OPENROUTER_BASE,
    }
}

/// Combined provider+base-url result with a displayable fallback for errors.
#[derive(Debug, Clone)]
pub struct LlmEndpoint {
    pub provider: ProviderKind,
    pub base_url: String,
}

impl LlmEndpoint {
    /// Resolve from config fields (which may be empty).
    /// Backward compat: when provider is absent/unknown -> OpenAi.
    /// When base_url is set it takes priority over the provider default.
    pub fn from_config(provider: Option<&str>, base_url: Option<&str>) -> Self {
        let provider = provider.filter(|&p| !p.is_empty());
        let base_url = base_url.filter(|&u| !u.is_empty());
        match (provider, base_url) {
            (Some(p), Some(u)) => Self {
                provider: ProviderKind::from_provider_str(p),
                base_url: u.to_string(),
            },
            (Some(p), None) => Self {
                provider: ProviderKind::from_provider_str(p),
                base_url: base_url_for_provider(p).to_string(),
            },
            (None, Some(u)) => Self {
                provider: ProviderKind::OpenAi,
                base_url: u.to_string(),
            },
            (None, None) => Self {
                provider: ProviderKind::OpenAi,
                base_url: OPENROUTER_BASE.to_string(),
            },
        }
    }

    pub fn openai(base_url: impl Into<String>) -> Self {
        Self {
            provider: ProviderKind::OpenAi,
            base_url: base_url.into(),
        }
    }
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
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let provider = if base_url.contains("api.anthropic.com") {
            ProviderKind::Anthropic
        } else {
            ProviderKind::OpenAi
        };
        Self {
            http: Client::builder()
                .build()
                .expect("reqwest RustLS client configuration is valid"),
            api_key: api_key.into(),
            model: model.into(),
            base_url,
            temperature: 0.7,
            max_tokens: 1024,
            timeout: DEFAULT_TIMEOUT,
            retry_delays: vec![Duration::from_millis(250), Duration::from_millis(500)],
            provider,
            #[cfg(test)]
            test_policy: None,
        }
    }

    fn tag_policy(&self, schema_name: &str) -> TagCorrectionPolicy {
        #[cfg(test)]
        if let Some(policy) = self.test_policy {
            return policy;
        }
        let canonical = reqwest::Url::parse(&self.base_url).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("openrouter.ai")
                && url.port_or_known_default() == Some(443)
                && url.path() == "/api/v1"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
        let targeted = canonical
            && self.provider == ProviderKind::OpenAi
            && self.model == "deepseek/deepseek-v4-flash-0731"
            && schema_name == "TagCorrectionResponse";
        TagCorrectionPolicy {
            disable_reasoning: targeted,
            performance_routing: targeted,
            router_metadata: targeted,
        }
    }

    pub fn with_generation(mut self, temperature: f64, max_tokens: u32) -> Self {
        self.temperature = temperature;
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_provider(mut self, provider: ProviderKind) -> Self {
        self.provider = provider;
        self
    }

    /// Send a minimal prompt to verify the provider, model, and credentials
    /// work.  Returns the responding model name on success.
    pub async fn test_connection(&self) -> Result<String, OpenRouterError> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "response": { "type": "string", "const": "ok" } },
            "required": ["response"],
            "additionalProperties": false
        });
        let messages = vec![
            ChatMessage::system(
                "Return a JSON object with exactly one field 'response' set to the string 'ok'.",
            ),
            ChatMessage::user("Say ok"),
        ];
        let response = self
            .complete_json(messages, "TestConnection", schema, &AtomicBool::new(false))
            .await?;
        Ok(response.model)
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
        self.complete_json_observed(
            messages,
            schema_name,
            schema,
            cancelled,
            &mut CompletionDiagnostics::default(),
        )
        .await
    }

    pub(crate) async fn complete_json_observed(
        &self,
        messages: Vec<ChatMessage>,
        schema_name: &str,
        schema: Value,
        cancelled: &AtomicBool,
        diagnostics: &mut CompletionDiagnostics,
    ) -> Result<OpenRouterResponse, OpenRouterError> {
        *diagnostics = CompletionDiagnostics::default();
        let started = Instant::now();
        let mut result = self
            .complete_json_inner(messages, schema_name, schema, cancelled, diagnostics)
            .await;
        diagnostics.total_elapsed_ms = started.elapsed().as_millis();
        tracing::debug!(requested_model = ?diagnostic_label(Some(&self.model)), schema = schema_name,
            total_elapsed_ms = diagnostics.total_elapsed_ms, http_attempts = diagnostics.attempts.len(),
            schema_repairs = diagnostics.attempts.last().map_or(0, |a| a.schema_repair),
            outcome = result.as_ref().err().map_or("success", OpenRouterError::diagnostic_code),
            "OpenRouter completion finished");
        if let Ok(response) = &mut result {
            response.diagnostics = diagnostics.clone();
        }
        result
    }

    async fn complete_json_inner(
        &self,
        mut messages: Vec<ChatMessage>,
        schema_name: &str,
        schema: Value,
        cancelled: &AtomicBool,
        diagnostics: &mut CompletionDiagnostics,
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
                        schema_repair: repair_attempt,
                    },
                    cancelled,
                    deadline,
                    diagnostics,
                )
                .await?;
            let content = extract_content(self.provider, &response);
            let current = &response;

            // Validate response structure based on provider format.
            let valid = match self.provider {
                ProviderKind::OpenAi => current
                    .get("choices")
                    .and_then(Value::as_array)
                    .is_some_and(|arr| !arr.is_empty()),
                ProviderKind::Anthropic => {
                    let has_tool_use = current.pointer("/content/0/type").and_then(Value::as_str)
                        == Some("tool_use");
                    let stop_reason = current
                        .get("stop_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    // Accept end_turn or stop_sequence as valid completion.
                    has_tool_use && !stop_reason.is_empty()
                }
            };
            if !valid {
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
                if repair_attempt == 0 {
                    messages.push(ChatMessage {
                        role: "assistant".into(),
                        content: trimmed.to_string(),
                    });
                    messages.push(ChatMessage::system(format!(
                        "Your previous response did not match the required {schema_name} JSON schema. Return only one JSON value that matches that schema. Put any clarification or question inside the schema instead of replying with plain text."
                    )));
                    payload = Some(response);
                    continue;
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
        diagnostics: &mut CompletionDiagnostics,
    ) -> Result<Value, OpenRouterError> {
        let attempts = self.retry_delays.len() + 1;
        tracing::debug!(
            model = ?diagnostic_label(Some(&self.model)),
            timeout_ms = self.timeout.as_millis(),
            max_tokens = request.max_tokens,
            reasoning_disabled = request.disable_reasoning || self.tag_policy(request.schema_name).disable_reasoning,
            performance_routing = self.tag_policy(request.schema_name).performance_routing,
            schema = %request.schema_name,
            attempts,
            "OpenRouter request started"
        );
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
            tracing::debug!(
                attempt,
                remaining_ms = remaining.as_millis(),
                "OpenRouter attempt"
            );
            match self
                .measured_attempt(request, cancelled, deadline, diagnostics)
                .await
            {
                Ok(response) => return Ok(response),
                Err(OpenRouterError::Timeout(ms)) => return Err(OpenRouterError::Timeout(ms)),
                Err(OpenRouterError::Http { status, body }) => {
                    let status =
                        StatusCode::from_u16(status).expect("HTTP status came from reqwest");
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
                Err(OpenRouterError::Cancelled) => return Err(OpenRouterError::Cancelled),
                Err(error) => {
                    // Once headers arrived, body/JSON failures were terminal before
                    // instrumentation and must not gain additional HTTP retries.
                    if diagnostics
                        .attempts
                        .last()
                        .is_some_and(|a| a.headers_elapsed_ms.is_some())
                    {
                        return Err(error);
                    }
                    if attempt + 1 == attempts {
                        return Err(error);
                    }
                    tracing::warn!(attempt, "OpenRouter retryable HTTP error — retrying");
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

    async fn measured_attempt(
        &self,
        request: CompletionRequest<'_>,
        cancelled: &AtomicBool,
        deadline: Instant,
        diagnostics: &mut CompletionDiagnostics,
    ) -> Result<Value, OpenRouterError> {
        let started = Instant::now();
        let mut attempt = AttemptDiagnostics {
            schema_repair: request.schema_repair,
            max_tokens: request.max_tokens,
            ..Default::default()
        };
        let result = async {
            let response = tokio::time::timeout(
                deadline.saturating_duration_since(Instant::now()),
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
            .map_err(|_| OpenRouterError::Timeout(self.timeout.as_millis()))??;
            attempt.headers_elapsed_ms = Some(started.elapsed().as_millis());
            let status = response.status();
            attempt.status = Some(status.as_u16());
            attempt.generation_id = diagnostic_label(
                response
                    .headers()
                    .get("x-generation-id")
                    .and_then(|v| v.to_str().ok()),
            );
            attempt.cache_status = diagnostic_label(
                response
                    .headers()
                    .get("x-openrouter-cache-status")
                    .and_then(|v| v.to_str().ok()),
            );
            let body_started = Instant::now();
            let body =
                read_response_text(response, cancelled, deadline, self.timeout.as_millis()).await;
            attempt.body_read_ms = Some(body_started.elapsed().as_millis());
            let body = body?;
            let parsed = serde_json::from_str::<Value>(&body);
            if let Ok(payload) = &parsed {
                attempt.capture(payload);
            }
            if !status.is_success() {
                return Err(OpenRouterError::Http {
                    status: status.as_u16(),
                    body,
                });
            }
            parsed.map_err(|error| OpenRouterError::Network(error.to_string()))
        }
        .await;
        attempt.total_elapsed_ms = started.elapsed().as_millis();
        tracing::debug!(http_attempt = diagnostics.attempts.len() + 1,
            outcome = result.as_ref().err().map_or("success", OpenRouterError::diagnostic_code),
            diagnostics = ?attempt, "OpenRouter attempt finished");
        diagnostics.attempts.push(attempt);
        result
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
        match self.provider {
            ProviderKind::Anthropic => {
                self.post_anthropic(messages, schema_name, schema, max_tokens, cancelled)
                    .await
            }
            ProviderKind::OpenAi => {
                self.post_openai(
                    messages,
                    schema_name,
                    schema,
                    max_tokens,
                    disable_reasoning,
                    cancelled,
                )
                .await
            }
        }
    }

    async fn post_openai(
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
        let policy = self.tag_policy(schema_name);
        if disable_reasoning || policy.disable_reasoning {
            body["reasoning"] = json!({ "enabled": false });
        }
        if policy.performance_routing {
            body["provider"] = json!({"sort": {"by": "price"}, "preferred_min_throughput": {"p90": 50}, "allow_fallbacks": true});
        }
        let mut request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body);
        if policy.router_metadata {
            request = request.header("X-OpenRouter-Metadata", "enabled");
        }
        let request = request.send();
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

    async fn post_anthropic(
        &self,
        messages: &[ChatMessage],
        schema_name: &str,
        schema: &Value,
        max_tokens: u32,
        cancelled: &AtomicBool,
    ) -> Result<reqwest::Response, OpenRouterError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(OpenRouterError::Cancelled);
        }
        // Wrap system message separately (Anthropic API puts it at top level).
        let system_content: String = messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let anthropic_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect();
        let body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "temperature": self.temperature,
            "system": if system_content.is_empty() { Value::Null } else { Value::String(system_content) },
            "messages": anthropic_messages,
            "tool_choice": {"type": "tool", "name": schema_name},
            "tools": [{
                "name": schema_name,
                "description": format!("Structured response for {}", schema_name),
                "input_schema": schema,
            }],
        });
        let request = self
            .http
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
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

/// Extract the raw JSON string content from a provider response.
fn extract_content(provider: ProviderKind, response: &Value) -> String {
    match provider {
        ProviderKind::Anthropic => response
            .pointer("/content/0/input")
            .map(|v| v.to_string())
            .unwrap_or_default(),
        ProviderKind::OpenAi => response
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn finish_reason(payload: Option<&Value>) -> Option<String> {
    payload?
        .pointer("/choices/0/finish_reason")
        .or_else(|| payload?.pointer("/stop_reason"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn response_detail(payload: &Value, fallback_model: &str) -> String {
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(fallback_model);
    let finish = finish_reason(Some(payload)).unwrap_or_else(|| "?".into());
    // Anthropic uses input_tokens/output_tokens; OpenAI uses prompt_tokens/completion_tokens.
    let completion = payload
        .pointer("/usage/completion_tokens")
        .or_else(|| payload.pointer("/usage/output_tokens"))
        .and_then(Value::as_u64)
        .map_or_else(|| "?".into(), |v| v.to_string());
    let reasoning = payload
        .pointer("/usage/reasoning_tokens")
        .and_then(Value::as_u64)
        .map_or_else(|| "?".into(), |v| v.to_string());
    format!("model={model} finish_reason={finish} completion_tokens={completion} reasoning_tokens={reasoning}")
}

fn extract_json_object(content: &str) -> Option<&str> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    (end > start).then_some(&content[start..=end])
}

fn build_response(payload: &Value, data: Value, fallback_model: &str) -> OpenRouterResponse {
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(fallback_model)
        .to_string();
    // Anthropic uses input_tokens/output_tokens; OpenAI uses prompt_tokens/completion_tokens.
    let prompt_tokens = payload
        .pointer("/usage/prompt_tokens")
        .or_else(|| payload.pointer("/usage/input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = payload
        .pointer("/usage/completion_tokens")
        .or_else(|| payload.pointer("/usage/output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = payload
        .pointer("/usage/total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(prompt_tokens + completion_tokens);
    OpenRouterResponse {
        data,
        usage: TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        },
        model,
        diagnostics: CompletionDiagnostics::default(),
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

    #[test]
    fn tag_policy_requires_exact_endpoint_model_and_schema() {
        let target = OpenRouterClient::new("secret", "deepseek/deepseek-v4-flash-0731");
        assert!(
            target
                .tag_policy("TagCorrectionResponse")
                .performance_routing
        );
        for url in [
            "https://openrouter.ai.evil.test/api/v1",
            "https://evil.test/openrouter.ai/api/v1",
            "http://openrouter.ai/api/v1",
            "https://openrouter.ai/api/v2",
            "https://user@openrouter.ai/api/v1",
            "https://openrouter.ai/api/v1?key=secret",
            "https://openrouter.ai:444/api/v1",
        ] {
            let client = OpenRouterClient::at("secret", "deepseek/deepseek-v4-flash-0731", url);
            assert_eq!(
                client.tag_policy("TagCorrectionResponse"),
                TagCorrectionPolicy::default(),
                "{url}"
            );
        }
        for model in [
            "deepseek/deepseek-chat",
            "~deepseek/deepseek-v4-flash-latest",
            "deepseek/deepseek-v4-flash-0731:nitro",
            "other/model",
        ] {
            assert_eq!(
                target
                    .clone()
                    .with_model(model)
                    .tag_policy("TagCorrectionResponse"),
                TagCorrectionPolicy::default()
            );
        }
        for schema in ["GenreFillResponse", "AssistantResponse", "AuditResponse"] {
            assert_eq!(target.tag_policy(schema), TagCorrectionPolicy::default());
        }
        assert_eq!(target.max_tokens, 1024);
        assert_eq!(target.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn targeted_policy_survives_schema_repair_and_sends_metadata_header() {
        let server = TestServer::start(vec![
            TestResponse::json(
                json!({"choices": [{"finish_reason": "length", "message": {"content": "{"}}]}),
            ),
            TestResponse::json(json!({"choices": [{"message": {"content": "{}"}}]})),
        ]);
        let mut client = OpenRouterClient::at("secret", "test/model", server.base_url())
            .with_generation(0.0, 5376);
        client.test_policy = Some(TagCorrectionPolicy {
            disable_reasoning: true,
            performance_routing: true,
            router_metadata: true,
        });
        client
            .complete_json(
                vec![],
                "TagCorrectionResponse",
                json!({}),
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        for i in 0..2 {
            let sent = server.request(i);
            assert_eq!(sent.body["reasoning"], json!({"enabled": false}));
            assert_eq!(
                sent.body["provider"],
                json!({"sort": {"by": "price"}, "preferred_min_throughput": {"p90": 50}, "allow_fallbacks": true})
            );
            assert_eq!(
                sent.headers
                    .get("x-openrouter-metadata")
                    .map(String::as_str),
                Some("enabled")
            );
        }
        assert_eq!(server.request(1).body["max_tokens"], 10752);
    }

    #[test]
    fn diagnostics_preserve_unknown_usage_and_ignore_untrusted_metadata() {
        let mut diagnostics = AttemptDiagnostics::default();
        diagnostics.capture(&json!({
            "id": "gen-test", "model": "test/model",
            "usage": {"completion_tokens": 12, "reasoning_tokens": 9,
                "completion_tokens_details": {"reasoning_tokens": 0}},
            "openrouter_metadata": {"attempt": 2, "attempts": [
                {"provider": "First", "status": 503},
                {"provider": "Second", "status": 200}],
                "endpoints": {"available": [{"provider": "Second", "selected": true}]},
                "summary": "secret prompt", "pipeline": [{"data": "secret prompt"}]},
            "choices": [{"finish_reason": "stop", "message": {"content": "secret prompt"}}]
        }));
        assert_eq!(diagnostics.reasoning_tokens, Some(0));
        assert_eq!(diagnostics.provider_attempts, Some(2));
        assert_eq!(diagnostics.selected_provider.as_deref(), Some("Second"));
        assert!(!serde_json::to_string(&diagnostics)
            .unwrap()
            .contains("secret prompt"));
        let mut missing = AttemptDiagnostics::default();
        missing.capture(&json!({"openrouter_metadata": ["bad"], "usage": {"reasoning_tokens": 7}}));
        assert_eq!(missing.reasoning_tokens, Some(7));
        assert_eq!(missing.completion_tokens, None);
        assert_eq!(missing.provider_attempts, None);
    }

    #[tokio::test]
    async fn cancellation_during_headers_body_and_retry_wait_is_terminal() {
        for phase in ["headers", "body", "retry"] {
            let mut reply =
                TestResponse::json(json!({"choices": [{"message": {"content": "{}"}}]}));
            if phase == "headers" {
                reply.headers_delay_ms = 100;
            }
            if phase == "body" {
                reply.body_delay_ms = 100;
            }
            if phase == "retry" {
                reply = TestResponse::status(503, "secret");
            }
            let server = TestServer::start(vec![reply]);
            let cancelled = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&cancelled);
            let cancel = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(30)).await;
                flag.store(true, Ordering::Release);
            });
            let mut diagnostics = CompletionDiagnostics::default();
            let result = OpenRouterClient::at("secret", "test/model", server.base_url())
                .with_retry_delays(vec![200, 200])
                .complete_json_observed(vec![], "Test", json!({}), &cancelled, &mut diagnostics)
                .await;
            cancel.await.unwrap();
            assert!(matches!(result, Err(OpenRouterError::Cancelled)), "{phase}");
            assert_eq!(diagnostics.attempts.len(), 1, "{phase}");
            assert_eq!(server.request_count(), 1, "{phase}");
        }
    }

    #[tokio::test]
    async fn schema_repair_and_body_read_share_the_original_deadline() {
        let mut first =
            TestResponse::json(json!({"choices": [{"message": {"content": "not JSON"}}]}));
        first.body_delay_ms = 40;
        let mut second = TestResponse::json(json!({"choices": [{"message": {"content": "{}"}}]}));
        second.body_delay_ms = 100;
        let server = TestServer::start(vec![first, second]);
        let mut diagnostics = CompletionDiagnostics::default();
        let result = OpenRouterClient::at("secret", "test/model", server.base_url())
            .with_timeout(Duration::from_millis(90))
            .complete_json_observed(
                vec![],
                "Test",
                json!({}),
                &AtomicBool::new(false),
                &mut diagnostics,
            )
            .await;
        assert!(matches!(result, Err(OpenRouterError::Timeout(90))));
        assert_eq!(diagnostics.attempts.len(), 2);
        assert_eq!(diagnostics.attempts[1].schema_repair, 1);
        assert!(diagnostics.total_elapsed_ms < 140);
    }

    #[tokio::test]
    async fn diagnostics_count_http_retries_separately_from_schema_repairs() {
        let server = TestServer::start(vec![
            TestResponse::status(503, "secret error"),
            TestResponse::json(
                json!({"choices": [{"finish_reason": "length", "message": {"content": "{\"tracks\":"}}]}),
            ),
            TestResponse::json(
                json!({"choices": [{"finish_reason": "stop", "message": {"content": "{}"}}]}),
            ),
        ]);
        let response = OpenRouterClient::at("secret", "test/model", server.base_url())
            .with_generation(0.0, 5376)
            .with_retry_delays(vec![0, 0])
            .complete_json(
                vec![ChatMessage::user("test")],
                "TagCorrectionResponse",
                json!({}),
                &AtomicBool::new(false),
            )
            .await
            .unwrap();
        assert_eq!(response.diagnostics.attempts.len(), 3);
        assert_eq!(
            response
                .diagnostics
                .attempts
                .iter()
                .map(|a| a.schema_repair)
                .collect::<Vec<_>>(),
            vec![0, 0, 1]
        );
        assert_eq!(response.diagnostics.attempts[0].status, Some(503));
        assert_eq!(server.request(2).body["max_tokens"], 10752);
    }

    #[tokio::test]
    async fn anthropic_response_format_is_parsed_correctly() {
        let server = TestServer::start(vec![TestResponse::json(json!({
            "id": "msg_01",
            "model": "claude-3-5-sonnet-20241022",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_01",
                "name": "AssistantResponse",
                "input": {"message": "Found 2 tracks", "actionBatch": null}
            }],
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        }))]);
        let client = OpenRouterClient::at("secret", "test/model", server.base_url())
            .with_provider(ProviderKind::Anthropic);

        let response = client
            .complete_json(
                vec![ChatMessage::user("list tracks")],
                "AssistantResponse",
                json!({"type": "object"}),
                &AtomicBool::new(false),
            )
            .await
            .unwrap();

        assert_eq!(response.data["message"], "Found 2 tracks");
        assert_eq!(response.usage.total_tokens, 15);
        assert_eq!(response.model, "claude-3-5-sonnet-20241022");
        // Verify the request was sent with x-api-key header (not Bearer).
        let request = server.request(0);
        assert_eq!(
            request.headers.get("x-api-key").map(String::as_str),
            Some("secret")
        );
        assert!(request.body["tool_choice"]["type"] == "tool");
        assert!(request.body["tools"][0]["name"] == "AssistantResponse");
    }

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
        assert!(request.body.get("provider").is_none());
        assert!(request.body.get("reasoning").is_none());
        assert!(!request.headers.contains_key("x-openrouter-metadata"));
        assert_eq!(
            request.body["response_format"]["json_schema"]["name"],
            "AuditResponse"
        );
    }

    #[tokio::test]
    async fn retries_non_json_content_once_with_explicit_schema_feedback() {
        let non_json =
            "I found 102 tracks missing a genre. Would you like me to set them all to \"Cantopop, Pop\"?";
        let server = TestServer::start(vec![
            TestResponse::status(429, "busy"),
            TestResponse::json(json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": non_json } }]
            })),
            TestResponse::json(json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": "{\"message\":\"I will prepare that change.\",\"toolCall\":null,\"actionBatch\":null}" } }]
            })),
        ]);
        let client = OpenRouterClient::at("secret", "test/model", server.base_url())
            .with_retry_delays(vec![0, 0]);

        let response = client
            .complete_json(
                vec![ChatMessage::user("audit")],
                "AssistantResponse",
                json!({ "type": "object" }),
                &AtomicBool::new(false),
            )
            .await
            .unwrap();

        assert_eq!(
            response.data,
            json!({
                "message": "I will prepare that change.",
                "toolCall": null,
                "actionBatch": null
            })
        );
        assert_eq!(server.request_count(), 3);
        let repair = server.request(2);
        assert_eq!(repair.body["reasoning"], json!({ "enabled": false }));
        let messages = repair.body["messages"].as_array().unwrap();
        assert_eq!(messages[messages.len() - 2]["role"], "assistant");
        assert_eq!(messages[messages.len() - 2]["content"], non_json);
        assert_eq!(messages[messages.len() - 1]["role"], "system");
        assert!(
            messages[messages.len() - 1]["content"]
                .as_str()
                .unwrap()
                .contains("AssistantResponse"),
            "repair feedback must restate which schema the model violated"
        );
    }

    #[tokio::test]
    async fn fails_loud_when_schema_repair_also_returns_non_json_content() {
        let server = TestServer::start(vec![
            TestResponse::json(json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": "Would you like me to continue?" } }]
            })),
            TestResponse::json(json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": "Yes or no?" } }]
            })),
        ]);
        let client = OpenRouterClient::at("secret", "test/model", server.base_url())
            .with_retry_delays(vec![0, 0]);

        let error = client
            .complete_json(
                vec![ChatMessage::user("audit")],
                "AssistantResponse",
                json!({ "type": "object" }),
                &AtomicBool::new(false),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "LLM returned non-JSON content: Yes or no?"
        );
        assert_eq!(
            server.request_count(),
            2,
            "schema repair must stay bounded to one retry"
        );
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

    /// Manual release gate: loads credentials through the root `justfile` and
    /// exercises the production endpoint, bearer auth, configured model,
    /// schema-constrained response, usage parsing, and Rustls transport.
    #[tokio::test]
    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and live OpenRouter access"]
    async fn live_openrouter_returns_schema_constrained_json() {
        let api_key = std::env::var("LLM_API_KEY").expect("LLM_API_KEY is required");
        let model = std::env::var("LLM_MODEL").expect("LLM_MODEL is required");
        let client = OpenRouterClient::new(api_key, model).with_generation(0.0, 128);
        let schema = json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean", "const": true } },
            "required": ["ok"],
            "additionalProperties": false
        });

        let response = client
            .complete_json(
                vec![
                    ChatMessage::system(
                        "Your entire response must be exactly {\"ok\":true} with no explanation.",
                    ),
                    ChatMessage::user("Return {\"ok\":true} and no other text."),
                ],
                "TauriMigrationSmoke",
                schema,
                &AtomicBool::new(false),
            )
            .await
            .expect("OpenRouter release gate should return schema-constrained JSON");

        assert_eq!(response.data, json!({ "ok": true }));
        assert!(!response.model.is_empty());
        assert!(response.usage.total_tokens > 0);
    }

    #[derive(Clone)]
    struct CapturedRequest {
        headers: HashMap<String, String>,
        body: Value,
    }

    struct TestResponse {
        status: u16,
        body: String,
        headers_delay_ms: u64,
        body_delay_ms: u64,
    }

    impl TestResponse {
        fn json(body: Value) -> Self {
            Self {
                status: 200,
                body: body.to_string(),
                headers_delay_ms: 0,
                body_delay_ms: 0,
            }
        }

        fn status(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.to_string(),
                headers_delay_ms: 0,
                body_delay_ms: 0,
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
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        response.status,
                        reason,
                        response.body.len(),
                    );
                    thread::sleep(Duration::from_millis(response.headers_delay_ms));
                    let _ = stream.write_all(reply.as_bytes());
                    thread::sleep(Duration::from_millis(response.body_delay_ms));
                    let _ = stream.write_all(response.body.as_bytes());
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
