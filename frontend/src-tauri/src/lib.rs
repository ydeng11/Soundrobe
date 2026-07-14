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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{Manager, PhysicalPosition, PhysicalSize, WindowEvent};
use tracing_subscriber::EnvFilter;

use crate::state::window_state::{DisplayWorkArea, WindowState};

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
            wire_window_lifecycle(app.get_webview_window("main"));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::meta::app_info])
        .run(tauri::generate_context!())
        .expect("error while running the Auto Tagger Tauri shell");
}

/// Apply saved startup geometry (with off-screen recovery), reveal the window
/// once the shell is ready, and persist `~/.auto-tagger/window-state.json` on
/// resize/move/maximize/close. Mirrors `electron/main.ts` createWindow + the
/// debounced savers + save-on-close behavior, using the same file in place.
fn wire_window_lifecycle(window: Option<tauri::WebviewWindow>) {
    let Some(window) = window else { return };

    let displays = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| DisplayWorkArea {
            x: m.position().x,
            y: m.position().y,
            width: m.size().width as i32,
            height: m.size().height as i32,
        })
        .collect::<Vec<_>>();

    let saved = dirs::home_dir().and_then(|h| WindowState::load(&WindowState::path(&h)));
    let bounds = WindowState::resolve(saved, &displays);

    // Apply saved size; place/center; honor maximized.
    let _ = window.set_size(PhysicalSize::new(bounds.width, bounds.height));
    if bounds.center || bounds.x.is_none() || bounds.y.is_none() {
        let _ = window.center();
    } else if let (Some(x), Some(y)) = (bounds.x, bounds.y) {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    if bounds.is_maximized {
        let _ = window.maximize();
    }

    // Electron ready-to-show -> show(): reveal now that geometry is applied.
    let _ = window.show();

    // WebviewWindow is a cheaply cloneable handle to the underlying window, so
    // we hand the event handler its own clone rather than borrowing the one we
    // call `on_window_event` on (which would conflict with the closure's move).
    // `handler_win` is the closure's own clone; maximize/unmaximize raise
    // `Resized` in Tauri, so geometry + `is_maximized` are captured there too.
    let handler_win = window.clone();
    let debounce = Arc::new(AtomicU64::new(0));

    window.on_window_event(move |event| match event {
        // Debounced saver for transient geometry events (Electron's 300ms).
        WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
            let epoch = debounce.fetch_add(1, Ordering::Relaxed) + 1;
            let d = Arc::clone(&debounce);
            let w = handler_win.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                if d.load(Ordering::Relaxed) == epoch {
                    save_window_state(&w);
                }
            });
        }
        WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
            save_window_state(&handler_win);
        }
        _ => {}
    });
}

/// Persist current geometry, mirroring Electron's `saveWindowState` (skip when
/// minimized/destroyed; best-effort on error).
fn save_window_state(window: &tauri::WebviewWindow) {
    let Some(home) = dirs::home_dir() else { return };
    if window.is_minimized().unwrap_or(false) {
        return;
    }
    let pos = window.outer_position().ok();
    let size = window.outer_size().ok();
    let is_maximized = window.is_maximized().unwrap_or(false);
    let (Some(pos), Some(size)) = (pos, size) else {
        return;
    };
    let state = WindowState {
        x: Some(pos.x),
        y: Some(pos.y),
        width: size.width as i32,
        height: size.height as i32,
        is_maximized,
    };
    let _ = WindowState::save(&WindowState::path(&home), &state);
}
