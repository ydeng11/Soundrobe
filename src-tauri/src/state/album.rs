//! Read-only album DTOs and metadata loading shared by headless HTTP handlers.

use crate::state::library::is_audio_file;
use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{ItemKey, Tag};
use serde::Serialize;
use std::{fs, io, path::Path};

const COVER_REMOVED_MARKER: &str = ".auto-tagger-cover-removed";
const MAX_COVER_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_COVER_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
const COVER_NAMES: &[&str] = &[
    "cover", "Cover", "COVER", "front", "Front", "FRONT", "folder", "Folder", "FOLDER",
    "albumart", "AlbumArt",
];

#[derive(Clone, Debug, Serialize)]
pub struct LyricsDocument {
    #[serde(rename = "syncedLyrics")]
    pub synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    pub plain_lyrics: String,
    pub language: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackData {
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    #[serde(rename = "albumArtist")]
    pub album_artist: Option<String>,
    #[serde(rename = "albumArtists")]
    pub album_artists: Vec<String>,
    #[serde(rename = "trackNumber")]
    pub track_number: Option<u32>,
    #[serde(rename = "trackTotal")]
    pub track_total: Option<u32>,
    #[serde(rename = "discNumber")]
    pub disc_number: Option<u32>,
    #[serde(rename = "discTotal")]
    pub disc_total: Option<u32>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub composer: Option<String>,
    pub comment: Option<String>,
    pub description: Option<String>,
    pub lyrics: Option<LyricsDocument>,
    pub compilation: Option<bool>,
    #[serde(rename = "musicbrainzTrackId")]
    pub musicbrainz_track_id: Option<String>,
    #[serde(rename = "musicbrainzAlbumId")]
    pub musicbrainz_album_id: Option<String>,
    #[serde(rename = "musicbrainzArtistId")]
    pub musicbrainz_artist_id: Option<String>,
    #[serde(rename = "discogsArtistId")]
    pub discogs_artist_id: Option<String>,
    #[serde(rename = "discogsReleaseId")]
    pub discogs_release_id: Option<String>,
    #[serde(rename = "hasCover")]
    pub has_cover: bool,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
    pub bitrate: Option<f64>,
    #[serde(rename = "sampleRate")]
    pub sample_rate: Option<u32>,
    pub codec: String,
    pub duration: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CoverInfo {
    pub path: Option<String>,
    pub source: String,
    #[serde(rename = "dataUrl")]
    pub data_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AlbumDetail {
    pub path: String,
    pub name: String,
    #[serde(rename = "artistHint")]
    pub artist_hint: String,
    #[serde(rename = "albumHint")]
    pub album_hint: String,
    pub tracks: Vec<TrackData>,
    #[serde(rename = "coverInfo")]
    pub cover_info: CoverInfo,
    pub status: String,
}

pub fn read_album(album_path: &Path) -> io::Result<AlbumDetail> {
    read_album_with_cancellation(album_path, &|| false)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::Interrupted, "album read was cancelled")
    })
}

pub fn cover_data_url(
    album_path: &Path,
    preferred_track_path: Option<&Path>,
) -> io::Result<Option<String>> {
    if album_path.join(COVER_REMOVED_MARKER).exists() {
        return Ok(None);
    }
    for name in COVER_NAMES {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            let candidate = album_path.join(format!("{name}.{extension}"));
            if !candidate.is_file() {
                continue;
            }
            let metadata = fs::metadata(&candidate)?;
            if metadata.len() > MAX_COVER_BYTES {
                continue;
            }
            let bytes = fs::read(candidate)?;
            if let Some(jpeg) = normalize_cover_image(&bytes) {
                let encoded = base64::engine::general_purpose::STANDARD.encode(jpeg);
                return Ok(Some(format!("data:image/jpeg;base64,{encoded}")));
            }
        }
    }

    if let Some(track_path) = preferred_track_path
        .filter(|path| path.is_file() && path.starts_with(album_path))
    {
        if let Some(data_url) = embedded_cover_data_url(track_path) {
            return Ok(Some(data_url));
        }
    }
    for entry in fs::read_dir(album_path)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.')
            || !entry.file_type()?.is_file()
            || !is_audio_file(&path)
        {
            continue;
        }
        if let Some(data_url) = embedded_cover_data_url(&path) {
            return Ok(Some(data_url));
        }
    }
    Ok(None)
}

fn embedded_cover_data_url(path: &Path) -> Option<String> {
    let tagged = lofty::read_from_path(path).ok()?;
    let picture = tagged
        .tags()
        .iter()
        .find_map(|tag| tag.pictures().first())?;
    let jpeg = normalize_cover_image(picture.data())?;
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

fn normalize_cover_image(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let image = if image.width() > 500 || image.height() > 500 {
        image.thumbnail(500, 500)
    } else {
        image
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&image)
        .ok()?;
    Some(jpeg)
}

pub fn write_cover_upload(album_path: &Path, bytes: &[u8]) -> io::Result<String> {
    if bytes.len() > MAX_COVER_UPLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cover image is too large",
        ));
    }
    let jpeg = normalize_cover_image(bytes).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "invalid cover image")
    })?;
    let temporary = album_path.join(format!(".cover-upload-{}.tmp", uuid::Uuid::new_v4()));
    let destination = album_path.join("cover.jpg");
    fs::write(&temporary, &jpeg)?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let _ = fs::remove_file(album_path.join(COVER_REMOVED_MARKER));
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

pub fn remove_cover(album_path: &Path) -> io::Result<bool> {
    for name in COVER_NAMES {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            let candidate = album_path.join(format!("{name}.{extension}"));
            if candidate.is_file() {
                fs::remove_file(candidate)?;
            }
        }
    }
    fs::write(album_path.join(COVER_REMOVED_MARKER), [])?;
    Ok(true)
}

pub fn read_album_with_cancellation<F>(
    album_path: &Path,
    is_cancelled: &F,
) -> io::Result<Option<AlbumDetail>>
where
    F: Fn() -> bool,
{
    if is_cancelled() {
        return Ok(None);
    }
    let mut audio_files = Vec::new();
    for entry in fs::read_dir(album_path)? {
        if is_cancelled() {
            return Ok(None);
        }
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_file() && is_audio_file(&path) {
            audio_files.push(path);
        }
    }
    audio_files.sort();

    let mut error_count = 0;
    let mut tracks = Vec::with_capacity(audio_files.len());
    for path in audio_files {
        if is_cancelled() {
            return Ok(None);
        }
        let size_bytes = fs::metadata(&path)?.len();
        let title = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match lofty::read_from_path(&path) {
            Ok(tagged) => tracks.push(track_from_lofty(&path, size_bytes, &tagged)),
            Err(_) => {
                error_count += 1;
                tracks.push(unreadable_track(&path, size_bytes, title));
            }
        }
    }

    let name = album_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let artist_hint = album_path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let external_cover = detect_external_cover(album_path);
    let source = if external_cover.is_some() {
        "external"
    } else if tracks.iter().any(|track| track.has_cover) {
        "embedded"
    } else {
        "missing"
    };

    let track_count = tracks.len();
    Ok(Some(AlbumDetail {
        path: album_path.to_string_lossy().into_owned(),
        name: name.clone(),
        artist_hint,
        album_hint: name,
        tracks,
        cover_info: CoverInfo {
            path: external_cover,
            source: source.to_string(),
            data_url: None,
        },
        status: if error_count == 0 {
            "ok".to_string()
        } else if error_count < track_count {
            "warning".to_string()
        } else {
            "error".to_string()
        },
    }))
}

fn unreadable_track(path: &Path, size_bytes: u64, title: String) -> TrackData {
    TrackData {
        path: path.to_string_lossy().into_owned(),
        title: Some(title),
        artist: None,
        artists: Vec::new(),
        album: None,
        album_artist: None,
        album_artists: Vec::new(),
        track_number: None,
        track_total: None,
        disc_number: None,
        disc_total: None,
        year: None,
        genre: None,
        composer: None,
        comment: None,
        description: None,
        lyrics: None,
        compilation: None,
        musicbrainz_track_id: None,
        musicbrainz_album_id: None,
        musicbrainz_artist_id: None,
        discogs_artist_id: None,
        discogs_release_id: None,
        has_cover: false,
        size_bytes,
        bitrate: None,
        sample_rate: None,
        codec: "unknown".to_string(),
        duration: 0.0,
    }
}

fn track_from_lofty(path: &Path, size_bytes: u64, tagged: &lofty::file::TaggedFile) -> TrackData {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let tags = tagged.tags();
    let album_artist = first_string(tags, ItemKey::AlbumArtist);
    TrackData {
        path: path.to_string_lossy().into_owned(),
        title: first_string(tags, ItemKey::TrackTitle),
        artist: first_string(tags, ItemKey::TrackArtist),
        artists: all_strings(tags, ItemKey::TrackArtists),
        album: first_string(tags, ItemKey::AlbumTitle),
        album_artist: album_artist.clone(),
        album_artists: album_artist.into_iter().collect(),
        track_number: first_number(tags, ItemKey::TrackNumber),
        track_total: first_number(tags, ItemKey::TrackTotal),
        disc_number: first_number(tags, ItemKey::DiscNumber),
        disc_total: first_number(tags, ItemKey::DiscTotal),
        year: first_string(tags, ItemKey::RecordingDate)
            .or_else(|| first_string(tags, ItemKey::Year))
            .map(|value| value.chars().take(4).collect()),
        genre: first_string(tags, ItemKey::Genre),
        composer: first_string(tags, ItemKey::Composer),
        comment: first_string(tags, ItemKey::Comment),
        description: first_string(tags, ItemKey::Description),
        lyrics: None,
        compilation: first_string(tags, ItemKey::FlagCompilation).and_then(parse_bool),
        musicbrainz_track_id: first_string(tags, ItemKey::MusicBrainzRecordingId),
        musicbrainz_album_id: first_string(tags, ItemKey::MusicBrainzReleaseId),
        musicbrainz_artist_id: first_string(tags, ItemKey::MusicBrainzArtistId),
        discogs_artist_id: None,
        discogs_release_id: None,
        has_cover: tags.iter().any(|tag| !tag.pictures().is_empty()),
        size_bytes,
        bitrate: tagged
            .properties()
            .overall_bitrate()
            .map(|value| f64::from(value.saturating_mul(1_000))),
        sample_rate: tagged.properties().sample_rate(),
        codec: codec_name(&extension),
        duration: tagged.properties().duration().as_secs_f64(),
    }
}

fn first_string(tags: &[Tag], key: ItemKey) -> Option<String> {
    tags.iter()
        .find_map(|tag| tag.get_string(key).map(ToOwned::to_owned))
}

fn all_strings(tags: &[Tag], key: ItemKey) -> Vec<String> {
    let mut values = Vec::new();
    for tag in tags {
        values.extend(
            tag.get_strings(key)
                .flat_map(|value| value.split(';'))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        );
    }
    if values.is_empty() {
        return first_string(tags, ItemKey::TrackArtist)
            .into_iter()
            .collect();
    }
    values.dedup();
    values
}

fn first_number(tags: &[Tag], key: ItemKey) -> Option<u32> {
    first_string(tags, key).and_then(|value| {
        value
            .split_once('/')
            .map_or(value.as_str(), |(number, _)| number)
            .trim()
            .parse()
            .ok()
    })
}

fn parse_bool(value: String) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn codec_name(extension: &str) -> String {
    match extension {
        "mp3" => "MPEG 1 Layer 3",
        "flac" => "FLAC",
        "wav" | "aiff" => "PCM",
        "m4a" | "mp4" => "MPEG-4/AAC",
        "ogg" => "Vorbis I",
        "opus" => "Opus",
        "ape" => "Monkey's Audio",
        _ => "unknown",
    }
    .to_string()
}

fn detect_external_cover(album_path: &Path) -> Option<String> {
    for name in ["cover", "folder", "front", "albumart"] {
        for extension in ["jpg", "jpeg", "png"] {
            let candidate = album_path.join(format!("{name}.{extension}"));
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_album(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("soundrobe-web-{label}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn cover_preview_rejects_invalid_bytes_and_honors_suppression() {
        let album = temp_album("cover-contract");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join("cover.jpg"), b"not-an-image").unwrap();
        assert!(cover_data_url(&album, None).unwrap().is_none());
        fs::write(album.join(COVER_REMOVED_MARKER), []).unwrap();
        assert!(cover_data_url(&album, None).unwrap().is_none());
        fs::remove_dir_all(album).unwrap();
    }

    #[test]
    fn cancellable_album_read_stops_before_work_begins() {
        let result = read_album_with_cancellation(Path::new("/missing"), &|| true)
            .expect("cancellation should not be an I/O error");
        assert!(result.is_none());
    }

    #[test]
    fn unreadable_track_preserves_file_identity_for_browser_reads() {
        let track = unreadable_track(Path::new("/libraries/music/01.flac"), 7, "01.flac".into());
        assert_eq!(track.title.as_deref(), Some("01.flac"));
        assert_eq!(track.size_bytes, 7);
        assert_eq!(track.codec, "unknown");
    }
}
