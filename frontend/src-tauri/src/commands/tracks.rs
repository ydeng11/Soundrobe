//! Track metadata read/write parity owner (`electron/handlers/tracks.ts`).
//!
//! Lofty-backed read normalization plus format-specific metadata fallbacks.
//! Atomic mutation cores live in `commands::mutations`; extra tags, rename, and
//! remaining formats are enabled only as their differential contracts turn green.

use crate::commands::library::is_audio_file;
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

#[tauri::command]
pub fn track_extra_tags_read(track_path: String) -> Vec<ExtraTag> {
    read_extra_tags(Path::new(&track_path))
}

pub fn read_extra_tags(path: &Path) -> Vec<ExtraTag> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut rows = Vec::new();
    match extension.as_str() {
        "mp3" => {
            if let Ok(mut file) = File::open(path) {
                if let Ok(parsed) =
                    MpegFile::read_from(&mut file, ParseOptions::new().read_properties(false))
                {
                    if let Some(tag) = parsed.id3v2() {
                        collect_id3_extra_tags(tag, "ID3v2", &mut rows);
                    }
                }
            }
        }
        "wav" => {
            if let Ok(mut file) = File::open(path) {
                if let Ok(parsed) =
                    WavFile::read_from(&mut file, ParseOptions::new().read_properties(false))
                {
                    if let Some(tag) = parsed.id3v2() {
                        collect_id3_extra_tags(tag, "ID3v2", &mut rows);
                    }
                }
            }
        }
        "flac" => {
            if let Ok(mut file) = File::open(path) {
                if let Ok(parsed) =
                    FlacFile::read_from(&mut file, ParseOptions::new().read_properties(false))
                {
                    if let Some(tag) = parsed.vorbis_comments() {
                        collect_vorbis_extra_tags(tag.items(), "vorbis", &mut rows);
                    }
                }
            }
        }
        "ogg" => {
            if let Ok(mut file) = File::open(path) {
                if let Ok(parsed) =
                    VorbisFile::read_from(&mut file, ParseOptions::new().read_properties(false))
                {
                    collect_vorbis_extra_tags(
                        parsed.vorbis_comments().items(),
                        "vorbis",
                        &mut rows,
                    );
                }
            }
        }
        "opus" => {
            if let Ok(mut file) = File::open(path) {
                if let Ok(parsed) =
                    OpusFile::read_from(&mut file, ParseOptions::new().read_properties(false))
                {
                    collect_vorbis_extra_tags(
                        parsed.vorbis_comments().items(),
                        "vorbis",
                        &mut rows,
                    );
                }
            }
        }
        "ape" => {
            if let Ok(bytes) = fs::read(path) {
                for (key, value) in parse_ape_items(&bytes) {
                    push_extra_tag(&mut rows, key, value, "APEv2");
                }
            }
        }
        _ => {}
    }
    deduplicate_extra_tags(rows)
}

fn collect_id3_extra_tags(tag: &Id3v2Tag, source: &str, rows: &mut Vec<ExtraTag>) {
    for frame in tag {
        match frame {
            Frame::UserText(frame) => push_extra_tag(
                rows,
                frame.description.to_string(),
                frame.content.to_string(),
                source,
            ),
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
    matches!(
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
            let mut track = from_lofty(path, size_bytes, &extension, &tagged);
            if extension == "flac" {
                apply_flac_native_fields(path, &mut track);
            } else if matches!(extension.as_str(), "ogg" | "opus") {
                apply_ogg_native_fields(path, &extension, &mut track);
            } else if matches!(extension.as_str(), "m4a" | "mp4") {
                apply_mp4_native_fields(path, &mut track);
            } else if extension == "wav" {
                apply_wav_native_fields(path, &mut track);
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
            read_mpeg_header_fallback(path, size_bytes)?.ok_or_else(|| error.into())
        }
        Err(error) if extension == "ogg" => {
            read_ogg_vorbis_fallback(path, size_bytes)?.ok_or_else(|| error.into())
        }
        Err(error) => Err(error.into()),
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

fn apply_wav_native_fields(path: &Path, track: &mut TrackData) {
    let Ok(mut file) = File::open(path) else {
        return;
    };
    let Ok(parsed) = WavFile::read_from(&mut file, ParseOptions::new().read_properties(false))
    else {
        return;
    };
    let Some(id3v2) = parsed.id3v2() else {
        return;
    };
    let artists = id3_user_text_values(path, "ARTISTS");
    if !artists.is_empty() {
        track.artists = artists;
    }
    let album_artists = id3_user_text_values(path, "ALBUMARTISTS");
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
        "wav" => bitrate = wav_bitrate(path).map(f64::from).or(bitrate),
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
        Some("wav") => wav_id3_offset(&data),
        _ => None,
    };
    start.map_or_else(Vec::new, |start| {
        id3_user_text_values_at(&data, start, wanted)
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

fn opus_audio_properties(path: &Path) -> Option<(f64, f64)> {
    let data = fs::read(path).ok()?;
    let packets = ogg_packets(&data);
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
    let granule = last_ogg_granule(&data)?;
    if granule <= pre_skip {
        return None;
    }
    let duration = (granule - pre_skip) as f64 / 48_000.0;
    Some((duration, audio_bytes as f64 * 8.0 / duration))
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
    let data = fs::read(path).ok()?;
    let mdhd = find_mp4_box(&data, 0, data.len(), b"mdhd")?;
    let version = *data.get(mdhd.0)?;
    let (timescale, duration_units) = if version == 1 {
        (u32_be(&data, mdhd.0 + 20)?, u64_be(&data, mdhd.0 + 24)?)
    } else {
        (
            u32_be(&data, mdhd.0 + 12)?,
            u64::from(u32_be(&data, mdhd.0 + 16)?),
        )
    };
    if timescale == 0 || duration_units == 0 {
        return None;
    }
    let stsz = find_mp4_box(&data, 0, data.len(), b"stsz")?;
    let sample_size = u32_be(&data, stsz.0 + 4)?;
    let sample_count = u32_be(&data, stsz.0 + 8)?;
    let audio_bytes = if sample_size > 0 {
        u64::from(sample_size) * u64::from(sample_count)
    } else {
        let mut total = 0_u64;
        let mut offset = stsz.0 + 12;
        for _ in 0..sample_count {
            total = total.checked_add(u64::from(u32_be(&data, offset)?))?;
            offset = offset.checked_add(4)?;
        }
        total
    };
    let duration = duration_units as f64 / f64::from(timescale);
    Some((duration, audio_bytes as f64 * 8.0 / duration))
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
    let data = fs::read(path).ok()?;
    if data.len() < 12 || &data[..4] != b"FORM" || &data[8..12] != b"AIFF" {
        return None;
    }
    let mut offset: usize = 12;
    let mut sample_frames = None;
    let mut audio_bytes = None;
    while offset.checked_add(8)? <= data.len() {
        let kind = data.get(offset..offset + 4)?;
        let size = u32_be(&data, offset + 4)? as usize;
        let payload = offset + 8;
        let end = payload.checked_add(size)?;
        if end > data.len() {
            return None;
        }
        if kind == b"COMM" && size >= 18 {
            sample_frames = Some(u64::from(u32_be(&data, payload + 2)?));
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
        bitrate: (duration > 0.0).then(|| ((size_bytes as f64 * 8.0) / duration).round()),
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
    use std::path::{Path, PathBuf};

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
