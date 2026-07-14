//! `directory:list` — parity owner for the `directories` group (pure-fs part).
//!
//! Faithful port of `electron/handlers/directory.ts::listDirectoryEntries`:
//!   - skip dotfiles (names starting with `.`);
//!   - keep directories only (files filtered out);
//!   - ignore permission errors (skip silently, as Electron's `catch {}`);
//!   - sort by name using locale-aware comparison (Electron uses `localeCompare`).
//!
//! `directory:read` is now backed by the internal Lofty/custom-fallback
//! `TrackData` reader. Its exposed command is read-only: one unparseable audio
//! file becomes a minimal row instead of rejecting the whole directory.

use crate::commands::library::is_audio_file;
use crate::commands::tracks::{read_track_metadata, unreadable_track_data, TrackData};
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
        // `entry.file_type()` does NOT follow symlinks, matching Electron's
        // `Dirent.isDirectory()` (both describe the entry, not its target).
        // A symlink-to-dir is therefore excluded from the tree in both shells.
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

/// `directory:read` response, matching Electron's `{ path, name, subdirs,
/// tracks, audioCount }` object exactly.
#[derive(Debug, Clone, Serialize)]
pub struct DirectoryData {
    pub path: String,
    pub name: String,
    pub subdirs: Vec<DirEntry>,
    pub tracks: Vec<TrackData>,
    #[serde(rename = "audioCount")]
    pub audio_count: usize,
}

/// Read direct audio children plus subdirectories. Read errors are deliberately
/// per-file: Electron catches `readTrackMetadata` failures and returns a minimal
/// `TrackData` whose title is the filename and size is `stat.size`.
pub fn read_directory(dir_path: &Path) -> DirectoryData {
    let subdirs = list_directory_entries(dir_path);
    let mut audio_files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir_path) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let full_path = entry.path();
            if file_type.is_file() && is_audio_file(&full_path) {
                audio_files.push(full_path);
            }
        }
    }
    audio_files.sort();

    let tracks = audio_files
        .into_iter()
        .map(|path| match read_track_metadata(&path) {
            Ok(mut track) => {
                // The internal reader intentionally returns a minimal DTO for
                // truncated FLAC (the corpus contract); Electron's directory
                // boundary would catch that read failure and set basename. Apply
                // the same renderer-facing fallback at this boundary.
                if track.codec == "unknown" && track.title.is_none() {
                    track.title = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                }
                track
            }
            Err(error) => {
                tracing::warn!("failed to read directory track {}: {error}", path.display());
                let size = fs::metadata(&path).map(|stat| stat.len()).unwrap_or(0);
                let title = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                unreadable_track_data(&path, size, title)
            }
        })
        .collect::<Vec<_>>();
    let name = dir_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    DirectoryData {
        path: dir_path.to_string_lossy().into_owned(),
        name,
        subdirs,
        audio_count: tracks.len(),
        tracks,
    }
}

/// `directory:read` command. Like Electron, missing/unreadable directories
/// resolve to empty subdirs/tracks rather than rejecting the folder tree.
#[tauri::command]
pub fn directory_read(dir_path: String) -> DirectoryData {
    read_directory(&PathBuf::from(dir_path))
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

    /// Intent: one unreadable audio-looking file cannot blank a directory;
    /// Electron returns a minimal row using stat.size and the basename so the
    /// renderer can still show/repair it. This is the key `directory:read`
    /// error-containment behavior.
    #[test]
    fn directory_read_keeps_unparseable_audio_with_real_size() {
        let dir = tmp();
        fs::create_dir_all(&dir).unwrap();
        let corrupt = dir.join("corrupt.flac");
        let bytes = vec![0_u8; 12_345];
        fs::write(&corrupt, &bytes).unwrap();

        let result = read_directory(&dir);
        assert_eq!(result.audio_count, 1);
        assert_eq!(result.tracks.len(), 1);
        let track = &result.tracks[0];
        assert_eq!(track.path, corrupt.to_string_lossy());
        assert_eq!(track.title.as_deref(), Some("corrupt.flac"));
        assert_eq!(track.size_bytes, 12_345);
        assert_eq!(track.codec, "unknown");
        assert_eq!(track.duration, 0.0);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Intent: directory:read joins the pure-fs tree with the shared reader:
    /// hidden loose files are excluded, subdirectories remain visible, and a
    /// parseable direct audio child preserves its renderer metadata/count.
    #[test]
    fn directory_read_combines_subdirs_and_parseable_track() {
        let dir = tmp();
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join(".hidden.mp3"), b"not counted").unwrap();
        let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("test/fixtures/tauri/media-corpus/minimal.ogg");
        fs::copy(corpus, dir.join("track.ogg")).unwrap();

        let result = read_directory(&dir);
        assert_eq!(result.name, dir.file_name().unwrap().to_string_lossy());
        assert_eq!(result.subdirs.len(), 1);
        assert_eq!(result.subdirs[0].name, "nested");
        assert_eq!(result.audio_count, 1);
        assert_eq!(result.tracks[0].title.as_deref(), Some("Corpus OGG"));
        assert_eq!(result.tracks[0].codec, "Vorbis I");
        fs::remove_dir_all(&dir).unwrap();
    }
}
