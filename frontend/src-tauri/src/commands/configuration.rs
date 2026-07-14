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
//! DEFERRED cross-slice dependency: Electron's `config:set` additionally calls
//! `setStoredConfig({ apiKey|model })` when `llmApiKey`/`llmModel` change, to
//! keep the assistant runtime's cached key in sync. The assistant slice is not
//! ported yet, so that notification is wired when [`crate::state`] gains an
//! assistant state handle; until then the on-disk + live config are still
//! correct and the assistant reads them on (re)init.

use serde_json::Value;
use tauri::State;

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
pub fn config_set(state: State<'_, ConfigState>, key: String, value: Value) {
    state.set(&key, &value);
}
