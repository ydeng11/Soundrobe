//! Provider-neutral agent completion interface with native tool support.
//!
//! Defines the internal contract for LLM completions with typed tool calls:
//!   - [`AgentCompletion`] — unified response: text, tool calls, finish reason, model, usage.
//!   - [`AgentToolCall`] — a single tool call with stable call ID, canonical name, arguments.
//!   - [`AgentMessage`] — structured history entries (system, user, assistant, tool-call, tool-result).
//!   - [`AgentToolDef`] — tool definition with collision-tested transport-safe name mapping.
//!   - [`AgentProtocol`] — protocol selection (native tools vs JSON fallback).
//!
//! Transport implementations:
//!   - [`TransportOpenAi`] — real `tools`/`tool_calls`/`role:"tool"` for OpenAI-compatible endpoints.
//!   - [`TransportAnthropic`] — real `tool_use`/`tool_result` blocks for Claude.
//!
//! The JSON envelope normalizer from `commands::assistant` is retained as a compatibility
//! fallback when a provider explicitly rejects native tools.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

// ── Internal contract types ─────────────────────────────────────────

/// A single tool call from the model, with a stable call ID for result correlation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentToolCall {
    /// Provider-assigned call ID (e.g. `call_abc123` for OpenAI, `toolu_xyz` for Anthropic).
    pub id: String,
    /// Canonical dotted name (e.g. `"metadata.patch"`).
    pub canonical_name: String,
    /// Arguments as a JSON object.
    pub arguments: Value,
}

/// The unified model response.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentCompletion {
    /// Optional assistant message text.
    pub text: Option<String>,
    /// Tool calls from this turn (empty if only text).
    pub tool_calls: Vec<AgentToolCall>,
    /// Finish reason: "stop", "tool_calls", "length", "error", etc.
    pub finish_reason: String,
    /// Model identifier.
    pub model: String,
    /// Token usage.
    pub usage: AgentTokenUsage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AgentTokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// Structured history message types.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        tool_calls: Vec<AgentToolCall>,
    },
    ToolResult {
        tool_call_id: String,
        canonical_name: String,
        content: String,
        is_error: bool,
    },
}

impl AgentMessage {
    pub fn system(content: impl Into<String>) -> Self {
        AgentMessage::System {
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        AgentMessage::User {
            content: content.into(),
        }
    }
    pub fn assistant(content: Option<String>, tool_calls: Vec<AgentToolCall>) -> Self {
        AgentMessage::Assistant {
            content,
            tool_calls,
        }
    }
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        canonical_name: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        AgentMessage::ToolResult {
            tool_call_id: tool_call_id.into(),
            canonical_name: canonical_name.into(),
            content: content.into(),
            is_error,
        }
    }
}

// ── Tool definition and name mapping ─────────────────────────────────

/// A tool definition exposed to LLM providers.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentToolDef {
    /// Canonical dotted name (e.g. `"metadata.patch"`).
    pub canonical_name: String,
    /// Transport-safe name (e.g. `"metadata_patch"`).
    pub transport_name: String,
    /// Description shown to the model.
    pub description: String,
    /// JSON Schema for the tool's input arguments.
    pub input_schema: Value,
}

/// Registry of known tool names and their transport-safe mappings.
/// Built once and cached; panics on construction if collisions are detected.
pub struct AgentToolRegistry {
    /// Canonical name → transport-safe name.
    canonical_to_transport: HashMap<String, String>,
    /// Transport-safe name → canonical name.
    transport_to_canonical: HashMap<String, String>,
    /// All tool definitions by canonical name.
    definitions: HashMap<String, AgentToolDef>,
}

impl AgentToolRegistry {
    /// Build a registry from a list of [`AgentToolDef`].
    /// Panics if any transport-safe name collides with another after mapping,
    /// or if the mapping loses information (two different canonical names
    /// mapping to the same transport name).
    pub fn new(tools: Vec<AgentToolDef>) -> Self {
        let mut canonical_to_transport = HashMap::new();
        let mut transport_to_canonical = HashMap::new();
        let mut definitions = HashMap::new();

        for tool in tools {
            let canonical = tool.canonical_name.clone();
            let transport = tool.transport_name.clone();

            // Detect collision: two canonical names → same transport name.
            if let Some(existing) = transport_to_canonical.get(&transport) {
                panic!(
                    "Tool name collision: '{}' and '{}' both map to '{}'",
                    existing, canonical, transport
                );
            }
            // Detect reverse collision: same canonical name → different transport name.
            if let Some(existing) = canonical_to_transport.get(&canonical) {
                panic!(
                    "Tool name collision: '{}' maps to both '{}' and '{}'",
                    canonical, existing, transport
                );
            }

            canonical_to_transport.insert(canonical.clone(), transport.clone());
            transport_to_canonical.insert(transport.clone(), canonical.clone());
            definitions.insert(canonical.clone(), tool);
        }

        Self {
            canonical_to_transport,
            transport_to_canonical,
            definitions,
        }
    }

    /// Look up the transport-safe name for a canonical name.
    pub fn transport_name(&self, canonical_name: &str) -> Option<&str> {
        self.canonical_to_transport
            .get(canonical_name)
            .map(String::as_str)
    }

    /// Look up the canonical name for a transport-safe name.
    pub fn canonical_name(&self, transport_name: &str) -> Option<&str> {
        self.transport_to_canonical
            .get(transport_name)
            .map(String::as_str)
    }

    /// Get a tool definition by canonical name.
    pub fn definition(&self, canonical_name: &str) -> Option<&AgentToolDef> {
        self.definitions.get(canonical_name)
    }

    /// All transport-safe tool definitions as JSON for OpenAI/Anthropic.
    pub fn transport_definitions(&self) -> Vec<&AgentToolDef> {
        self.definitions.values().collect()
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }
}

/// Convert a canonical dotted name to a transport-safe underscore name.
/// `"metadata.patch"` → `"metadata_patch"`.
/// `"library.run_task"` → `"library_run_task"`.
pub fn canonical_to_transport_name(name: &str) -> String {
    name.replace('.', "_")
}

/// Convert a transport-safe underscore name back to a canonical dotted name.
/// Uses the global registry first for exact matches, then falls back to a heuristic
/// that replaces only the first dot-level boundary (before the last double-underscore-safe segment).
/// `"metadata_patch"` → `"metadata.patch"`.
/// `"library_run_task"` → `"library.run_task"` (from registry).
pub fn transport_to_canonical_name(name: &str) -> String {
    // Try the global registry first.
    if let Some(canonical) = global_registry().canonical_name(name) {
        return canonical.to_string();
    }
    // Fallback heuristic: iterate through all known canonical names and check.
    // Since we failed the registry lookup, try the simple replacement
    // but only for the last segment boundary to avoid run_task -> run.task.
    // The heuristic: replace every consecutive group of `_` characters that
    // appears before a known tool segment boundary.
    // For safety, just use the registry — but we registered all tools, so
    // if we get here it means this is an unknown tool, and the simple
    // replacement is the best we can do.
    name.replace('_', ".")
}

/// Build the default tool registry from the assistant tool catalog.
/// Each public tool gets a transport-safe name; returns the registry and
/// the list of definitions suitable for API calls.
pub fn default_tool_registry() -> AgentToolRegistry {
    let public_tools = crate::commands::assistant_tools::public_tool_definitions();
    let tools: Vec<AgentToolDef> = public_tools
        .into_iter()
        .map(|def| {
            let canonical = def.name.to_string();
            let transport = canonical_to_transport_name(&canonical);
            AgentToolDef {
                canonical_name: canonical,
                transport_name: transport,
                description: def.description.to_string(),
                input_schema: def.input_schema,
            }
        })
        .collect();
    AgentToolRegistry::new(tools)
}

/// Runtime registry held via `OnceLock` for thread-safe global access.
fn global_registry() -> &'static AgentToolRegistry {
    static REGISTRY: OnceLock<AgentToolRegistry> = OnceLock::new();
    REGISTRY.get_or_init(default_tool_registry)
}

// ── Protocol selection ─────────────────────────────────────────────

/// Whether the provider supports native tool calling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentProtocol {
    /// Native tool calling via `tools`/`tool_calls` (OpenAI/Anthropic).
    Native,
    /// JSON schema envelope fallback (legacy).
    JsonFallback,
}

/// Reason for native tool rejection — used to decide whether to fall back.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolRejectionKind {
    /// Provider explicitly returned an error indicating native tools are not supported.
    UnsupportedTools,
    /// Authentication error (401/403) — do not fall back.
    Auth,
    /// Rate limit or server error (429/5xx) — do not fall back.
    Server,
    /// Timeout — do not fall back.
    Timeout,
    /// Request was cancelled — do not fall back.
    Cancelled,
    /// Network/transport error — do not fall back.
    Network,
    /// Other unrecoverable error.
    Other,
}

/// Classify an HTTP error to determine whether to fall back to JSON mode.
pub fn classify_tool_error(status: u16, body: &str, provider: &str) -> ToolRejectionKind {
    match status {
        400 if body.contains("tools")
            || body.contains("function")
            || body.contains("not supported") =>
        {
            ToolRejectionKind::UnsupportedTools
        }
        400 if body.contains("provider_name") && provider == "openrouter" => {
            ToolRejectionKind::UnsupportedTools
        }
        401 | 403 => ToolRejectionKind::Auth,
        429 => ToolRejectionKind::Server,
        500..=599 => ToolRejectionKind::Server,
        _ => {
            // Check body for tool-unsupported indicators regardless of status.
            if body.contains("tool calls") && body.contains("not supported") {
                ToolRejectionKind::UnsupportedTools
            } else {
                ToolRejectionKind::Other
            }
        }
    }
}

// ── Transport: OpenAI-compatible ────────────────────────────────────

/// Build the request body for an OpenAI-compatible `/chat/completions` call with native tools.
pub fn build_openai_tool_request(
    messages: &[AgentMessage],
    tools: &[&AgentToolDef],
    model: &str,
    temperature: f64,
    max_tokens: u32,
) -> Value {
    let openai_messages: Vec<Value> = messages.iter().map(agent_message_to_openai).collect();

    let tool_defs: Vec<Value> = tools
        .iter()
        .map(|def| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": def.transport_name,
                    "description": def.description,
                    "parameters": def.input_schema,
                }
            })
        })
        .collect();

    serde_json::json!({
        "model": model,
        "messages": openai_messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "tools": tool_defs,
        "tool_choice": "auto",
    })
}

fn agent_message_to_openai(msg: &AgentMessage) -> Value {
    match msg {
        AgentMessage::System { content } => {
            serde_json::json!({"role": "system", "content": content})
        }
        AgentMessage::User { content } => {
            serde_json::json!({"role": "user", "content": content})
        }
        AgentMessage::Assistant {
            content,
            tool_calls,
        } => {
            let mut entry = serde_json::json!({"role": "assistant"});
            if let Some(text) = content {
                entry["content"] = Value::String(text.clone());
            } else {
                entry["content"] = Value::Null;
            }
            if !tool_calls.is_empty() {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": canonical_to_transport_name(&tc.canonical_name),
                                "arguments": tc.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                entry["tool_calls"] = Value::Array(calls);
            }
            entry
        }
        AgentMessage::ToolResult {
            tool_call_id,
            content,
            is_error,
            ..
        } => {
            serde_json::json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content,
                "is_error": is_error,
            })
        }
    }
}

/// Parse an OpenAI-compatible response into an [`AgentCompletion`].
pub fn parse_openai_completion(
    response: &Value,
    fallback_model: &str,
) -> Result<AgentCompletion, String> {
    let choices = response
        .pointer("/choices")
        .and_then(Value::as_array)
        .ok_or_else(|| "OpenAI response missing 'choices'".to_string())?;

    let choice = choices
        .first()
        .ok_or_else(|| "OpenAI response has empty choices".to_string())?;

    let message = choice
        .pointer("/message")
        .ok_or_else(|| "OpenAI choice missing 'message'".to_string())?;

    let text = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(str::to_string);

    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop")
        .to_string();

    let model = response
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(fallback_model)
        .to_string();

    let usage = response
        .pointer("/usage")
        .map(parse_usage_openai)
        .unwrap_or_default();

    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(parse_openai_tool_call)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(AgentCompletion {
        text,
        tool_calls,
        finish_reason,
        model,
        usage,
    })
}

fn parse_openai_tool_call(call: &Value) -> Option<AgentToolCall> {
    let id = call.get("id")?.as_str()?.to_string();
    let function = call.get("function")?;
    let transport_name = function.get("name")?.as_str()?.to_string();
    // Resolve via registry for correct bidirectional mapping.
    let canonical_name = global_registry()
        .canonical_name(&transport_name)
        .unwrap_or(&transport_name)
        .to_string();
    let arguments_str = function.get("arguments")?.as_str().unwrap_or("{}");
    let arguments: Value = serde_json::from_str(arguments_str).unwrap_or_default();

    Some(AgentToolCall {
        id,
        canonical_name,
        arguments,
    })
}

fn parse_usage_openai(usage: &Value) -> AgentTokenUsage {
    AgentTokenUsage {
        prompt_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

// ── Transport: Anthropic ────────────────────────────────────────────

/// Build the request body for Anthropic's `/v1/messages` API with native tools.
pub fn build_anthropic_tool_request(
    messages: &[AgentMessage],
    tools: &[&AgentToolDef],
    model: &str,
    max_tokens: u32,
) -> Value {
    let system_content: String = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::System { content } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let anthropic_messages: Vec<Value> = messages
        .iter()
        .filter_map(agent_message_to_anthropic)
        .collect();

    let tool_defs: Vec<Value> = tools
        .iter()
        .map(|def| {
            serde_json::json!({
                "name": def.transport_name,
                "description": def.description,
                "input_schema": def.input_schema,
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": anthropic_messages,
        "tools": tool_defs,
    });

    if !system_content.is_empty() {
        body["system"] = Value::String(system_content);
    }

    body
}

fn agent_message_to_anthropic(msg: &AgentMessage) -> Option<Value> {
    match msg {
        AgentMessage::System { .. } => None,
        AgentMessage::User { content } => {
            Some(serde_json::json!({"role": "user", "content": content}))
        }
        AgentMessage::Assistant {
            content,
            tool_calls,
        } => {
            let mut blocks = Vec::new();
            if let Some(text) = content {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": text,
                }));
            }
            for tc in tool_calls {
                blocks.push(serde_json::json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": canonical_to_transport_name(&tc.canonical_name),
                    "input": tc.arguments,
                }));
            }
            Some(serde_json::json!({"role": "assistant", "content": blocks}))
        }
        AgentMessage::ToolResult {
            tool_call_id,
            content,
            ..
        } => Some(serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_call_id,
                "content": content,
            }]
        })),
    }
}

/// Parse an Anthropic response into an [`AgentCompletion`].
pub fn parse_anthropic_completion(
    response: &Value,
    fallback_model: &str,
) -> Result<AgentCompletion, String> {
    let content = response
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| "Anthropic response missing 'content' array".to_string())?;

    let mut text: Option<String> = None;
    let mut tool_calls: Vec<AgentToolCall> = Vec::new();

    for block in content {
        let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
        match block_type {
            "text" => {
                let t = block.get("text").and_then(Value::as_str).unwrap_or("");
                if !t.is_empty() {
                    text = Some(t.to_string());
                }
            }
            "tool_use" => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let transport_name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // Resolve via registry for correct bidirectional mapping.
                let canonical_name = global_registry()
                    .canonical_name(&transport_name)
                    .unwrap_or(&transport_name)
                    .to_string();
                let arguments = block.get("input").cloned().unwrap_or_default();
                tool_calls.push(AgentToolCall {
                    id,
                    canonical_name,
                    arguments,
                });
            }
            _ => {}
        }
    }

    let stop_reason = response
        .get("stop_reason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn")
        .to_string();

    let model = response
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(fallback_model)
        .to_string();

    let usage = parse_usage_anthropic(response);

    let finish_reason = match stop_reason.as_str() {
        "end_turn" => "stop",
        "tool_use" => "tool_calls",
        "max_tokens" => "length",
        "stop_sequence" => "stop",
        other => other,
    };

    Ok(AgentCompletion {
        text,
        tool_calls,
        finish_reason: finish_reason.to_string(),
        model,
        usage,
    })
}

fn parse_usage_anthropic(response: &Value) -> AgentTokenUsage {
    let input = response
        .pointer("/usage/input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = response
        .pointer("/usage/output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    AgentTokenUsage {
        prompt_tokens: input,
        completion_tokens: output,
        total_tokens: input + output,
    }
}

// ── Normalization: validate a turn before execution ─────────────────

/// Result of validating a model turn.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnValidation {
    /// All calls are read-only; they may be executed in parallel order.
    AllReadOnly(Vec<AgentToolCall>),
    /// A single mutating call; must be the only mutation in this turn.
    SingleMutation(AgentToolCall),
    /// No tool calls — text-only response.
    MessageOnly,
    /// Multiple mutating calls were found; cannot execute.
    MultipleMutations(Vec<AgentToolCall>),
    /// Mixed read-only and mutating calls in one turn.
    MixedCalls,
}

/// Validate a model turn before execution.
/// - All read-only calls are allowed (in order).
/// - If any mutation exists, require it to be the only call.
/// - Reject mixed mutations or multiple mutations.
pub fn validate_turn(tool_calls: &[AgentToolCall]) -> TurnValidation {
    if tool_calls.is_empty() {
        return TurnValidation::MessageOnly;
    }

    let registry = global_registry();
    let mut read_only_calls = Vec::new();
    let mut mutating_calls = Vec::new();

    for call in tool_calls {
        let canonical_name = &call.canonical_name;
        if registry.definition(canonical_name).is_some() {
            let is_read_only =
                crate::commands::assistant_tools::registered_tool_is_read_only(canonical_name)
                    .unwrap_or(false);
            if is_read_only {
                read_only_calls.push(call.clone());
            } else {
                mutating_calls.push(call.clone());
            }
        } else {
            // Unknown tool — treat as error.
            return TurnValidation::MultipleMutations(tool_calls.to_vec());
        }
    }

    match (read_only_calls.len(), mutating_calls.len()) {
        (_, 0) => TurnValidation::AllReadOnly(tool_calls.to_vec()),
        (0, 1) => TurnValidation::SingleMutation(mutating_calls.into_iter().next().unwrap()),
        (0, _) => TurnValidation::MultipleMutations(mutating_calls),
        (_, _) => TurnValidation::MixedCalls,
    }
}

/// Normalize the tool-call state: if we detected mutations in the turn,
/// the first (and only) mutation is used; everything else is discarded.
pub fn normalize_turn_for_execution(
    tool_calls: Vec<AgentToolCall>,
) -> Result<Vec<AgentToolCall>, String> {
    match validate_turn(&tool_calls) {
        TurnValidation::AllReadOnly(calls) => Ok(calls),
        TurnValidation::SingleMutation(call) => Ok(vec![call]),
        TurnValidation::MessageOnly => Ok(vec![]),
        TurnValidation::MultipleMutations(_) => Err(
            "Model returned multiple mutating tool calls; call mutations one at a time".to_string(),
        ),
        TurnValidation::MixedCalls => Err(
            "Model mixed read-only and mutating calls in one turn; call them separately"
                .to_string(),
        ),
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Name mapping ────────────────────────────────────────────────

    #[test]
    fn transports_are_underscore_dotted_round_trips() {
        assert_eq!(
            canonical_to_transport_name("metadata.patch"),
            "metadata_patch"
        );
        assert_eq!(
            canonical_to_transport_name("api.lyricsSearch"),
            "api_lyricsSearch"
        );
        assert_eq!(
            canonical_to_transport_name("tracks.search"),
            "tracks_search"
        );
        // Registry-based resolution:
        assert_eq!(
            global_registry().canonical_name("metadata_patch"),
            Some("metadata.patch")
        );
        assert_eq!(
            global_registry().canonical_name("library_run_task"),
            Some("library.run_task")
        );
        assert_eq!(
            global_registry().canonical_name("tracks_search"),
            Some("tracks.search")
        );
        assert_eq!(
            global_registry().transport_name("metadata.patch"),
            Some("metadata_patch")
        );
        assert_eq!(
            global_registry().transport_name("library.run_task"),
            Some("library_run_task")
        );
        assert_eq!(
            global_registry().transport_name("api.musicbrainzSearch"),
            Some("api_musicbrainzSearch")
        );
    }

    #[test]
    fn simple_name_unchanged() {
        assert_eq!(canonical_to_transport_name("summarize"), "summarize");
        assert_eq!(transport_to_canonical_name("summarize"), "summarize");
    }

    #[test]
    fn registry_detects_collisions() {
        let tools = vec![AgentToolDef {
            canonical_name: "a.b".to_string(),
            transport_name: "a_b".to_string(),
            description: "".to_string(),
            input_schema: json!({}),
        }];
        // No collision — one tool.
        let reg = AgentToolRegistry::new(tools.clone());
        assert_eq!(reg.len(), 1);

        // Collision: same transport name.
        let mut colliding = tools.clone();
        colliding.push(AgentToolDef {
            canonical_name: "a.c".to_string(),
            transport_name: "a_b".to_string(),
            description: "".to_string(),
            input_schema: json!({}),
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            AgentToolRegistry::new(colliding);
        }));
        assert!(
            result.is_err(),
            "registry must panic on transport name collision"
        );
    }

    #[test]
    fn default_registry_has_all_public_tools() {
        let reg = default_tool_registry();
        // 16 public tools
        assert_eq!(reg.len(), 16);
        assert!(reg.transport_name("metadata.patch").is_some());
        assert!(reg.transport_name("library.summarize").is_some());
        assert!(reg.transport_name("files.relocate").is_some());
        assert_eq!(reg.transport_name("metadata.patch"), Some("metadata_patch"));
        assert_eq!(reg.canonical_name("metadata_patch"), Some("metadata.patch"));
    }

    #[test]
    fn default_registry_has_no_legacy_tools() {
        let reg = default_tool_registry();
        assert!(reg.definition("edit_metadata").is_none());
        assert!(reg.definition("create_plan").is_none());
    }

    // ── Turn validation ─────────────────────────────────────────────

    fn read_only_call(name: &str) -> AgentToolCall {
        AgentToolCall {
            id: format!("call_{}", name.replace('.', "_")),
            canonical_name: name.to_string(),
            arguments: json!({}),
        }
    }

    fn mutating_call(name: &str) -> AgentToolCall {
        AgentToolCall {
            id: format!("call_{}", name.replace('.', "_")),
            canonical_name: name.to_string(),
            arguments: json!({"target_scope": "selected"}),
        }
    }

    #[test]
    fn empty_turn_is_message_only() {
        assert_eq!(validate_turn(&[]), TurnValidation::MessageOnly);
    }

    #[test]
    fn single_read_only_turn_accepted() {
        let calls = vec![read_only_call("library.summarize")];
        assert!(matches!(
            validate_turn(&calls),
            TurnValidation::AllReadOnly(_)
        ));
    }

    #[test]
    fn multiple_read_only_turn_accepted() {
        let calls = vec![
            read_only_call("library.summarize"),
            read_only_call("tracks.search"),
        ];
        assert!(matches!(
            validate_turn(&calls),
            TurnValidation::AllReadOnly(_)
        ));
    }

    #[test]
    fn single_mutating_call_accepted() {
        let calls = vec![mutating_call("metadata.patch")];
        assert!(matches!(
            validate_turn(&calls),
            TurnValidation::SingleMutation(_)
        ));
    }

    #[test]
    fn multiple_mutations_rejected() {
        let calls = vec![
            mutating_call("metadata.patch"),
            mutating_call("metadata.transform"),
        ];
        assert!(matches!(
            validate_turn(&calls),
            TurnValidation::MultipleMutations(_)
        ));
    }

    #[test]
    fn mixed_read_only_and_mutating_rejected() {
        let calls = vec![
            read_only_call("library.summarize"),
            mutating_call("metadata.patch"),
        ];
        assert_eq!(validate_turn(&calls), TurnValidation::MixedCalls);
    }

    #[test]
    fn normalize_rejects_multiple_mutations() {
        let calls = vec![
            mutating_call("metadata.patch"),
            mutating_call("metadata.transform"),
        ];
        assert!(normalize_turn_for_execution(calls).is_err());
    }

    #[test]
    fn normalize_accepts_single_mutation() {
        let calls = vec![mutating_call("metadata.patch")];
        let result = normalize_turn_for_execution(calls).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].canonical_name, "metadata.patch");
    }

    #[test]
    fn normalize_accepts_multiple_read_only() {
        let calls = vec![
            read_only_call("library.summarize"),
            read_only_call("tracks.search"),
        ];
        let result = normalize_turn_for_execution(calls).unwrap();
        assert_eq!(result.len(), 2);
    }

    // ── OpenAI transport ───────────────────────────────────────────

    #[test]
    fn openai_request_builds_correctly() {
        let tools = default_tool_registry();
        let msg = AgentMessage::user("set genre to Pop");
        let request = build_openai_tool_request(
            &[msg],
            &tools.transport_definitions(),
            "test-model",
            0.0,
            4096,
        );

        assert_eq!(request["model"], "test-model");
        assert_eq!(request["messages"][0]["role"], "user");
        assert_eq!(request["messages"][0]["content"], "set genre to Pop");

        let tools_arr = request["tools"].as_array().unwrap();
        assert!(!tools_arr.is_empty());
        // Check tool_choice is auto
        assert_eq!(request["tool_choice"], "auto");
        // Verify a known tool is present
        let has_metadata_patch = tools_arr
            .iter()
            .any(|t| t["function"]["name"] == "metadata_patch");
        assert!(has_metadata_patch);
    }

    #[test]
    fn openai_parse_completion_with_tool_calls() {
        let response = json!({
            "id": "chatcmpl-abc",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "metadata_patch",
                            "arguments": r#"{"target_scope": "library", "changes": [{"field": "genre", "action": "set", "value": "Pop", "only_if_missing": true}]}"#
                        }
                    }]
                }
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });

        let completion = parse_openai_completion(&response, "fallback").unwrap();

        assert_eq!(completion.text, None);
        assert_eq!(completion.finish_reason, "tool_calls");
        assert_eq!(completion.model, "gpt-4o");
        assert_eq!(completion.usage.total_tokens, 15);
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].id, "call_abc123");
        assert_eq!(completion.tool_calls[0].canonical_name, "metadata.patch");
        assert!(completion.tool_calls[0]
            .arguments
            .get("target_scope")
            .is_some());
    }

    #[test]
    fn openai_parse_text_only() {
        let response = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": "Hello!"}
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        });

        let completion = parse_openai_completion(&response, "model").unwrap();

        assert_eq!(completion.text.as_deref(), Some("Hello!"));
        assert!(completion.tool_calls.is_empty());
        assert_eq!(completion.finish_reason, "stop");
    }

    #[test]
    fn openai_parse_missing_choices_is_error() {
        let response = json!({"error": "missing"});
        assert!(parse_openai_completion(&response, "model").is_err());
    }

    // ── Anthropic transport ─────────────────────────────────────────

    #[test]
    fn anthropic_request_builds_correctly() {
        let tools = default_tool_registry();
        let messages = vec![
            AgentMessage::system("You are a helpful assistant."),
            AgentMessage::user("set genre to Pop"),
        ];

        let request = build_anthropic_tool_request(
            &messages,
            &tools.transport_definitions(),
            "claude-3.5-sonnet",
            4096,
        );

        assert_eq!(request["model"], "claude-3.5-sonnet");
        assert_eq!(request["system"], "You are a helpful assistant.");
        assert_eq!(request["messages"][0]["role"], "user");
        assert_eq!(request["messages"][0]["content"], "set genre to Pop");

        let tools_arr = request["tools"].as_array().unwrap();
        assert!(!tools_arr.is_empty());
        let has_metadata_patch = tools_arr.iter().any(|t| t["name"] == "metadata_patch");
        assert!(has_metadata_patch);
    }

    #[test]
    fn anthropic_parse_completion_with_tool_use() {
        let response = json!({
            "id": "msg_01",
            "model": "claude-3-5-sonnet-20241022",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "I'll set the missing genres."},
                {
                    "type": "tool_use",
                    "id": "toolu_xyz",
                    "name": "metadata_patch",
                    "input": {"target_scope": "library", "changes": [{"field": "genre", "action": "set", "value": "Pop", "only_if_missing": true}]}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 8}
        });

        let completion = parse_anthropic_completion(&response, "fallback").unwrap();

        assert_eq!(
            completion.text.as_deref(),
            Some("I'll set the missing genres.")
        );
        assert_eq!(completion.finish_reason, "tool_calls");
        assert_eq!(completion.model, "claude-3-5-sonnet-20241022");
        assert_eq!(completion.usage.prompt_tokens, 10);
        assert_eq!(completion.usage.completion_tokens, 8);
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].id, "toolu_xyz");
        assert_eq!(completion.tool_calls[0].canonical_name, "metadata.patch");
    }

    #[test]
    fn anthropic_parse_text_only() {
        let response = json!({
            "content": [{"type": "text", "text": "Sure, done."}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 5, "output_tokens": 3}
        });

        let completion = parse_anthropic_completion(&response, "model").unwrap();

        assert_eq!(completion.text.as_deref(), Some("Sure, done."));
        assert!(completion.tool_calls.is_empty());
        assert_eq!(completion.finish_reason, "stop");
    }

    // ── Tool result message building ────────────────────────────────

    #[test]
    fn agent_message_tool_result_formats_correctly() {
        let result_msg = AgentMessage::tool_result("call_abc", "metadata.patch", "Success", false);

        // OpenAI format
        let openai = agent_message_to_openai(&result_msg);
        assert_eq!(openai["role"], "tool");
        assert_eq!(openai["tool_call_id"], "call_abc");
        assert_eq!(openai["content"], "Success");

        // Anthropic format
        let anthropic = agent_message_to_anthropic(&result_msg).unwrap();
        assert_eq!(anthropic["role"], "user");
        assert_eq!(anthropic["content"][0]["type"], "tool_result");
        assert_eq!(anthropic["content"][0]["tool_use_id"], "call_abc");
        assert_eq!(anthropic["content"][0]["content"], "Success");
    }

    #[test]
    fn agent_message_assistant_with_tool_calls_formats_correctly() {
        let tc = AgentToolCall {
            id: "call_1".into(),
            canonical_name: "metadata.patch".into(),
            arguments: json!({"target_scope": "library"}),
        };
        let msg = AgentMessage::assistant(Some("I'll do that.".to_string()), vec![tc]);

        let openai = agent_message_to_openai(&msg);
        assert_eq!(openai["role"], "assistant");
        assert_eq!(openai["content"], "I'll do that.");
        assert_eq!(openai["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            openai["tool_calls"][0]["function"]["name"],
            "metadata_patch"
        );

        let anthropic = agent_message_to_anthropic(&msg).unwrap();
        assert_eq!(anthropic["role"], "assistant");
        assert_eq!(anthropic["content"][0]["type"], "text");
        assert_eq!(anthropic["content"][0]["text"], "I'll do that.");
        assert_eq!(anthropic["content"][1]["type"], "tool_use");
        assert_eq!(anthropic["content"][1]["id"], "call_1");
        assert_eq!(anthropic["content"][1]["name"], "metadata_patch");
    }

    // ── Error classification ───────────────────────────────────────

    #[test]
    fn classify_unsupported_tools_by_body() {
        assert_eq!(
            classify_tool_error(400, "tool calls not supported by this provider", "openai"),
            ToolRejectionKind::UnsupportedTools
        );
        assert_eq!(
            classify_tool_error(400, "The model does not support function calling", "openai"),
            ToolRejectionKind::UnsupportedTools
        );
    }

    #[test]
    fn classify_auth_errors_no_fallback() {
        assert_eq!(
            classify_tool_error(401, "unauthorized", "openai"),
            ToolRejectionKind::Auth
        );
        assert_eq!(
            classify_tool_error(403, "forbidden", "openai"),
            ToolRejectionKind::Auth
        );
    }

    #[test]
    fn classify_server_errors_no_fallback() {
        assert_eq!(
            classify_tool_error(429, "rate limit", "openai"),
            ToolRejectionKind::Server
        );
        assert_eq!(
            classify_tool_error(503, "service unavailable", "openai"),
            ToolRejectionKind::Server
        );
    }
}
