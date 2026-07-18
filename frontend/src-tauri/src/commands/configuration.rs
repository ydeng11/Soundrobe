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

use serde_json::Value;
use tauri::State;

use crate::state::{assistant::AssistantServicesState, config::ConfigState};

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
        _ => None,
    };
    if let Some(value) = assistant_value {
        assistant.update_config_value(&key, &value);
    }
}
