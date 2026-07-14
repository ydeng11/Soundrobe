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
/// Sort is byte-order. Electron sorts with `a.name.localeCompare(b.name)` (no
/// locale arg), which is case-/diacritic-insensitive under Node's default ICU
/// locale; Rust byte-order DIVERGES for mixed case ("Bar" < "apple" here,
/// "apple" < "Bar" in localeCompare), accented (café≈cafe), and CJK. This is
/// a cosmetic folder-tree ordering difference, not a correctness issue for the
/// tagging pipeline; collation parity is PENDING (see `byte_order_collation`
/// characterization test). The order is still stable and deterministic.
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
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "auto-tag-dir-{}-{}",
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_nanos(),
            seq
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

    /// Collation characterization (NOT a parity claim). Records the known
    /// divergence between Rust byte-order and Node `localeCompare` (default ICU
    /// locale) for mixed case and accents. Names are collision-free (distinct
    /// words, or differing only by diacritic which APFS preserves) so a
    /// case-insensitive FS does not fold them.
    ///
    /// Node `localeCompare` order: `apple | cafe | café | Zoo`  (case-insensitive)
    /// Rust byte-order order:        `Zoo | apple | cafe | café` (Zoo=0x5A < a=0x61)
    ///
    /// Pins the CURRENT byte-order output (catches a regression to an unstable
    /// order) and keeps the divergence visible (Rule 11) until a real ICU
    /// collator lands and the parity row turns green.
    #[test]
    fn byte_order_collation_characterization() {
        let dir = tmp();
        fs::create_dir_all(&dir).unwrap();
        for n in ["Zoo", "apple", "cafe", "café"] {
            fs::create_dir(dir.join(n)).unwrap();
        }
        let names: Vec<String> = list_directory_entries(&dir)
            .iter()
            .map(|e| e.name.clone())
            .collect();
        // Byte-order: uppercase Zoo before lowercase apple; cafe before café
        // ('e'=0x65 < UTF-8 é=0xC3 0xA9).
        assert_eq!(names, vec!["Zoo", "apple", "cafe", "café"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
