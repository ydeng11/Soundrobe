//! `config:get` / `config:set` — parity owner for the `configuration` group.
//!
//! Mirrors `electron/main.ts` `config:get`/`config:set` handlers backed by
//! [`crate::state::config::ConfigState`] (the live, managed config). Parity:
//!   - `config_get` returns the **redacted** renderer view, or `{}` on internal
//!     error — Electron's handler catches and returns an empty object; it never
//!     rejects, so this command returns the value directly (no `Result`).
//!   - `config_set` writes the camelCase key to disk + refreshes, and never
//!     rejects — Electron's handler catches and logs; the write/refresh failure
//!     is logged inside [`ConfigState::set`], and the command returns `()`.
//!
//! `llmApiKey` and `llmModel` changes also refresh the assistant service's
//! cached credentials from the resulting live config, matching Electron's
//! `setStoredConfig` synchronization without exposing secrets to the renderer.

use serde_json::{json, Value};
use tauri::State;

use crate::error::ApiError;
use crate::infra::is_not_redacted;
use crate::infra::openrouter::{LlmEndpoint, OpenRouterClient};
use crate::state::assistant::AssistantServicesState;
use crate::state::config::ConfigState;

/// `getConfig()` — redacted renderer view. Sync because `ConfigState` is a
/// `Mutex` snapshot (no async work); never rejects so renderer `try/catch` is a
/// no-op (matches Electron, which catches and returns / logs).
#[tauri::command]
pub fn config_get(state: State<'_, ConfigState>) -> Value {
    state.redacted()
}

/// `setConfig(key, value)` — persist a renderer camelCase key and refresh.
/// Sync; never rejects — failures are logged inside `ConfigState::set`.
#[tauri::command]
pub fn config_set(
    state: State<'_, ConfigState>,
    assistant: State<'_, AssistantServicesState>,
    key: String,
    value: Value,
) {
    state.set(&key, &value);
    let live = state.raw();
    let assistant_value = match key.as_str() {
        "llmApiKey" => Some(Value::String(live.llm_api_key.unwrap_or_default())),
        "llmModel" => Some(Value::String(live.llm_model.unwrap_or_default())),
        "llmProvider" | "llmBaseUrl" => Some(value.clone()),
        _ => None,
    };
    if let Some(value) = assistant_value {
        assistant.update_config_value(&key, &value);
    }
}

/// Test an LLM connection by sending a minimal prompt to the specified
/// provider/model combination.  Returns the responding model name on success.
///
/// When `api_key` is empty the command resolves credentials from the
/// persistent config (file/env).  When `provider` is absent the command
/// falls back to the stored config value.  This lets users test an
/// already-configured key without re-entering it in Settings.
#[tauri::command]
pub async fn test_llm_connection(
    api_key: String,
    model: String,
    provider: Option<String>,
    base_url: Option<String>,
    config: State<'_, ConfigState>,
) -> Result<serde_json::Value, ApiError> {
    let raw = config.raw();
    // Resolve api_key: explicit (from renderer) → config file/env.
    // Redacted placeholders ("****...") from the UI must be rejected —
    // the real key always lives in ConfigState.
    let effective_key = if api_key.is_empty() || !is_not_redacted(&api_key) {
        raw.llm_api_key
            .as_deref()
            .filter(|k| is_not_redacted(k))
            .ok_or_else(|| {
                ApiError::Message(
                    "No API key provided and none configured. Set LLM_API_KEY in Settings or env."
                        .into(),
                )
            })?
            .to_string()
    } else {
        api_key
    };
    let effective_model = if model.is_empty() {
        raw.llm_model
            .as_deref()
            .filter(|m| !m.is_empty())
            .ok_or_else(|| {
                ApiError::Message(
                    "No model provided and none configured. Set LLM_MODEL in Settings or env."
                        .into(),
                )
            })?
            .to_string()
    } else {
        model
    };
    // Resolve provider + base_url: explicit → config → defaults.
    let effective_provider = provider
        .as_deref()
        .filter(|p| !p.is_empty())
        .or(raw.llm_provider.as_deref());
    let endpoint = LlmEndpoint::from_config(
        effective_provider,
        base_url
            .as_deref()
            .filter(|u| !u.is_empty())
            .or(raw.llm_base_url.as_deref()),
    );
    let client = OpenRouterClient::at(&effective_key, &effective_model, &endpoint.base_url)
        .with_provider(endpoint.provider)
        .with_generation(0.0, 64)
        .with_timeout(std::time::Duration::from_secs(15));
    let responding_model = client
        .test_connection()
        .await
        .map_err(|e| ApiError::Message(format!("LLM test failed: {e}")))?;
    Ok(json!({"model": responding_model}))
}
