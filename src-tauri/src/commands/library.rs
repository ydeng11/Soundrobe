//! `library:scan` — parity owner for the `library` behavioral group (pure-fs
//! grouping part).
//!
//! Faithful port of `electron/handlers/library.ts`:
//!   - `SUPPORTED_EXTENSIONS` set + `isAudioFile` (extension match, lowercase);
//!   - `isHiddenDir` (name starts with '.');
//!   - `parseArtistAlbumHint` — "Artist - Album" dashed pattern (skip when the
//!     left side is a 4-digit year, fall back to parent = artist, dir = album),
//!     and the standard parent=artist/dir=album hierarchy;
//!   - `collectAudioFiles` — non-hidden audio files at exactly one level, sorted;
//!   - `scanDirectory` — walk top-level entries, group by album directory with
//!     the artist- vs album-directory strategy, collect loose root audio files,
//!     and the single-file-as-album branch.
//!
//! `album:refresh` is DEFERRED: Electron's handler calls `readAlbum` which
//! reads per-track metadata via the `music-metadata` Node library; that depends
//! on the Rust audio-tag strategy decided separately.

use crate::commands::tracks::{read_album, AlbumDetail};
use crate::error::ApiError;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Extensions Electron treats as audio (`SUPPORTED_EXTENSIONS`).
const AUDIO_EXTENSIONS: &[&str] = &[
    ".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus", ".aiff", ".ape",
];

/// Album summary as the renderer sees it (matches `AlbumInfo`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AlbumInfo {
    pub path: String,
    pub name: String,
    #[serde(rename = "artistHint")]
    pub artist_hint: String,
    #[serde(rename = "albumHint")]
    pub album_hint: String,
    #[serde(rename = "trackCount")]
    pub track_count: usize,
}

/// True if `path`'s extension is a supported audio extension (case-insensitive),
/// matching Electron's `SUPPORTED_EXTENSIONS.has(ext.toLowerCase())`.
pub fn is_audio_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let dot = format!(".{}", ext.to_lowercase());
    AUDIO_EXTENSIONS.contains(&dot.as_str())
}

/// Non-dotfile + audio check (Electron's `isHiddenDir` guard + `isAudioFile`).
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Audio files at one level under `dirPath`, non-hidden, sorted. Empty on read
/// error (Electron's `catch {}`).
pub fn collect_audio_files(dir_path: &Path) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    let Ok(entries) = fs::read_dir(dir_path) else {
        return result;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if is_hidden(&name) {
            continue;
        }
        let full_path = entry.path();
        // `entry.isFile()` parity: skip directories.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            // Electron Dirent.isFile() is false for dirs and symlinks-to-dirs.
            continue;
        }
        if is_audio_file(&full_path) {
            result.push(full_path.to_string_lossy().into_owned());
        }
    }
    result.sort();
    result
}

/// Guess artist/album hints from a directory path + its parent dir name.
/// Matches `parseArtistAlbumHint`:
///   - "Artist - Album" dashed pattern wins unless the left side is a 4-digit
///     year (then fall through to parent = artist, dir = album);
///   - otherwise standard hierarchy (parent = artist, dir = album).
pub fn parse_artist_album_hint(dir_path: &Path, parent_dir: &str) -> (String, String) {
    let dir_name = Path::new(dir_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    if let Some((left, right)) = split_dash(&dir_name) {
        let artist_candidate = left.trim();
        let is_year =
            artist_candidate.len() == 4 && artist_candidate.chars().all(|c| c.is_ascii_digit());
        if !is_year {
            // Plausible "Artist - Album".
            return (artist_candidate.to_string(), right.trim().to_string());
        }
        // "2025 - Album" → fall through to parent/dir.
    }

    (parent_dir.to_string(), dir_name)
}

/// Electron's `/^(.+?)\s*-\s*(.+)$/` — first ` - ` (lazy left, whitespace
/// tolerant). Returns `(left, right)` if a dash separator exists.
fn split_dash(name: &str) -> Option<(String, String)> {
    // Find the first " - " occurrence (whitespace-dash-whitespace). Electron's
    // regex is `\s*-\s*` so surrounding spaces are trimmed by the caller; we
    // match the first `-` and trim each side.
    let dash = name.find('-')?;
    let left = name[..dash].trim();
    let right = name[dash + 1..].trim();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    Some((left.to_string(), right.to_string()))
}

/// Walk top-level entries and group audio files by album directory, mirroring
/// `scanDirectory`. Returns the album list in scan order (Electron returns
/// `Array.from(albums.values())` — insertion order of the JS Map).
pub fn scan_directory(library_path: &Path) -> Vec<AlbumInfo> {
    let mut albums: Vec<AlbumInfo> = Vec::new();

    if !library_path.exists() {
        // Electron throws `Library path not found`; the Tauri command surfaces
        // that as a rejected Error (parity). Here we return empty — the
        // command wraps the error.
        return albums;
    }

    let stat = match fs::metadata(library_path) {
        Ok(s) => s,
        Err(_) => return albums,
    };

    if stat.is_file() {
        // Single file — wrap as one album whose "album dir" is the parent.
        let parent = library_path.parent().unwrap_or_else(|| Path::new(""));
        let grand_parent = parent.parent().unwrap_or_else(|| Path::new(""));
        let grand_name = grand_parent
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (artist_hint, album_hint) = parse_artist_album_hint(parent, &grand_name);
        albums.push(AlbumInfo {
            path: parent.to_string_lossy().into_owned(),
            name: album_hint.clone(),
            artist_hint,
            album_hint,
            track_count: 1,
        });
        return albums;
    }

    let entries = match fs::read_dir(library_path) {
        Ok(e) => e,
        Err(_) => return albums,
    };

    let mut root_audio_files: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let full_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            // Strategy 1: this dir is an album (audio files at 1 level).
            let audio_files = collect_audio_files(&full_path);
            let album_dir = &full_path;
            let parent_name = library_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            if audio_files.is_empty() {
                // Strategy 2: this dir is an artist; subdirs are albums.
                let sub_entries = match fs::read_dir(&full_path) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let entry_name = name.into_owned();
                for sub in sub_entries.flatten() {
                    let sub_name = sub.file_name();
                    let sub_name = sub_name.to_string_lossy();
                    if sub_name.starts_with('.') {
                        continue;
                    }
                    let sub_path = sub.path();
                    let Ok(sub_type) = sub.file_type() else {
                        continue;
                    };
                    if !sub_type.is_dir() {
                        continue;
                    }
                    let sub_audio = collect_audio_files(&sub_path);
                    if !sub_audio.is_empty() {
                        let (artist_hint, album_hint) =
                            parse_artist_album_hint(&sub_path, &entry_name);
                        albums.push(AlbumInfo {
                            path: sub_path.to_string_lossy().into_owned(),
                            name: album_hint.clone(),
                            artist_hint,
                            album_hint,
                            track_count: sub_audio.len(),
                        });
                    }
                }
                continue;
            }

            // Direct album directory.
            let (artist_hint, album_hint) = parse_artist_album_hint(album_dir, &parent_name);
            albums.push(AlbumInfo {
                path: album_dir.to_string_lossy().into_owned(),
                name: album_hint.clone(),
                artist_hint,
                album_hint,
                track_count: audio_files.len(),
            });
        } else if is_audio_file(&full_path) {
            root_audio_files.push(full_path.to_string_lossy().into_owned());
        }
    }

    // Group root-level audio files as a single album entry.
    if !root_audio_files.is_empty() {
        let name = library_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        albums.push(AlbumInfo {
            path: library_path.to_string_lossy().into_owned(),
            name: name.clone(),
            artist_hint: String::new(),
            album_hint: name,
            track_count: root_audio_files.len(),
        });
    }

    albums
}

/// `library:scan` command. Mirrors Electron: throws on a missing library path,
/// otherwise returns the album list. We surface the missing-path error to keep
/// renderer-visible behavior identical (Electron's `scanDirectory` throws and
/// the handler propagates it).
#[tauri::command]
pub fn library_scan(dir_path: String) -> Result<Vec<AlbumInfo>, String> {
    let path = PathBuf::from(&dir_path);
    if !path.exists() {
        return Err(format!("Library path not found: {dir_path}"));
    }
    Ok(scan_directory(&path))
}

/// `album:refresh` / `refreshAlbum()`: Electron delegates directly to
/// `readAlbum`, so use the same read-only implementation and error behavior.
#[tauri::command]
pub fn album_refresh(album_path: String) -> Result<AlbumDetail, ApiError> {
    read_album(Path::new(&album_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "auto-tag-lib-{}-{}",
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_nanos(),
            seq
        ))
    }

    fn touch(p: &Path, bytes: &[u8]) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, bytes).unwrap();
    }

    /// Intent: only supported extensions count as audio so non-media files
    /// (covers, cues, text) don't inflate track counts.
    #[test]
    fn is_audio_file_extension_match() {
        assert!(is_audio_file(Path::new("/lib/album/01.mp3")));
        assert!(is_audio_file(Path::new("/lib/album/02.FLAC")));
        assert!(is_audio_file(Path::new("/lib/x/track.m4a")));
        assert!(!is_audio_file(Path::new("/lib/album/cover.jpg")));
        assert!(!is_audio_file(Path::new("/lib/album/notes.txt")));
        assert!(!is_audio_file(Path::new("/lib/album/noext")));
    }

    /// Intent: "Artist - Album" dashed dirs parse the artist hint from the dash,
    /// so flat libraries still get a meaningful artist hint.
    #[test]
    fn hint_dashed_artist_album() {
        let (a, al) = parse_artist_album_hint(Path::new("/lib/Beatles - Abbey Road"), "lib");
        assert_eq!(a, "Beatles");
        assert_eq!(al, "Abbey Road");
    }

    /// Intent: a 4-digit-year prefix ("2025 - Album") must NOT become artist
    /// "2025"; it falls back to parent = artist, dir = album.
    #[test]
    fn hint_year_prefix_falls_back_to_parent() {
        let (a, al) = parse_artist_album_hint(Path::new("/Artist/2025 - Album"), "Artist");
        assert_eq!(a, "Artist");
        assert_eq!(al, "2025 - Album");
    }

    /// Intent: standard hierarchy (parent = artist, dir = album) when no dash.
    #[test]
    fn hint_standard_hierarchy() {
        let (a, al) = parse_artist_album_hint(Path::new("/Pink Floyd/The Wall"), "Pink Floyd");
        assert_eq!(a, "Pink Floyd");
        assert_eq!(al, "The Wall");
    }

    /// Intent: collectAudioFiles skips dotfiles + non-audio + subdirs, and
    /// returns sorted paths so album track ordering is stable.
    #[test]
    fn collect_audio_files_skips_hidden_and_sorts() {
        let dir = tmp().join("album");
        touch(&dir.join("02.flac"), b"x");
        touch(&dir.join("01.mp3"), b"x");
        touch(&dir.join(".hidden.mp3"), b"x");
        touch(&dir.join("cover.jpg"), b"x");
        fs::create_dir_all(dir.join("subdir")).unwrap();
        let out = collect_audio_files(&dir);
        assert_eq!(out.len(), 2);
        assert!(out[0].ends_with("01.mp3"));
        assert!(out[1].ends_with("02.flac"));
        fs::remove_dir_all(tmp()).ok();
    }

    /// Intent: a direct album directory (audio files at level 1) becomes one
    /// AlbumInfo with the parent dir as artist hint.
    #[test]
    fn scan_direct_album_directory() {
        let lib = tmp();
        let album = lib.join("Artist").join("Album");
        touch(&album.join("01.mp3"), b"x");
        touch(&album.join("02.mp3"), b"x");
        let albums = scan_directory(&lib);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].track_count, 2);
        assert_eq!(albums[0].artist_hint, "Artist");
        assert_eq!(albums[0].album_hint, "Album");
        fs::remove_dir_all(&lib).ok();
    }

    /// Intent: an artist directory whose subdirs hold audio (artist/album
    /// layout) yields one AlbumInfo per album subdir.
    #[test]
    fn scan_artist_directory_with_album_subdirs() {
        let lib = tmp();
        let artist = lib.join("Artist");
        for album in ["Album1", "Album2"] {
            touch(&artist.join(album).join("01.mp3"), b"x");
        }
        let albums = scan_directory(&lib);
        assert_eq!(albums.len(), 2);
        for a in &albums {
            assert_eq!(a.artist_hint, "Artist");
            assert_eq!(a.track_count, 1);
        }
        let names: Vec<_> = albums.iter().map(|a| a.album_hint.clone()).collect();
        assert!(names.contains(&"Album1".to_string()));
        assert!(names.contains(&"Album2".to_string()));
        fs::remove_dir_all(&lib).ok();
    }

    /// Intent: loose audio files at the library root group into a single root
    /// album entry (artist hint empty, album hint = the library dir name).
    #[test]
    fn scan_root_audio_files_group_as_one_album() {
        let lib = tmp();
        touch(&lib.join("track01.mp3"), b"x");
        touch(&lib.join("track02.flac"), b"x");
        let albums = scan_directory(&lib);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].track_count, 2);
        assert_eq!(albums[0].artist_hint, "");
        fs::remove_dir_all(&lib).ok();
    }

    /// Intent: a missing library path surfaces an error (Electron throws), so
    /// the renderer can show "library not found" rather than an empty grid.
    #[test]
    fn library_scan_missing_path_errors() {
        let res = library_scan("/nonexistent/does-not-exist-12345".to_string());
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("not found"));
    }

    /// Intent: a single audio file (not a dir) is wrapped as one album whose
    /// path is its parent dir, matching the single-file library branch.
    #[test]
    fn scan_single_file_wraps_as_album() {
        let lib = tmp();
        let file = lib.join("Artist - Album").join("01.mp3");
        touch(&file, b"x");
        let albums = scan_directory(&file);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].track_count, 1);
        // "Artist - Album" parses the dash (parent grand-name is the lib dir).
        assert_eq!(albums[0].artist_hint, "Artist");
        assert_eq!(albums[0].album_hint, "Album");
        fs::remove_dir_all(&lib).ok();
    }

    /// This is the Rust half of the shared Electron/Tauri `library:scan`
    /// baseline. See `test/handlers/library.test.ts`: Electron runs its actual
    /// `scanDirectory` over the same committed tree and asserts this JSON. Here
    /// we normalize absolute paths to fixture-relative paths and sort them, then
    /// require the same shape/content/order. This catches real cross-runtime
    /// grouping or DTO drift rather than two independent test expectations.
    #[test]
    fn shared_electron_library_scan_fixture_matches() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("test/fixtures/tauri/library-scan")
            .canonicalize()
            .expect("committed Electron/Rust fixture exists");
        let expected: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join("expected.json"))
                .expect("shared expected Electron response exists"),
        )
        .expect("shared expected response is valid JSON");

        let mut actual: Vec<(String, serde_json::Value)> = scan_directory(&root)
            .into_iter()
            .map(|album| {
                let relative = Path::new(&album.path)
                    .strip_prefix(&root)
                    .expect("album path stays under fixture root");
                let path = if relative.as_os_str().is_empty() {
                    ".".to_string()
                } else {
                    relative.to_string_lossy().replace('\\', "/")
                };
                let row = serde_json::json!({
                    "path": path,
                    "name": album.name,
                    "artistHint": album.artist_hint,
                    "albumHint": album.album_hint,
                    "trackCount": album.track_count,
                });
                let sort_key = row["path"].as_str().expect("path is a string").to_string();
                (sort_key, row)
            })
            .collect();
        actual.sort_by(|a, b| a.0.cmp(&b.0));
        let actual = serde_json::Value::Array(actual.into_iter().map(|(_, row)| row).collect());

        assert_eq!(actual, expected);
    }

    /// Intent: album:refresh is not a separate metadata algorithm; Electron
    /// delegates it to readAlbum. Keep that single source of truth so refresh
    /// never drifts from album:read's hints, covers, statuses, or fallbacks.
    #[test]
    fn album_refresh_delegates_to_album_reader() {
        let root = tmp();
        let album = root.join("Artist").join("Album");
        fs::create_dir_all(&album).unwrap();
        let result = album_refresh(album.to_string_lossy().into_owned())
            .expect("readable album refresh should resolve");
        assert_eq!(result.name, "Album");
        assert_eq!(result.artist_hint, "Artist");
        assert_eq!(result.status, "ok");
        assert!(result.tracks.is_empty());
        fs::remove_dir_all(&root).unwrap();
    }
}
