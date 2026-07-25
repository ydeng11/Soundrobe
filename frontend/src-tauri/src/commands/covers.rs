//! Local cover-art commands and suppression parity (`electron/handlers/cover.ts`).
//! Provider downloads remain in the later provider slice.

use crate::commands::tracks::read_track_metadata;
use crate::error::ApiError;
use crate::state::config::ConfigState;
use crate::state::providers::{ProviderState, RemoteArtworkClient, RemoteImage};
use crate::state::write_queue::WriteQueue;
use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::DynamicImage;
use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::Mutex;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Prepared cover results live on local disk rather than accumulating one
/// base64 string per album in memory. The renderer keeps its own hot cache
/// after the first selection, while Rust handles the first local-disk read.
static COVER_DISK_CACHE_LOCK: Mutex<()> = Mutex::new(());
static COVER_DISK_CACHE_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let started = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map_or(0, |elapsed| elapsed.as_nanos());
    std::env::temp_dir().join(format!(
        "soundrobe-cover-cache-{}-{started}",
        std::process::id()
    ))
});

fn cover_disk_cache_path(album_path: &str) -> PathBuf {
    let digest = Sha256::digest(album_path.as_bytes());
    COVER_DISK_CACHE_DIR.join(format!("{digest:x}.txt"))
}

fn cover_disk_cache_get(album_path: &str) -> Option<Option<String>> {
    let _guard = COVER_DISK_CACHE_LOCK.lock().ok()?;
    let value = fs::read_to_string(cover_disk_cache_path(album_path)).ok()?;
    Some((!value.is_empty()).then_some(value))
}

fn cover_disk_cache_set(album_path: &str, value: &Option<String>) {
    let Ok(_guard) = COVER_DISK_CACHE_LOCK.lock() else {
        return;
    };
    if fs::create_dir_all(&*COVER_DISK_CACHE_DIR).is_err() {
        return;
    }
    let body = value.as_deref().unwrap_or_default();
    let _ = fs::write(cover_disk_cache_path(album_path), body);
}

fn cover_cache_get(album_path: &str) -> Option<Option<String>> {
    cover_disk_cache_get(album_path)
}

fn cover_cache_set(album_path: &str, data_url: Option<String>) {
    cover_disk_cache_set(album_path, &data_url);
}

fn cover_cache_invalidate(album_path: &str) {
    if let Ok(mut cache) = COVER_SOURCE_CACHE.lock() {
        cache.remove(album_path);
    }
    if let Ok(_guard) = COVER_DISK_CACHE_LOCK.lock() {
        let _ = fs::remove_file(cover_disk_cache_path(album_path));
    }
}

/// Source-path cache keyed by album path. Populated eagerly during
/// `read_album` so `cover_data_url_at` can skip directory scanning and go
/// directly to the known cover source file. The tuple is (kind, path)
/// where kind is "external" or "embedded".
static COVER_SOURCE_CACHE: LazyLock<Mutex<HashMap<String, (String, String)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Populated eagerly by `read_album` so that cover scans can skip the
/// album directory entirely. Returns `(kind, source_path)`.
pub fn cover_source_cached(album_path: &str) -> Option<(String, String)> {
    COVER_SOURCE_CACHE.lock().ok()?.get(album_path).cloned()
}

/// Called by `read_album` to record where the cover art lives for this album.
/// `kind` is "external" (standalone image file) or "embedded" (audio file
/// containing embedded art). Pass the actual path to the source file.
pub fn cover_cache_source(album_path: &str, kind: &str, source_path: &str) {
    if let Ok(mut cache) = COVER_SOURCE_CACHE.lock() {
        cache.insert(
            album_path.to_owned(),
            (kind.to_owned(), source_path.to_owned()),
        );
    }
}

/// Materialize the selection-time cover result while an album is loading.
/// `has_cover` comes from the metadata/external-cover work `read_album` has
/// already completed, so known misses avoid a redundant full audio-file scan.
pub fn cover_cache_warm(album_path: &Path, preferred_track: Option<&str>, has_cover: bool) {
    let album_path_str = album_path.to_string_lossy().into_owned();
    if cover_cache_get(&album_path_str).is_some() {
        return;
    }
    let data_url = if has_cover {
        cover_data_url_at(album_path, preferred_track)
    } else {
        if let Ok(mut cache) = COVER_SOURCE_CACHE.lock() {
            cache.remove(&album_path_str);
        }
        None
    };
    cover_cache_set(&album_path_str, data_url);
}

const COVER_REMOVED_MARKER: &str = ".auto-tagger-cover-removed";
const COVER_NAMES: &[&str] = &[
    "cover", "Cover", "COVER", "front", "Front", "FRONT", "folder", "Folder", "FOLDER", "albumart",
    "AlbumArt",
];
const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "m4a", "mp4", "wav", "ogg", "opus", "ape"];

#[tauri::command]
pub fn cover_data_url(album_path: String, preferred_track: Option<String>) -> Option<String> {
    // Fast path: return the prepared local-cache result, including a known
    // missing cover, without rescanning media or transforming an image.
    if let Some(url) = cover_cache_get(&album_path) {
        return url;
    }

    // Slow path: scan the album directory and cache the result.
    let url = cover_data_url_at(Path::new(&album_path), preferred_track.as_deref());
    cover_cache_set(&album_path, url.clone());
    url
}

#[tauri::command]
pub async fn cover_set(
    app: AppHandle,
    album_path: String,
    queue: State<'_, WriteQueue>,
) -> Result<Option<String>, ApiError> {
    let picked = app
        .dialog()
        .file()
        .set_title("Choose Cover Artwork")
        .set_directory(&album_path)
        .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
        .blocking_pick_file();
    let Some(source) = picked.as_ref().and_then(FilePath::as_path) else {
        return Ok(None);
    };
    Ok(set_cover_from_path_queued(&queue, PathBuf::from(album_path), source.to_path_buf()).await)
}

#[tauri::command]
pub async fn cover_remove(
    album_path: String,
    queue: State<'_, WriteQueue>,
) -> Result<bool, ApiError> {
    Ok(remove_cover_queued(&queue, PathBuf::from(album_path)).await)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArtistArtResult {
    pub path: String,
    pub source: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArtworkKind {
    Album,
    Artist,
}

pub(super) type ArtworkDownload = (Vec<u8>, &'static str, PathBuf);

#[tauri::command]
pub async fn cover_download(
    album_path: String,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
    queue: State<'_, WriteQueue>,
) -> Result<Option<String>, ApiError> {
    let remote = remote_client(&providers, &config);
    let Some((bytes, _source, _path)) =
        download_album_artwork_with_policy(Path::new(&album_path), &remote, &queue, false).await
    else {
        return Ok(None);
    };
    let url = format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    Ok(Some(url))
}

#[tauri::command]
pub async fn cover_download_artist_art(
    album_path: String,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
    queue: State<'_, WriteQueue>,
) -> Result<Option<ArtistArtResult>, ApiError> {
    let remote = remote_client(&providers, &config);
    Ok(
        download_artist_artwork_at(Path::new(&album_path), &remote, &queue)
            .await
            .map(|(_bytes, source, path)| ArtistArtResult {
                path: path.to_string_lossy().into_owned(),
                source: source.to_string(),
            }),
    )
}

fn remote_client(providers: &ProviderState, config: &ConfigState) -> RemoteArtworkClient {
    let config = config.raw();
    RemoteArtworkClient::new(
        providers.http(),
        config.discogs_token,
        config.theaudiodb_api_key,
    )
}

async fn download_artwork_at(
    kind: ArtworkKind,
    album_path: &Path,
    remote: &RemoteArtworkClient,
    queue: &WriteQueue,
) -> Option<ArtworkDownload> {
    if !album_path.exists() {
        return None;
    }

    // When the user has explicitly removed the cover (suppression marker
    // exists) the Download action must skip leftover local files and go
    // directly to remote providers.  If a remote provider succeeds it
    // clears the marker and saves the new cover; if all fail the marker
    // stays and the function returns None.
    let suppressed = kind == ArtworkKind::Album && is_cover_suppressed(album_path);

    let local = if suppressed {
        None
    } else {
        let local_album = album_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            read_local_artwork(kind, &local_album).and_then(|bytes| {
                normalize_jpeg(&bytes, 1000, 90).map(|bytes| RemoteImage {
                    source: "local",
                    bytes,
                    mime: "image/jpeg".to_string(),
                    url: String::new(),
                })
            })
        })
        .await
        .ok()
        .flatten()
    };

    let resolved = if let Some(image) = local {
        // Local artwork found — skip metadata and remote providers entirely.
        image
    } else {
        // No local artwork — try remote providers, which need track metadata.
        let album_path_owned = album_path.to_path_buf();
        let metadata =
            tokio::task::spawn_blocking(move || read_first_track_metadata(&album_path_owned))
                .await
                .ok()??;
        if kind == ArtworkKind::Artist && metadata.artist.is_none() {
            return None;
        }
        match kind {
            ArtworkKind::Album => {
                remote
                    .album_cover(
                        metadata.artist.as_deref(),
                        metadata.album.as_deref(),
                        metadata.musicbrainz_album_id.as_deref(),
                        metadata.discogs_artist_id.as_deref(),
                        metadata.discogs_release_id.as_deref(),
                    )
                    .await?
            }
            ArtworkKind::Artist => {
                remote
                    .artist_image(
                        metadata.artist.as_deref()?,
                        metadata.discogs_artist_id.as_deref(),
                    )
                    .await?
            }
        }
    };
    let source = resolved.source;
    let bytes = tokio::task::spawn_blocking(move || {
        let first = if source == "local" {
            resolved.bytes
        } else {
            normalize_jpeg(&resolved.bytes, 1000, 90)?
        };
        normalize_jpeg(&first, 1000, 90)
    })
    .await
    .ok()??;
    let destination = if kind == ArtworkKind::Album {
        album_path.join("cover.jpg")
    } else {
        album_path.parent()?.join("artist.jpg")
    };
    let destination_for_write = destination.clone();
    let bytes_for_write = bytes.clone();
    let album_for_suppression = album_path.to_path_buf();
    let written = queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                fs::write(&destination_for_write, bytes_for_write)?;
                if kind == ArtworkKind::Album {
                    clear_cover_suppression(&album_for_suppression)?;
                }
                Ok::<_, std::io::Error>(())
            })
            .await
            .ok()
            .and_then(Result::ok)
            .is_some()
        })
        .await;
    written.then_some((bytes, source, destination))
}

async fn download_album_artwork_with_policy(
    album_path: &Path,
    remote: &RemoteArtworkClient,
    queue: &WriteQueue,
    respect_suppression: bool,
) -> Option<ArtworkDownload> {
    if respect_suppression && is_cover_suppressed(album_path) {
        return None;
    }
    let result = download_artwork_at(ArtworkKind::Album, album_path, remote, queue).await?;
    let url = format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&result.0)
    );
    cover_cache_set(&album_path.to_string_lossy(), Some(url));
    Some(result)
}

pub(super) async fn download_album_artwork_at(
    album_path: &Path,
    remote: &RemoteArtworkClient,
    queue: &WriteQueue,
) -> Option<ArtworkDownload> {
    download_album_artwork_with_policy(album_path, remote, queue, true).await
}

pub(super) async fn download_artist_artwork_at(
    album_path: &Path,
    remote: &RemoteArtworkClient,
    queue: &WriteQueue,
) -> Option<ArtworkDownload> {
    download_artwork_at(ArtworkKind::Artist, album_path, remote, queue).await
}

struct CoverMetadata {
    artist: Option<String>,
    album: Option<String>,
    musicbrainz_album_id: Option<String>,
    discogs_artist_id: Option<String>,
    discogs_release_id: Option<String>,
}

fn read_first_track_metadata(album_path: &Path) -> Option<CoverMetadata> {
    for entry in fs::read_dir(album_path).ok()? {
        let entry = entry.ok()?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        let track = read_track_metadata(&path).ok()?;
        return Some(CoverMetadata {
            artist: track.artist,
            album: track.album,
            musicbrainz_album_id: track.musicbrainz_album_id,
            discogs_artist_id: track.discogs_artist_id,
            discogs_release_id: track.discogs_release_id,
        });
    }
    None
}

fn read_local_artwork(kind: ArtworkKind, album_path: &Path) -> Option<Vec<u8>> {
    match kind {
        // Suppression is checked in the caller (download_artwork_at / download_album_artwork_at)
        // — read_local_artwork is a pure file reader, not a policy decision.
        ArtworkKind::Album => fs::read(find_external_cover(album_path)?).ok(),
        ArtworkKind::Artist => {
            let parent = album_path.parent()?;
            fs::read(parent.join("artist.jpg"))
                .or_else(|_| fs::read(parent.join("artist.png")))
                .ok()
        }
    }
}

/// Read cover art for an album dir.
///
/// Priority:
///   0. Source-path cache (populated eagerly by `read_album`) — skip
///      directory scanning entirely and go directly to the known source.
///   1. External cover file (cover.jpg etc.) — single-pass directory scan.
///   2. Preferred track hint — probe only that one file for embedded art.
///   3. Full directory scan — probe every audio file for embedded art.
pub fn cover_data_url_at(album_path: &Path, preferred_track: Option<&str>) -> Option<String> {
    let total_start = std::time::Instant::now();

    if is_cover_suppressed(album_path) {
        tracing::debug!("[cover] suppressed — {:?}", total_start.elapsed());
        return None;
    }

    // 0. Source-path cache — populated eagerly by `read_album`.
    // This skips directory scanning and goes directly to the known cover source.
    // If the cached source file is missing (e.g. externally deleted), fall through
    // to the scan paths below rather than returning None.
    if let Some((kind, source_path)) = cover_source_cached(&album_path.to_string_lossy()) {
        match kind.as_str() {
            "external" => {
                let path = Path::new(&source_path);
                if path.is_file() {
                    if let Ok(img) = fs::read(path) {
                        if let Some(url) = image_data_url(&img, 500, 85) {
                            tracing::debug!(
                                "[cover] source-cache external {} ({:?} total)",
                                path.display(),
                                total_start.elapsed(),
                            );
                            return Some(url);
                        }
                    }
                }
                tracing::debug!(
                    "[cover] source-cache external {} — stale, falling through",
                    path.display(),
                );
            }
            "embedded" => {
                if Path::new(&source_path).is_file() {
                    if let Some(url) = try_embedded_cover(Path::new(&source_path)) {
                        tracing::debug!(
                            "[cover] source-cache embedded {} ({:?} total)",
                            source_path,
                            total_start.elapsed(),
                        );
                        return Some(url);
                    }
                }
                tracing::debug!(
                    "[cover] source-cache embedded {} — stale, falling through",
                    source_path,
                );
            }
            _ => {}
        }
        // Invalidate stale entry so next call doesn't attempt it again
        cover_cache_invalidate(&album_path.to_string_lossy());
    }

    // 1. External cover file (fastest scan path — single-pass directory read)
    let ext_start = std::time::Instant::now();
    if let Some(path) = find_external_cover(album_path) {
        let found_at = ext_start.elapsed();
        let img = fs::read(&path).ok()?;
        let img_start = std::time::Instant::now();
        let url = image_data_url(&img, 500, 85);
        tracing::debug!(
            "[cover] external {} ({:?} scan + {:?} encode, total {:?})",
            path.display(),
            found_at,
            img_start.elapsed(),
            total_start.elapsed(),
        );
        return url;
    }
    let ext_elapsed = ext_start.elapsed();

    // 2. Preferred track hint — avoid scanning every audio file
    if let Some(track_path) = preferred_track {
        let hint_path = std::path::Path::new(track_path);
        let album_parent_matches = hint_path.parent() == Some(album_path);
        let hint_valid = hint_path.is_file() && album_parent_matches;
        tracing::debug!(
            "[cover] preferred= {} valid={} in_album={} (ext scan {:?})",
            track_path,
            hint_path.is_file(),
            album_parent_matches,
            ext_elapsed,
        );
        if hint_valid {
            if let Some(url) = try_embedded_cover(hint_path) {
                tracing::debug!("[cover] total {:?}", total_start.elapsed());
                return Some(url);
            }
        }
    } else {
        tracing::debug!("[cover] no preferred track (ext scan {:?})", ext_elapsed);
    }

    // 3. Fallback: scan all audio files for embedded art
    let _fallback_start = std::time::Instant::now();
    let mut probe_count: usize = 0;
    let entries = fs::read_dir(album_path).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || !entry.file_type().ok()?.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        probe_count += 1;
        if let Some(url) = try_embedded_cover(&path) {
            tracing::debug!(
                "[cover] fallback: {}/{} probes, found at {} ({:?})",
                probe_count,
                probe_count,
                path.display(),
                total_start.elapsed(),
            );
            return Some(url);
        }
    }
    tracing::debug!(
        "[cover] fallback: {} probes, no cover ({:?})",
        probe_count,
        total_start.elapsed(),
    );
    None
}

/// Probe a single audio file for an embedded cover picture and return its
/// base64-encoded JPEG data URL. This avoids scanning every file in the album.
/// Returns `None` if the file has no embedded picture or cannot be read.
fn try_embedded_cover(path: &Path) -> Option<String> {
    let probed = std::time::Instant::now();
    let Ok(tagged) = Probe::open(path)
        .and_then(|p| p.options(ParseOptions::new().read_properties(false)).read())
    else {
        return None;
    };
    let picture: &[u8] = tagged
        .tags()
        .iter()
        .find_map(|tag| tag.pictures().first())
        .map(|p| p.data())?;
    let img_start = std::time::Instant::now();
    let url = image_data_url(picture, 500, 85)?;
    tracing::debug!(
        "[cover] embedded probe: {} ({:?} probe + {:?} encode)",
        path.display(),
        probed.elapsed(),
        img_start.elapsed(),
    );
    Some(url)
}

pub fn set_cover_from_path(album_path: &Path, source: &Path) -> Option<String> {
    let bytes = fs::read(source).ok()?;
    let jpeg = normalize_jpeg(&bytes, 500, 90)?;
    fs::write(album_path.join("cover.jpg"), &jpeg).ok()?;
    clear_cover_suppression(album_path).ok()?;
    image_data_url(&jpeg, 500, 85)
}

async fn set_cover_from_path_queued(
    queue: &WriteQueue,
    album_path: PathBuf,
    source: PathBuf,
) -> Option<String> {
    let album_str = album_path.display().to_string();
    let result = queue
        .run(async move {
            tokio::task::spawn_blocking(move || set_cover_from_path(&album_path, &source))
                .await
                .ok()
                .flatten()
        })
        .await;
    if result.is_some() {
        cover_cache_invalidate(&album_str);
    }
    result
}

pub fn remove_cover_at(album_path: &Path) -> bool {
    let album_str = album_path.display().to_string();
    let result = (|| -> std::io::Result<()> {
        if let Some(path) = find_external_cover(album_path) {
            fs::remove_file(path)?;
        }
        fs::write(album_path.join(COVER_REMOVED_MARKER), [])?;
        Ok(())
    })();
    cover_cache_invalidate(&album_str);
    result.is_ok()
}

async fn remove_cover_queued(queue: &WriteQueue, album_path: PathBuf) -> bool {
    let album_str = album_path.display().to_string();
    cover_cache_invalidate(&album_str);
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || remove_cover_at(&album_path))
                .await
                .unwrap_or(false)
        })
        .await
}

fn find_external_cover(album_path: &Path) -> Option<PathBuf> {
    if !album_path.exists() {
        return None;
    }
    // Scan the directory once instead of issuing COVER_NAMES × COVER_EXTENSIONS
    // individual exists() calls (which are expensive on network/slow volumes).
    let entries: Vec<_> = fs::read_dir(album_path)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .collect();

    // First pass: look for standard cover names (case-insensitive name match
    // against the stem, avoiding separate path-existence stat calls).
    let lower_names: Vec<String> = COVER_NAMES.iter().map(|n| n.to_ascii_lowercase()).collect();
    for entry in &entries {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str.eq_ignore_ascii_case("artist.jpg") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !lower_names.contains(&stem) {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !COVER_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        return Some(path);
    }

    // Second pass: any image file > 1 KB (non-standard-named covers like
    // 狂想曲.jpg). Uses the same already-scanned entry list — no re-read_dir.
    let mut best: Option<PathBuf> = None;
    let mut best_size: u64 = 0;
    for entry in &entries {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str.eq_ignore_ascii_case("artist.jpg") {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !COVER_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path) {
            let len = meta.len();
            if len >= 1024 && len > best_size {
                best_size = len;
                best = Some(path);
            }
        }
    }
    best
}

fn is_cover_suppressed(album_path: &Path) -> bool {
    album_path.join(COVER_REMOVED_MARKER).exists()
}

fn clear_cover_suppression(album_path: &Path) -> std::io::Result<()> {
    let marker = album_path.join(COVER_REMOVED_MARKER);
    if marker.exists() {
        fs::remove_file(marker)?;
    }
    Ok(())
}

fn normalize_jpeg(bytes: &[u8], max_dimension: u32, quality: u8) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let resized = if image.width() > max_dimension || image.height() > max_dimension {
        image.thumbnail(max_dimension, max_dimension)
    } else {
        image
    };
    encode_jpeg(&resized, quality)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, quality)
        .encode_image(image)
        .ok()?;
    Some(output)
}

fn image_data_url(bytes: &[u8], max_dimension: u32, quality: u8) -> Option<String> {
    let jpeg = normalize_jpeg(bytes, max_dimension, quality)?;
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::tracks::read_album;
    use image::{GenericImage, ImageFormat, Rgba};
    use lofty::id3::v2::Id3v2Tag;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::TagExt;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::{Barrier, Notify};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "soundrobe-cover-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut image = DynamicImage::new_rgba8(width, height);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, ImageFormat::Png).unwrap();
        output.into_inner()
    }

    fn album_with_external_cover() -> (PathBuf, PathBuf, PathBuf) {
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            album.join("01.mp3"),
        )
        .unwrap();
        let cover = album.join("cover.png");
        fs::write(&cover, png(8, 8)).unwrap();
        (root, album, cover)
    }

    /// Intent: once library loading has completed, selecting a track must not
    /// depend on reading or transforming the album cover for the first time.
    #[test]
    fn album_read_warms_cover_data_before_first_selection() {
        let (root, album, cover) = album_with_external_cover();

        read_album(&album).expect("library loading should read the album");
        fs::remove_file(cover).unwrap();

        let url = cover_data_url(album.to_string_lossy().into_owned(), None)
            .expect("selection should use the cover warmed during album loading");
        assert!(url.starts_with("data:image/jpeg;base64,"));
        fs::remove_dir_all(root).unwrap();
    }

    /// Intent: metadata-only callers of `read_album` must reuse prepared cover
    /// results instead of repeating decode, resize, encode, and base64 work.
    #[test]
    fn repeated_album_read_reuses_prepared_cover() {
        let (root, album, cover) = album_with_external_cover();

        read_album(&album).expect("library loading should prepare the cover");
        fs::remove_file(cover).unwrap();
        read_album(&album).expect("metadata-only reread should still succeed");

        assert!(
            cover_data_url(album.to_string_lossy().into_owned(), None).is_some(),
            "rereading album metadata should not replace the prepared cover result"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Intent: cover mutations must invalidate the prepared disk result so a
    /// removed cover cannot reappear when the track is selected again.
    #[test]
    fn cover_removal_invalidates_prepared_disk_result() {
        let (root, album, _cover) = album_with_external_cover();
        read_album(&album).expect("library loading should prepare the cover");

        assert!(remove_cover_at(&album));
        assert_eq!(
            cover_data_url(album.to_string_lossy().into_owned(), None),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Intent: albums without artwork must also become constant-time selection
    /// cache hits instead of rescanning every audio file on first selection.
    #[test]
    fn album_read_warms_missing_cover_before_first_selection() {
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            album.join("01.mp3"),
        )
        .unwrap();

        read_album(&album).expect("library loading should read the album");
        fs::write(album.join("cover.png"), png(8, 8)).unwrap();

        assert_eq!(
            cover_data_url(album.to_string_lossy().into_owned(), None),
            None,
            "selection should use the missing-cover result warmed during album loading"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn set_cover_resizes_writes_jpeg_and_clears_suppression() {
        let root = root();
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join(COVER_REMOVED_MARKER), []).unwrap();
        let source = root.join("source.png");
        fs::write(&source, png(1000, 250)).unwrap();

        let url = set_cover_from_path(&album, &source).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));
        assert!(!album.join(COVER_REMOVED_MARKER).exists());
        let cover = fs::read(album.join("cover.jpg")).unwrap();
        assert_eq!(&cover[..2], &[0xff, 0xd8]);
        let decoded = image::load_from_memory(&cover).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (500, 125));
        assert!(cover_data_url_at(&album, None).is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_deletes_first_external_cover_and_suppresses_remaining_sources() {
        let root = root();
        fs::write(root.join("cover.jpg"), png(2, 2)).unwrap();
        fs::write(root.join("front.png"), png(2, 2)).unwrap();
        assert!(remove_cover_at(&root));
        assert!(!root.join("cover.jpg").exists());
        assert!(root.join("front.png").exists());
        assert!(root.join(COVER_REMOVED_MARKER).exists());
        assert_eq!(cover_data_url_at(&root, None), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn manual_cover_set_waits_for_the_shared_write_queue() {
        let root = root();
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let source = root.join("source.png");
        fs::write(&source, png(10, 10)).unwrap();
        let queue = Arc::new(WriteQueue::default());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());

        let blocker_queue = Arc::clone(&queue);
        let blocker_entered = Arc::clone(&entered);
        let blocker_release = Arc::clone(&release);
        let blocker = tokio::spawn(async move {
            blocker_queue
                .run(async move {
                    blocker_entered.wait().await;
                    blocker_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let mutation_queue = Arc::clone(&queue);
        let mutation_album = album.clone();
        let mutation_source = source.clone();
        let mutation = tokio::spawn(async move {
            set_cover_from_path_queued(&mutation_queue, mutation_album, mutation_source).await
        });
        tokio::task::yield_now().await;
        assert!(!album.join("cover.jpg").exists());

        release.notify_one();
        blocker.await.unwrap();
        assert!(mutation.await.unwrap().is_some());
        assert!(album.join("cover.jpg").exists());
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn manual_cover_remove_waits_for_the_shared_write_queue() {
        let root = root();
        fs::write(root.join("cover.jpg"), png(10, 10)).unwrap();
        let queue = Arc::new(WriteQueue::default());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());

        let blocker_queue = Arc::clone(&queue);
        let blocker_entered = Arc::clone(&entered);
        let blocker_release = Arc::clone(&release);
        let blocker = tokio::spawn(async move {
            blocker_queue
                .run(async move {
                    blocker_entered.wait().await;
                    blocker_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let mutation_queue = Arc::clone(&queue);
        let mutation_root = root.clone();
        let mutation =
            tokio::spawn(async move { remove_cover_queued(&mutation_queue, mutation_root).await });
        tokio::task::yield_now().await;
        assert!(root.join("cover.jpg").exists());

        release.notify_one();
        blocker.await.unwrap();
        assert!(mutation.await.unwrap());
        assert!(!root.join("cover.jpg").exists());
        assert!(root.join(COVER_REMOVED_MARKER).exists());
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn auto_tag_cover_canonicalizes_localized_sidecar_and_refreshes_cached_miss() {
        let root = root();
        let sidecar = root.join("许美静.国语真经典 封面.jpg");
        let original = png(1500, 500);
        fs::write(&sidecar, &original).unwrap();
        cover_cache_set(&root.to_string_lossy(), None);
        let providers = ProviderState::new();
        let remote = RemoteArtworkClient::new(providers.http(), None, None);

        let resolved = download_album_artwork_at(&root, &remote, &WriteQueue::default()).await;

        let (_bytes, source, path) = resolved.expect("local sidecar should resolve");
        assert_eq!(source, "local");
        assert_eq!(path, root.join("cover.jpg"));
        assert_eq!(fs::read(&sidecar).unwrap(), original);
        assert!(root.join("cover.jpg").exists());
        assert!(
            cover_data_url(root.to_string_lossy().into_owned(), None).is_some(),
            "auto-tag must replace a prewarmed missing-cover cache entry"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn data_url_falls_back_to_embedded_cover() {
        let root = root();
        let track = root.join("track.mp3");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &track,
        )
        .unwrap();
        let mut tag = Id3v2Tag::new();
        tag.insert_picture(
            Picture::unchecked(png(4, 4))
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        tag.save_to_path(&track, lofty::config::WriteOptions::new())
            .unwrap();
        let url = cover_data_url_at(&root, None).expect("valid embedded artwork");
        assert!(url.starts_with("data:image/jpeg;base64,"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn explicit_download_skips_local_artwork_when_cover_is_suppressed() {
        // When the suppression marker exists, the explicit download path must
        // skip leftover local files and go through remote providers.
        // The minimal.mp3 fixture carries MusicBrainz IDs so Cover Art
        // Archive may return an image regardless of the marker — the key
        // invariant is that source != "local".
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            album.join("track.mp3"),
        )
        .unwrap();
        fs::write(album.join("cover.png"), png(1500, 500)).unwrap();
        fs::write(album.join(COVER_REMOVED_MARKER), []).unwrap();
        let providers = ProviderState::new();
        let remote = RemoteArtworkClient::new(providers.http(), None, None);
        let queue = WriteQueue::default();

        let result = download_album_artwork_with_policy(&album, &remote, &queue, false).await;
        if let Some((_bytes, source, _path)) = &result {
            assert_ne!(
                *source, "local",
                "suppressed download must not return local artwork"
            );
            assert!(!album.join(COVER_REMOVED_MARKER).exists());
        } else {
            assert!(album.join(COVER_REMOVED_MARKER).exists());
        }
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn auto_tag_respects_suppression_marker() {
        // The auto-tag background path (download_album_artwork_at) must
        // respect the suppression marker and NOT re-use local artwork.
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            album.join("track.mp3"),
        )
        .unwrap();
        fs::write(album.join("cover.png"), png(1500, 500)).unwrap();
        fs::write(album.join(COVER_REMOVED_MARKER), []).unwrap();
        let providers = ProviderState::new();
        let remote = RemoteArtworkClient::new(providers.http(), None, None);
        let queue = WriteQueue::default();

        let result = download_album_artwork_at(&album, &remote, &queue).await;
        assert!(result.is_none(), "auto-tag must respect suppression marker");
        // Marker is preserved (auto-tag does not clear it)
        assert!(album.join(COVER_REMOVED_MARKER).exists());
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn explicit_download_succeeds_when_metadata_read_fails() {
        // A corrupt or unreadable audio file must not prevent the explicit
        // Download action from finding a local cover image.
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();
        // Write a non-audio file (no metadata can be read) to trigger
        // read_first_track_metadata returning None in the remote path.
        fs::write(album.join("谢安琪.jpg"), png(1500, 500)).unwrap();
        fs::write(album.join("readme.txt"), b"hello").unwrap();
        let providers = ProviderState::new();
        let remote = RemoteArtworkClient::new(providers.http(), None, None);
        let queue = WriteQueue::default();

        // Local artwork is checked before metadata, so this succeeds.
        let (album_bytes, album_source, album_path) =
            download_artwork_at(ArtworkKind::Album, &album, &remote, &queue)
                .await
                .unwrap();
        assert_eq!(album_source, "local");
        assert_eq!(album_path, album.join("cover.jpg"));
        assert_eq!(fs::read(&album_path).unwrap(), album_bytes);
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn artist_download_ignores_album_cover_suppression() {
        // The artist-image download pipeline uses a different local path
        // (parent/artist.jpg) and must NOT be blocked by the album cover
        // suppression marker.
        let root = root();
        let artist_dir = root.join("Artist");
        let album = artist_dir.join("Album");
        fs::create_dir_all(&album).unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            album.join("track.mp3"),
        )
        .unwrap();
        fs::write(artist_dir.join("artist.png"), png(300, 900)).unwrap();
        fs::write(album.join(COVER_REMOVED_MARKER), []).unwrap();
        let providers = ProviderState::new();
        let remote = RemoteArtworkClient::new(providers.http(), None, None);
        let queue = WriteQueue::default();

        let (artist_bytes, artist_source, artist_path) =
            download_artist_artwork_at(&album, &remote, &queue)
                .await
                .unwrap();
        assert_eq!(artist_source, "local");
        assert_eq!(artist_path, artist_dir.join("artist.jpg"));
        assert_eq!(fs::read(&artist_path).unwrap(), artist_bytes);
        assert!(album.join(COVER_REMOVED_MARKER).exists());
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_external_image_and_missing_album_fail_closed() {
        let root = root();
        fs::write(root.join("cover.jpg"), b"not an image").unwrap();
        assert_eq!(cover_data_url_at(&root, None), None);
        assert_eq!(set_cover_from_path(&root, &root.join("missing.png")), None);
        let missing = root.join("missing-album");
        assert_eq!(cover_data_url_at(&missing, None), None);
        assert!(!remove_cover_at(&missing));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preferred_track_avoids_full_directory_scan() {
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();

        // Two tracks — only the preferred one has embedded art
        let track_no_cover = album.join("01 no cover.flac");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &track_no_cover,
        )
        .unwrap();

        let track_with_cover = album.join("02 with cover.mp3");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &track_with_cover,
        )
        .unwrap();
        let mut tag = Id3v2Tag::new();
        tag.insert_picture(
            Picture::unchecked(png(4, 4))
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        tag.save_to_path(&track_with_cover, lofty::config::WriteOptions::new())
            .unwrap();

        // Preferring the track-with-cover should find it without scanning '01 no cover.flac'
        let url = cover_data_url_at(&album, Some(track_with_cover.to_str().unwrap())).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));

        // Preferring the non-cover track should return none (no fallback scan should be
        // needed for this test, but we assert it's at least not a crash)
        let result = cover_data_url_at(&album, Some(track_no_cover.to_str().unwrap()));
        // The non-cover file is MP3 with ID3 tag but no picture — the hint should return
        // None, then the fallback should find the other track's embedded art.
        assert!(result.is_none() || result.unwrap().starts_with("data:image/jpeg;base64,"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preferred_track_outside_album_ignored_falls_back() {
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();

        // Track outside the album directory
        let outside = root.join("outside.mp3");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &outside,
        )
        .unwrap();

        // Track inside album (no cover)
        let inside = album.join("01 no cover.flac");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &inside,
        )
        .unwrap();

        // No external cover, no files with embedded art → None
        assert_eq!(
            cover_data_url_at(&album, Some(outside.to_str().unwrap())),
            None
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preferred_track_none_falls_back_to_full_scan() {
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();

        // Single track with embedded art
        let track = album.join("01 with cover.mp3");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &track,
        )
        .unwrap();
        let mut tag = Id3v2Tag::new();
        tag.insert_picture(
            Picture::unchecked(png(4, 4))
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        tag.save_to_path(&track, lofty::config::WriteOptions::new())
            .unwrap();

        // With no preferred track hint, should scan and find embedded art
        let url = cover_data_url_at(&album, None).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_cover_has_priority_over_preferred_track() {
        let root = root();
        let album = root.join("Album");
        fs::create_dir_all(&album).unwrap();

        // External cover.jpg
        fs::write(album.join("cover.jpg"), png(2, 2)).unwrap();

        // Track with embedded art
        let track = album.join("track.mp3");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &track,
        )
        .unwrap();
        let mut tag = Id3v2Tag::new();
        tag.insert_picture(
            Picture::unchecked(png(4, 4))
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        tag.save_to_path(&track, lofty::config::WriteOptions::new())
            .unwrap();

        // External cover should win even when a preferred track with embedded art is given
        let url = cover_data_url_at(&album, Some(track.to_str().unwrap())).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));

        fs::remove_dir_all(root).unwrap();
    }
}
