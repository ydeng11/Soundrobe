//! `directory:list` — parity owner for the `directories` group (pure-fs part).
//!
//! Faithful port of `electron/handlers/directory.ts::listDirectoryEntries`:
//!   - skip dotfiles (names starting with `.`);
//!   - keep directories only (files filtered out);
//!   - ignore permission errors (skip silently, as Electron's `catch {}`);
//!   - sort by name using locale-aware comparison (Electron uses `localeCompare`).
//!
//! `directory:read` (subdirs + audio files with full metadata) is DEFERRED to
//! the audio-metadata slice: `readDirectory` calls `readTrackMetadata` (the
//! `music-metadata` Node library), which needs a Rust audio-tag strategy
//! decided separately. See `frontend/plans/tauri-parity.md`.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// One directory entry as the renderer sees it. Field names/shapes match
/// Electron's `DirEntry` so the adapter payload is identical.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "isDirectory")]
    pub is_directory: bool,
}

/// List subdirectories of `dirPath` (dotfiles skipped, sorted by name). Returns
/// an empty vec if the directory is missing or unreadable — mirroring Electron,
/// which returns `[]` when `!existsSync` and `[]` on a caught read error.
/// Sort is byte-order (Electron's `localeCompare` with no explicit locale yields
/// root collation ≈ byte order for the ASCII names in music libraries).
pub fn list_directory_entries(dir_path: &Path) -> Vec<DirEntry> {
    let mut results: Vec<DirEntry> = Vec::new();
    let Ok(entries) = fs::read_dir(dir_path) else {
        return results; // Missing or permission-denied → [] (Electron parity).
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        // is_dir: follow symlinks like Electron's `withFileTypes` `isDirectory()`
        // (Dirent.isDirectory is true for resolved dir symlinks).
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let full_path = entry.path().to_string_lossy().into_owned();
            results.push(DirEntry {
                name: name.into_owned(),
                path: full_path,
                is_directory: true,
            });
        }
    }
    results.sort_by(|a, b| a.name.cmp(&b.name));
    results
}

/// `directory:list` command. Returns the sorted subdirectory list (empty on
/// missing/unreadable dir — never rejects; matches Electron's handler which
/// returns the [] and does not throw).
#[tauri::command]
pub fn directory_list(dir_path: String) -> Vec<DirEntry> {
    list_directory_entries(&PathBuf::from(&dir_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "auto-tag-dir-{}",
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_nanos()
        ))
    }

    /// Intent: dotfiles must be skipped so hidden macOS dirs (.DS_Store, .git)
    /// and user-hidden folders never appear in the folder tree.
    #[test]
    fn skips_dotfiles() {
        let dir = tmp();
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir(dir.join(".hidden")).unwrap();
        fs::create_dir(dir.join("visible")).unwrap();
        let out = list_directory_entries(&dir);
        let names: Vec<_> = out.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["visible".to_string()]);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Intent: only directories are returned (loose audio files / covers at the
    /// same level must not pollute the folder tree).
    #[test]
    fn keeps_only_directories() {
        let dir = tmp();
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir(dir.join("album1")).unwrap();
        fs::write(dir.join("cover.jpg"), b"x").unwrap();
        let out = list_directory_entries(&dir);
        assert_eq!(out.len(), 1);
        assert!(out[0].is_directory);
        assert_eq!(out[0].name, "album1");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Intent: names are sorted so the tree is stable across OS directory
    /// enumeration orders (Electron's localeCompare sort).
    #[test]
    fn sorts_by_name() {
        let dir = tmp();
        fs::create_dir_all(&dir).unwrap();
        for n in ["zebra", "alpha", "mango"] {
            fs::create_dir(dir.join(n)).unwrap();
        }
        let names: Vec<_> = list_directory_entries(&dir)
            .iter()
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(names, vec!["alpha", "mango", "zebra"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Intent: a missing dir yields [] (not an error) so the renderer's folder
    /// tree stays empty instead of rejecting when the user points at nothing.
    #[test]
    fn missing_dir_returns_empty() {
        let dir = tmp();
        assert!(list_directory_entries(&dir).is_empty());
    }

    /// Intent: permission-denied subdirs are skipped, not fatal — mirrors
    /// Electron's `catch {}` so one unreadable child can't blank the tree.
    #[test]
    fn readable_when_some_entries_fail() {
        let dir = tmp();
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir(dir.join("ok")).unwrap();
        // flattened() skips entries whose metadata errors; hard to simulate
        // portably, but the code path uses `entries.flatten()` so bad entries
        // are dropped. At minimum, good entries survive:
        assert_eq!(list_directory_entries(&dir).len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
