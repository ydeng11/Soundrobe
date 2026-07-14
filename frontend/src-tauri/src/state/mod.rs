//! Managed state for the Tauri shell.
//!
//! Per the plan: configuration, task registry, cancellation tokens, write queue,
//! provider clients, and SQLite connections. Held behind a [`tauri::State`]
//! guard and split so SQLite locks are never held across network requests.
//!
//! Not yet ported — populated per slice.

pub mod config;
pub mod providers;
pub mod sqlite;
pub mod tasks;
pub mod write_queue;

/// `~/.auto-tagger/window-state.json` persistence + off-screen recovery.
pub mod window_state;
