//! `~/.auto-tagger/window-state.json` persistence and off-screen recovery.
//!
//! Pure logic ported from `electron/main.ts`:
//!   - persists `{ x, y, width, height, isMaximized }` to the same file in place
//!     (no move, no schema change);
//!   - on load, when BOTH `x` and `y` are saved, the position is checked
//!     against every display work area minus a 100px inner margin; an off-screen
//!     position is centered on the primary display, and an on-screen one is
//!     restored. When one or both axes are missing, the OS places the window
//!     (Electron's `x != null && y != null` guard) — never over-center;
//!   - a corrupted file is ignored (matches Electron's `catch {}`), never panics.
//!
//! The Tauri wiring (debounced save on resize/move, save on close, apply-on
//! startup) lives in `crate::run`; this module holds the testable core.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Defaults from Electron's `createWindow`: `1200 x 800`.
pub const DEFAULT_WIDTH: i32 = 1200;
pub const DEFAULT_HEIGHT: i32 = 800;
/// Margin used by Electron's on-screen check (`width - 100`).
const ON_SCREEN_MARGIN: i32 = 100;

/// A display work area, mirroring Electron's `screen.getAllDisplays()[i].workArea`.
#[derive(Debug, Clone, Copy)]
pub struct DisplayWorkArea {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Persisted window geometry. Field names match the JSON Electron writes.
/// `x`/`y` are optional in both Electron's writer and Rust's deserialize
/// (a stale file may omit them);
/// `width`/`height` are required like Electron's `WindowState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub is_maximized: bool,
}

/// What `createWindow` should do about the window position. Matches Electron
/// exactly:
///   - [`LeaveUnspecified`] — no saved position (or one axis missing): the OS
///     places the window; Electron never centers in this case.
///   - [`SetPosition`] — both x/y saved and on a display work area; restore them.
///   - [`Center`] — both x/y saved but off-screen; Electron calls
///     `mainWindow.center()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionAction {
    LeaveUnspecified,
    SetPosition { x: i32, y: i32 },
    Center,
}

/// Resolved startup geometry: what to actually apply to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedBounds {
    pub position: PositionAction,
    pub width: i32,
    pub height: i32,
    pub is_maximized: bool,
}

impl WindowState {
    /// Electron: when BOTH x and y are saved, the position is "on screen" iff
    /// it sits inside some display work area minus a 100px inner margin. Returns
    /// `false` when x or y were never saved (so the `x != null && y != null`
    /// guard in `createWindow` skips the center check entirely).
    pub fn is_on_any_screen(&self, displays: &[DisplayWorkArea]) -> bool {
        let (Some(x), Some(y)) = (self.x, self.y) else {
            return false;
        };
        displays.iter().any(|d| {
            x >= d.x
                && x < d.x + d.width - ON_SCREEN_MARGIN
                && y >= d.y
                && y < d.y + d.height - ON_SCREEN_MARGIN
        })
    }

    /// Decide startup geometry from a (possibly absent) saved state and the
    /// available displays. Mirrors `createWindow`: size defaults when absent,
    /// restore a complete on-screen position, center only a complete off-screen
    /// position, and otherwise leave placement to the OS.
    pub fn resolve(saved: Option<WindowState>, displays: &[DisplayWorkArea]) -> ResolvedBounds {
        match saved {
            None => ResolvedBounds {
                position: PositionAction::LeaveUnspecified,
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
                is_maximized: false,
            },
            Some(s) => {
                // Electron's guard: only both-present positions are checked.
                let position = match (s.x, s.y) {
                    (Some(x), Some(y)) => {
                        if s.is_on_any_screen(displays) {
                            PositionAction::SetPosition { x, y }
                        } else {
                            PositionAction::Center
                        }
                    }
                    // One axis missing: Electron passes the partial coords to
                    // BrowserWindow as `undefined` for the missing axis, which the
                    // OS replaces — so we too leave placement to the OS.
                    _ => PositionAction::LeaveUnspecified,
                };
                ResolvedBounds {
                    position,
                    width: s.width,
                    height: s.height,
                    is_maximized: s.is_maximized,
                }
            }
        }
    }

    /// Path to the persisted state. Same location Electron uses.
    pub fn path(home: &Path) -> PathBuf {
        home.join(".auto-tagger").join("window-state.json")
    }

    /// Load the state, ignoring missing/corrupt files (Electron's `catch {}`).
    pub fn load(path: &Path) -> Option<WindowState> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<WindowState>(&bytes).ok(),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            // Corrupted/permission error — never surface; fall back to defaults.
            Err(_) => None,
        }
    }

    /// Persist the state, creating the parent directory. Mirrors Electron's
    /// best-effort write (`JSON.stringify(state, null, 2)`).
    pub fn save(path: &Path, state: &WindowState) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
        fs::write(path, json + "\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disp(x: i32, y: i32, w: i32, h: i32) -> DisplayWorkArea {
        DisplayWorkArea {
            x,
            y,
            width: w,
            height: h,
        }
    }

    /// Intent: a saved position exactly at the work-area origin is on screen,
    /// but one shifted past `width - 100` is treated off-screen (Electron's
    /// 100px guard prevents restoring windows whose top-left edge is in the
    /// far margin of a secondary display).
    #[test]
    fn on_screen_check_uses_100px_margin() {
        let s = WindowState {
            x: Some(0),
            y: Some(0),
            width: 1200,
            height: 800,
            is_maximized: false,
        };
        assert!(s.is_on_any_screen(&[disp(0, 0, 1440, 900)]));

        // Just inside the 100px inner margin -> still on screen.
        let inside = WindowState {
            x: Some(1339),
            y: Some(0),
            width: 1200,
            height: 800,
            is_maximized: false,
        };
        assert!(inside.is_on_any_screen(&[disp(0, 0, 1440, 900)]));

        // At 1340 (== 1440 - 100) the boundary is exclusive -> off screen.
        let edge = WindowState {
            x: Some(1340),
            y: Some(0),
            width: 1200,
            height: 800,
            is_maximized: false,
        };
        assert!(!edge.is_on_any_screen(&[disp(0, 0, 1440, 900)]));
    }

    /// Intent: a saved position on a secondary display's work area must be
    /// recognized so multi-monitor restores work; a negative offset is off-screen.
    #[test]
    fn multiple_displays_cover_secondary() {
        let displays = [disp(0, 0, 1440, 900), disp(1440, 0, 1920, 1080)];
        let on_secondary = WindowState {
            x: Some(1500),
            y: Some(100),
            width: 1200,
            height: 800,
            is_maximized: false,
        };
        assert!(on_secondary.is_on_any_screen(&displays));

        let left_void = WindowState {
            x: Some(-500),
            y: Some(-500),
            width: 1200,
            height: 800,
            is_maximized: false,
        };
        assert!(!left_void.is_on_any_screen(&displays));
    }

    #[test]
    fn missing_position_is_not_on_screen() {
        let s = WindowState {
            x: None,
            y: None,
            width: 1200,
            height: 800,
            is_maximized: false,
        };
        assert!(!s.is_on_any_screen(&[disp(0, 0, 1440, 900)]));
    }

    #[test]
    fn resolve_defaults_when_no_saved_state() {
        let r = WindowState::resolve(None, &[disp(0, 0, 1440, 900)]);
        assert_eq!(
            r,
            ResolvedBounds {
                position: PositionAction::LeaveUnspecified,
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
                is_maximized: false
            }
        );
    }

    #[test]
    fn resolve_centers_when_off_screen() {
        let off = WindowState {
            x: Some(-500),
            y: Some(-500),
            width: 1100,
            height: 700,
            is_maximized: false,
        };
        let r = WindowState::resolve(Some(off), &[disp(0, 0, 1440, 900)]);
        assert_eq!(r.position, PositionAction::Center);
        assert_eq!(r.width, 1100);
    }

    #[test]
    fn resolve_restores_when_on_screen() {
        let on_ = WindowState {
            x: Some(100),
            y: Some(100),
            width: 1100,
            height: 700,
            is_maximized: true,
        };
        let r = WindowState::resolve(Some(on_), &[disp(0, 0, 1440, 900)]);
        assert_eq!(r.position, PositionAction::SetPosition { x: 100, y: 100 });
        assert!(r.is_maximized);
    }

    /// Intent: when x or y was never saved (Electron's
    /// `savedState?.x != null && savedState?.y != null` guard is false),
    /// Electron does NOT center — it leaves the missing axis to the OS. The
    /// Tauri port must not over-center and override a reasonable OS placement.
    #[test]
    fn resolve_leaves_unspecified_when_axis_missing() {
        let missing_y = WindowState {
            x: Some(100),
            y: None,
            width: 1100,
            height: 700,
            is_maximized: false,
        };
        let r = WindowState::resolve(Some(missing_y), &[disp(0, 0, 1440, 900)]);
        assert_eq!(r.position, PositionAction::LeaveUnspecified);

        let missing_x = WindowState {
            x: None,
            y: Some(200),
            width: 1100,
            height: 700,
            is_maximized: false,
        };
        assert_eq!(
            WindowState::resolve(Some(missing_x), &[disp(0, 0, 1440, 900)]).position,
            PositionAction::LeaveUnspecified
        );
    }

    /// Intent: a corrupted state file must NOT crash the app — Electron
    /// silently falls back to defaults; the Tauri port must too.
    #[test]
    fn load_ignores_corrupt_file() {
        let dir = tempdir_state();
        let file = dir.join("window-state.json");
        fs::write(&file, "{ not valid json").unwrap();
        assert_eq!(WindowState::load(&file), None);
    }

    #[test]
    fn load_returns_none_for_missing() {
        assert_eq!(
            WindowState::load(Path::new("/no/such/path/here/state.json")),
            None
        );
    }

    /// Intent: persistence round-trips the exact shape Electron writes so both
    /// runtimes can read each other's file during migration.
    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir_state();
        let file = dir.join("window-state.json");
        let s = WindowState {
            x: Some(7),
            y: Some(9),
            width: 1200,
            height: 800,
            is_maximized: true,
        };
        WindowState::save(&file, &s).unwrap();
        let loaded = WindowState::load(&file).unwrap();
        assert_eq!(loaded, s);
    }

    /// Intent: missing keys default gracefully (Electron wrote `isMaximized`
    /// only sometimes); a minimal `{width,height}` file loads without error.
    #[test]
    fn save_creates_parent_and_minimal_payload_loads() {
        let dir = tempdir_state();
        let nested = dir.join("nested/deep/window-state.json");
        let s = WindowState {
            x: None,
            y: None,
            width: 1000,
            height: 600,
            is_maximized: false,
        };
        WindowState::save(&nested, &s).unwrap();
        assert!(nested.exists());
        assert_eq!(WindowState::load(&nested), Some(s));
    }

    fn tempdir_state() -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("auto-tagger-window-state-{}", std::process::id()));
        let unique = base.join(format!(
            "{}",
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&unique).unwrap();
        unique
    }
}
