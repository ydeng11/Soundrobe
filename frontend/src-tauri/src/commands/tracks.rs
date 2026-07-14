//! Track metadata read/write parity owner (`electron/handlers/tracks.ts`).
//!
//! The first media-safety increment is **read only**: parse the shared committed
//! Electron/Rust corpus with Lofty and normalize it into the renderer's exact
//! `TrackData` DTO. Mutation, extra tags, rename, queueing, and writers remain
//! intentionally absent until this differential reader contract is green.

use crate::commands::library::is_audio_file;
use crate::error::ApiError;
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::mpeg::MpegFile;
use lofty::tag::{ItemKey, Tag};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::path::Path;

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
    pub lyrics: Option<String>,
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
    pub bitrate: Option<u32>,
    #[serde(rename = "sampleRate")]
    pub sample_rate: Option<u32>,
    pub codec: String,
    pub duration: f64,
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
pub fn album_read(album_path: String) -> Result<AlbumDetail, ApiError> {
    read_album(Path::new(&album_path))
}

/// Read one track into the renderer DTO. Generic containers use Lofty; FLAC
/// falls back to a bounded metadata scanner for damaged/no-frame files, and APE
/// uses the raw APEv2 fallback because a trailing ID3v1 tag makes normal parsers
/// unreliable (matching Electron's post-parse fallback policy).
pub fn read_track_metadata(path: &Path) -> Result<TrackData, ApiError> {
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

    match lofty::read_from_path(path) {
        Ok(tagged) => {
            let track = from_lofty(path, size_bytes, &extension, &tagged);
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
            read_mpeg_header_fallback(path, size_bytes)?.ok_or_else(|| error.into())
        }
        Err(error) if extension == "ogg" => {
            read_ogg_vorbis_fallback(path, size_bytes)?.ok_or_else(|| error.into())
        }
        Err(error) => Err(error.into()),
    }
}

fn from_lofty(
    path: &Path,
    size_bytes: u64,
    extension: &str,
    tagged: &lofty::file::TaggedFile,
) -> TrackData {
    let mut bitrate = tagged
        .properties()
        .overall_bitrate()
        .map(|kilobits| kilobits.saturating_mul(1_000));
    // Electron's music-metadata returns precise bits/second for PCM WAV rather
    // than Lofty's integer-kbps representation. For PCM the byte rate can be
    // derived without reading the audio payload.
    if extension == "wav" {
        bitrate = wav_bitrate(path).or(bitrate);
    }
    from_tags(
        path,
        size_bytes,
        extension,
        tagged.tags(),
        tagged.properties().duration().as_secs_f64(),
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
    bitrate: Option<u32>,
    sample_rate: Option<u32>,
) -> TrackData {
    let artist = first_string(tags, ItemKey::TrackArtist);
    let album_artist = first_string(tags, ItemKey::AlbumArtist);
    let mut artists = all_strings(tags, ItemKey::TrackArtist);
    artists.extend(all_strings(tags, ItemKey::TrackArtists));
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
        lyrics: first_string(tags, ItemKey::Lyrics)
            .or_else(|| first_string(tags, ItemKey::UnsyncLyrics)),
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
        "wav" => "PCM",
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
        Some(128_000),
        Some(44_100),
    );
    if let Some(id3v2) = id3v2 {
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
    let bitrate = u32_le(identification, 20);
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
        lyrics: first_comment(&comments, "LYRICS"),
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

fn wav_bitrate(path: &Path) -> Option<u32> {
    let data = fs::read(path).ok()?;
    if data.len() < 36 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let mut offset: usize = 12;
    while offset.checked_add(8)? <= data.len() {
        let id = &data[offset..offset + 4];
        let length = u32_le(&data, offset + 4)? as usize;
        let data_offset = offset.checked_add(8)?;
        if id == b"fmt " && length >= 16 && data_offset.checked_add(16)? <= data.len() {
            let byte_rate = u32_le(&data, data_offset + 8)?;
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
        artists: comments.get("ARTIST").cloned().unwrap_or_default(),
        album: first_comment(&comments, "ALBUM"),
        album_artist: album_artist.clone(),
        album_artists: album_artist.into_iter().collect(),
        track_number: track.0,
        track_total: track.1,
        disc_number: disc.0,
        disc_total: disc.1,
        year: date.map(|value| value.chars().take(4).collect()),
        genre: first_comment(&comments, "GENRE"),
        composer: first_comment(&comments, "COMPOSER"),
        comment: first_comment(&comments, "COMMENT"),
        description: first_comment(&comments, "DESCRIPTION"),
        lyrics: first_comment(&comments, "LYRICS"),
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
        bitrate: Some(0),
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
    let data = fs::read(path)?;
    let items = parse_ape_items(&data);
    if items.is_empty() {
        return Ok(None);
    }
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    for (key, value) in items {
        tags.entry(key.to_ascii_uppercase())
            .or_default()
            .push(value);
    }
    let (sample_rate, duration) = ape_stream_info(&data);
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

    Ok(Some(TrackData {
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
        lyrics: first_tag(&tags, "LYRICS"),
        compilation: None,
        musicbrainz_track_id: first_tag(&tags, "MUSICBRAINZ_TRACKID"),
        musicbrainz_album_id: first_tag(&tags, "MUSICBRAINZ_ALBUMID"),
        musicbrainz_artist_id: first_tag(&tags, "MUSICBRAINZ_ARTISTID"),
        discogs_artist_id: first_tag(&tags, "DISCOGS_ARTIST_ID"),
        discogs_release_id: first_tag(&tags, "DISCOGS_RELEASE_ID"),
        has_cover: false,
        size_bytes,
        bitrate: (duration > 0.0).then(|| ((size_bytes as f64 * 8.0) / duration).round() as u32),
        sample_rate,
        codec: "Monkey's Audio".to_string(),
        duration,
    }))
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
                items.push((key.to_string(), value.to_string()));
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
    use std::path::{Path, PathBuf};

    const CORPUS_FILES: &[&str] = &[
        "minimal.mp3",
        "minimal.flac",
        "minimal.wav",
        "minimal.ogg",
        "ape-id3v1-fallback.ape",
        "malformed-truncated.flac",
        "malformed-vorbis-length.flac",
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
        normalize_duration(
            serde_json::to_value(track).expect("TrackData serializes to the renderer DTO"),
        )
    }

    /// Electron/Node and Rust round the APE block-count division one ULP apart.
    /// Metadata payload comparison therefore normalizes ONLY finite duration to
    /// 12 decimal places; every other DTO field remains exact. JSON null (the
    /// intentional non-finite FLAC representation) stays null.
    fn normalize_duration(mut value: serde_json::Value) -> serde_json::Value {
        if let Some(duration) = value.get("duration").and_then(serde_json::Value::as_f64) {
            value["duration"] =
                serde_json::json!((duration * 1_000_000_000_000.0).round() / 1_000_000_000_000.0);
        }
        value
    }

    /// Differential contract: every Lofty/custom-fallback result serializes to
    /// the same normalized renderer payload Electron produced from these exact
    /// committed bytes. This covers normal MP3/FLAC/WAV/OGG, APE+ID3v1 raw-tag
    /// fallback, truncated FLAC, and malformed Vorbis-length FLAC before any
    /// writer code can be introduced.
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
                .map(normalize_duration)
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
}
