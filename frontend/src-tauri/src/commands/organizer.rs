//! Local `files:sort-by-album` workflow.

use crate::commands::library::is_audio_file;
use crate::commands::tracks::read_track_metadata;
use crate::error::ApiError;
use crate::state::write_queue::WriteQueue;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortByAlbumOptions {
    pub copy: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortByAlbumResult {
    pub source_dir: String,
    pub albums: Vec<SortByAlbumEntry>,
    pub total_files: usize,
    pub skipped_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortByAlbumEntry {
    pub album_name: String,
    pub dest_dir: String,
    pub files: Vec<SortByAlbumFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortByAlbumFile {
    pub source_path: String,
    pub dest_path: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[tauri::command]
pub async fn files_sort_by_album(
    source_dir: String,
    options: Option<SortByAlbumOptions>,
    queue: State<'_, WriteQueue>,
) -> Result<SortByAlbumResult, ApiError> {
    let copy = options.and_then(|options| options.copy).unwrap_or(true);
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || sort_by_album(Path::new(&source_dir), copy))
                .await
                .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await
}

pub fn sort_by_album(source_dir: &Path, copy: bool) -> Result<SortByAlbumResult, ApiError> {
    let audio_files = collect_all_audio_files(source_dir)?;
    let mut result = SortByAlbumResult {
        source_dir: source_dir.to_string_lossy().into_owned(),
        albums: Vec::new(),
        total_files: audio_files.len(),
        skipped_files: 0,
    };
    let mut groups: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for path in audio_files {
        let Ok(metadata) = read_track_metadata(&path) else {
            result.skipped_files += 1;
            continue;
        };
        let album = metadata
            .album
            .as_deref()
            .map(str::trim)
            .filter(|album| !album.is_empty())
            .unwrap_or("Unknown Album");
        let album = sanitize_dir_name(album);
        if let Some((_, files)) = groups.iter_mut().find(|(name, _)| *name == album) {
            files.push(path);
        } else {
            groups.push((album, vec![path]));
        }
    }

    for (album_name, files) in groups {
        let dest_dir = source_dir.join(&album_name);
        if !dest_dir.exists() {
            fs::create_dir_all(&dest_dir)?;
        }
        let mut album_files = Vec::new();
        for source_path in files {
            let file_name = source_path.file_name().unwrap_or_default();
            let dest_path = dest_dir.join(file_name);
            let operation = if copy {
                fs::copy(&source_path, &dest_path).map(|_| ())
            } else {
                fs::rename(&source_path, &dest_path)
            };
            match operation {
                Ok(()) => album_files.push(SortByAlbumFile {
                    source_path: source_path.to_string_lossy().into_owned(),
                    dest_path: dest_path.to_string_lossy().into_owned(),
                    success: true,
                    error: None,
                }),
                Err(error) => {
                    result.skipped_files += 1;
                    album_files.push(SortByAlbumFile {
                        source_path: source_path.to_string_lossy().into_owned(),
                        dest_path: dest_path.to_string_lossy().into_owned(),
                        success: false,
                        error: Some(error.to_string()),
                    });
                }
            }
        }
        result.albums.push(SortByAlbumEntry {
            album_name,
            dest_dir: dest_dir.to_string_lossy().into_owned(),
            files: album_files,
        });
    }
    Ok(result)
}

fn collect_all_audio_files(path: &Path) -> Result<Vec<PathBuf>, ApiError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut results = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            results.extend(collect_all_audio_files(&entry.path())?);
        } else if file_type.is_file() && is_audio_file(&entry.path()) {
            results.push(entry.path());
        }
    }
    results.sort();
    Ok(results)
}

fn sanitize_dir_name(name: &str) -> String {
    let mut value = name
        .chars()
        .filter(|character| !((*character as u32) <= 0x1f || *character == '\u{7f}'))
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    value = value.trim().to_string();
    if value.starts_with('.') {
        value = format!("_{}", value.trim_start_matches('.'));
    }
    if value.ends_with('.') {
        value = format!("{}{}", value.trim_end_matches('.'), '_');
    }
    let mut collapsed = String::new();
    let mut in_separator = false;
    for character in value.chars() {
        if character == '_' || character == ' ' {
            if !in_separator {
                collapsed.push(' ');
            }
            in_separator = true;
        } else {
            collapsed.push(character);
            in_separator = false;
        }
    }
    let collapsed = collapsed.trim();
    if collapsed.is_empty() {
        "Unknown Album".to_string()
    } else {
        collapsed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "auto-tagger-organizer-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus")
            .join(name)
    }

    #[test]
    fn sanitizes_directory_names_like_electron() {
        assert_eq!(sanitize_dir_name(" ..Bad:<Name>?... "), "Bad Name");
        assert_eq!(sanitize_dir_name("A___   B"), "A B");
        assert_eq!(sanitize_dir_name("\0\u{1f}\u{7f}"), "Unknown Album");
        assert_eq!(sanitize_dir_name("正常 專輯"), "正常 專輯");
    }

    #[test]
    fn missing_directory_returns_empty_result() {
        let path = root().join("missing");
        let result = sort_by_album(&path, true).unwrap();
        assert_eq!(result.source_dir, path.to_string_lossy());
        assert_eq!(result.total_files, 0);
        assert_eq!(result.skipped_files, 0);
        assert!(result.albums.is_empty());
    }

    #[test]
    fn recursively_groups_and_copies_while_skipping_bad_metadata() {
        let root = root();
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::copy(fixture("minimal.mp3"), nested.join("song.mp3")).unwrap();
        fs::copy(fixture("minimal.wav"), root.join("unknown.wav")).unwrap();
        fs::write(root.join("bad.mp3"), b"bad").unwrap();
        fs::copy(fixture("minimal.mp3"), root.join(".hidden.mp3")).unwrap();

        let result = sort_by_album(&root, true).unwrap();
        assert_eq!(result.total_files, 3);
        assert_eq!(result.skipped_files, 1);
        assert_eq!(result.albums.len(), 2);
        assert!(root.join("Corpus Album/song.mp3").exists());
        assert!(root.join("Unknown Album/unknown.wav").exists());
        assert!(nested.join("song.mp3").exists());
        assert!(root.join("unknown.wav").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn move_option_removes_source_after_success() {
        let root = root();
        let source = root.join("song.mp3");
        fs::copy(fixture("minimal.mp3"), &source).unwrap();
        let result = sort_by_album(&root, false).unwrap();
        assert_eq!(result.total_files, 1);
        assert_eq!(result.skipped_files, 0);
        assert!(!source.exists());
        assert!(root.join("Corpus Album/song.mp3").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
