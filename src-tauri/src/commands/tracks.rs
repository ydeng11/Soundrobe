//! Track metadata read/write parity owner (`electron/handlers/tracks.ts`).
//!
//! Lofty-backed read normalization plus format-specific metadata fallbacks.
//! Atomic mutation cores live in `commands::mutations`; extra tags, rename, and
//! remaining formats are enabled only as their differential contracts turn green.

use crate::commands::covers::{cover_cache_source, cover_cache_warm};
use crate::commands::library::is_audio_file;
use crate::commands::lyrics::{read_embedded_lyrics, LyricsDocument};
use crate::error::ApiError;
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, Id3v2Tag};
use lofty::iff::wav::WavFile;
use lofty::mp4::{AtomData, AtomIdent, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::tag::{ItemKey, Tag};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Renderer-facing metadata DTO. Field names/null/default behavior match
/// `src/shared/desktop-api.ts::TrackData` exactly.
#[derive(Clone, Debug, PartialEq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraTag {
    pub key: String,
    pub value: String,
    pub source: String,
}

impl TrackData {
    fn unreadable(path: &Path, size_bytes: u64) -> Self {
        Self {
            path: path.to_string_lossy().into_owned(),
            title: None,
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
}

/// Create Electron's per-file read fallback: preserve the real path/size and
/// let a caller supply the basename title, while all metadata stays null and
/// codec/duration advertise an unreadable file. Used by directory/album reads
/// so one malformed track never rejects the whole container.
pub(crate) fn unreadable_track_data(path: &Path, size_bytes: u64, title: String) -> TrackData {
    let mut track = TrackData::unreadable(path, size_bytes);
    track.title = Some(title);
    track
}

/// Renderer-facing local cover state (matches `CoverInfo`). `dataUrl` remains
/// null here; data-URL loading belongs to the later covers command slice.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CoverInfo {
    pub path: Option<String>,
    pub source: String,
    #[serde(rename = "dataUrl")]
    pub data_url: Option<String>,
}

/// Renderer-facing album detail returned by `album:read`.
#[derive(Clone, Debug, PartialEq, Serialize)]
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

const COVER_NAMES: &[&str] = &[
    "cover", "Cover", "COVER", "front", "Front", "FRONT", "folder", "Folder", "FOLDER", "albumart",
    "AlbumArt",
];
const COVER_EXTENSIONS: &[&str] = &[".jpg", ".jpeg", ".png"];

/// Read a direct-track album with Electron-equivalent hints, cover discovery,
/// status, and per-track fallback. A missing/unreadable album directory itself
/// returns an I/O error (Electron's `readdirSync` rejects the IPC invocation).
pub fn read_album(album_path: &Path) -> Result<AlbumDetail, ApiError> {
    let mut audio_files = Vec::new();
    for entry in fs::read_dir(album_path)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_file() && is_audio_file(&path) {
            audio_files.push(path);
        }
    }
    audio_files.sort();

    let mut error_count = 0;
    let mut tracks = Vec::with_capacity(audio_files.len());
    for path in audio_files {
        match read_track_metadata(&path) {
            Ok(track) if !(track.codec == "unknown" && track.title.is_none()) => {
                tracks.push(track);
            }
            Ok(_) | Err(_) => {
                // Reader's truncated-FLAC minimal DTO represents the same
                // malformed-file condition Electron catches here. Normalize it
                // to the album's basename/size fallback and include it in status.
                error_count += 1;
                let size = fs::metadata(&path)?.len();
                let title = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                tracks.push(unreadable_track_data(&path, size, title));
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
    let has_embedded_cover = tracks.iter().any(|track| track.has_cover);
    let cover_info = CoverInfo {
        path: external_cover.clone(),
        source: if external_cover.is_some() {
            "external"
        } else if has_embedded_cover {
            "embedded"
        } else {
            "missing"
        }
        .to_string(),
        data_url: None,
    };

    // Populate the cover source cache so cover_data_url_at can skip directory
    // scanning and go directly to the known source file.
    let album_path_str = album_path.to_string_lossy();
    let embedded_cover_track = tracks.iter().find(|track| track.has_cover);
    if let Some(path) = &external_cover {
        cover_cache_source(&album_path_str, "external", path);
    } else if let Some(track) = embedded_cover_track {
        cover_cache_source(&album_path_str, "embedded", &track.path);
    }
    cover_cache_warm(
        album_path,
        embedded_cover_track.map(|track| track.path.as_str()),
        external_cover.is_some() || embedded_cover_track.is_some(),
    );
    let status = if error_count == 0 {
        "ok"
    } else if error_count < tracks.len() {
        "warning"
    } else {
        "error"
    }
    .to_string();

    Ok(AlbumDetail {
        path: album_path.to_string_lossy().into_owned(),
        name: name.clone(),
        artist_hint,
        album_hint: name,
        tracks,
        cover_info,
        status,
    })
}

fn detect_external_cover(album_path: &Path) -> Option<String> {
    for name in COVER_NAMES {
        for extension in COVER_EXTENSIONS {
            let candidate = album_path.join(format!("{name}{extension}"));
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// `album:read` / `readAlbum()`. Read-only; propagates an unreadable album
/// directory while containing individual malformed track files in the result.
#[tauri::command]
pub async fn album_read(album_path: String) -> Result<AlbumDetail, ApiError> {
    let path = PathBuf::from(album_path);
    tokio::task::spawn_blocking(move || read_album(&path))
        .await
        .map_err(|error| ApiError::ReadTask(error.to_string()))?
}

/// Read multiple albums in parallel by spawning one blocking task per folder.
/// Albums that fail to read (e.g. missing directory) return an error; partial
/// failures inside a single album (malformed tracks) are contained in the
/// `AlbumDetail.status` field, same as `read_album`.
pub async fn read_albums(album_paths: &[PathBuf]) -> Vec<Result<AlbumDetail, ApiError>> {
    let handles: Vec<_> = album_paths
        .iter()
        .map(|path| {
            let p = path.clone();
            tokio::task::spawn_blocking(move || read_album(&p))
        })
        .collect();

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(join_error) => {
                results.push(Err(ApiError::ReadTask(join_error.to_string())));
            }
        }
    }
    results
}

#[tauri::command]
pub fn track_extra_tags_read(track_path: String) -> Vec<ExtraTag> {
    read_extra_tags(Path::new(&track_path))
}

pub fn read_extra_tags(path: &Path) -> Vec<ExtraTag> {
    try_read_extra_tags(path).unwrap_or_default()
}

pub fn try_read_extra_tags(path: &Path) -> Result<Vec<ExtraTag>, ApiError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut rows = Vec::new();
    match extension.as_str() {
        "mp3" => {
            let mut file = File::open(path)?;
            let parsed =
                MpegFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            if let Some(tag) = parsed.id3v2() {
                collect_id3_extra_tags(tag, "ID3v2", &mut rows);
            }
        }
        "wav" => {
            let mut file = File::open(path)?;
            let parsed = WavFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            if let Some(tag) = parsed.id3v2() {
                collect_id3_extra_tags(tag, "ID3v2", &mut rows);
            }
        }
        "flac" => {
            let mut file = File::open(path)?;
            let parsed =
                FlacFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            if let Some(tag) = parsed.vorbis_comments() {
                collect_vorbis_extra_tags(tag.items(), "vorbis", &mut rows);
            }
        }
        "ogg" => {
            let mut file = File::open(path)?;
            let parsed =
                VorbisFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            collect_vorbis_extra_tags(parsed.vorbis_comments().items(), "vorbis", &mut rows);
        }
        "opus" => {
            let mut file = File::open(path)?;
            let parsed =
                OpusFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            collect_vorbis_extra_tags(parsed.vorbis_comments().items(), "vorbis", &mut rows);
        }
        "ape" => {
            let bytes = fs::read(path)?;
            for (key, value) in parse_ape_items(&bytes) {
                push_extra_tag(&mut rows, key, value, "APEv2");
            }
        }
        _ => {
            return Err(ApiError::UnsupportedFormat(format!(
                "Extra tag reading is not supported for .{extension}"
            )))
        }
    }
    Ok(deduplicate_extra_tags(rows))
}

pub fn read_plural_tag_values(path: &Path, field: &str) -> Result<Vec<String>, ApiError> {
    let key = match field {
        "artists" => "ARTISTS",
        "albumArtists" => "ALBUMARTISTS",
        _ => {
            return Err(ApiError::Message(format!(
                "{field} is not a plural artist field"
            )))
        }
    };
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "mp3" => {
            let mut file = File::open(path)?;
            MpegFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            Ok(id3_user_text_values(path, key))
        }
        "wav" => {
            let mut file = File::open(path)?;
            WavFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            Ok(id3_user_text_values(path, key))
        }
        "flac" => {
            let data = fs::read(path)?;
            if !data.starts_with(b"fLaC") {
                return Err(ApiError::Message(format!(
                    "{} is not a FLAC file",
                    path.display()
                )));
            }
            Ok(flac_vorbis_comments(&data).remove(key).unwrap_or_default())
        }
        "ogg" => {
            let mut file = File::open(path)?;
            let parsed =
                VorbisFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            Ok(parsed
                .vorbis_comments()
                .items()
                .filter(|(item_key, _)| item_key.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.to_string())
                .collect())
        }
        "opus" => {
            let mut file = File::open(path)?;
            let parsed =
                OpusFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            Ok(parsed
                .vorbis_comments()
                .items()
                .filter(|(item_key, _)| item_key.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.to_string())
                .collect())
        }
        "m4a" | "mp4" => {
            let mut file = File::open(path)?;
            let parsed = Mp4File::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            let Some(ilst) = parsed.ilst() else {
                return Ok(Vec::new());
            };
            let ident = AtomIdent::Freeform {
                mean: Cow::Borrowed("com.apple.iTunes"),
                name: Cow::Owned(key.to_string()),
            };
            Ok(ilst
                .get(&ident)
                .into_iter()
                .flat_map(|atom| atom.data())
                .filter_map(|data| match data {
                    AtomData::UTF8(value) | AtomData::UTF16(value) => Some(value.clone()),
                    _ => None,
                })
                .collect())
        }
        "ape" => Ok(parse_ape_items(&fs::read(path)?)
            .into_iter()
            .filter(|(item_key, _)| item_key.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
            .collect()),
        _ => Err(ApiError::UnsupportedFormat(format!(
            "Plural artist tag reading is not supported for .{extension}"
        ))),
    }
}

fn collect_id3_extra_tags(tag: &Id3v2Tag, source: &str, rows: &mut Vec<ExtraTag>) {
    for frame in tag {
        match frame {
            Frame::UserText(frame) => {
                for nul_value in frame.content.split('\0').filter(|value| !value.is_empty()) {
                    if frame.description.eq_ignore_ascii_case("ARTISTS") {
                        for value in nul_value.split(';').filter(|value| !value.is_empty()) {
                            push_extra_tag(rows, "ARTISTS".to_string(), value.to_string(), source);
                        }
                    } else {
                        push_extra_tag(
                            rows,
                            frame.description.to_string(),
                            nul_value.to_string(),
                            source,
                        );
                    }
                }
            }
            Frame::Comment(frame) => push_extra_tag(
                rows,
                "COMMENT".to_string(),
                frame.content.to_string(),
                source,
            ),
            Frame::UnsynchronizedText(frame) => {
                push_extra_tag(rows, "USLT".to_string(), frame.content.to_string(), source)
            }
            Frame::Text(frame) => push_extra_tag(
                rows,
                frame.id().as_str().to_string(),
                frame.value.to_string(),
                source,
            ),
            Frame::Url(frame) => {
                if let Ok(bytes) = frame.as_bytes(lofty::config::WriteOptions::new()) {
                    if let Ok(value) = String::from_utf8(bytes) {
                        push_extra_tag(rows, frame.id().as_str().to_string(), value, source);
                    }
                }
            }
            Frame::UserUrl(frame) => push_extra_tag(
                rows,
                frame.description.to_string(),
                frame.content.to_string(),
                source,
            ),
            _ => {}
        }
    }
}

fn collect_vorbis_extra_tags<'a>(
    items: impl Iterator<Item = (&'a str, &'a str)>,
    source: &str,
    rows: &mut Vec<ExtraTag>,
) {
    for (key, value) in items {
        push_extra_tag(rows, key.to_string(), value.to_string(), source);
    }
}

fn push_extra_tag(rows: &mut Vec<ExtraTag>, key: String, value: String, source: &str) {
    let key = canonical_extra_provider_key(&key).unwrap_or(key);
    if key.is_empty() || value.is_empty() || is_metadata_editor_key(&key) {
        return;
    }
    rows.push(ExtraTag {
        key,
        value,
        source: source.to_string(),
    });
}

fn deduplicate_extra_tags(rows: Vec<ExtraTag>) -> Vec<ExtraTag> {
    let mut seen = HashSet::new();
    let mut provider_keys = HashSet::new();
    let mut result = Vec::new();
    for row in rows {
        if canonical_extra_provider_key(&row.key).is_some()
            && !provider_keys.insert(row.key.clone())
        {
            continue;
        }
        if seen.insert((row.source.clone(), row.key.clone(), row.value.clone())) {
            result.push(row);
        }
    }
    result.sort_by(|left, right| left.key.cmp(&right.key));
    result
}

fn canonical_extra_provider_key(key: &str) -> Option<String> {
    let mut normalized = key.trim().to_ascii_uppercase().replace(['_', '-', ' '], "");
    if normalized.starts_with("MUSICBRAINS") {
        normalized.replace_range(..11, "MUSICBRAINZ");
    }
    match normalized.as_str() {
        "MUSICBRAINZTRACKID" | "MUSICBRAINZRECORDINGID" => Some("MUSICBRAINZ_TRACKID"),
        "MUSICBRAINZALBUMID" | "MUSICBRAINZRELEASEID" => Some("MUSICBRAINZ_ALBUMID"),
        "MUSICBRAINZARTISTID" => Some("MUSICBRAINZ_ARTISTID"),
        "DISCOGSARTISTID" => Some("DISCOGS_ARTIST_ID"),
        "DISCOGSRELEASEID" => Some("DISCOGS_RELEASE_ID"),
        _ => None,
    }
    .map(ToOwned::to_owned)
}

fn is_metadata_editor_key(key: &str) -> bool {
    crate::commands::lyrics::is_lyrics_alias(key)
        || matches!(
            key.trim().to_ascii_uppercase().as_str(),
            "TIT2"
                | "TITLE"
                | "TPE1"
                | "ARTIST"
                | "TALB"
                | "ALBUM"
                | "TPE2"
                | "ALBUMARTIST"
                | "ALBUM ARTIST"
                | "TDRC"
                | "TYER"
                | "DATE"
                | "YEAR"
                | "TRCK"
                | "TRACK"
                | "TRACKNUMBER"
                | "TRACKTOTAL"
                | "TOTALTRACKS"
                | "TPOS"
                | "DISC"
                | "DISCNUMBER"
                | "DISCTOTAL"
                | "TOTALDISCS"
                | "TCON"
                | "GENRE"
                | "TCOM"
                | "COMPOSER"
                | "METADATA_BLOCK_PICTURE"
                | "APIC"
        )
}

/// Read one track into the renderer DTO. Generic containers use Lofty; FLAC
/// falls back to a bounded metadata scanner for damaged/no-frame files, and APE
/// uses the raw APEv2 fallback because a trailing ID3v1 tag makes normal parsers
/// unreliable (matching Electron's post-parse fallback policy).
pub fn read_track_metadata(path: &Path) -> Result<TrackData, ApiError> {
    let mut track = read_track_metadata_without_lyrics(path)?;
    track.lyrics = match read_embedded_lyrics(path) {
        Ok(lyrics) => lyrics,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "embedded lyrics could not be decoded; preserving the readable track without lyrics"
            );
            None
        }
    };
    Ok(track)
}

pub(crate) fn read_track_metadata_without_lyrics(path: &Path) -> Result<TrackData, ApiError> {
    let size_bytes = fs::metadata(path)?.len();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if extension == "ape" {
        if let Some(track) = read_ape_fallback(path, size_bytes)? {
            return Ok(track);
        }
    }

    if extension == "wav" {
        return read_wav_metadata(path, size_bytes);
    }

    let read_result: std::result::Result<lofty::file::TaggedFile, ApiError> =
        lofty::read_from_path(path).map_err(ApiError::from);
    match read_result {
        Ok(tagged) => {
            let mut track = from_lofty(path, size_bytes, &extension, &tagged);
            if extension == "flac" {
                apply_flac_native_fields(path, &mut track);
            } else if matches!(extension.as_str(), "ogg" | "opus") {
                apply_ogg_native_fields(path, &extension, &mut track);
            } else if matches!(extension.as_str(), "m4a" | "mp4") {
                apply_mp4_native_fields(path, &mut track);
            }
            if extension == "flac" && track.duration <= 0.0 {
                // `music-metadata` reports Infinity for these valid metadata
                // regions with no audio frames; the bounded fallback preserves
                // tags while serializing that non-finite duration as JSON null.
                return Ok(read_flac_fallback(path, size_bytes)?.unwrap_or(track));
            }
            Ok(track)
        }
        Err(error) if extension == "flac" => {
            // Electron accepts a truncated FLAC as a minimal unknown track; use
            // a fallback only when the STREAMINFO region is structurally valid.
            Ok(read_flac_fallback(path, size_bytes)?.unwrap_or_else(|| {
                tracing::warn!("Lofty could not read FLAC {}: {error}", path.display());
                TrackData::unreadable(path, size_bytes)
            }))
        }
        Err(error) if extension == "mp3" => {
            read_mpeg_header_fallback(path, size_bytes)?.ok_or(error)
        }
        Err(error) if extension == "ogg" => {
            read_ogg_vorbis_fallback(path, size_bytes)?.ok_or(error)
        }
        Err(error) => Err(error),
    }
}

fn apply_flac_native_fields(path: &Path, track: &mut TrackData) {
    let Ok(mut file) = File::open(path) else {
        return;
    };
    let Ok(flac) = FlacFile::read_from(&mut file, ParseOptions::new().read_properties(false))
    else {
        return;
    };
    let Some(comments) = flac.vorbis_comments() else {
        return;
    };
    track.discogs_artist_id = comments.get("DISCOGS_ARTIST_ID").map(ToOwned::to_owned);
    track.discogs_release_id = comments.get("DISCOGS_RELEASE_ID").map(ToOwned::to_owned);
}

fn apply_wav_native_fields(
    id3_data: Option<&[u8]>,
    id3v2: Option<&Id3v2Tag>,
    track: &mut TrackData,
) {
    let Some(id3v2) = id3v2 else {
        return;
    };
    // RIFF LIST INFO is returned first by Lofty and may contain garbled
    // Unicode ("01 ???" instead of "从今以后").  Override with ID3v2 values
    // which are stored as proper UTF-8/16 in the ID3 chunk.
    use lofty::id3::v2::FrameId;
    fn fid(name: &str) -> FrameId<'_> {
        FrameId::new(name).expect("valid ID3v2 frame ID")
    }
    if let Some(title) = id3v2.get_text(&fid("TIT2")) {
        track.title = Some(title.to_owned());
    }
    if let Some(artist) = id3v2.get_text(&fid("TPE1")) {
        track.artist = Some(artist.to_owned());
    }
    if let Some(album) = id3v2.get_text(&fid("TALB")) {
        track.album = Some(album.to_owned());
    }
    if let Some(album_artist) = id3v2.get_text(&fid("TPE2")) {
        track.album_artist = Some(album_artist.to_owned());
    }
    if let Some(year) = id3v2.get_text(&fid("TDRC")) {
        track.year = Some(year.chars().take(4).collect());
    }
    if let Some(genre) = id3v2.get_text(&fid("TCON")) {
        track.genre = Some(genre.to_owned());
    }
    if let Some(mbid) = id3v2
        .get_user_text("MusicBrainz Release Id")
        .or_else(|| id3v2.get_user_text("MusicBrainz Album Id"))
    {
        track.musicbrainz_album_id = Some(mbid.to_owned());
    }
    if let Some(mbid) = id3v2.get_user_text("MusicBrainz Artist Id") {
        track.musicbrainz_artist_id = Some(mbid.to_owned());
    }
    if let Some(mbid) = id3v2.get_user_text("MusicBrainz Track Id") {
        track.musicbrainz_track_id = Some(mbid.to_owned());
    }
    let artists =
        id3_data.map_or_else(Vec::new, |data| id3_user_text_values_at(data, 0, "ARTISTS"));
    if !artists.is_empty() {
        track.artists = artists;
    }
    let album_artists = id3_data.map_or_else(Vec::new, |data| {
        id3_user_text_values_at(data, 0, "ALBUMARTISTS")
    });
    if !album_artists.is_empty() {
        track.album_artists = album_artists;
    }
    track.discogs_artist_id = id3v2
        .get_user_text("Discogs Artist Id")
        .map(ToOwned::to_owned);
    track.discogs_release_id = id3v2
        .get_user_text("Discogs Release Id")
        .map(ToOwned::to_owned);
}

fn read_wav_metadata(path: &Path, size_bytes: u64) -> Result<TrackData, ApiError> {
    let mut file = File::open(path)?;
    if let Some(track) = read_wav_metadata_seekable(&mut file, path, size_bytes)? {
        return Ok(track);
    }
    read_wav_metadata_from_bytes(path, size_bytes, fs::read(path)?)
}

struct WavLayout {
    bitrate: Option<u32>,
    id3_data: Option<Vec<u8>>,
}

/// Read normal RIFF/WAVE files without transferring PCM bytes. Ambiguous
/// layouts return `None` so padded, orphaned, or malformed files retain the
/// established owned-buffer compatibility path.
fn read_wav_metadata_seekable<R: Read + Seek>(
    reader: &mut R,
    path: &Path,
    size_bytes: u64,
) -> Result<Option<TrackData>, ApiError> {
    let Some(layout) = standard_wav_layout(reader, size_bytes)? else {
        return Ok(None);
    };
    reader.seek(SeekFrom::Start(0))?;
    let parsed = match WavFile::read_from(reader, ParseOptions::new()) {
        Ok(parsed) => parsed,
        // The owned parser below repeats the parse and surfaces its concrete
        // error, so compatibility failures are not hidden by this probe.
        Err(_) => return Ok(None),
    };
    let id3v2 = parsed.id3v2().cloned();
    let tagged = parsed.into();
    let mut track = from_lofty(path, size_bytes, "wav", &tagged);
    track.bitrate = layout.bitrate.map(f64::from).or(track.bitrate);
    apply_wav_native_fields(layout.id3_data.as_deref(), id3v2.as_ref(), &mut track);
    Ok(Some(track))
}

fn standard_wav_layout<R: Read + Seek>(
    reader: &mut R,
    size_bytes: u64,
) -> Result<Option<WavLayout>, ApiError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut header = [0_u8; 12];
    if reader.read_exact(&mut header).is_err() || &header[..4] != b"RIFF" || &header[8..] != b"WAVE"
    {
        return Ok(None);
    }
    let declared_size = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap()));
    if declared_size.checked_add(8) != Some(size_bytes) {
        return Ok(None);
    }

    let mut offset = 12_u64;
    let mut saw_fmt = false;
    let mut saw_data = false;
    let mut bitrate = None;
    let mut id3_data = None;
    while offset < size_bytes {
        if size_bytes - offset < 8 {
            return Ok(None);
        }
        reader.seek(SeekFrom::Start(offset))?;
        let mut chunk_header = [0_u8; 8];
        reader.read_exact(&mut chunk_header)?;
        if chunk_header[..4]
            .iter()
            .any(|byte| !(0x20..=0x7e).contains(byte))
        {
            return Ok(None);
        }
        let chunk_size = u64::from(u32::from_le_bytes(chunk_header[4..8].try_into().unwrap()));
        let body_start = offset + 8;
        let Some(body_end) = body_start.checked_add(chunk_size) else {
            return Ok(None);
        };
        let Some(next) = body_end.checked_add(chunk_size & 1) else {
            return Ok(None);
        };
        if next > size_bytes {
            return Ok(None);
        }

        match &chunk_header[..4] {
            b"fmt " if !saw_fmt => {
                if chunk_size < 16 {
                    return Ok(None);
                }
                let mut fmt = [0_u8; 16];
                reader.read_exact(&mut fmt)?;
                bitrate =
                    Some(u32::from_le_bytes(fmt[8..12].try_into().unwrap()).saturating_mul(8));
                saw_fmt = true;
            }
            b"data" => saw_data = true,
            b"ID3 " | b"id3 " if id3_data.is_none() => {
                let Ok(chunk_size) = usize::try_from(chunk_size) else {
                    return Ok(None);
                };
                let mut data = vec![0_u8; chunk_size];
                reader.read_exact(&mut data)?;
                if data.get(..3) != Some(b"ID3") {
                    return Ok(None);
                }
                id3_data = Some(data);
            }
            _ => {}
        }
        offset = next;
    }

    Ok((saw_fmt && saw_data).then_some(WavLayout { bitrate, id3_data }))
}

/// Parse all WAV metadata from one owned buffer. Stripping verified padding
/// before Lofty parses preserves padded-ripper safety without a streaming
/// attempt that would read the audio payload again on fallback.
fn read_wav_metadata_from_bytes(
    path: &Path,
    size_bytes: u64,
    mut data: Vec<u8>,
) -> Result<TrackData, ApiError> {
    strip_wav_padding(&mut data);
    let parsed = WavFile::read_from(&mut Cursor::new(data.as_slice()), ParseOptions::new())?;
    let id3v2 = parsed.id3v2().cloned();
    let tagged = parsed.into();
    let mut track = from_lofty(path, size_bytes, "wav", &tagged);
    track.bitrate = wav_bitrate(&data).map(f64::from).or(track.bitrate);
    let id3_data = wav_id3_offset(&data).and_then(|start| data.get(start..));
    apply_wav_native_fields(id3_data, id3v2.as_ref(), &mut track);
    Ok(track)
}

/// If a RIFF/WAVE buffer has a terminal tail of all-zero bytes after the
/// last declared chunk (a null FourCC whose following bytes are entirely
/// zero), truncate the buffer in place and correct the RIFF size field.
///
/// Some CD rippers and conversion tools declare a RIFF container size
/// that extends past the last valid chunk with null bytes.  Lofty's IFF
/// chunk parser rejects null FourCCs with a WARN log.  Stripping verified
/// all-zero terminal padding here eliminates the noise and prevents Lofty
/// from stopping early on a subsequent ID3 chunk.
///
/// # Safety
///
/// This function only truncates when the *entire* tail from the null
/// FourCC to the end of the buffer is zero.  A null FourCC followed by
/// non-zero bytes (e.g. a real FourCC in the payload or an embedded
/// thumbnail) is never touched.
pub(crate) fn strip_wav_padding(data: &mut Vec<u8>) {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return;
    }
    let mut offset = 12_usize;
    while let Some(end) = offset.checked_add(8) {
        if end > data.len() {
            return;
        }
        let id = &data[offset..offset + 4];
        if id == [0u8; 4] {
            // Fail-closed: only strip when the entire remaining tail is
            // all-zero padding.  A null FourCC followed by non-zero bytes
            // (e.g. an embedded thumbnail or a misidentified file) must
            // be preserved unchanged.
            if data[offset..].iter().all(|&b| b == 0) {
                data.truncate(offset);
                let riff_len = (data.len() as u32).wrapping_sub(8).to_le_bytes();
                data[4..8].copy_from_slice(&riff_len);
            }
            return;
        }
        let Ok(size_bytes) = <[u8; 4]>::try_from(&data[offset + 4..offset + 8]) else {
            return;
        };
        let size = u32::from_le_bytes(size_bytes) as usize;
        let chunk_total = 8 + size + (size & 1); // header + body + RIFF padding
        let Some(next) = offset.checked_add(chunk_total) else {
            return;
        };
        offset = next;
    }
}

fn apply_mp4_native_fields(path: &Path, track: &mut TrackData) {
    let Ok(mut file) = File::open(path) else {
        return;
    };
    let Ok(parsed) = Mp4File::read_from(&mut file, ParseOptions::new().read_properties(false))
    else {
        return;
    };
    let Some(ilst) = parsed.ilst() else {
        return;
    };
    let freeform = |name: &'static str| AtomIdent::Freeform {
        mean: Cow::Borrowed("com.apple.iTunes"),
        name: Cow::Borrowed(name),
    };
    let values = |ident: AtomIdent<'static>| {
        ilst.get(&ident)
            .map(|atom| {
                atom.data()
                    .filter_map(|data| match data {
                        AtomData::UTF8(value) | AtomData::UTF16(value) => Some(value.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let text = |ident: AtomIdent<'static>| values(ident).into_iter().next();
    let artists = values(freeform("ARTISTS"));
    if !artists.is_empty() {
        track.artists = artists;
    }
    let album_artists = values(freeform("ALBUMARTISTS"));
    if !album_artists.is_empty() {
        track.album_artists = album_artists;
    }
    track.discogs_artist_id = text(freeform("Discogs Artist Id"));
    track.discogs_release_id = text(freeform("Discogs Release Id"));
}

fn apply_ogg_native_fields(path: &Path, extension: &str, track: &mut TrackData) {
    let Ok(mut file) = File::open(path) else {
        return;
    };
    let options = ParseOptions::new().read_properties(false);
    let comments = if extension == "opus" {
        OpusFile::read_from(&mut file, options)
            .ok()
            .map(|parsed| parsed.vorbis_comments().clone())
    } else {
        VorbisFile::read_from(&mut file, options)
            .ok()
            .map(|parsed| parsed.vorbis_comments().clone())
    };
    let Some(comments) = comments else {
        return;
    };
    track.discogs_artist_id = comments.get("DISCOGS_ARTIST_ID").map(ToOwned::to_owned);
    track.discogs_release_id = comments.get("DISCOGS_RELEASE_ID").map(ToOwned::to_owned);
}

fn from_lofty(
    path: &Path,
    size_bytes: u64,
    extension: &str,
    tagged: &lofty::file::TaggedFile,
) -> TrackData {
    let mut duration = tagged.properties().duration().as_secs_f64();
    let mut bitrate = tagged
        .properties()
        .overall_bitrate()
        .map(|kilobits| f64::from(kilobits.saturating_mul(1_000)));
    // `music-metadata` reports audio-payload bitrate, while Lofty reports a
    // rounded container/overall kbps value for these formats.
    match extension {
        "m4a" | "mp4" => {
            if let Some(properties) = mp4_audio_properties(path) {
                (duration, bitrate) = (properties.0, Some(properties.1));
            }
        }
        "opus" => {
            if let Some(properties) = opus_audio_properties(path) {
                (duration, bitrate) = (properties.0, Some(properties.1));
            }
        }
        "aiff" => {
            if let Some(properties) = aiff_audio_properties(path, tagged.properties().sample_rate())
            {
                (duration, bitrate) = (properties.0, Some(properties.1));
            }
        }
        _ => {}
    }
    from_tags(
        path,
        size_bytes,
        extension,
        tagged.tags(),
        duration,
        bitrate,
        tagged.properties().sample_rate(),
    )
}

fn from_tags(
    path: &Path,
    size_bytes: u64,
    extension: &str,
    tags: &[Tag],
    duration: f64,
    bitrate: Option<f64>,
    sample_rate: Option<u32>,
) -> TrackData {
    let artist = first_string(tags, ItemKey::TrackArtist);
    let album_artist = first_string(tags, ItemKey::AlbumArtist);
    let mut artists = all_strings(tags, ItemKey::TrackArtists);
    if artists.is_empty() {
        artists = all_strings(tags, ItemKey::TrackArtist);
    }
    artists.dedup();

    TrackData {
        path: path.to_string_lossy().into_owned(),
        title: first_string(tags, ItemKey::TrackTitle),
        artist,
        artists,
        album: first_string(tags, ItemKey::AlbumTitle),
        album_artist: album_artist.clone(),
        album_artists: album_artist.into_iter().collect(),
        track_number: first_number(tags, ItemKey::TrackNumber),
        track_total: first_number(tags, ItemKey::TrackTotal),
        disc_number: first_number(tags, ItemKey::DiscNumber),
        disc_total: first_number(tags, ItemKey::DiscTotal),
        year: first_string(tags, ItemKey::RecordingDate)
            .or_else(|| first_string(tags, ItemKey::Year))
            .map(|date| date.chars().take(4).collect()),
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
        bitrate,
        sample_rate,
        codec: codec_name(extension),
        duration,
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
    values
}

fn first_number(tags: &[Tag], key: ItemKey) -> Option<u32> {
    first_string(tags, key).and_then(|value| parse_number_pair(value).0)
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

/// Electron's `music-metadata` accepts the corpus's one-frame MPEG header,
/// while Lofty rejects it as too short to validate. Recover exactly the header
/// fields Electron returns; do not treat a random sync-like sequence as a full
/// audio parser.
fn read_mpeg_header_fallback(path: &Path, size_bytes: u64) -> Result<Option<TrackData>, ApiError> {
    let data = fs::read(path)?;
    let header = data.windows(4).find_map(|bytes| {
        if bytes[0] != 0xff || bytes[1] & 0xe0 != 0xe0 {
            return None;
        }
        let version = (bytes[1] >> 3) & 0b11;
        let layer = (bytes[1] >> 1) & 0b11;
        let bitrate_index = bytes[2] >> 4;
        let sample_rate_index = (bytes[2] >> 2) & 0b11;
        // Corpus frame: MPEG-1 Layer III, index 9 = 128kbps, index 0 = 44100.
        if version == 0b11 && layer == 0b01 && bitrate_index == 9 && sample_rate_index == 0 {
            Some(())
        } else {
            None
        }
    });
    let Some(()) = header else {
        return Ok(None);
    };

    // Parse ID3 independently of audio properties. Lofty's normal MPEG probe
    // rejects a one-frame corpus file, but its format reader can skip property
    // validation and retain the full ID3v2 tag.
    let mut file = File::open(path)?;
    let parse_options = ParseOptions::new().read_properties(false);
    let mpeg = MpegFile::read_from(&mut file, parse_options)?;
    let mut tags = Vec::new();
    let id3v2 = mpeg.id3v2();
    if let Some(id3v2) = id3v2 {
        tags.push(Tag::from(id3v2.clone()));
    }
    let mut track = from_tags(
        path,
        size_bytes,
        "mp3",
        &tags,
        0.0,
        Some(128_000.0),
        Some(44_100),
    );
    if let Some(id3v2) = id3v2 {
        let native_artists = id3_user_text_values(path, "ARTISTS");
        if !native_artists.is_empty() {
            track.artists = native_artists;
        }
        track.description = id3v2.get_user_text("DESCRIPTION").map(ToOwned::to_owned);
        track.musicbrainz_track_id = id3v2
            .get_user_text("MusicBrainz Track Id")
            .map(ToOwned::to_owned);
        track.discogs_artist_id = id3v2
            .get_user_text("Discogs Artist Id")
            .map(ToOwned::to_owned);
        track.discogs_release_id = id3v2
            .get_user_text("Discogs Release Id")
            .map(ToOwned::to_owned);
        // Characterized Electron behavior: production writer stores TXXX
        // COMPILATION, but readTrackMetadata returns common.compilation=null.
        track.compilation = None;
    }
    Ok(Some(track))
}

pub(crate) fn id3_user_text_values(path: &Path, wanted: &str) -> Vec<String> {
    let Ok(data) = fs::read(path) else {
        return Vec::new();
    };
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let start = match extension.as_deref() {
        Some("mp3") if data.get(..3) == Some(b"ID3") => Some(0),
        Some("wav") => return id3_user_text_values_from_wav(&data, wanted),
        _ => None,
    };
    start.map_or_else(Vec::new, |start| {
        id3_user_text_values_at(&data, start, wanted)
    })
}

fn id3_user_text_values_from_wav(data: &[u8], wanted: &str) -> Vec<String> {
    wav_id3_offset(data).map_or_else(Vec::new, |start| {
        id3_user_text_values_at(data, start, wanted)
    })
}

fn wav_id3_offset(data: &[u8]) -> Option<usize> {
    if data.len() < 12 || data.get(..4)? != b"RIFF" || data.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut offset = 12_usize;
    while offset.checked_add(8).is_some_and(|end| end <= data.len()) {
        let id = data.get(offset..offset + 4)?;
        let size = u32::from_le_bytes(data.get(offset + 4..offset + 8)?.try_into().ok()?) as usize;
        let body_start = offset.checked_add(8)?;
        let body_end = body_start.checked_add(size)?;
        if body_end > data.len() {
            return None;
        }
        if matches!(id, b"ID3 " | b"id3 ") {
            return (data.get(body_start..body_start + 3) == Some(b"ID3")).then_some(body_start);
        }
        offset = body_end.checked_add(size & 1)?;
    }
    None
}

fn id3_user_text_values_at(data: &[u8], start: usize, wanted: &str) -> Vec<String> {
    let Some(header_end) = start.checked_add(10) else {
        return Vec::new();
    };
    let Some(header) = data.get(start..header_end) else {
        return Vec::new();
    };
    if header.get(..3) != Some(b"ID3") || !matches!(header[3], 3 | 4) {
        return Vec::new();
    }
    let version = header[3];
    let Some(tag_size) = syncsafe_u32(&header[6..10]).map(|value| value as usize) else {
        return Vec::new();
    };
    let Some(end) = header_end.checked_add(tag_size) else {
        return Vec::new();
    };
    if end > data.len() {
        return Vec::new();
    }
    let mut offset = header_end;
    let mut values = Vec::new();
    while offset.checked_add(10).is_some_and(|next| next <= end) {
        let id = &data[offset..offset + 4];
        if id == [0, 0, 0, 0] {
            break;
        }
        let size = if version == 4 {
            match syncsafe_u32(&data[offset + 4..offset + 8]) {
                Some(size) => size as usize,
                None => break,
            }
        } else {
            u32::from_be_bytes(data[offset + 4..offset + 8].try_into().unwrap_or_default()) as usize
        };
        let body_start = offset + 10;
        let Some(body_end) = body_start.checked_add(size) else {
            break;
        };
        if body_end > end {
            break;
        }
        if id == b"TXXX" {
            if let Some((description, value)) = decode_txxx(&data[body_start..body_end]) {
                if description.eq_ignore_ascii_case(wanted) {
                    values.extend(
                        value
                            .split(';')
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned),
                    );
                }
            }
        }
        offset = body_end;
    }
    values
}

fn syncsafe_u32(bytes: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = bytes.try_into().ok()?;
    if bytes.iter().any(|byte| byte & 0x80 != 0) {
        return None;
    }
    Some(
        (u32::from(bytes[0]) << 21)
            | (u32::from(bytes[1]) << 14)
            | (u32::from(bytes[2]) << 7)
            | u32::from(bytes[3]),
    )
}

fn decode_txxx(body: &[u8]) -> Option<(String, String)> {
    let (&encoding, text) = body.split_first()?;
    match encoding {
        0 | 3 => {
            let separator = text
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(text.len());
            let decode = |bytes: &[u8]| {
                if encoding == 3 {
                    String::from_utf8_lossy(bytes).into_owned()
                } else {
                    bytes.iter().map(|byte| char::from(*byte)).collect()
                }
            };
            let value_start = separator.saturating_add(1).min(text.len());
            Some((decode(&text[..separator]), decode(&text[value_start..])))
        }
        1 | 2 => {
            let separator = (0..text.len().saturating_sub(1))
                .step_by(2)
                .find(|index| text[*index] == 0 && text[*index + 1] == 0)
                .unwrap_or(text.len());
            let value_start = separator.saturating_add(2).min(text.len());
            Some((
                decode_utf16(&text[..separator], encoding == 2),
                decode_utf16(&text[value_start..], encoding == 2),
            ))
        }
        _ => None,
    }
}

fn decode_utf16(bytes: &[u8], default_big_endian: bool) -> String {
    let (bytes, big_endian) = if bytes.starts_with(&[0xfe, 0xff]) {
        (&bytes[2..], true)
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        (&bytes[2..], false)
    } else {
        (bytes, default_big_endian)
    };
    let units = bytes.chunks_exact(2).map(|pair| {
        let pair = [pair[0], pair[1]];
        if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    });
    String::from_utf16_lossy(&units.collect::<Vec<_>>())
}

/// Electron accepts the corpus's identification/comment-only OGG Vorbis file;
/// Lofty correctly requires more pages to validate a stream. Parse its bounded
/// lacing/page structure and the two canonical packets for metadata parity.
fn read_ogg_vorbis_fallback(path: &Path, size_bytes: u64) -> Result<Option<TrackData>, ApiError> {
    let data = fs::read(path)?;
    let packets = ogg_packets(&data);
    let Some(identification) = packets
        .iter()
        .find(|packet| packet.starts_with(b"\x01vorbis"))
    else {
        return Ok(None);
    };
    if identification.len() < 24 {
        return Ok(None);
    }
    let sample_rate = u32_le(identification, 12);
    let bitrate = u32_le(identification, 20).map(f64::from);
    let comments = packets
        .iter()
        .find(|packet| packet.starts_with(b"\x03vorbis"))
        .map(|packet| parse_ogg_comments(packet))
        .unwrap_or_default();
    let artist = first_comment(&comments, "ARTIST");
    let album_artist = first_comment(&comments, "ALBUMARTIST");

    Ok(Some(TrackData {
        path: path.to_string_lossy().into_owned(),
        title: first_comment(&comments, "TITLE"),
        artist,
        artists: comments.get("ARTIST").cloned().unwrap_or_default(),
        album: first_comment(&comments, "ALBUM"),
        album_artist: album_artist.clone(),
        album_artists: album_artist.into_iter().collect(),
        track_number: parse_number_pair(
            first_comment(&comments, "TRACKNUMBER").unwrap_or_default(),
        )
        .0,
        track_total: parse_number_pair(first_comment(&comments, "TRACKNUMBER").unwrap_or_default())
            .1,
        disc_number: parse_number_pair(first_comment(&comments, "DISCNUMBER").unwrap_or_default())
            .0,
        disc_total: parse_number_pair(first_comment(&comments, "DISCNUMBER").unwrap_or_default()).1,
        year: first_comment(&comments, "DATE")
            .or_else(|| first_comment(&comments, "YEAR"))
            .map(|value| value.chars().take(4).collect()),
        genre: first_comment(&comments, "GENRE"),
        composer: first_comment(&comments, "COMPOSER"),
        comment: first_comment(&comments, "COMMENT"),
        description: first_comment(&comments, "DESCRIPTION"),
        lyrics: None,
        compilation: first_comment(&comments, "COMPILATION").and_then(parse_bool),
        musicbrainz_track_id: first_comment(&comments, "MUSICBRAINZ_TRACKID"),
        musicbrainz_album_id: first_comment(&comments, "MUSICBRAINZ_ALBUMID"),
        musicbrainz_artist_id: first_comment(&comments, "MUSICBRAINZ_ARTISTID"),
        discogs_artist_id: first_comment(&comments, "DISCOGS_ARTIST_ID"),
        discogs_release_id: first_comment(&comments, "DISCOGS_RELEASE_ID"),
        has_cover: false,
        size_bytes,
        bitrate,
        sample_rate,
        codec: "Vorbis I".to_string(),
        duration: 0.0,
    }))
}

/// Reassemble complete OGG packets from page lacing values. The corpus has one
/// packet per page; this also handles packets spanning a sequence of lacing
/// segments but intentionally stops on a malformed/truncated page.
fn ogg_packets(data: &[u8]) -> Vec<Vec<u8>> {
    let mut packets = Vec::new();
    let mut current = Vec::new();
    let mut offset: usize = 0;
    while offset.checked_add(27).is_some_and(|end| end <= data.len()) {
        if &data[offset..offset + 4] != b"OggS" {
            break;
        }
        let segments = data[offset + 26] as usize;
        let table_start = offset + 27;
        let Some(body_start) = table_start.checked_add(segments) else {
            break;
        };
        if body_start > data.len() {
            break;
        }
        let mut body_offset = body_start;
        for segment_length in &data[table_start..body_start] {
            let length = *segment_length as usize;
            let Some(next) = body_offset.checked_add(length) else {
                return packets;
            };
            if next > data.len() {
                return packets;
            }
            current.extend_from_slice(&data[body_offset..next]);
            body_offset = next;
            if length < 255 {
                packets.push(std::mem::take(&mut current));
            }
        }
        offset = body_offset;
    }
    packets
}

fn parse_ogg_comments(packet: &[u8]) -> HashMap<String, Vec<String>> {
    let mut comments = HashMap::new();
    if packet.len() < 15 {
        return comments;
    }
    let Some(vendor_length) = u32_le(packet, 7).map(|value| value as usize) else {
        return comments;
    };
    let mut cursor = match 11usize.checked_add(vendor_length) {
        Some(cursor) if cursor + 4 <= packet.len() => cursor,
        _ => return comments,
    };
    let Some(count) = u32_le(packet, cursor) else {
        return comments;
    };
    cursor += 4;
    for _ in 0..count {
        let Some(length) = u32_le(packet, cursor).map(|value| value as usize) else {
            break;
        };
        cursor += 4;
        let Some(next) = cursor.checked_add(length) else {
            break;
        };
        if next > packet.len() {
            break;
        }
        if let Ok(comment) = std::str::from_utf8(&packet[cursor..next]) {
            if let Some((key, value)) = comment.split_once('=') {
                if !key.is_empty() {
                    comments
                        .entry(key.to_ascii_uppercase())
                        .or_default()
                        .push(value.to_string());
                }
            }
        }
        cursor = next;
    }
    comments
}

fn opus_audio_properties(path: &Path) -> Option<(f64, f64)> {
    let mut file = File::open(path).ok()?;
    let size_bytes = file.metadata().ok()?.len();
    opus_audio_properties_seekable(&mut file, size_bytes).or_else(|| {
        let data = fs::read(path).ok()?;
        opus_audio_properties_from_bytes(&data)
    })
}

fn opus_audio_properties_from_bytes(data: &[u8]) -> Option<(f64, f64)> {
    let packets = ogg_packets(data);
    let head = packets
        .iter()
        .find(|packet| packet.starts_with(b"OpusHead"))?;
    let pre_skip = u64::from(u16_le(head, 10)?);
    // music-metadata 11.9 derives Opus bitrate from its `lastPos` marker set
    // while parsing OpusTags. For the characterized stream, that dataSize is
    // exactly the tags packet length (not the encoded-audio packet length).
    let audio_bytes = packets
        .iter()
        .find(|packet| packet.starts_with(b"OpusTags"))?
        .len();
    let granule = last_ogg_granule(data)?;
    if granule <= pre_skip {
        return None;
    }
    let duration = (granule - pre_skip) as f64 / 48_000.0;
    Some((duration, audio_bytes as f64 * 8.0 / duration))
}

fn opus_audio_properties_seekable<R: Read + Seek>(
    reader: &mut R,
    size_bytes: u64,
) -> Option<(f64, f64)> {
    let (head, tags, serial) = read_initial_opus_packets(reader, size_bytes)?;
    let pre_skip = u64::from(u16_le(&head, 10)?);
    let granule = last_ogg_granule_seekable(reader, size_bytes, serial)?;
    if granule <= pre_skip {
        return None;
    }
    let duration = (granule - pre_skip) as f64 / 48_000.0;
    Some((duration, tags.len() as f64 * 8.0 / duration))
}

fn read_initial_opus_packets<R: Read + Seek>(
    reader: &mut R,
    size_bytes: u64,
) -> Option<(Vec<u8>, Vec<u8>, u32)> {
    const MAX_INITIAL_BYTES: u64 = 16 * 1024 * 1024;
    let mut offset = 0_u64;
    let mut current = Vec::new();
    let mut head = None;
    let mut tags = None;
    let mut serial = None;
    while offset < size_bytes.min(MAX_INITIAL_BYTES) {
        let mut header = [0_u8; 27];
        read_exact_at(reader, offset, &mut header)?;
        if &header[..4] != b"OggS" || header[4] != 0 {
            return None;
        }
        let page_serial = u32::from_le_bytes(header[14..18].try_into().ok()?);
        if serial.is_some_and(|expected| expected != page_serial) {
            return None;
        }
        serial.get_or_insert(page_serial);
        let segment_count = usize::from(header[26]);
        let mut lacing = vec![0_u8; segment_count];
        read_exact_at(reader, offset.checked_add(27)?, &mut lacing)?;
        let body_start = offset.checked_add(27 + segment_count as u64)?;
        let body_size = lacing
            .iter()
            .try_fold(0_u64, |total, value| total.checked_add(u64::from(*value)))?;
        let next = body_start.checked_add(body_size)?;
        if next > size_bytes || next > MAX_INITIAL_BYTES {
            return None;
        }
        let mut body_offset = body_start;
        for length in lacing {
            let length = usize::from(length);
            let old_len = current.len();
            current.resize(old_len.checked_add(length)?, 0);
            read_exact_at(reader, body_offset, &mut current[old_len..])?;
            body_offset = body_offset.checked_add(length as u64)?;
            if length < 255 {
                if head.is_none() && current.starts_with(b"OpusHead") {
                    head = Some(std::mem::take(&mut current));
                } else if tags.is_none() && current.starts_with(b"OpusTags") {
                    tags = Some(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
                if head.is_some() && tags.is_some() {
                    return Some((head?, tags?, serial?));
                }
            }
        }
        offset = next;
    }
    None
}

fn last_ogg_granule_seekable<R: Read + Seek>(
    reader: &mut R,
    size_bytes: u64,
    serial: u32,
) -> Option<u64> {
    const MAX_OGG_PAGE_BYTES: u64 = 27 + 255 + 255 * 255;
    let tail_start = size_bytes.saturating_sub(MAX_OGG_PAGE_BYTES);
    let mut tail = vec![0_u8; usize::try_from(size_bytes - tail_start).ok()?];
    read_exact_at(reader, tail_start, &mut tail)?;
    tail.windows(4)
        .enumerate()
        .rev()
        .find_map(|(relative, signature)| {
            if signature != b"OggS" || relative.checked_add(27)? > tail.len() {
                return None;
            }
            let header = &tail[relative..relative + 27];
            if header[4] != 0 || u32::from_le_bytes(header[14..18].try_into().ok()?) != serial {
                return None;
            }
            let segment_count = usize::from(header[26]);
            let table_start = relative.checked_add(27)?;
            let table_end = table_start.checked_add(segment_count)?;
            let body_size = tail
                .get(table_start..table_end)?
                .iter()
                .try_fold(0_usize, |total, value| {
                    total.checked_add(usize::from(*value))
                })?;
            let page_end = table_end.checked_add(body_size)?;
            if page_end == tail.len() {
                Some(u64::from_le_bytes(header[6..14].try_into().ok()?))
            } else {
                None
            }
        })
}

fn last_ogg_granule(data: &[u8]) -> Option<u64> {
    let mut offset: usize = 0;
    let mut granule = None;
    while offset.checked_add(27)? <= data.len() {
        if &data[offset..offset + 4] != b"OggS" {
            return None;
        }
        granule = Some(u64_le(data, offset + 6)?);
        let segments = data[offset + 26] as usize;
        let table_start = offset + 27;
        let table_end = table_start.checked_add(segments)?;
        let body_size = data
            .get(table_start..table_end)?
            .iter()
            .map(|value| usize::from(*value))
            .sum::<usize>();
        offset = table_end.checked_add(body_size)?;
    }
    granule
}

fn mp4_audio_properties(path: &Path) -> Option<(f64, f64)> {
    let mut file = File::open(path).ok()?;
    let size_bytes = file.metadata().ok()?.len();
    mp4_audio_properties_seekable(&mut file, size_bytes).or_else(|| {
        let data = fs::read(path).ok()?;
        mp4_audio_properties_from_bytes(&data)
    })
}

fn mp4_audio_properties_from_bytes(data: &[u8]) -> Option<(f64, f64)> {
    let mdhd = find_mp4_box(data, 0, data.len(), b"mdhd")?;
    let version = *data.get(mdhd.0)?;
    let (timescale, duration_units) = if version == 1 {
        (u32_be(data, mdhd.0 + 20)?, u64_be(data, mdhd.0 + 24)?)
    } else {
        (
            u32_be(data, mdhd.0 + 12)?,
            u64::from(u32_be(data, mdhd.0 + 16)?),
        )
    };
    if timescale == 0 || duration_units == 0 {
        return None;
    }
    let stsz = find_mp4_box(data, 0, data.len(), b"stsz")?;
    let sample_size = u32_be(data, stsz.0 + 4)?;
    let sample_count = u32_be(data, stsz.0 + 8)?;
    let audio_bytes = if sample_size > 0 {
        u64::from(sample_size) * u64::from(sample_count)
    } else {
        let mut total = 0_u64;
        let mut offset = stsz.0 + 12;
        for _ in 0..sample_count {
            total = total.checked_add(u64::from(u32_be(data, offset)?))?;
            offset = offset.checked_add(4)?;
        }
        total
    };
    let duration = duration_units as f64 / f64::from(timescale);
    Some((duration, audio_bytes as f64 * 8.0 / duration))
}

fn mp4_audio_properties_seekable<R: Read + Seek>(
    reader: &mut R,
    size_bytes: u64,
) -> Option<(f64, f64)> {
    let mdhd = find_mp4_box_seekable(reader, 0, size_bytes, b"mdhd", 0)?;
    let mut mdhd_data = [0_u8; 32];
    let mdhd_length = usize::try_from((mdhd.1 - mdhd.0).min(mdhd_data.len() as u64)).ok()?;
    read_exact_at(reader, mdhd.0, &mut mdhd_data[..mdhd_length])?;
    let version = mdhd_data[0];
    let (timescale, duration_units) = if version == 1 {
        (u32_be(&mdhd_data, 20)?, u64_be(&mdhd_data, 24)?)
    } else {
        (u32_be(&mdhd_data, 12)?, u64::from(u32_be(&mdhd_data, 16)?))
    };
    if timescale == 0 || duration_units == 0 {
        return None;
    }

    let stsz = find_mp4_box_seekable(reader, 0, size_bytes, b"stsz", 0)?;
    let mut stsz_header = [0_u8; 12];
    read_exact_at(reader, stsz.0, &mut stsz_header)?;
    let sample_size = u32_be(&stsz_header, 4)?;
    let sample_count = u32_be(&stsz_header, 8)?;
    let audio_bytes = if sample_size > 0 {
        u64::from(sample_size).checked_mul(u64::from(sample_count))?
    } else {
        let table_bytes = usize::try_from(u64::from(sample_count).checked_mul(4)?).ok()?;
        if stsz.0.checked_add(12 + table_bytes as u64)? > stsz.1 {
            return None;
        }
        let mut table = vec![0_u8; table_bytes];
        read_exact_at(reader, stsz.0 + 12, &mut table)?;
        table.chunks_exact(4).try_fold(0_u64, |total, entry| {
            total.checked_add(u64::from(u32::from_be_bytes(entry.try_into().ok()?)))
        })?
    };
    let duration = duration_units as f64 / f64::from(timescale);
    Some((duration, audio_bytes as f64 * 8.0 / duration))
}

fn find_mp4_box_seekable<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
    wanted: &[u8; 4],
    depth: u8,
) -> Option<(u64, u64)> {
    const CONTAINERS: [&[u8; 4]; 8] = [
        b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts", b"dinf", b"udta",
    ];
    if depth > 16 {
        return None;
    }
    let mut offset = start;
    while offset.checked_add(8)? <= end {
        let mut header = [0_u8; 16];
        read_exact_at(reader, offset, &mut header[..8])?;
        let size32 = u32::from_be_bytes(header[..4].try_into().ok()?);
        let kind: [u8; 4] = header[4..8].try_into().ok()?;
        let (header_size, size) = if size32 == 1 {
            read_exact_at(reader, offset + 8, &mut header[8..16])?;
            (16_u64, u64::from_be_bytes(header[8..16].try_into().ok()?))
        } else if size32 == 0 {
            (8_u64, end.checked_sub(offset)?)
        } else {
            (8_u64, u64::from(size32))
        };
        if size < header_size {
            return None;
        }
        let box_end = offset.checked_add(size)?;
        if box_end > end {
            return None;
        }
        let payload = offset.checked_add(header_size)?;
        if &kind == wanted {
            return Some((payload, box_end));
        }
        if CONTAINERS.contains(&&kind) {
            if let Some(found) = find_mp4_box_seekable(reader, payload, box_end, wanted, depth + 1)
            {
                return Some(found);
            }
        }
        offset = box_end;
    }
    None
}

/// Find one MP4 atom payload recursively through known container atoms.
fn find_mp4_box(data: &[u8], start: usize, end: usize, wanted: &[u8; 4]) -> Option<(usize, usize)> {
    const CONTAINERS: [&[u8; 4]; 8] = [
        b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts", b"dinf", b"udta",
    ];
    let mut offset = start;
    while offset.checked_add(8)? <= end && offset + 8 <= data.len() {
        let size32 = u32_be(data, offset)?;
        let kind: &[u8; 4] = data.get(offset + 4..offset + 8)?.try_into().ok()?;
        let (header, size) = if size32 == 1 {
            (16_usize, usize::try_from(u64_be(data, offset + 8)?).ok()?)
        } else if size32 == 0 {
            (8_usize, end.checked_sub(offset)?)
        } else {
            (8_usize, size32 as usize)
        };
        if size < header {
            return None;
        }
        let box_end = offset.checked_add(size)?;
        if box_end > end || box_end > data.len() {
            return None;
        }
        let payload = offset + header;
        if kind == wanted {
            return Some((payload, box_end));
        }
        if CONTAINERS.contains(&kind) {
            if let Some(found) = find_mp4_box(data, payload, box_end, wanted) {
                return Some(found);
            }
        }
        offset = box_end;
    }
    None
}

fn aiff_audio_properties(path: &Path, sample_rate: Option<u32>) -> Option<(f64, f64)> {
    let mut file = File::open(path).ok()?;
    let size_bytes = file.metadata().ok()?.len();
    aiff_audio_properties_seekable(&mut file, size_bytes, sample_rate).or_else(|| {
        let data = fs::read(path).ok()?;
        aiff_audio_properties_from_bytes(&data, sample_rate)
    })
}

fn aiff_audio_properties_from_bytes(data: &[u8], sample_rate: Option<u32>) -> Option<(f64, f64)> {
    if data.len() < 12 || &data[..4] != b"FORM" || &data[8..12] != b"AIFF" {
        return None;
    }
    let mut offset: usize = 12;
    let mut sample_frames = None;
    let mut audio_bytes = None;
    while offset.checked_add(8)? <= data.len() {
        let kind = data.get(offset..offset + 4)?;
        let size = u32_be(data, offset + 4)? as usize;
        let payload = offset + 8;
        let end = payload.checked_add(size)?;
        if end > data.len() {
            return None;
        }
        if kind == b"COMM" && size >= 18 {
            sample_frames = Some(u64::from(u32_be(data, payload + 2)?));
        } else if kind == b"SSND" && size >= 8 {
            // music-metadata includes SSND's offset/block-size header when
            // deriving bitrate, even though those eight bytes are not PCM.
            audio_bytes = Some(size as u64);
        }
        offset = end.checked_add(size % 2)?;
    }
    let duration = sample_frames? as f64 / f64::from(sample_rate?);
    let audio_bytes = audio_bytes?;
    (duration > 0.0).then(|| (duration, audio_bytes as f64 * 8.0 / duration))
}

fn aiff_audio_properties_seekable<R: Read + Seek>(
    reader: &mut R,
    size_bytes: u64,
    sample_rate: Option<u32>,
) -> Option<(f64, f64)> {
    let mut header = [0_u8; 12];
    read_exact_at(reader, 0, &mut header)?;
    if &header[..4] != b"FORM" || &header[8..12] != b"AIFF" {
        return None;
    }
    let mut offset = 12_u64;
    let mut sample_frames = None;
    let mut audio_bytes = None;
    while offset.checked_add(8)? <= size_bytes {
        let mut chunk = [0_u8; 8];
        read_exact_at(reader, offset, &mut chunk)?;
        let size = u64::from(u32::from_be_bytes(chunk[4..8].try_into().ok()?));
        let payload = offset.checked_add(8)?;
        let end = payload.checked_add(size)?;
        if end > size_bytes {
            return None;
        }
        if &chunk[..4] == b"COMM" && size >= 18 {
            let mut comm = [0_u8; 18];
            read_exact_at(reader, payload, &mut comm)?;
            sample_frames = Some(u64::from(u32::from_be_bytes(comm[2..6].try_into().ok()?)));
        } else if &chunk[..4] == b"SSND" && size >= 8 {
            audio_bytes = Some(size);
        }
        offset = end.checked_add(size % 2)?;
    }
    let duration = sample_frames? as f64 / f64::from(sample_rate?);
    let audio_bytes = audio_bytes?;
    (duration > 0.0).then(|| (duration, audio_bytes as f64 * 8.0 / duration))
}

fn wav_bitrate(data: &[u8]) -> Option<u32> {
    if data.len() < 36 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let mut offset: usize = 12;
    while offset.checked_add(8)? <= data.len() {
        let id = &data[offset..offset + 4];
        let length = u32_le(data, offset + 4)? as usize;
        let data_offset = offset.checked_add(8)?;
        if id == b"fmt " && length >= 16 && data_offset.checked_add(16)? <= data.len() {
            let byte_rate = u32_le(data, data_offset + 8)?;
            return Some(byte_rate.saturating_mul(8));
        }
        offset = data_offset.checked_add(length + (length % 2))?;
    }
    None
}

/// Bounded FLAC fallback faithful to Electron's no-frame/corrupt metadata
/// behavior. It never scans past a declared metadata block and accepts only a
/// 34-byte STREAMINFO first block.
fn read_flac_fallback(path: &Path, size_bytes: u64) -> Result<Option<TrackData>, ApiError> {
    let data = fs::read(path)?;
    if data.len() < 42 || &data[..4] != b"fLaC" {
        return Ok(None);
    }
    let Some((sample_rate, _total_samples)) = flac_stream_info(&data) else {
        return Ok(None);
    };
    let comments = flac_vorbis_comments(&data);
    let track = parse_number_pair(first_comment(&comments, "TRACKNUMBER").unwrap_or_default());
    let disc = parse_number_pair(first_comment(&comments, "DISCNUMBER").unwrap_or_default());
    let album_artist = first_comment(&comments, "ALBUMARTIST")
        .or_else(|| first_comment(&comments, "ALBUM ARTIST"));
    let date = first_comment(&comments, "DATE").or_else(|| first_comment(&comments, "YEAR"));
    let title = first_comment(&comments, "TITLE").or_else(|| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    });

    Ok(Some(TrackData {
        path: path.to_string_lossy().into_owned(),
        title,
        artist: first_comment(&comments, "ARTIST"),
        artists: comments
            .get("ARTISTS")
            .cloned()
            .unwrap_or_else(|| comments.get("ARTIST").cloned().unwrap_or_default()),
        album: first_comment(&comments, "ALBUM"),
        album_artist: album_artist.clone(),
        album_artists: comments
            .get("ALBUMARTISTS")
            .or_else(|| comments.get("ALBUM ARTISTS"))
            .cloned()
            .unwrap_or_else(|| album_artist.into_iter().collect()),
        track_number: track.0,
        track_total: track.1,
        disc_number: disc.0,
        disc_total: disc.1,
        year: date.map(|value| value.chars().take(4).collect()),
        genre: first_comment(&comments, "GENRE"),
        composer: first_comment(&comments, "COMPOSER"),
        comment: first_comment(&comments, "COMMENT"),
        description: first_comment(&comments, "DESCRIPTION"),
        lyrics: None,
        compilation: None,
        musicbrainz_track_id: first_comment(&comments, "MUSICBRAINZ_TRACKID"),
        musicbrainz_album_id: first_comment(&comments, "MUSICBRAINZ_ALBUMID"),
        musicbrainz_artist_id: first_comment(&comments, "MUSICBRAINZ_ARTISTID"),
        discogs_artist_id: first_comment(&comments, "DISCOGS_ARTIST_ID"),
        discogs_release_id: first_comment(&comments, "DISCOGS_RELEASE_ID"),
        has_cover: flac_has_picture(&data),
        size_bytes,
        // Electron's parseFile reports 0 for metadata-only valid FLAC. Its
        // Infinity duration serializes to JSON null, which Rust matches here.
        bitrate: Some(0.0),
        sample_rate: Some(sample_rate),
        codec: "FLAC".to_string(),
        duration: f64::INFINITY,
    }))
}

fn flac_stream_info(data: &[u8]) -> Option<(u32, u64)> {
    if data.get(4).map(|header| header & 0x7f) != Some(0) || flac_block_length(data, 4)? != 34 {
        return None;
    }
    let offset = 8;
    let sample_rate = (u32::from(*data.get(offset + 10)?) << 12)
        | (u32::from(*data.get(offset + 11)?) << 4)
        | (u32::from(*data.get(offset + 12)?) >> 4);
    let total_samples =
        (u64::from(data[offset + 13] & 0x0f) << 32) | u64::from(u32_be(data, offset + 14)?);
    Some((sample_rate, total_samples))
}

fn flac_vorbis_comments(data: &[u8]) -> HashMap<String, Vec<String>> {
    let Some((offset, length)) = find_flac_block(data, 4) else {
        return HashMap::new();
    };
    let mut comments = HashMap::new();
    let end = match offset.checked_add(length) {
        Some(end) if end <= data.len() => end,
        _ => return comments,
    };
    if offset.checked_add(8).is_none_or(|start| start > end) {
        return comments;
    }
    let Some(vendor_length) = u32_le(data, offset) else {
        return comments;
    };
    let mut cursor = match offset.checked_add(4 + vendor_length as usize) {
        Some(cursor) if cursor + 4 <= end => cursor,
        _ => return comments,
    };
    let Some(count) = u32_le(data, cursor) else {
        return comments;
    };
    cursor += 4;
    for _ in 0..count {
        let Some(length) = u32_le(data, cursor).map(|value| value as usize) else {
            break;
        };
        cursor += 4;
        let Some(next) = cursor.checked_add(length) else {
            break;
        };
        if next > end {
            break;
        }
        let Ok(comment) = std::str::from_utf8(&data[cursor..next]) else {
            break;
        };
        cursor = next;
        let Some((key, value)) = comment.split_once('=') else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        comments
            .entry(key.to_ascii_uppercase())
            .or_default()
            .push(value.to_string());
    }
    comments
}

fn flac_has_picture(data: &[u8]) -> bool {
    find_flac_block(data, 6).is_some()
}

fn find_flac_block(data: &[u8], desired_type: u8) -> Option<(usize, usize)> {
    let mut offset: usize = 4;
    while offset.checked_add(4)? <= data.len() {
        let header = *data.get(offset)?;
        let block_type = header & 0x7f;
        let length = flac_block_length(data, offset)?;
        let data_offset = offset.checked_add(4)?;
        let next = data_offset.checked_add(length)?;
        if block_type > 6 || next > data.len() {
            return None;
        }
        if block_type == desired_type {
            return Some((data_offset, length));
        }
        if header & 0x80 != 0 {
            return None;
        }
        offset = next;
    }
    None
}

fn flac_block_length(data: &[u8], offset: usize) -> Option<usize> {
    Some(
        (usize::from(*data.get(offset + 1)?) << 16)
            | (usize::from(*data.get(offset + 2)?) << 8)
            | usize::from(*data.get(offset + 3)?),
    )
}

fn first_comment(comments: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    comments.get(key).and_then(|values| values.first()).cloned()
}

/// APEv2 raw fallback, bounded to valid footer/item spans. This intentionally
/// accepts text items with the same 0 / 0x20000000 flags Electron writes and
/// searches for the footer anywhere before the trailing ID3v1 block.
fn read_ape_fallback(path: &Path, size_bytes: u64) -> Result<Option<TrackData>, ApiError> {
    let mut file = File::open(path)?;
    if let Some(track) = read_ape_fallback_seekable(&mut file, path, size_bytes)? {
        return Ok(Some(track));
    }
    let data = fs::read(path)?;
    let items = parse_ape_items(&data);
    if items.is_empty() {
        return Ok(None);
    }
    Ok(Some(ape_track_from_parts(
        path,
        size_bytes,
        items,
        ape_stream_info(&data),
    )))
}

fn read_ape_fallback_seekable<R: Read + Seek>(
    reader: &mut R,
    path: &Path,
    size_bytes: u64,
) -> Result<Option<TrackData>, ApiError> {
    const FOOTER_BYTES: u64 = 32;
    const ID3V1_BYTES: u64 = 128;
    const MAX_TAG_SIZE: u64 = 16 * 1024 * 1024;
    const MAX_STREAM_HEADER_BYTES: u64 = 1024 * 1024;
    if size_bytes < FOOTER_BYTES {
        return Ok(None);
    }
    let mut prefix = vec![0_u8; usize::try_from(size_bytes.min(76)).unwrap_or(76)];
    reader.seek(SeekFrom::Start(0))?;
    reader.read_exact(&mut prefix)?;
    if prefix.len() >= 16 && &prefix[..4] == b"MAC " {
        let descriptor_bytes = u64::from(u32_le(&prefix, 8).unwrap_or(0));
        let header_bytes = u64::from(u32_le(&prefix, 12).unwrap_or(0));
        if descriptor_bytes >= 52 && header_bytes >= 24 {
            let required = descriptor_bytes.saturating_add(24);
            if required > MAX_STREAM_HEADER_BYTES {
                return Ok(None);
            }
            if required <= size_bytes && required > prefix.len() as u64 {
                prefix.resize(
                    usize::try_from(required).expect("stream header limit fits usize"),
                    0,
                );
                reader.seek(SeekFrom::Start(0))?;
                reader.read_exact(&mut prefix)?;
            }
        }
    }

    let mut footer_offsets = vec![size_bytes - FOOTER_BYTES];
    if size_bytes >= FOOTER_BYTES + ID3V1_BYTES {
        let id3_offset = size_bytes - ID3V1_BYTES;
        let mut signature = [0_u8; 3];
        reader.seek(SeekFrom::Start(id3_offset))?;
        reader.read_exact(&mut signature)?;
        if &signature == b"TAG" {
            footer_offsets.push(id3_offset - FOOTER_BYTES);
        }
    }
    for footer_offset in footer_offsets.into_iter().rev() {
        let mut footer = [0_u8; 32];
        reader.seek(SeekFrom::Start(footer_offset))?;
        reader.read_exact(&mut footer)?;
        if &footer[..8] != b"APETAGEX" {
            continue;
        }
        let tag_size = u64::from(u32_le(&footer, 12).unwrap_or(0));
        let item_count = u32_le(&footer, 16).unwrap_or(u32::MAX);
        let flags = u32_le(&footer, 20).unwrap_or(0x2000_0000);
        if !(FOOTER_BYTES..=MAX_TAG_SIZE).contains(&tag_size)
            || item_count > 100_000
            || flags & 0x2000_0000 != 0
            || tag_size > footer_offset + FOOTER_BYTES
        {
            continue;
        }
        let tag_start = footer_offset + FOOTER_BYTES - tag_size;
        let mut tag = vec![0_u8; usize::try_from(tag_size).expect("tag limit fits usize")];
        reader.seek(SeekFrom::Start(tag_start))?;
        reader.read_exact(&mut tag)?;
        let items = parse_ape_items(&tag);
        if items.is_empty() {
            continue;
        }
        return Ok(Some(ape_track_from_parts(
            path,
            size_bytes,
            items,
            ape_stream_info(&prefix),
        )));
    }
    Ok(None)
}

fn ape_track_from_parts(
    path: &Path,
    size_bytes: u64,
    items: Vec<(String, String)>,
    stream_info: (Option<u32>, Option<f64>),
) -> TrackData {
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    for (key, value) in items {
        tags.entry(key.to_ascii_uppercase())
            .or_default()
            .push(value);
    }
    let (sample_rate, duration) = stream_info;
    let duration = duration.unwrap_or(0.0);
    let album_artist = first_tag(&tags, "ALBUM ARTIST").or_else(|| first_tag(&tags, "ALBUMARTIST"));
    let date = first_tag(&tags, "DATE").or_else(|| first_tag(&tags, "YEAR"));
    let track = parse_number_pair(
        first_tag(&tags, "TRACK")
            .or_else(|| first_tag(&tags, "TRACKNUMBER"))
            .unwrap_or_default(),
    );
    let disc = parse_number_pair(
        first_tag(&tags, "DISC")
            .or_else(|| first_tag(&tags, "DISCNUMBER"))
            .unwrap_or_default(),
    );

    TrackData {
        path: path.to_string_lossy().into_owned(),
        title: first_tag(&tags, "TITLE"),
        artist: first_tag(&tags, "ARTIST"),
        artists: tags.get("ARTIST").cloned().unwrap_or_default(),
        album: first_tag(&tags, "ALBUM"),
        album_artist: album_artist.clone(),
        album_artists: album_artist.into_iter().collect(),
        track_number: track.0,
        track_total: track.1,
        disc_number: disc.0,
        disc_total: disc.1,
        year: date.map(|value| value.chars().take(4).collect()),
        genre: first_tag(&tags, "GENRE"),
        composer: first_tag(&tags, "COMPOSER"),
        comment: first_tag(&tags, "COMMENT"),
        description: first_tag(&tags, "DESCRIPTION"),
        lyrics: None,
        compilation: None,
        musicbrainz_track_id: first_tag(&tags, "MUSICBRAINZ_TRACKID"),
        musicbrainz_album_id: first_tag(&tags, "MUSICBRAINZ_ALBUMID"),
        musicbrainz_artist_id: first_tag(&tags, "MUSICBRAINZ_ARTISTID"),
        discogs_artist_id: first_tag(&tags, "DISCOGS_ARTIST_ID"),
        discogs_release_id: first_tag(&tags, "DISCOGS_RELEASE_ID"),
        has_cover: false,
        size_bytes,
        bitrate: (duration > 0.0).then(|| ((size_bytes as f64 * 8.0) / duration).round()),
        sample_rate,
        codec: "Monkey's Audio".to_string(),
        duration,
    }
}

fn parse_ape_items(data: &[u8]) -> Vec<(String, String)> {
    const SIGNATURE: &[u8; 8] = b"APETAGEX";
    const FOOTER_BYTES: usize = 32;
    const MAX_TAG_SIZE: usize = 16 * 1024 * 1024;
    const MAX_ITEM_COUNT: u32 = 100_000;
    const HEADER_FLAG: u32 = 0x2000_0000;
    const ITEM_TYPE_MASK: u32 = 0x6000_0000;
    const BINARY_ITEM_TYPE: u32 = 0x4000_0000;

    let footer = data
        .windows(SIGNATURE.len())
        .enumerate()
        .filter_map(|(offset, window)| {
            if window != SIGNATURE || offset + FOOTER_BYTES > data.len() {
                return None;
            }
            let tag_size = u32_le(data, offset + 12)? as usize;
            let item_count = u32_le(data, offset + 16)?;
            let flags = u32_le(data, offset + 20)?;
            if flags & HEADER_FLAG != 0
                || !(FOOTER_BYTES..=MAX_TAG_SIZE).contains(&tag_size)
                || tag_size > data.len()
                || item_count > MAX_ITEM_COUNT
            {
                return None;
            }
            let items_start = offset.checked_add(FOOTER_BYTES)?.checked_sub(tag_size)?;
            (items_start <= offset).then_some((offset, items_start, item_count))
        })
        .next_back();
    let Some((footer_offset, mut cursor, item_count)) = footer else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for _ in 0..item_count {
        let Some(value_size) = u32_le(data, cursor).map(|size| size as usize) else {
            return Vec::new();
        };
        let Some(flags) = u32_le(data, cursor + 4) else {
            return Vec::new();
        };
        cursor += 8;
        let Some(key_end) = data[cursor..footer_offset]
            .iter()
            .position(|byte| *byte == 0)
            .map(|relative| cursor + relative)
        else {
            return Vec::new();
        };
        let Ok(key) = std::str::from_utf8(&data[cursor..key_end]) else {
            return Vec::new();
        };
        if key.len() < 2 || key.len() > 255 || key.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
            return Vec::new();
        }
        cursor = key_end + 1;
        let Some(value_end) = cursor.checked_add(value_size) else {
            return Vec::new();
        };
        if value_end > footer_offset {
            return Vec::new();
        }
        let item_type = flags & ITEM_TYPE_MASK;
        if item_type != BINARY_ITEM_TYPE && item_type != ITEM_TYPE_MASK {
            if let Ok(value) = std::str::from_utf8(&data[cursor..value_end]) {
                items.extend(
                    value
                        .split('\0')
                        .map(|value| (key.to_string(), value.to_string())),
                );
            }
        }
        cursor = value_end;
    }
    if cursor == footer_offset {
        items
    } else {
        Vec::new()
    }
}

pub(crate) fn ape_text_values(path: &Path, wanted: &str) -> Vec<String> {
    fs::read(path).map_or_else(
        |_| Vec::new(),
        |bytes| {
            parse_ape_items(&bytes)
                .into_iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(wanted))
                .map(|(_, value)| value)
                .collect()
        },
    )
}

fn ape_stream_info(data: &[u8]) -> (Option<u32>, Option<f64>) {
    if data.len() < 76 || &data[..4] != b"MAC " {
        return (None, None);
    }
    let Some(descriptor_bytes) = u32_le(data, 8).map(|value| value as usize) else {
        return (None, None);
    };
    let Some(header_bytes) = u32_le(data, 12).map(|value| value as usize) else {
        return (None, None);
    };
    if descriptor_bytes < 52 || header_bytes < 24 || descriptor_bytes + 24 > data.len() {
        return (None, None);
    }
    let blocks_per_frame = u32_le(data, descriptor_bytes + 4).unwrap_or(0);
    let final_frame_blocks = u32_le(data, descriptor_bytes + 8).unwrap_or(0);
    let total_frames = u32_le(data, descriptor_bytes + 12).unwrap_or(0);
    let sample_rate = u32_le(data, descriptor_bytes + 20).unwrap_or(0);
    if sample_rate == 0 || total_frames == 0 || blocks_per_frame == 0 {
        return ((sample_rate != 0).then_some(sample_rate), None);
    }
    let blocks = u64::from(total_frames.saturating_sub(1)) * u64::from(blocks_per_frame)
        + u64::from(if final_frame_blocks == 0 {
            blocks_per_frame
        } else {
            final_frame_blocks
        });
    (
        Some(sample_rate),
        (blocks > 0).then(|| blocks as f64 / sample_rate as f64),
    )
}

fn first_tag(tags: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    tags.get(key).and_then(|values| values.first()).cloned()
}

fn parse_number_pair(value: String) -> (Option<u32>, Option<u32>) {
    let mut parts = value.split('/');
    let number = parts.next().and_then(parse_positive_u32);
    let total = parts.next().and_then(parse_positive_u32);
    (number, total)
}

fn parse_positive_u32(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok().filter(|value| *value > 0)
}

fn read_exact_at<R: Read + Seek>(reader: &mut R, offset: u64, buffer: &mut [u8]) -> Option<()> {
    reader.seek(SeekFrom::Start(offset)).ok()?;
    reader.read_exact(buffer).ok()
}

fn u16_le(data: &[u8], offset: usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(offset..offset + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn u64_le(data: &[u8], offset: usize) -> Option<u64> {
    let bytes: [u8; 8] = data.get(offset..offset + 8)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn u64_be(data: &[u8], offset: usize) -> Option<u64> {
    let bytes: [u8; 8] = data.get(offset..offset + 8)?.try_into().ok()?;
    Some(u64::from_be_bytes(bytes))
}

fn u32_le(data: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn u32_be(data: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{Read, Seek, SeekFrom};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    const CORPUS_FILES: &[&str] = &[
        "minimal.mp3",
        "minimal.flac",
        "minimal.wav",
        "minimal.ogg",
        "ape-id3v1-fallback.ape",
        "malformed-truncated.flac",
        "malformed-vorbis-length.flac",
        "minimal.m4a",
        "minimal.mp4",
        "minimal.opus",
        "minimal.aiff",
    ];

    fn corpus_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("test/fixtures/tauri/media-corpus")
            .canonicalize()
            .expect("committed Electron/Rust media corpus exists")
    }

    fn normalize_track_json(mut track: TrackData, root: &Path) -> serde_json::Value {
        let relative = Path::new(&track.path)
            .strip_prefix(root)
            .expect("corpus reader returns a path under its input root")
            .to_string_lossy()
            .replace('\\', "/");
        track.path = relative;
        normalize_numeric_representation(
            serde_json::to_value(track).expect("TrackData serializes to the renderer DTO"),
        )
    }

    /// Electron/Node and Rust round one duration ULP apart, while serde_json
    /// preserves integer-vs-float representation that JavaScript cannot see.
    /// Values themselves remain exact except finite duration at 12 decimals.
    fn normalize_numeric_representation(mut value: serde_json::Value) -> serde_json::Value {
        if let Some(duration) = value.get("duration").and_then(serde_json::Value::as_f64) {
            value["duration"] =
                serde_json::json!((duration * 1_000_000_000_000.0).round() / 1_000_000_000_000.0);
        }
        // JavaScript has one numeric type; serde_json preserves an integer vs
        // float representation that is not observable in command payloads.
        if let Some(bitrate) = value.get("bitrate").and_then(serde_json::Value::as_f64) {
            value["bitrate"] = serde_json::json!(bitrate);
        }
        value
    }

    /// Differential contract: every Lofty/custom-fallback result serializes to
    /// Electron's normalized renderer payload from these exact eleven files:
    /// MP3/FLAC/WAV/OGG/APE, malformed FLAC, M4A/MP4, Opus, and AIFF.
    #[test]
    fn shared_electron_media_corpus_matches_track_data() {
        let root = corpus_root();
        let expected: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("expected.json"))
                .expect("Electron normalized baseline exists"),
        )
        .expect("Electron baseline is valid JSON");
        let expected = serde_json::Value::Array(
            expected
                .as_array()
                .expect("Electron baseline is an array")
                .iter()
                .cloned()
                .map(normalize_numeric_representation)
                .collect(),
        );
        let actual = serde_json::Value::Array(
            CORPUS_FILES
                .iter()
                .map(|file| {
                    normalize_track_json(
                        read_track_metadata(&root.join(file))
                            .expect("corpus reader must not reject a committed case"),
                        &root,
                    )
                })
                .collect(),
        );
        assert_eq!(actual, expected);
    }

    /// Container parsers are exposed to untrusted library files. Truncated
    /// atom/page/chunk structures must return None, never panic or over-read.
    #[test]
    fn container_property_parsers_reject_truncated_input() {
        let root = album_test_root();
        std::fs::create_dir_all(&root).unwrap();
        for name in ["bad.m4a", "bad.opus", "bad.aiff"] {
            std::fs::write(root.join(name), b"short").unwrap();
        }
        assert_eq!(mp4_audio_properties(&root.join("bad.m4a")), None);
        assert_eq!(opus_audio_properties(&root.join("bad.opus")), None);
        assert_eq!(
            aiff_audio_properties(&root.join("bad.aiff"), Some(44_100)),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Intent: MP4 compatibility properties live in metadata atoms, so a
    /// leading media payload must not be transferred merely to reach metadata.
    #[test]
    fn mp4_seekable_properties_skip_large_mdat_payload() {
        let mut mp4 = mp4_atom(b"ftyp", b"M4A ");
        mp4.extend_from_slice(&mp4_atom(b"mdat", &vec![0_u8; 8 * 1024 * 1024]));
        let mut mdhd = vec![0_u8; 20];
        mdhd[12..16].copy_from_slice(&48_000_u32.to_be_bytes());
        mdhd[16..20].copy_from_slice(&96_000_u32.to_be_bytes());
        let mut stsz = vec![0_u8; 12];
        stsz[4..8].copy_from_slice(&1_000_u32.to_be_bytes());
        stsz[8..12].copy_from_slice(&4_u32.to_be_bytes());
        let mdia = mp4_atom(
            b"mdia",
            &[
                mp4_atom(b"mdhd", &mdhd),
                mp4_atom(b"minf", &mp4_atom(b"stbl", &mp4_atom(b"stsz", &stsz))),
            ]
            .concat(),
        );
        mp4.extend_from_slice(&mp4_atom(b"moov", &mp4_atom(b"trak", &mdia)));
        let expected = mp4_audio_properties_from_bytes(&mp4)
            .expect("owned compatibility parser should resolve fixture");

        let bytes_read = Rc::new(Cell::new(0));
        let mut reader = CountingReader::new(Cursor::new(mp4), Rc::clone(&bytes_read));
        let size_bytes = reader.len();
        let properties = mp4_audio_properties_seekable(&mut reader, size_bytes)
            .expect("valid metadata atoms should resolve");

        assert_eq!(properties, (2.0, 16_000.0));
        assert_eq!(properties, expected);
        assert!(
            bytes_read.get() < 4 * 1024,
            "MP4 property scan transferred {} bytes across an 8 MiB mdat",
            bytes_read.get()
        );
    }

    /// Intent: Opus duration needs only the identification/tags packets and
    /// final page granule; encoded packet bodies between them stay untouched.
    #[test]
    fn opus_seekable_properties_skip_encoded_packet_bodies() {
        let mut head = vec![0_u8; 19];
        head[..8].copy_from_slice(b"OpusHead");
        head[10..12].copy_from_slice(&312_u16.to_le_bytes());
        let tags = b"OpusTagsbounded";
        let mut opus = ogg_page(&head, 0, 0);
        opus.extend_from_slice(&ogg_page(tags, 0, 1));
        for sequence in 2..130 {
            opus.extend_from_slice(&ogg_page(&vec![0_u8; 65_024], 0, sequence));
        }
        opus.extend_from_slice(&ogg_page(b"last", 96_312, 130));
        let expected = opus_audio_properties_from_bytes(&opus)
            .expect("owned compatibility parser should resolve fixture");

        let bytes_read = Rc::new(Cell::new(0));
        let mut reader = CountingReader::new(Cursor::new(opus), Rc::clone(&bytes_read));
        let size_bytes = reader.len();
        let properties = opus_audio_properties_seekable(&mut reader, size_bytes)
            .expect("valid Opus pages should resolve");

        assert_eq!(properties, (2.0, tags.len() as f64 * 4.0));
        assert_eq!(properties, expected);
        assert!(
            bytes_read.get() < 128 * 1024,
            "Opus property scan transferred {} bytes across 8 MiB of packets",
            bytes_read.get()
        );
    }

    /// Intent: AIFF properties are declared by COMM and the SSND chunk header,
    /// so PCM bytes must be skipped instead of copied into memory.
    #[test]
    fn aiff_seekable_properties_skip_large_ssnd_payload() {
        let mut comm = vec![0_u8; 18];
        comm[2..6].copy_from_slice(&44_100_u32.to_be_bytes());
        let mut aiff = Vec::from(&b"FORM\0\0\0\0AIFF"[..]);
        append_aiff_chunk(&mut aiff, b"COMM", &comm);
        append_aiff_chunk(&mut aiff, b"SSND", &vec![0_u8; 8 * 1024 * 1024 + 8]);
        let form_size = (aiff.len() as u32 - 8).to_be_bytes();
        aiff[4..8].copy_from_slice(&form_size);
        let expected = aiff_audio_properties_from_bytes(&aiff, Some(44_100))
            .expect("owned compatibility parser should resolve fixture");

        let bytes_read = Rc::new(Cell::new(0));
        let mut reader = CountingReader::new(Cursor::new(aiff), Rc::clone(&bytes_read));
        let size_bytes = reader.len();
        let properties = aiff_audio_properties_seekable(&mut reader, size_bytes, Some(44_100))
            .expect("valid AIFF chunks should resolve");

        assert_eq!(properties.0, 1.0);
        assert_eq!(properties.1, (8 * 1024 * 1024 + 8) as f64 * 8.0);
        assert_eq!(properties, expected);
        assert!(
            bytes_read.get() < 1024,
            "AIFF property scan transferred {} bytes across an 8 MiB SSND",
            bytes_read.get()
        );
    }

    /// Intent: Monkey's Audio metadata lives in a bounded header and trailing
    /// APEv2 tag; the encoded audio between them must remain unread.
    #[test]
    fn ape_seekable_fallback_skips_large_audio_payload() {
        let descriptor_bytes = 100_usize;
        let mut ape = vec![0_u8; descriptor_bytes + 24];
        ape[..4].copy_from_slice(b"MAC ");
        ape[8..12].copy_from_slice(&(descriptor_bytes as u32).to_le_bytes());
        ape[12..16].copy_from_slice(&24_u32.to_le_bytes());
        ape[descriptor_bytes + 4..descriptor_bytes + 8].copy_from_slice(&73_728_u32.to_le_bytes());
        ape[descriptor_bytes + 8..descriptor_bytes + 12].copy_from_slice(&44_100_u32.to_le_bytes());
        ape[descriptor_bytes + 12..descriptor_bytes + 16].copy_from_slice(&1_u32.to_le_bytes());
        ape[descriptor_bytes + 20..descriptor_bytes + 24]
            .copy_from_slice(&44_100_u32.to_le_bytes());
        ape.resize(8 * 1024 * 1024, 0);
        let mut item = Vec::new();
        item.extend_from_slice(&7_u32.to_le_bytes());
        item.extend_from_slice(&0_u32.to_le_bytes());
        item.extend_from_slice(b"Title\0Bounded");
        let tag_size = (item.len() + 32) as u32;
        ape.extend_from_slice(&item);
        ape.extend_from_slice(b"APETAGEX");
        ape.extend_from_slice(&2000_u32.to_le_bytes());
        ape.extend_from_slice(&tag_size.to_le_bytes());
        ape.extend_from_slice(&1_u32.to_le_bytes());
        ape.extend_from_slice(&0_u32.to_le_bytes());
        ape.extend_from_slice(&[0_u8; 8]);
        ape.extend_from_slice(b"TAG");
        ape.resize(ape.len() + 125, 0);
        let path = Path::new("bounded.ape");
        let size_bytes = ape.len() as u64;
        let expected = ape_track_from_parts(
            path,
            size_bytes,
            parse_ape_items(&ape),
            ape_stream_info(&ape),
        );

        let bytes_read = Rc::new(Cell::new(0));
        let mut reader = CountingReader::new(Cursor::new(ape), Rc::clone(&bytes_read));
        let track = read_ape_fallback_seekable(&mut reader, path, size_bytes)
            .expect("bounded APE read should not fail")
            .expect("valid trailing APEv2 tag should resolve");

        assert_eq!(track.title.as_deref(), Some("Bounded"));
        assert_eq!(track.sample_rate, Some(44_100));
        assert_eq!(track.duration, 1.0);
        assert_eq!(track, expected);
        assert!(
            bytes_read.get() < 4 * 1024,
            "APE fallback transferred {} bytes across an 8 MiB payload",
            bytes_read.get()
        );
    }

    /// Intent: album:read must retain the renderer's folder hints and report a
    /// local `cover.jpg` before embedded art. It reuses the real reader instead
    /// of manufacturing TrackData, so the whole read-only vertical slice is
    /// exercised from directory bytes to AlbumDetail DTO.
    #[test]
    fn album_read_reports_hints_external_cover_and_ok_status() {
        let root = album_test_root();
        let album = root.join("Artist").join("Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::copy(corpus_root().join("minimal.ogg"), album.join("01.ogg")).unwrap();
        std::fs::write(album.join("cover.jpg"), b"cover").unwrap();

        let result = read_album(&album).expect("readable album should resolve");
        assert_eq!(result.name, "Album");
        assert_eq!(result.artist_hint, "Artist");
        assert_eq!(result.album_hint, "Album");
        assert_eq!(result.status, "ok");
        assert_eq!(result.tracks.len(), 1);
        assert_eq!(result.tracks[0].title.as_deref(), Some("Corpus OGG"));
        assert_eq!(result.cover_info.source, "external");
        assert_eq!(
            result.cover_info.path,
            Some(album.join("cover.jpg").to_string_lossy().into_owned())
        );
        assert_eq!(result.cover_info.data_url, None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Intent: album loading now performs cover decoding and encoding, so the
    /// Tauri command must keep that blocking work off the async runtime thread.
    #[test]
    fn album_read_command_is_async() {
        fn assert_future<F>(_: F)
        where
            F: std::future::Future<Output = Result<AlbumDetail, ApiError>>,
        {
        }

        assert_future(album_read("/missing/album".to_string()));
    }

    /// Intent: one malformed track is visible but must downgrade a otherwise
    /// healthy album to warning, so callers can distinguish partial results
    /// from a clean scan without losing the good metadata.
    #[test]
    fn album_read_reports_warning_for_partial_track_failure() {
        let root = album_test_root();
        let album = root.join("Artist").join("Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::copy(corpus_root().join("minimal.ogg"), album.join("01.ogg")).unwrap();
        std::fs::write(album.join("02-corrupt.flac"), vec![0_u8; 128]).unwrap();

        let result = read_album(&album).expect("album stays readable");
        assert_eq!(result.status, "warning");
        assert_eq!(result.tracks.len(), 2);
        assert_eq!(result.tracks[1].title.as_deref(), Some("02-corrupt.flac"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Intent: all malformed tracks produce `error`, not warning/ok, matching
    /// Electron's `errorCount === tracks.length` status rule.
    #[test]
    fn album_read_reports_error_when_all_tracks_fail() {
        let root = album_test_root();
        let album = root.join("Artist").join("Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join("corrupt.flac"), vec![0_u8; 128]).unwrap();

        let result = read_album(&album).expect("album directory stays readable");
        assert_eq!(result.status, "error");
        assert_eq!(result.tracks[0].title.as_deref(), Some("corrupt.flac"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn extra_tags_read_filters_editor_fields_and_keeps_rich_id3_rows() {
        let rows = read_extra_tags(&corpus_root().join("minimal.mp3"));
        assert!(rows.iter().any(|row| row.key == "DESCRIPTION"));
        assert!(rows.iter().any(|row| row.key == "ARTISTS"));
        assert!(rows.iter().any(|row| row.key == "MUSICBRAINZ_ALBUMID"));
        assert!(rows.iter().all(|row| row.source == "ID3v2"));
        assert!(!rows.iter().any(|row| matches!(
            row.key.as_str(),
            "TIT2" | "TITLE" | "TPE1" | "ARTIST" | "TALB" | "ALBUM"
        )));
    }

    #[test]
    fn extra_tags_provider_aliases_are_canonical_and_deduplicated() {
        let rows = deduplicate_extra_tags(
            vec![
                ExtraTag {
                    key: "MusicBrainz Album Id".to_string(),
                    value: "first".to_string(),
                    source: "ID3v2".to_string(),
                },
                ExtraTag {
                    key: "MUSICBRAINS_ALBUMID".to_string(),
                    value: "second".to_string(),
                    source: "ID3v2".to_string(),
                },
            ]
            .into_iter()
            .map(|mut row| {
                row.key = canonical_extra_provider_key(&row.key).unwrap_or(row.key);
                row
            })
            .collect(),
        );
        assert_eq!(
            rows,
            [ExtraTag {
                key: "MUSICBRAINZ_ALBUMID".to_string(),
                value: "first".to_string(),
                source: "ID3v2".to_string(),
            }]
        );
    }

    #[test]
    fn extra_tags_read_gracefully_handles_malformed_and_unsupported_files() {
        let root = album_test_root();
        std::fs::create_dir_all(&root).unwrap();
        let malformed = root.join("bad.flac");
        let unsupported = root.join("notes.txt");
        std::fs::write(&malformed, b"bad").unwrap();
        std::fs::write(&unsupported, b"TITLE=not metadata").unwrap();
        assert!(read_extra_tags(&malformed).is_empty());
        assert!(read_extra_tags(&unsupported).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wav_id3_scanner_reads_only_declared_id3_chunks() {
        let value = b"ARTISTS\0One;Two";
        let mut frame = Vec::from(&b"TXXX"[..]);
        frame.extend_from_slice(&(value.len() as u32 + 1).to_be_bytes());
        frame.extend_from_slice(&[0, 0, 3]);
        frame.extend_from_slice(value);
        let mut tag = Vec::from(&b"ID3\x03\0\0"[..]);
        tag.extend_from_slice(&[0, 0, 0, frame.len() as u8]);
        tag.extend_from_slice(&frame);
        let mut wav = Vec::from(&b"RIFF\0\0\0\0WAVEID3 "[..]);
        wav.extend_from_slice(&(tag.len() as u32).to_le_bytes());
        wav.extend_from_slice(&tag);
        let offset = wav_id3_offset(&wav).expect("declared ID3 chunk");
        assert_eq!(
            id3_user_text_values_at(&wav, offset, "ARTISTS"),
            ["One", "Two"]
        );
    }

    #[test]
    fn wav_id3_scanner_ignores_id3_signature_inside_pcm() {
        let pcm = b"noise-ID3\x03\0\0\0\0\0\0-audio";
        let mut wav = Vec::from(&b"RIFF\0\0\0\0WAVEdata"[..]);
        wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(pcm);
        assert_eq!(wav_id3_offset(&wav), None);
    }

    #[test]
    fn wav_id3_scanner_rejects_oversized_malformed_chunk() {
        let mut wav = Vec::from(&b"RIFF\0\0\0\0WAVEID3 "[..]);
        wav.extend_from_slice(&u32::MAX.to_le_bytes());
        wav.extend_from_slice(b"ID3");
        assert_eq!(wav_id3_offset(&wav), None);
    }

    /// Intent: WAV metadata extraction must reuse the one padded-safe buffer
    /// for tags, native overrides, plural credits, properties, and artwork.
    /// A nonexistent path makes any redundant audio-payload read fail loudly.
    #[test]
    fn wav_metadata_from_owned_bytes_preserves_fields_without_rereading_path() {
        let path = album_test_root().join("already-read.wav");
        let mut wav = wav_before_payload();
        let data_size = 1_764_u32;
        let size_offset = wav.len() - 4;
        wav[size_offset..].copy_from_slice(&data_size.to_le_bytes());
        wav.extend_from_slice(&vec![0_u8; data_size as usize]);
        append_riff_chunk(&mut wav, b"LIST", &list_info_title("stale LIST title"));
        append_riff_chunk(&mut wav, b"ID3 ", &metadata_rich_id3v23());
        wav.extend_from_slice(&[0_u8; 8]);
        riff_fix_size(&mut wav);
        let size_bytes = wav.len() as u64;

        let track = read_wav_metadata_from_bytes(&path, size_bytes, wav)
            .expect("owned WAV bytes should be sufficient for all metadata extraction");

        assert_eq!(track.title.as_deref(), Some("ID3 title"));
        assert_eq!(track.artist.as_deref(), Some("Primary artist"));
        assert_eq!(track.album.as_deref(), Some("Album"));
        assert_eq!(track.album_artist.as_deref(), Some("Album artist"));
        assert_eq!(track.artists, ["Primary artist", "Guest artist"]);
        assert_eq!(track.album_artists, ["Album artist", "Guest album artist"]);
        assert_eq!(track.year.as_deref(), Some("2026"));
        assert_eq!(track.genre.as_deref(), Some("Test"));
        assert_eq!(track.musicbrainz_album_id.as_deref(), Some("mb-release"));
        assert_eq!(track.musicbrainz_artist_id.as_deref(), Some("mb-artist"));
        assert_eq!(track.musicbrainz_track_id.as_deref(), Some("mb-track"));
        assert_eq!(track.discogs_artist_id.as_deref(), Some("discogs-artist"));
        assert_eq!(track.discogs_release_id.as_deref(), Some("discogs-release"));
        assert_eq!(track.duration, 0.01);
        assert_eq!(track.bitrate, Some(1_411_200.0));
        assert_eq!(track.sample_rate, Some(44_100));
        assert!(track.has_cover);
        assert_eq!(track.size_bytes, size_bytes);
    }

    /// Intent: consolidating WAV parsing must not turn malformed input into a
    /// partial success merely because all extraction now shares one buffer.
    #[test]
    fn wav_metadata_from_owned_bytes_rejects_malformed_input() {
        let result = read_wav_metadata_from_bytes(Path::new("missing.wav"), 5, b"short".to_vec());
        assert!(matches!(result, Err(ApiError::Lofty(_))));
    }

    /// Intent: normal WAV reads must scale with metadata, not PCM size, so
    /// large lossless files do not require transferring their audio payload.
    #[test]
    fn wav_seekable_metadata_read_skips_large_pcm_payload() {
        let path = Path::new("bounded.wav");
        let mut wav = wav_before_payload();
        let data_size = 8 * 1024 * 1024_u32;
        let size_offset = wav.len() - 4;
        wav[size_offset..].copy_from_slice(&data_size.to_le_bytes());
        wav.resize(wav.len() + data_size as usize, 0);
        append_riff_chunk(&mut wav, b"ID3 ", &metadata_rich_id3v23());
        riff_fix_size(&mut wav);

        let bytes_read = Rc::new(Cell::new(0));
        let mut reader = CountingReader::new(Cursor::new(wav), Rc::clone(&bytes_read));
        let size_bytes = reader.len();
        let track = read_wav_metadata_seekable(&mut reader, path, size_bytes)
            .expect("standard WAV should parse")
            .expect("standard WAV should use the seekable path");

        assert_eq!(track.title.as_deref(), Some("ID3 title"));
        assert_eq!(track.artists, ["Primary artist", "Guest artist"]);
        assert!(
            bytes_read.get() < 64 * 1024,
            "metadata read transferred {} bytes from an 8 MiB PCM payload",
            bytes_read.get()
        );
    }

    /// Intent: the seekable path must preserve ID3-over-LIST precedence and
    /// rich ID3 fields whether metadata appears before or after audio data.
    #[test]
    fn wav_seekable_metadata_matches_owned_reader_across_chunk_order() {
        let path = Path::new("ordered.wav");
        for id3_before_data in [false, true] {
            let wav = metadata_rich_wav(id3_before_data);
            let size_bytes = wav.len() as u64;
            let expected = read_wav_metadata_from_bytes(path, size_bytes, wav.clone())
                .expect("owned compatibility reader should parse fixture");
            let actual = read_wav_metadata_seekable(&mut Cursor::new(wav), path, size_bytes)
                .expect("seekable reader should parse fixture")
                .expect("standard fixture should use the seekable path");

            assert_eq!(actual, expected);
            assert_eq!(actual.title.as_deref(), Some("ID3 title"));
            assert_eq!(actual.artists, ["Primary artist", "Guest artist"]);
            assert!(actual.has_cover);
        }
    }

    /// Intent: ambiguous RIFF layouts must keep using the proven owned-buffer
    /// compatibility path rather than being partially interpreted as normal.
    #[test]
    fn wav_seekable_metadata_defers_compatibility_layouts() {
        let path = Path::new("compatibility.wav");

        let mut padded = metadata_rich_wav(false);
        padded.extend_from_slice(&[0_u8; 8]);
        riff_fix_size(&mut padded);
        let padded_size = padded.len() as u64;
        assert!(
            read_wav_metadata_seekable(&mut Cursor::new(padded), path, padded_size)
                .expect("layout inspection should not fail")
                .is_none()
        );

        let mut orphaned = metadata_rich_wav(false);
        orphaned.extend_from_slice(b"ORPHAN!!");
        let orphaned_size = orphaned.len() as u64;
        assert!(
            read_wav_metadata_seekable(&mut Cursor::new(orphaned), path, orphaned_size)
                .expect("layout inspection should not fail")
                .is_none()
        );

        let mut malformed_id3 = metadata_rich_wav(false);
        let id3 = malformed_id3
            .windows(4)
            .position(|bytes| bytes == b"ID3 ")
            .expect("fixture ID3 chunk");
        malformed_id3[id3 + 8..id3 + 11].copy_from_slice(b"BAD");
        let malformed_size = malformed_id3.len() as u64;
        assert!(
            read_wav_metadata_seekable(&mut Cursor::new(malformed_id3), path, malformed_size,)
                .expect("layout inspection should not fail")
                .is_none()
        );
    }

    fn metadata_rich_wav(id3_before_data: bool) -> Vec<u8> {
        let prefix = wav_before_payload();
        let mut wav = prefix[..prefix.len() - 8].to_vec();
        let id3 = metadata_rich_id3v23();
        let list = list_info_title("stale LIST title");
        if id3_before_data {
            append_riff_chunk(&mut wav, b"ID3 ", &id3);
        } else {
            append_riff_chunk(&mut wav, b"LIST", &list);
        }
        append_riff_chunk(&mut wav, b"data", &[0_u8; 1_764]);
        if id3_before_data {
            append_riff_chunk(&mut wav, b"LIST", &list);
        } else {
            append_riff_chunk(&mut wav, b"ID3 ", &id3);
        }
        riff_fix_size(&mut wav);
        wav
    }

    struct CountingReader<R> {
        inner: R,
        bytes_read: Rc<Cell<u64>>,
        len: u64,
    }

    impl CountingReader<Cursor<Vec<u8>>> {
        fn new(inner: Cursor<Vec<u8>>, bytes_read: Rc<Cell<u64>>) -> Self {
            let len = inner.get_ref().len() as u64;
            Self {
                inner,
                bytes_read,
                len,
            }
        }

        fn len(&self) -> u64 {
            self.len
        }
    }

    impl<R: Read> Read for CountingReader<R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.inner.read(buffer)?;
            self.bytes_read
                .set(self.bytes_read.get().saturating_add(read as u64));
            Ok(read)
        }
    }

    impl<R: Seek> Seek for CountingReader<R> {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(position)
        }
    }

    fn metadata_rich_id3v23() -> Vec<u8> {
        let mut frames = Vec::new();
        for (id, value) in [
            (b"TIT2", "ID3 title"),
            (b"TPE1", "Primary artist"),
            (b"TALB", "Album"),
            (b"TPE2", "Album artist"),
            (b"TYER", "2026"),
            (b"TCON", "Test"),
        ] {
            append_id3v23_frame(&mut frames, id, &[&[3], value.as_bytes()].concat());
        }
        for (description, value) in [
            ("ARTISTS", "Primary artist;Guest artist"),
            ("ALBUMARTISTS", "Album artist;Guest album artist"),
            ("MusicBrainz Release Id", "mb-release"),
            ("MusicBrainz Artist Id", "mb-artist"),
            ("MusicBrainz Track Id", "mb-track"),
            ("Discogs Artist Id", "discogs-artist"),
            ("Discogs Release Id", "discogs-release"),
        ] {
            let payload = [&[3][..], description.as_bytes(), &[0], value.as_bytes()].concat();
            append_id3v23_frame(&mut frames, b"TXXX", &payload);
        }
        let picture = [&[3][..], b"image/jpeg\0", &[3, 0], b"cover"].concat();
        append_id3v23_frame(&mut frames, b"APIC", &picture);

        let mut tag = Vec::from(&b"ID3\x03\0\0"[..]);
        tag.extend_from_slice(&syncsafe_bytes(frames.len() as u32));
        tag.extend_from_slice(&frames);
        tag
    }

    fn append_id3v23_frame(tag: &mut Vec<u8>, id: &[u8; 4], payload: &[u8]) {
        tag.extend_from_slice(id);
        tag.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        tag.extend_from_slice(&[0, 0]);
        tag.extend_from_slice(payload);
    }

    fn syncsafe_bytes(value: u32) -> [u8; 4] {
        [
            ((value >> 21) & 0x7f) as u8,
            ((value >> 14) & 0x7f) as u8,
            ((value >> 7) & 0x7f) as u8,
            (value & 0x7f) as u8,
        ]
    }

    fn list_info_title(title: &str) -> Vec<u8> {
        let mut info = Vec::from(&b"INFO"[..]);
        append_riff_chunk(&mut info, b"INAM", title.as_bytes());
        info
    }

    fn append_riff_chunk(wav: &mut Vec<u8>, id: &[u8; 4], payload: &[u8]) {
        wav.extend_from_slice(id);
        wav.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        wav.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            wav.push(0);
        }
    }

    fn mp4_atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut atom = Vec::with_capacity(payload.len() + 8);
        atom.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
        atom.extend_from_slice(kind);
        atom.extend_from_slice(payload);
        atom
    }

    fn ogg_page(packet: &[u8], granule: u64, sequence: u32) -> Vec<u8> {
        assert!(packet.len() <= 65_025);
        let full_segments = packet.len() / 255;
        let remainder = packet.len() % 255;
        let segment_count = full_segments + usize::from(remainder > 0);
        let mut page = Vec::with_capacity(27 + segment_count + packet.len());
        page.extend_from_slice(b"OggS");
        page.extend_from_slice(&[0, 0]);
        page.extend_from_slice(&granule.to_le_bytes());
        page.extend_from_slice(&1_u32.to_le_bytes());
        page.extend_from_slice(&sequence.to_le_bytes());
        page.extend_from_slice(&0_u32.to_le_bytes());
        page.push(segment_count as u8);
        page.extend(std::iter::repeat_n(255, full_segments));
        if remainder > 0 {
            page.push(remainder as u8);
        }
        page.extend_from_slice(packet);
        page
    }

    fn append_aiff_chunk(aiff: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        aiff.extend_from_slice(kind);
        aiff.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        aiff.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            aiff.push(0);
        }
    }

    fn album_test_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "auto-tag-album-read-{}-{}",
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ))
    }

    /// A RIFF/WAVE with trailing all-zero bytes after the last chunk has
    /// the padding stripped and the RIFF size field corrected.
    #[test]
    fn strip_wav_padding_trims_zero_tail() {
        let mut raw = build_riff_wave();
        // Append trailing zero padding
        raw.extend_from_slice(&[0u8; 8]);
        riff_fix_size(&mut raw);

        let before_len = raw.len();
        strip_wav_padding(&mut raw);

        // Trailing zeros gone, RIFF size corrected
        assert_eq!(raw.len(), before_len - 8, "trailing zero padding stripped");
        let riff_size = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_size, raw.len() - 8, "RIFF size matches new length");
        assert!(raw.windows(4).any(|w| w == b"fmt "), "fmt chunk preserved");
        assert!(raw.windows(4).any(|w| w == b"data"), "data chunk preserved");
    }

    /// A null FourCC followed by non-zero bytes must not be touched.
    #[test]
    fn strip_wav_padding_preserves_nonzero_tail() {
        let mut raw = build_riff_wave();
        // Null FourCC but NOT all-zero tail
        raw.extend_from_slice(&[0u8; 4]); // null FourCC
        raw.extend_from_slice(&4u32.to_le_bytes()); // declares a 4-byte body
        raw.extend_from_slice(b"NONZERO"); // non-zero content
        riff_fix_size(&mut raw);

        let original_len = raw.len();
        strip_wav_padding(&mut raw);

        assert_eq!(
            raw.len(),
            original_len,
            "non-zero tail after null FourCC preserved unchanged"
        );
    }

    /// A data chunk filled with zeros (valid PCM silence) is NOT
    /// trailing padding and must not be touched.
    #[test]
    fn strip_wav_padding_ignores_zero_filled_data() {
        // Build a clean RIFF/WAVE with a zero-filled data payload.
        let mut raw = wav_before_payload();
        // Overwrite the default size (8) with 24
        let pos = raw.len();
        raw[pos - 4..pos].copy_from_slice(&24u32.to_le_bytes());
        raw.extend_from_slice(&[0u8; 24]);
        riff_fix_size(&mut raw);

        let original = raw.len();
        strip_wav_padding(&mut raw);

        assert_eq!(
            raw.len(),
            original,
            "zero-filled data not treated as padding"
        );
        let sz = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        assert_eq!(sz, raw.len() - 8, "RIFF size unchanged");
    }

    /// RIFF/WAVE up to end of the data-chunk size field, no PCM payload yet.
    fn wav_before_payload() -> Vec<u8> {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"RIFF");
        raw.extend_from_slice(&[0u8; 4]);
        raw.extend_from_slice(b"WAVE");
        raw.extend_from_slice(b"fmt ");
        raw.extend_from_slice(&16u32.to_le_bytes());
        raw.extend_from_slice(&[
            0x01, 0x00, 0x02, 0x00, 0x44, 0xac, 0x00, 0x00, 0x10, 0xb1, 0x02, 0x00, 0x04, 0x00,
            0x10, 0x00,
        ]);
        raw.extend_from_slice(b"data");
        raw.extend_from_slice(&8u32.to_le_bytes()); // placeholder
        raw
    }

    /// Build a minimal RIFF/WAVE with fmt+data+8 zero PCM bytes, no padding.
    fn build_riff_wave() -> Vec<u8> {
        let mut raw = wav_before_payload();
        raw.extend_from_slice(&[0u8; 8]);
        riff_fix_size(&mut raw);
        raw
    }

    /// Patch the RIFF size field at offset 4-8 to match current length.
    fn riff_fix_size(raw: &mut [u8]) {
        let size = (raw.len() as u32).wrapping_sub(8);
        raw[4..8].copy_from_slice(&size.to_le_bytes());
    }
}
