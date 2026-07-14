//! Auto Tagger — Tauri 2 native shell (Electron migration).
//!
//! The renderer-neutral `window.api` contract lives in
//! `frontend/src/shared/desktop-api.ts`. This crate implements the same
//! surface with Tauri commands (request/response via `invoke`) and Tauri
//! events (the low-frequency `auto-tag:event`, `audit:event`,
//! `assistant:event`, and `debug:log` streams). Structured Rust errors
//! are converted to rejected JavaScript `Error` objects so renderer
//! behavior is unchanged.
//!
//! Crate layout follows the plan's behavioral groups:
//!   - [`state`] — managed state (configuration, task registry, cancellation
//!     tokens, write queue, provider clients, SQLite connections).
//!   - [`commands`] — Tauri commands (library, tracks, directories, covers,
//!     configuration, dataset, tasks, audit, assistant, organizer, conversation).
//!   - [`infra`] — filesystem, SQLite, HTTP, logging, artwork, encoding
//!     conversion, and audio tag I/O.
//!
//! Each slice is ported behind a failing contract test first (TDD); nothing
//! here is wired to the renderer until its parity row is green.

pub mod commands;
pub mod infra;
pub mod state;

mod error;

use tauri::Manager;
use tracing_subscriber::EnvFilter;

/// Initialise structured logging. Debug forwarding to the renderer's
/// `debug:log` stream is wired in the debug slice; until then logs land in
/// `~/.auto-tagger/auto-tagger.log` via the ` AUTOTAGGER_LOG` env filter.
pub fn init_logging() {
    let filter =
        EnvFilter::try_from_env("AUTOTAGGER_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// Build and launch the Tauri application. Stays the single entry point for
/// both dev and production builds; recipes drive Vite alongside this.
pub fn run() {
    init_logging();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // Mirrors the Electron "show:false + ready-to-show -> show()" flow:
            // the window is created hidden and revealed once the shell is ready.
            // Off-screen position recovery and window-state persistence land in
            // the native-shell slice; the scaffold just shows the main window.
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::meta::app_info])
        .run(tauri::generate_context!())
        .expect("error while running the Auto Tagger Tauri shell");
}
