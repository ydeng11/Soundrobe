//! Managed state for the Tauri shell.
//!
//! Per the plan: configuration, task registry, cancellation tokens, write queue,
//! provider clients, and SQLite connections. Held behind a [`tauri::State`]
//! guard and split so SQLite locks are never held across network requests.
//!
//! Not yet ported — populated per slice.

pub mod paths;

#[cfg(any(feature = "desktop", feature = "server"))]
pub mod album;
#[cfg(feature = "desktop")]
pub mod assistant;
#[cfg(feature = "desktop")]
pub mod assistant_task;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod audit;
/// Background-tagging settings, schedule evaluation, and run history.
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod background;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod config;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod conversation;
/// In-process crontab evaluation for the background-tagging schedule.
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod cron;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod events;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod library;
#[cfg(feature = "server")]
pub mod operation;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod providers;
#[cfg(feature = "desktop")]
pub mod quit_guard;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod sqlite;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod tag_source;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod tasks;
#[cfg(feature = "desktop")]
pub mod updater;
#[cfg(feature = "server")]
pub mod worker;
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod write_queue;

/// `~/.soundrobe/window-state.json` persistence + off-screen recovery.
#[cfg(feature = "desktop")]
pub mod window_state;
