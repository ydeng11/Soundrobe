//! Debug subscription and mode commands.

use crate::infra::logging::DebugState;
use crate::state::config::ConfigState;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

#[tauri::command]
pub fn debug_subscribe() -> Value {
    json!({ "subscribed": true })
}

#[tauri::command]
pub fn debug_status(debug: State<'_, DebugState>) -> Value {
    json!({
        "enabled": debug.enabled(),
        "logFile": debug.log_file().map(|path| path.to_string_lossy().into_owned()),
        "forwardedCount": debug.forwarded_count(),
    })
}

#[tauri::command]
pub fn debug_toggle(enabled: bool, debug: State<'_, DebugState>) -> Value {
    debug.set_enabled(enabled);
    json!({ "enabled": debug.enabled() })
}

#[tauri::command]
pub fn debug_set_mode(
    enabled: bool,
    app: AppHandle,
    debug: State<'_, DebugState>,
    config: State<'_, ConfigState>,
) {
    debug.set_enabled(enabled);
    config.set("debug", &Value::Bool(enabled));
    if enabled {
        let message = debug
            .log_file()
            .map(|path| format!("Debug logging enabled → {}", path.to_string_lossy()))
            .unwrap_or_else(|| "Debug logging enabled".to_string());
        debug.emit(&app, "info", "debug", message, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_returns_electron_contract_shape() {
        assert_eq!(debug_subscribe(), json!({ "subscribed": true }));
    }
}
