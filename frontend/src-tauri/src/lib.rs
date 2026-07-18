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

use tauri::webview::PageLoadEvent;
use tauri::{Manager, PhysicalPosition, PhysicalSize, RunEvent, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::EnvFilter;

use crate::commands::shell::ContextMenuState;
use crate::infra::logging::DebugState;
use crate::state::assistant::{AssistantRuntimeState, AssistantServicesState};
use crate::state::audit::AuditState;
use crate::state::config::ConfigState;
use crate::state::conversation::ConversationState;
use crate::state::providers::ProviderState;
use crate::state::quit_guard::QuitGuard;
use crate::state::sqlite::CacheState;
use crate::state::tasks::TaskRegistry;
use crate::state::window_state::{DisplayWorkArea, PositionAction, WindowState};
use crate::state::write_queue::WriteQueue;

/// Initialise structured logging to stderr and append the same records to the
/// existing `~/.auto-tagger/auto-tagger.log`. `AUTOTAGGER_LOG` controls the
/// filter without changing the persisted path.
pub fn init_logging() {
    let filter =
        EnvFilter::try_from_env("AUTOTAGGER_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false);
    if let Some((_, writer)) =
        dirs::home_dir().and_then(|home| crate::infra::logging::general_log_writer(&home).ok())
    {
        let _ = builder.with_writer(writer.and(std::io::stderr)).try_init();
    } else {
        let _ = builder.with_writer(std::io::stderr).try_init();
    }
}

/// Build and launch the Tauri application. Stays the single entry point for
/// both dev and production builds; recipes drive Vite alongside this.
pub fn run() {
    init_logging();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .on_page_load(|webview, payload| {
            if should_reveal_window(webview.label(), payload.event()) {
                let _ = webview.window().show();
            }
        });
    #[cfg(feature = "wdio")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());

    let app = builder
        .setup(|app| {
            // Managed config: load once from ~/.auto-tagger/config.yaml + env,
            // mirroring Electron's `initializeAssistantServices(getRawApi
            // Config())` config bootstrapping (the full auto-tag TaskManager
            // port lands in a later slice; config is the first managed state).
            if let Some(home) = dirs::home_dir() {
                let config = ConfigState::init(home.clone());
                let raw_config = config.raw();
                let debug_enabled = raw_config.debug.unwrap_or(false);
                let cache = CacheState::new(home.clone());
                let _ = cache.initialize(raw_config.cache_path.as_deref());
                app.manage(DebugState::new(home.clone(), debug_enabled));
                app.manage(ConversationState::new(home));
                app.manage(cache);
                app.manage(config);
            }
            app.manage(AssistantRuntimeState::default());
            app.manage(AssistantServicesState::default());
            app.manage(AuditState::default());
            app.manage(ContextMenuState::default());
            app.manage(ProviderState::default());
            app.manage(WriteQueue::default());
            app.manage(QuitGuard::default());
            app.manage(TaskRegistry::default());
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
            commands::tracks::track_extra_tags_read,
            commands::mutations::track_write,
            commands::mutations::tracks_batch_write,
            commands::mutations::track_extra_tags_write,
            commands::mutations::tracks_batch_write_extra_tags,
            commands::mutations::track_rename,
            commands::mutations::file_exists,
            commands::mutations::track_delete_files,
            commands::covers::cover_data_url,
            commands::covers::cover_set,
            commands::covers::cover_remove,
            commands::covers::cover_download,
            commands::covers::cover_download_artist_art,
            commands::organizer::files_sort_by_album,
            commands::debug::debug_subscribe,
            commands::debug::debug_set_mode,
            commands::debug::debug_status,
            commands::debug::debug_toggle,
            commands::dataset::dataset_status,
            commands::tasks::task_progress,
            commands::tasks::task_cancel,
            commands::auto_tag::album_auto_tag,
            commands::lyrics::lyrics_fetch,
            commands::lyrics::album_download_lyrics,
            commands::conversation::assistant_init_runtime,
            commands::conversation::assistant_list_sessions,
            commands::conversation::assistant_get_conversation,
            commands::conversation::assistant_get_session,
            commands::conversation::assistant_current_session,
            commands::assistant::assistant_init_services,
            commands::assistant::assistant_send,
            commands::assistant::assistant_cancel,
            commands::assistant::assistant_clear,
            commands::assistant::assistant_apply_actions,
            commands::assistant::assistant_reject_actions,
            commands::assistant::assistant_get_batches,
            commands::audit::audit_run,
            commands::audit::audit_run_specified,
            commands::audit::audit_run_album,
            commands::audit::audit_apply_fixes,
            commands::audit::audit_cancel,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Auto Tagger Tauri shell");

    app.run(|app, event| {
        let RunEvent::ExitRequested { api, .. } = event else {
            return;
        };
        let queue = app.state::<WriteQueue>();
        let guard = app.state::<QuitGuard>();
        if !guard.should_prompt(queue.is_active()) {
            return;
        }
        api.prevent_exit();
        if !guard.begin_dialog() {
            return;
        }

        let prompt_app = app.clone();
        app.dialog()
            .message(concat!(
                "Tags are currently being written to disk.\n\n",
                "Quitting now may leave some files partially updated. ",
                "Do you want to quit anyway?"
            ))
            .title("Write in Progress")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Quit Anyway".to_string(),
                "Cancel".to_string(),
            ))
            .show(move |quit_anyway| {
                prompt_app.state::<QuitGuard>().finish_dialog(quit_anyway);
                if quit_anyway {
                    prompt_app.exit(0);
                }
            });
    });
}

fn should_reveal_window(label: &str, event: PageLoadEvent) -> bool {
    label == "main" && event == PageLoadEvent::Finished
}

/// Apply saved startup geometry (with off-screen recovery) and persist
/// `~/.auto-tagger/window-state.json` on
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveals_only_the_main_window_after_page_load_finishes() {
        assert!(!should_reveal_window("main", PageLoadEvent::Started));
        assert!(!should_reveal_window("settings", PageLoadEvent::Finished));
        assert!(should_reveal_window("main", PageLoadEvent::Finished));
    }
}
