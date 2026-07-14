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

use crate::commands::shell::ContextMenuState;
use crate::state::config::ConfigState;
use crate::state::window_state::{DisplayWorkArea, PositionAction, WindowState};
use crate::state::write_queue::WriteQueue;

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
            // Managed config: load once from ~/.auto-tagger/config.yaml + env,
            // mirroring Electron's `initializeAssistantServices(getRawApi
            // Config())` config bootstrapping (the full auto-tag TaskManager
            // port lands in a later slice; config is the first managed state).
            if let Some(home) = dirs::home_dir() {
                app.manage(ConfigState::init(home));
            }
            app.manage(ContextMenuState::default());
            app.manage(WriteQueue::default());
            // Tauri menu events are global; ContextMenuState scopes recognized
            // IDs to the single active popup so ordinary app/tray menu events
            // cannot resolve a renderer context-menu promise.
            app.on_menu_event(|app, event| {
                let id: &str = event.id().as_ref();
                commands::shell::handle_context_menu_event(app, id);
            });
            wire_window_lifecycle(app.get_webview_window("main"));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::meta::app_info,
            commands::configuration::config_get,
            commands::configuration::config_set,
            commands::shell::dialog_open_folder,
            commands::shell::track_context_menu,
            commands::shell::window_focused,
            commands::directories::directory_list,
            commands::directories::directory_read,
            commands::library::library_scan,
            commands::library::album_refresh,
            commands::tracks::album_read,
            commands::mutations::track_write,
        ])
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

    // Apply saved size; place/center/leave; honor maximized.
    let _ = window.set_size(PhysicalSize::new(bounds.width, bounds.height));
    match bounds.position {
        PositionAction::SetPosition { x, y } => {
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
        PositionAction::Center => {
            let _ = window.center();
        }
        PositionAction::LeaveUnspecified => {} // OS places, like Electron's `undefined` x/y.
    }
    if bounds.is_maximized {
        let _ = window.maximize();
    }

    // Reveal the window after geometry is applied. PENDING PARITY: Electron
    // waits for the renderer `ready-to-show` event (first paint) before
    // `show()`. Tauri 2's exact equivalent (`on_page_load(Finished)`) requires
    // the `unstable` feature; without it we reveal immediately once geometry is
    // applied. For a pre-rendered `frontendDist` build this is visually
    // indistinguishable; the no-flash guarantee is verified separately under a
    // real display session and tracked as a pending window-lifecycle row.
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
