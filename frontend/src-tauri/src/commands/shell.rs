//! Native-shell commands ported from `electron/main.ts`: folder selection
//! (`dialog:open-folder`), and (later) the track context menu + clipboard and
//! the quit-during-write guard.
//!
//! Folder dialog parity with Electron's `dialog:open-folder`:
//!   - `AUTO_TAGGER_E2E_LIBRARY_PATH` env override returns that path without a
//!     GUI prompt (the original Playwright E2E escape hatch; the WebdriverIO
//!     Tauri E2E tests use the same hook).
//!   - otherwise shows a native single-folder picker titled "Open Music Folder".
//!   - returns the selected path, or `null` if cancelled.
//!   - rejects (surfaces an `Error`) on a dialog failure, like Electron's
//!     `throw error` in the catch.

use crate::error::ApiError;
use std::path::Path;
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};

/// The env override Electron's handler reads to skip the GUI folder picker in
/// E2E/tests. Public so a contract test can set it and assert the no-GUI path.
pub const E2E_LIBRARY_PATH_ENV: &str = "AUTO_TAGGER_E2E_LIBRARY_PATH";

/// Resolve the folder-dialog result from the env override, if set. Pure and
/// testable without a GUI: returns `Some(path)` when the override is present,
/// `None` otherwise (meaning the GUI picker must run).
pub fn override_path(env_get: impl Fn(&str) -> Option<String>) -> Option<String> {
    env_get(E2E_LIBRARY_PATH_ENV).filter(|s| !s.is_empty())
}

/// `dialog:open-folder` / `openFolderDialog()`. Honors the E2E override, then
/// falls back to the native folder picker. Returns `null` on cancel, rejects on
/// error (Electron parity).
#[tauri::command]
pub fn dialog_open_folder(app: AppHandle) -> Result<Option<String>, ApiError> {
    if let Some(p) = override_path(|name| std::env::var(name).ok()) {
        return Ok(Some(p));
    }
    let picked = app
        .dialog()
        .file()
        .set_title("Open Music Folder")
        .blocking_pick_folder();
    Ok(picked_to_string(picked))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Intent: the E2E override short-circuits the GUI dialog so automated
    /// tests (Playwright/WebdriverIO) can pick a library folder without a
    /// native prompt — matching Electron's `AUTO_TAGGER_E2E_LIBRARY_PATH` hook.
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
}
