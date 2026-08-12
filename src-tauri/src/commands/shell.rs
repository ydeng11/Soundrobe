//! Native-shell commands ported from `electron/main.ts`: folder selection
//! (`dialog:open-folder`), and (later) the track context menu + clipboard and
//! the quit-during-write guard.
//!
//! Folder dialog parity with Electron's `dialog:open-folder`:
//!   - `SOUNDROBE_E2E_LIBRARY_PATH` env override returns that path without a
//!     GUI prompt (the original Playwright E2E escape hatch; the WebdriverIO
//!     Tauri E2E tests use the same hook).
//!   - otherwise shows a native single-folder picker titled "Open Music Folder".
//!   - returns the selected path, or `null` for cancellation **or a plugin GUI
//!     failure**. The callback picker exposes only `Option<FilePath>`, so Tauri
//!     2 cannot distinguish/reject a native-dialog failure here. Electron
//!     rethrows those failures; that exact error parity is GUI-unobservable and
//!     remains pending a real-display smoke test.

use crate::error::ApiError;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, FilePath};
use tokio::sync::oneshot;

/// The env override Electron's handler reads to skip the GUI folder picker in
/// E2E/tests. Public so a contract test can set it and assert the no-GUI path.
pub const E2E_LIBRARY_PATH_ENV: &str = "SOUNDROBE_E2E_LIBRARY_PATH";

/// Resolve the folder-dialog result from the env override, if set. Pure and
/// testable without a GUI: returns `Some(path)` when the override is present,
/// `None` otherwise (meaning the GUI picker must run).
pub fn override_path(env_get: impl Fn(&str) -> Option<String>) -> Option<String> {
    env_get(E2E_LIBRARY_PATH_ENV).filter(|s| !s.is_empty())
}

/// `dialog:open-folder` / `openFolderDialog()`. Honors the E2E override, then
/// falls back to the native picker. Returns `null` on cancellation **or a GUI
/// failure** because the plugin provides no error result; Electron’s distinct
/// rejection path remains pending display validation.
#[tauri::command]
pub async fn dialog_open_folder(app: AppHandle) -> Option<String> {
    if let Some(p) = override_path(|name| std::env::var(name).ok()) {
        return Some(p);
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Open Music Folder")
        .pick_folder(move |picked| {
            let _ = sender.send(picked);
        });
    picked_to_string(receiver.await.ok().flatten())
}

/// Convert a picked [`FilePath`] to its display path (tests only; the GUI path
/// is verified under a display session).
pub fn picked_to_string(picked: Option<FilePath>) -> Option<String> {
    picked
        .as_ref()
        .and_then(FilePath::as_path)
        .map(Path::to_string_lossy)
        .map(String::from)
}

/// Electron's E2E hook for deterministic native context-menu selection.
pub const E2E_TRACK_CONTEXT_ACTION_ENV: &str = "SOUNDROBE_E2E_TRACK_CONTEXT_ACTION";

const MENU_EXTRA_TAGS: &str = "soundrobe.track-context.extra-tags";
const MENU_DELETE_FILES: &str = "soundrobe.track-context.delete-files";
const MENU_COPY_TITLE: &str = "soundrobe.track-context.copy-title";
const MENU_COPY_ARTIST: &str = "soundrobe.track-context.copy-artist";
const MENU_COPY_ALBUM_ARTIST: &str = "soundrobe.track-context.copy-album-artist";
const MENU_COPY_ALBUM: &str = "soundrobe.track-context.copy-album";
const MENU_COPY_PATH: &str = "soundrobe.track-context.copy-path";
const MENU_COPY_ALL: &str = "soundrobe.track-context.copy-all";

/// The only action values the renderer's `showTrackContextMenu()` contract
/// accepts. Serializes exactly as Electron's `"extra-tags"` / `"delete-files"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextMenuAction {
    ExtraTags,
    DeleteFiles,
}

/// Resolve Electron's E2E override. Unknown/empty values deliberately fall
/// through to the native menu rather than inventing a renderer-visible action.
pub fn e2e_context_action(env_get: impl Fn(&str) -> Option<String>) -> Option<ContextMenuAction> {
    match env_get(E2E_TRACK_CONTEXT_ACTION_ENV).as_deref() {
        Some("extra-tags") => Some(ContextMenuAction::ExtraTags),
        Some("delete-files") => Some(ContextMenuAction::DeleteFiles),
        _ => None,
    }
}

struct PendingContextMenu {
    track_path: String,
    labels: HashMap<String, String>,
    selection: Option<oneshot::Sender<ContextMenuAction>>,
}

/// Shared native-menu state. The global Tauri menu-event listener completes the
/// request after `popup_menu()` returns. One request at a time intentionally
/// mirrors Electron's modal native popup and prevents labels/actions crossing
/// requests.
#[derive(Default)]
pub struct ContextMenuState {
    pending: Mutex<Option<PendingContextMenu>>,
}

impl ContextMenuState {
    pub fn begin(
        &self,
        track_path: String,
        labels: HashMap<String, String>,
    ) -> Result<oneshot::Receiver<ContextMenuAction>, ApiError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| ApiError::ContextMenuStatePoisoned)?;
        if pending.is_some() {
            return Err(ApiError::ContextMenuAlreadyActive);
        }
        let (sender, receiver) = oneshot::channel();
        *pending = Some(PendingContextMenu {
            track_path,
            labels,
            selection: Some(sender),
        });
        Ok(receiver)
    }

    /// Handle a global Tauri menu event. Returns text that must be copied to the
    /// system clipboard and completes the pending command for every recognized
    /// item. Unknown IDs are unrelated menus.
    pub fn handle_menu_item(&self, id: &str) -> Option<String> {
        let mut pending = match self.pending.lock() {
            Ok(pending) => pending,
            Err(_) => {
                tracing::error!("track context menu state poisoned; ignoring menu event");
                return None;
            }
        };
        let request = pending.as_mut()?;
        let (selection, text) = match id {
            MENU_EXTRA_TAGS => (Some(Some(ContextMenuAction::ExtraTags)), None),
            MENU_DELETE_FILES => (Some(Some(ContextMenuAction::DeleteFiles)), None),
            MENU_COPY_TITLE => (Some(None), copy_label(&request.labels, "title")),
            MENU_COPY_ARTIST => (Some(None), copy_label(&request.labels, "artist")),
            MENU_COPY_ALBUM_ARTIST => (Some(None), copy_label(&request.labels, "albumArtist")),
            MENU_COPY_ALBUM => (Some(None), copy_label(&request.labels, "album")),
            MENU_COPY_PATH => (Some(None), non_empty(&request.track_path)),
            MENU_COPY_ALL => (Some(None), Some(copy_all_details(request))),
            _ => (None, None),
        };
        if let Some(selection) = selection {
            if let Some(sender) = request.selection.take() {
                if let Some(action) = selection {
                    let _ = sender.send(action);
                }
            }
        }
        text
    }

    /// Clear the active request after its selection is delivered or the native
    /// popup is dismissed. A poisoned state is logged rather than panicking.
    pub fn finish(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.take();
        } else {
            tracing::error!("track context menu state poisoned while clearing request");
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn copy_label(labels: &HashMap<String, String>, key: &str) -> Option<String> {
    labels.get(key).and_then(|value| non_empty(value))
}

fn label_or_dash<'a>(labels: &'a HashMap<String, String>, key: &str) -> &'a str {
    labels
        .get(key)
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .unwrap_or("-")
}

/// Exact Electron Copy All Details payload (labels, fallback, order, newlines).
fn copy_all_details(request: &PendingContextMenu) -> String {
    format!(
        "Title: {}\nArtist: {}\nAlbum Artist: {}\nAlbum: {}\nYear: {}\nTrack: {}\nGenre: {}\nPath: {}",
        label_or_dash(&request.labels, "title"),
        label_or_dash(&request.labels, "artist"),
        label_or_dash(&request.labels, "albumArtist"),
        label_or_dash(&request.labels, "album"),
        label_or_dash(&request.labels, "year"),
        label_or_dash(&request.labels, "track"),
        label_or_dash(&request.labels, "genre"),
        request.track_path,
    )
}

fn build_track_context_menu<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<Menu<R>, ApiError> {
    let extra_tags = MenuItem::with_id(app, MENU_EXTRA_TAGS, "Extra Tags...", true, None::<&str>)?;
    let delete_files =
        MenuItem::with_id(app, MENU_DELETE_FILES, "Delete File(s)", true, None::<&str>)?;
    let copy_title = MenuItem::with_id(app, MENU_COPY_TITLE, "Copy Title", true, None::<&str>)?;
    let copy_artist = MenuItem::with_id(app, MENU_COPY_ARTIST, "Copy Artist", true, None::<&str>)?;
    let copy_album_artist = MenuItem::with_id(
        app,
        MENU_COPY_ALBUM_ARTIST,
        "Copy Album Artist",
        true,
        None::<&str>,
    )?;
    let copy_album = MenuItem::with_id(app, MENU_COPY_ALBUM, "Copy Album", true, None::<&str>)?;
    let copy_path = MenuItem::with_id(app, MENU_COPY_PATH, "Copy Path", true, None::<&str>)?;
    let copy_all = MenuItem::with_id(app, MENU_COPY_ALL, "Copy All Details", true, None::<&str>)?;
    let first_separator = PredefinedMenuItem::separator(app)?;
    let second_separator = PredefinedMenuItem::separator(app)?;
    let third_separator = PredefinedMenuItem::separator(app)?;
    Ok(Menu::with_items(
        app,
        &[
            &extra_tags,
            &first_separator,
            &delete_files,
            &second_separator,
            &copy_title,
            &copy_artist,
            &copy_album_artist,
            &copy_album,
            &copy_path,
            &third_separator,
            &copy_all,
        ],
    )?)
}

/// Called by the single global Tauri menu-event listener in `run()`. Copy
/// effects are intentionally best-effort: Electron's `clipboard.writeText` does
/// not alter the modal action result; log a plugin failure but still resolve
/// `null` for the copy selection.
pub fn handle_context_menu_event(app: &AppHandle, id: &str) {
    let text = app.state::<ContextMenuState>().handle_menu_item(id);
    if let Some(text) = text {
        if let Err(error) = app.clipboard().write_text(text) {
            tracing::error!("failed to copy track context-menu text: {error}");
        }
    }
}

/// `track:context-menu` / `showTrackContextMenu()`. Honors Electron's E2E
/// override, otherwise shows the same native menu and returns the modal action
/// or null for dismissal/copy. Tauri queues the global menu event after
/// `popup_menu()` returns, so the command waits briefly for that deferred event
/// instead of clearing its request before the click can be delivered.
#[tauri::command]
pub async fn track_context_menu(
    app: AppHandle,
    state: State<'_, ContextMenuState>,
    track_path: String,
    labels: HashMap<String, String>,
) -> Result<Option<ContextMenuAction>, ApiError> {
    if let Some(action) = e2e_context_action(|name| std::env::var(name).ok()) {
        return Ok(Some(action));
    }
    let Some(window) = app.get_webview_window("main") else {
        return Ok(None);
    };

    let selection = state.begin(track_path, labels)?;
    let menu = build_track_context_menu(&app)?;
    if let Err(error) = window.popup_menu(&menu) {
        state.finish();
        return Err(error.into());
    }
    let action = tokio::time::timeout(Duration::from_millis(500), selection)
        .await
        .ok()
        .and_then(Result::ok);
    state.finish();
    Ok(action)
}

/// `window:focused` — deliberately no-op. Electron exposes this hook so the
/// renderer can notify the main process of focus; no Electron state changes,
/// and Tauri preserves that no-op contract.
#[tauri::command]
pub fn window_focused() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    /// Intent: native folder selection must yield control while the operating
    /// system dialog is open; a synchronous command can block Tauri's desktop
    /// event loop and leave both the window and picker unresponsive.
    #[test]
    fn folder_dialog_command_is_async() {
        fn assert_async_command<F, Fut>(_: F)
        where
            F: Fn(AppHandle) -> Fut,
            Fut: Future<Output = Option<String>>,
        {
        }

        assert_async_command(dialog_open_folder);
    }

    /// Intent: the E2E override short-circuits the GUI dialog so automated
    /// tests (Playwright/WebdriverIO) can pick a library folder without a
    /// native prompt — matching Electron's `SOUNDROBE_E2E_LIBRARY_PATH` hook.
    #[test]
    fn override_path_returns_env_value_when_set() {
        let get = |_: &str| Some("/test/library".to_string());
        assert_eq!(override_path(get).as_deref(), Some("/test/library"));
    }

    #[test]
    fn override_path_none_when_unset() {
        let get = |_: &str| None;
        assert_eq!(override_path(get), None);
    }

    /// Intent: an empty override must not be treated as a path, or the dialog
    /// would be skipped and the renderer would receive "" (a broken selection).
    #[test]
    fn override_path_ignores_empty_value() {
        let get = |_: &str| Some(String::new());
        assert_eq!(override_path(get), None);
    }

    /// Intent: `picked_to_string` extracts the path from a FilePath, and returns
    /// None when the user cancelled (None) — the dialog returns null on cancel.
    #[test]
    fn picked_to_string_and_none_on_cancel() {
        assert_eq!(picked_to_string(None), None);
    }

    /// Intent: the context-menu E2E hook accepts ONLY renderer-visible actions;
    /// a typo/unknown value must fall through to the native menu rather than
    /// fabricate a successful action.
    #[test]
    fn e2e_context_action_allows_only_known_actions() {
        assert_eq!(
            e2e_context_action(|_| Some("extra-tags".to_string())),
            Some(ContextMenuAction::ExtraTags)
        );
        assert_eq!(
            e2e_context_action(|_| Some("delete-files".to_string())),
            Some(ContextMenuAction::DeleteFiles)
        );
        assert_eq!(e2e_context_action(|_| Some("unknown".to_string())), None);
        assert_eq!(e2e_context_action(|_| None), None);
    }

    /// Intent: Copy All Details exactly preserves Electron's labels, `-`
    /// fallbacks, field order, and newline-separated payload before it reaches
    /// the system clipboard.
    #[test]
    fn copy_all_details_matches_electron_format() {
        let state = ContextMenuState::default();
        let labels = std::collections::HashMap::from([
            ("title".to_string(), "Song".to_string()),
            ("albumArtist".to_string(), "Album Artist".to_string()),
            ("year".to_string(), "2024".to_string()),
            ("track".to_string(), "03".to_string()),
        ]);
        let receiver = state.begin("/music/Song.mp3".to_string(), labels).unwrap();
        assert_eq!(
            state.handle_menu_item(MENU_COPY_ALL).as_deref(),
            Some(
                "Title: Song\nArtist: -\nAlbum Artist: Album Artist\nAlbum: -\nYear: 2024\nTrack: 03\nGenre: -\nPath: /music/Song.mp3"
            )
        );
        assert!(receiver.blocking_recv().is_err());
        state.finish();
    }

    /// Intent: only the two modal menu items resolve a renderer action; menu
    /// clicks must be scoped to their active popup and the state must be cleared
    /// after it closes so a later right-click starts fresh.
    #[test]
    fn menu_state_returns_action_and_clears_after_finish() {
        let state = ContextMenuState::default();
        let receiver = state
            .begin("/music/Song.mp3".to_string(), Default::default())
            .unwrap();
        assert_eq!(state.handle_menu_item(MENU_EXTRA_TAGS), None);
        assert_eq!(
            receiver.blocking_recv().unwrap(),
            ContextMenuAction::ExtraTags,
            "the command must receive a menu selection delivered after popup_menu returns"
        );
        state.finish();
    }

    /// Intent: every selectable native item must complete the active request;
    /// otherwise the command can remain stuck or clear the labels before the
    /// renderer action / clipboard effect is observed.
    #[test]
    fn every_context_menu_item_completes_the_active_request() {
        let labels = HashMap::from([
            ("title".to_string(), "Title".to_string()),
            ("artist".to_string(), "Artist".to_string()),
            ("albumArtist".to_string(), "Album Artist".to_string()),
            ("album".to_string(), "Album".to_string()),
        ]);

        let state = ContextMenuState::default();
        let receiver = state
            .begin("/music/Song.mp3".to_string(), labels.clone())
            .unwrap();
        assert_eq!(state.handle_menu_item(MENU_DELETE_FILES), None);
        assert_eq!(
            receiver.blocking_recv().unwrap(),
            ContextMenuAction::DeleteFiles
        );
        state.finish();

        let copy_cases = [
            (MENU_COPY_TITLE, "Title"),
            (MENU_COPY_ARTIST, "Artist"),
            (MENU_COPY_ALBUM_ARTIST, "Album Artist"),
            (MENU_COPY_ALBUM, "Album"),
            (MENU_COPY_PATH, "/music/Song.mp3"),
        ];
        for (menu_id, expected_text) in copy_cases {
            let receiver = state
                .begin("/music/Song.mp3".to_string(), labels.clone())
                .unwrap();
            assert_eq!(
                state.handle_menu_item(menu_id).as_deref(),
                Some(expected_text)
            );
            assert!(receiver.blocking_recv().is_err());
            state.finish();
        }
    }

    /// Intent: concurrent popups must fail loud rather than overwrite labels /
    /// resolve the wrong renderer promise (Electron's native popup is modal).
    #[test]
    fn menu_state_rejects_overlapping_request() {
        let state = ContextMenuState::default();
        let _receiver = state
            .begin("/music/one.mp3".to_string(), Default::default())
            .unwrap();
        let error = state
            .begin("/music/two.mp3".to_string(), Default::default())
            .unwrap_err();
        assert_eq!(error.to_string(), "track context menu already active");
        state.finish();
    }
}
