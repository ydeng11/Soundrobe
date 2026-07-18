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
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, FilePath};

const COVER_REMOVED_MARKER: &str = ".auto-tagger-cover-removed";
const COVER_NAMES: &[&str] = &[
    "cover", "Cover", "COVER", "front", "Front", "FRONT", "folder", "Folder", "FOLDER", "albumart",
    "AlbumArt",
];
const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "m4a", "mp4", "wav", "ogg", "opus", "ape"];

#[tauri::command]
pub fn cover_data_url(album_path: String) -> Option<String> {
    cover_data_url_at(Path::new(&album_path))
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

#[tauri::command]
pub async fn cover_download(
    album_path: String,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
    queue: State<'_, WriteQueue>,
) -> Result<Option<String>, ApiError> {
    let remote = remote_client(&providers, &config);
    let Some((bytes, _source, _path)) =
        download_artwork_at(ArtworkKind::Album, Path::new(&album_path), &remote, &queue).await
    else {
        return Ok(None);
    };
    Ok(Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
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
        download_artwork_at(ArtworkKind::Artist, Path::new(&album_path), &remote, &queue)
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
) -> Option<(Vec<u8>, &'static str, PathBuf)> {
    if !album_path.exists() {
        return None;
    }
    let album_path_owned = album_path.to_path_buf();
    let metadata =
        tokio::task::spawn_blocking(move || read_first_track_metadata(&album_path_owned))
            .await
            .ok()??;
    if kind == ArtworkKind::Artist && metadata.artist.is_none() {
        return None;
    }
    let local_album = album_path.to_path_buf();
    let local = tokio::task::spawn_blocking(move || {
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
    .flatten();
    let resolved = match local {
        Some(image) => Some(image),
        None if kind == ArtworkKind::Album => {
            remote
                .album_cover(
                    metadata.artist.as_deref(),
                    metadata.album.as_deref(),
                    metadata.musicbrainz_album_id.as_deref(),
                    metadata.discogs_artist_id.as_deref(),
                    metadata.discogs_release_id.as_deref(),
                )
                .await
        }
        None => {
            remote
                .artist_image(
                    metadata.artist.as_deref()?,
                    metadata.discogs_artist_id.as_deref(),
                )
                .await
        }
    }?;
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

pub async fn download_album_artwork_at(
    album_path: &Path,
    remote: &RemoteArtworkClient,
    queue: &WriteQueue,
) -> Option<PathBuf> {
    if is_cover_suppressed(album_path) {
        return None;
    }
    if let Some(path) = find_external_cover(album_path) {
        return Some(path);
    }
    download_artwork_at(ArtworkKind::Album, album_path, remote, queue)
        .await
        .map(|(_, _, path)| path)
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
        ArtworkKind::Album => fs::read(find_external_cover(album_path)?).ok(),
        ArtworkKind::Artist => {
            let parent = album_path.parent()?;
            fs::read(parent.join("artist.jpg"))
                .or_else(|_| fs::read(parent.join("artist.png")))
                .ok()
        }
    }
}

pub fn cover_data_url_at(album_path: &Path) -> Option<String> {
    if is_cover_suppressed(album_path) {
        return None;
    }
    if let Some(path) = find_external_cover(album_path) {
        return image_data_url(&fs::read(path).ok()?, 500, 85);
    }
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
        let Ok(probe) = Probe::open(&path) else {
            continue;
        };
        let Ok(tagged) = probe
            .options(ParseOptions::new().read_properties(false))
            .read()
        else {
            continue;
        };
        for tag in tagged.tags() {
            if let Some(picture) = tag.pictures().first() {
                if let Some(url) = image_data_url(picture.data(), 500, 85) {
                    return Some(url);
                }
            }
        }
    }
    None
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
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || set_cover_from_path(&album_path, &source))
                .await
                .ok()
                .flatten()
        })
        .await
}

pub fn remove_cover_at(album_path: &Path) -> bool {
    let result = (|| -> std::io::Result<()> {
        if let Some(path) = find_external_cover(album_path) {
            fs::remove_file(path)?;
        }
        fs::write(album_path.join(COVER_REMOVED_MARKER), [])?;
        Ok(())
    })();
    result.is_ok()
}

async fn remove_cover_queued(queue: &WriteQueue, album_path: PathBuf) -> bool {
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
    for name in COVER_NAMES {
        for extension in COVER_EXTENSIONS {
            let candidate = album_path.join(format!("{name}.{extension}"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
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
        assert!(cover_data_url_at(&album).is_some());
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
        assert_eq!(cover_data_url_at(&root), None);
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
    async fn auto_tag_cover_keeps_existing_sidecar_without_provider_lookup() {
        let root = root();
        let sidecar = root.join("folder.png");
        let original = png(2, 2);
        fs::write(&sidecar, &original).unwrap();
        let providers = ProviderState::new();
        let remote = RemoteArtworkClient::new(providers.http(), None, None);

        let resolved = download_album_artwork_at(&root, &remote, &WriteQueue::default()).await;

        assert_eq!(resolved.as_deref(), Some(sidecar.as_path()));
        assert_eq!(fs::read(&sidecar).unwrap(), original);
        assert!(!root.join("cover.jpg").exists());
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
        let url = cover_data_url_at(&root).expect("valid embedded artwork");
        assert!(url.starts_with("data:image/jpeg;base64,"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn downloads_local_album_and_artist_art_with_exact_paths_and_suppression_policy() {
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
        fs::write(album.join("cover.png"), png(1500, 500)).unwrap();
        fs::write(artist_dir.join("artist.png"), png(300, 900)).unwrap();
        fs::write(album.join(COVER_REMOVED_MARKER), []).unwrap();
        let providers = ProviderState::new();
        let remote = RemoteArtworkClient::new(providers.http(), None, None);
        let queue = WriteQueue::default();

        let (album_bytes, album_source, album_path) =
            download_artwork_at(ArtworkKind::Album, &album, &remote, &queue)
                .await
                .unwrap();
        assert_eq!(album_source, "local");
        assert_eq!(album_path, album.join("cover.jpg"));
        assert_eq!(fs::read(&album_path).unwrap(), album_bytes);
        let decoded = image::load_from_memory(&album_bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1000, 333));
        assert!(!album.join(COVER_REMOVED_MARKER).exists());

        fs::write(album.join(COVER_REMOVED_MARKER), []).unwrap();
        let (artist_bytes, artist_source, artist_path) =
            download_artwork_at(ArtworkKind::Artist, &album, &remote, &queue)
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
        assert_eq!(cover_data_url_at(&root), None);
        assert_eq!(set_cover_from_path(&root, &root.join("missing.png")), None);
        let missing = root.join("missing-album");
        assert_eq!(cover_data_url_at(&missing), None);
        assert!(!remove_cover_at(&missing));
        fs::remove_dir_all(root).unwrap();
    }
}
