//! Media mutation core. Commands and queue integration land only after each
//! format's pure writer passes differential and payload-safety tests.

use crate::commands::tracks::{
    id3_user_text_values, read_track_metadata, unreadable_track_data, TrackData,
};
use crate::error::ApiError;
use crate::state::write_queue::WriteQueue;
use lofty::ape::{ApeFile, ApeItem, ApeTag};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, FrameId, Id3v2Tag, TextInformationFrame, UnsynchronizedTextFrame};
use lofty::iff::wav::WavFile;
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::tag::{Accessor, ItemValue, TagExt};
use lofty::TextEncoding;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::State;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A renderer patch must distinguish missing, explicit null, and a value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Patch<T> {
    #[default]
    Omitted,
    Null,
    Value(T),
}

impl<'de, T> Deserialize<'de> for Patch<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(|value| value.map_or(Self::Null, Self::Value))
    }
}

impl<T> Patch<T> {
    pub(crate) fn is_omitted(&self) -> bool {
        matches!(self, Self::Omitted)
    }

    #[cfg(test)]
    pub(crate) fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Omitted | Self::Null => None,
        }
    }
}

impl<T: Serialize> Serialize for Patch<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Omitted | Self::Null => serializer.serialize_none(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum StringList {
    One(String),
    Many(Vec<String>),
}

impl StringList {
    fn normalized(&self) -> Vec<String> {
        let values = match self {
            Self::One(value) => vec![value.clone()],
            Self::Many(values) => values.clone(),
        };
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect()
    }
}

/// MP3 fields currently exposed by `DesktopAPI.writeTrack`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackPatch {
    #[serde(default)]
    pub title: Patch<String>,
    #[serde(default)]
    pub artist: Patch<String>,
    #[serde(default)]
    pub artists: Patch<StringList>,
    #[serde(default)]
    pub album: Patch<String>,
    #[serde(default)]
    pub album_artist: Patch<String>,
    #[serde(default)]
    pub album_artists: Patch<StringList>,
    #[serde(default)]
    pub year: Patch<String>,
    #[serde(default)]
    pub track_number: Patch<u32>,
    #[serde(default)]
    pub track_total: Patch<u32>,
    #[serde(default)]
    pub disc_number: Patch<u32>,
    #[serde(default)]
    pub disc_total: Patch<u32>,
    #[serde(default)]
    pub genre: Patch<String>,
    #[serde(default)]
    pub composer: Patch<String>,
    #[serde(default)]
    pub comment: Patch<String>,
    #[serde(default)]
    pub description: Patch<String>,
    #[serde(default)]
    pub lyrics: Patch<String>,
    #[serde(default)]
    pub compilation: Patch<bool>,
    #[serde(default)]
    pub musicbrainz_track_id: Patch<String>,
    #[serde(default)]
    pub musicbrainz_album_id: Patch<String>,
    #[serde(default)]
    pub musicbrainz_artist_id: Patch<String>,
    #[serde(default)]
    pub discogs_artist_id: Patch<String>,
    #[serde(default)]
    pub discogs_release_id: Patch<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFileResult {
    pub path: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtraTagUpdate {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtraTagBatchUpdate {
    pub path: String,
    pub tags: Vec<ExtraTagUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackUpdate {
    pub path: String,
    pub fields: TrackPatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackWriteOutcome {
    Skipped,
    Replaced,
}

#[tauri::command]
pub async fn track_write(
    path: String,
    fields: TrackPatch,
    queue: State<'_, WriteQueue>,
) -> Result<(), ApiError> {
    write_track_queued(&queue, PathBuf::from(path), fields).await
}

#[tauri::command]
pub async fn tracks_batch_write(
    updates: Vec<TrackUpdate>,
    queue: State<'_, WriteQueue>,
) -> Result<(), ApiError> {
    batch_write_queued(&queue, updates).await
}

#[tauri::command]
pub async fn track_extra_tags_write(
    track_path: String,
    tags: Vec<ExtraTagUpdate>,
    queue: State<'_, WriteQueue>,
) -> Result<TrackData, ApiError> {
    let path = PathBuf::from(track_path);
    write_extra_tags_queued(&queue, path.clone(), tags).await?;
    read_track_metadata(&path)
}

#[tauri::command]
pub fn file_exists(file_path: String) -> bool {
    Path::new(&file_path).exists()
}

#[tauri::command]
pub async fn track_delete_files(
    file_paths: Vec<String>,
    queue: State<'_, WriteQueue>,
) -> Result<Vec<DeleteFileResult>, ApiError> {
    Ok(delete_files_queued(&queue, file_paths).await)
}

#[tauri::command]
pub async fn track_rename(
    old_path: String,
    new_path: String,
    queue: State<'_, WriteQueue>,
) -> Result<TrackData, ApiError> {
    rename_track_queued(&queue, PathBuf::from(old_path), PathBuf::from(new_path)).await
}

async fn delete_files_queued(queue: &WriteQueue, file_paths: Vec<String>) -> Vec<DeleteFileResult> {
    let fallback_paths = file_paths.clone();
    match queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                file_paths
                    .into_iter()
                    .map(|path| match fs::remove_file(&path) {
                        Ok(()) => DeleteFileResult {
                            path,
                            success: true,
                            error: None,
                        },
                        Err(error) => DeleteFileResult {
                            path,
                            success: false,
                            error: Some(error.to_string()),
                        },
                    })
                    .collect::<Vec<_>>()
            })
            .await
        })
        .await
    {
        Ok(results) => results,
        Err(error) => fallback_paths
            .into_iter()
            .map(|path| DeleteFileResult {
                path,
                success: false,
                error: Some(format!("background delete task failed: {error}")),
            })
            .collect(),
    }
}

async fn rename_track_queued(
    queue: &WriteQueue,
    old_path: PathBuf,
    new_path: PathBuf,
) -> Result<TrackData, ApiError> {
    let readback_path = new_path.clone();
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                if let Some(parent) = new_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(old_path, new_path)?;
                Ok::<(), ApiError>(())
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await?;
    read_track_metadata(&readback_path)
}

#[tauri::command]
pub async fn tracks_batch_write_extra_tags(
    updates: Vec<ExtraTagBatchUpdate>,
    queue: State<'_, WriteQueue>,
) -> Result<Vec<TrackData>, ApiError> {
    let supported = updates
        .iter()
        .filter(|update| validate_extra_tag_extension(Path::new(&update.path)).is_ok())
        .cloned()
        .collect::<Vec<_>>();
    if !supported.is_empty() {
        batch_write_extra_tags_queued(&queue, supported).await?;
    }
    updates
        .into_iter()
        .map(|update| {
            let path = PathBuf::from(update.path);
            read_track_metadata(&path).or_else(|_| {
                let size = fs::metadata(&path)?.len();
                let title = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string();
                Ok(unreadable_track_data(&path, size, title))
            })
        })
        .collect()
}

fn validated_track_extension(path: &Path) -> Result<String, ApiError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if extension.as_deref() == Some("aiff") {
        return Err(ApiError::UnsupportedFormat(
            "AIFF metadata writing is not supported".to_string(),
        ));
    }
    if !matches!(
        extension.as_deref(),
        Some("mp3" | "flac" | "ogg" | "opus" | "m4a" | "mp4" | "wav" | "ape")
    ) {
        return Err(ApiError::NotImplemented(
            "track:write for formats other than MP3/FLAC/OGG/Opus/M4A/MP4/WAV/APE",
        ));
    }
    Ok(extension.unwrap_or_default())
}

pub(crate) fn write_track_dispatch(
    path: &Path,
    patch: &TrackPatch,
) -> Result<TrackWriteOutcome, ApiError> {
    match validated_track_extension(path)?.as_str() {
        "mp3" => write_mp3_atomic(path, patch),
        "flac" => write_flac_atomic(path, patch),
        "ogg" | "opus" => write_ogg_atomic(path, patch),
        "m4a" | "mp4" => write_mp4_atomic(path, patch),
        "wav" => write_wav_atomic(path, patch),
        _ => write_ape_atomic(path, patch),
    }
}

async fn write_track_queued(
    queue: &WriteQueue,
    path: PathBuf,
    patch: TrackPatch,
) -> Result<(), ApiError> {
    validated_track_extension(&path)?;
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || write_track_dispatch(&path, &patch))
                .await
                .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await?;
    Ok(())
}

async fn batch_write_queued(queue: &WriteQueue, updates: Vec<TrackUpdate>) -> Result<(), ApiError> {
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                for update in updates {
                    write_track_dispatch(Path::new(&update.path), &update.fields)?;
                }
                Ok::<(), ApiError>(())
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await
}

async fn write_extra_tags_queued(
    queue: &WriteQueue,
    path: PathBuf,
    tags: Vec<ExtraTagUpdate>,
) -> Result<(), ApiError> {
    validate_extra_tag_extension(&path)?;
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || write_extra_tags_dispatch(&path, &tags))
                .await
                .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await?;
    Ok(())
}

async fn batch_write_extra_tags_queued(
    queue: &WriteQueue,
    updates: Vec<ExtraTagBatchUpdate>,
) -> Result<(), ApiError> {
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                let mut failures = Vec::new();
                for update in updates {
                    if let Err(error) =
                        write_extra_tags_dispatch(Path::new(&update.path), &update.tags)
                    {
                        failures.push(format!("{}: {error}", update.path));
                    }
                }
                if failures.is_empty() {
                    Ok(())
                } else {
                    Err(ApiError::Message(format!(
                        "Batch extra-tag write failed for {} file(s): {}",
                        failures.len(),
                        failures.join("; ")
                    )))
                }
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await
}

fn validate_extra_tag_extension(path: &Path) -> Result<String, ApiError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "ape"
    ) {
        let label = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!(".{extension}"))
            .unwrap_or_else(|| "this file type".to_string());
        return Err(ApiError::UnsupportedFormat(format!(
            "Extra tag editing is not supported for {label}"
        )));
    }
    Ok(extension)
}

fn write_extra_tags_dispatch(
    path: &Path,
    tags: &[ExtraTagUpdate],
) -> Result<TrackWriteOutcome, ApiError> {
    match validate_extra_tag_extension(path)?.as_str() {
        "mp3" => write_id3_extra_tags_atomic(path, tags, false),
        "wav" => write_id3_extra_tags_atomic(path, tags, true),
        "flac" => write_flac_extra_tags_atomic(path, tags),
        "ogg" | "opus" => write_ogg_extra_tags_atomic(path, tags),
        _ => write_ape_extra_tags_atomic(path, tags),
    }
}

fn normalized_extra_tags(tags: &[ExtraTagUpdate]) -> Vec<ExtraTagUpdate> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for tag in tags {
        let raw_key = tag.key.trim();
        let mut key = if raw_key.eq_ignore_ascii_case("COMM") {
            "COMMENT".to_string()
        } else {
            raw_key.to_string()
        };
        let normalized = normalize_provider_key(&key);
        key = match normalized.as_str() {
            "MUSICBRAINZTRACKID" | "MUSICBRAINZRECORDINGID" => "MUSICBRAINZ_TRACKID",
            "MUSICBRAINZALBUMID" | "MUSICBRAINZRELEASEID" => "MUSICBRAINZ_ALBUMID",
            "MUSICBRAINZARTISTID" => "MUSICBRAINZ_ARTISTID",
            "DISCOGSARTISTID" => "DISCOGS_ARTIST_ID",
            "DISCOGSRELEASEID" => "DISCOGS_RELEASE_ID",
            _ => &key,
        }
        .to_string();
        let value = tag.value.trim().to_string();
        let upper = key.to_ascii_uppercase();
        if key.is_empty()
            || value.is_empty()
            || (is_reserved_extra_key(&upper) && upper != "ARTISTS")
            || !seen.insert((upper, value.clone()))
        {
            continue;
        }
        result.push(ExtraTagUpdate { key, value });
    }
    result
}

fn is_reserved_extra_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_uppercase().as_str(),
        "TITLE"
            | "ARTIST"
            | "ARTISTS"
            | "ALBUM"
            | "ALBUMARTIST"
            | "ALBUM ARTIST"
            | "DATE"
            | "YEAR"
            | "GENRE"
            | "COMPOSER"
            | "LYRICS"
            | "UNSYNCEDLYRICS"
            | "UNSYNCHRONISEDLYRICS"
            | "TRACK"
            | "TRACKNUMBER"
            | "TRACKTOTAL"
            | "TOTALTRACKS"
            | "DISC"
            | "DISCNUMBER"
            | "DISCTOTAL"
            | "TOTALDISCS"
            | "METADATA_BLOCK_PICTURE"
    )
}

fn apply_id3_extra_tags(tag: &mut Id3v2Tag, updates: &[ExtraTagUpdate]) {
    let descriptions = (&*tag)
        .into_iter()
        .filter_map(|frame| match frame {
            Frame::UserText(frame) => Some(frame.description.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for description in descriptions {
        let upper = description.trim().to_ascii_uppercase();
        if !is_reserved_extra_key(&upper) || upper == "ARTISTS" {
            tag.remove_user_text(&description);
        }
    }
    tag.remove_comment();
    let normalized = normalized_extra_tags(updates);
    let comment = normalized
        .iter()
        .find(|tag| tag.key.eq_ignore_ascii_case("COMMENT"));
    if let Some(comment) = comment {
        tag.set_comment(comment.value.clone());
    }
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for update in normalized
        .iter()
        .filter(|tag| !tag.key.eq_ignore_ascii_case("COMMENT"))
    {
        let description = id3_extra_description(&update.key);
        if let Some((_, values)) = groups
            .iter_mut()
            .find(|(key, _)| key.eq_ignore_ascii_case(&description))
        {
            values.push(update.value.clone());
        } else {
            groups.push((description, vec![update.value.clone()]));
        }
    }
    for (description, values) in groups {
        let separator = if description.eq_ignore_ascii_case("ARTISTS") {
            ";"
        } else {
            "\0"
        };
        tag.insert_user_text(description, values.join(separator));
    }
}

fn id3_extra_description(key: &str) -> String {
    match key.to_ascii_uppercase().as_str() {
        "MUSICBRAINZ_TRACKID" => "MusicBrainz Track Id",
        "MUSICBRAINZ_ALBUMID" => "MusicBrainz Album Id",
        "MUSICBRAINZ_ARTISTID" => "MusicBrainz Artist Id",
        "DISCOGS_ARTIST_ID" => "Discogs Artist Id",
        "DISCOGS_RELEASE_ID" => "Discogs Release Id",
        _ => key,
    }
    .to_string()
}

fn apply_vorbis_extra_tags(tag: &mut lofty::ogg::VorbisComments, updates: &[ExtraTagUpdate]) {
    let keys = tag
        .items()
        .map(|(key, _)| key.to_string())
        .collect::<Vec<_>>();
    for key in keys {
        let upper = key.to_ascii_uppercase();
        if !is_reserved_extra_key(&upper) || upper == "ARTISTS" {
            drop(tag.remove(&key));
        }
    }
    for update in normalized_extra_tags(updates) {
        tag.push(update.key.to_ascii_uppercase(), update.value);
    }
}

fn apply_ape_extra_tags(tag: &mut ApeTag, updates: &[ExtraTagUpdate]) -> Result<(), ApiError> {
    let keys = (&*tag)
        .into_iter()
        .map(|item| item.key().to_string())
        .collect::<Vec<_>>();
    for key in keys {
        let upper = key.to_ascii_uppercase();
        if !is_reserved_extra_key(&upper) || upper == "ARTISTS" {
            tag.remove(&key);
        }
    }
    for update in normalized_extra_tags(updates) {
        tag.push(ApeItem::new(
            update.key.to_ascii_uppercase(),
            ItemValue::Text(update.value),
        )?);
    }
    Ok(())
}

fn write_id3_extra_tags_atomic(
    path: &Path,
    updates: &[ExtraTagUpdate],
    wav: bool,
) -> Result<TrackWriteOutcome, ApiError> {
    let original = fs::read(path)?;
    let mut file = File::open(path)?;
    let mut tag = if wav {
        WavFile::read_from(&mut file, ParseOptions::new().read_properties(false))?
            .id3v2()
            .cloned()
            .unwrap_or_default()
    } else {
        MpegFile::read_from(&mut file, ParseOptions::new().read_properties(false))?
            .id3v2()
            .cloned()
            .unwrap_or_default()
    };
    apply_id3_extra_tags(&mut tag, updates);
    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::copy(path, &temporary)?;
        tag.save_to_path(&temporary, WriteOptions::new())?;
        let candidate = fs::read(&temporary)?;
        let payload_equal = if wav {
            let before = wav_data_payloads(&original)
                .ok_or_else(|| ApiError::MediaSafety("invalid WAV chunk structure".to_string()))?;
            wav_data_payloads(&candidate) == Some(before)
        } else {
            let before = mpeg_payload(&original)
                .ok_or_else(|| ApiError::MediaSafety("invalid ID3v2 boundary".to_string()))?;
            mpeg_payload(&candidate) == Some(before)
        };
        if !payload_equal {
            return Err(ApiError::MediaSafety(
                "audio payload changed during extra-tag write".to_string(),
            ));
        }
        read_track_metadata(&temporary)?;
        if candidate == original {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_flac_extra_tags_atomic(
    path: &Path,
    updates: &[ExtraTagUpdate],
) -> Result<TrackWriteOutcome, ApiError> {
    let original = fs::read(path)?;
    let (prepared, repairs) = prepare_flac_source(&original)
        .ok_or_else(|| ApiError::MediaSafety("invalid FLAC metadata boundary".to_string()))?;
    let payload = flac_audio_payload(&prepared).ok_or_else(|| {
        ApiError::MediaSafety("invalid prepared FLAC metadata boundary".to_string())
    })?;
    let target_offset = prepared.len() - payload.len();
    let payload = payload.to_vec();
    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::write(&temporary, &prepared)?;
        let flac = read_flac(&temporary)?;
        let mut comments = flac.vorbis_comments().cloned().unwrap_or_default();
        apply_vorbis_extra_tags(&mut comments, updates);
        comments.save_to_path(&temporary, WriteOptions::new())?;
        let candidate = fs::read(&temporary)?;
        if flac_audio_payload(&candidate) != Some(payload.as_slice()) {
            return Err(ApiError::MediaSafety(
                "FLAC audio payload changed during extra-tag write".to_string(),
            ));
        }
        if !repairs.force_full_rewrite {
            if let Some(repacked) = repack_flac_metadata(&candidate, target_offset, &payload) {
                fs::write(&temporary, repacked)?;
            }
        }
        read_track_metadata(&temporary)?;
        if !repairs.any() && fs::read(&temporary)? == original {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_ogg_extra_tags_atomic(
    path: &Path,
    updates: &[ExtraTagUpdate],
) -> Result<TrackWriteOutcome, ApiError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let header_packets = if extension.eq_ignore_ascii_case("opus") {
        2
    } else {
        3
    };
    let original = fs::read(path)?;
    let original_audio = ogg_audio_packets(&original, header_packets)
        .ok_or_else(|| ApiError::MediaSafety("invalid OGG packet structure".to_string()))?;
    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::copy(path, &temporary)?;
        let mut file = File::open(path)?;
        if extension.eq_ignore_ascii_case("opus") {
            let mut parsed =
                OpusFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            apply_vorbis_extra_tags(parsed.vorbis_comments_mut(), updates);
            parsed.save_to_path(&temporary, WriteOptions::new())?;
        } else {
            let mut parsed =
                VorbisFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
            apply_vorbis_extra_tags(parsed.vorbis_comments_mut(), updates);
            parsed.save_to_path(&temporary, WriteOptions::new())?;
        }
        let candidate = fs::read(&temporary)?;
        if ogg_audio_packets(&candidate, header_packets) != Some(original_audio) {
            return Err(ApiError::MediaSafety(
                "OGG audio packets changed during extra-tag write".to_string(),
            ));
        }
        read_track_metadata(&temporary)?;
        if candidate == original {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_ape_extra_tags_atomic(
    path: &Path,
    updates: &[ExtraTagUpdate],
) -> Result<TrackWriteOutcome, ApiError> {
    let original = fs::read(path)?;
    let core = ape_audio_core(&original)
        .ok_or_else(|| ApiError::MediaSafety("invalid Monkey audio boundary".to_string()))?;
    let mut file = File::open(path)?;
    let parsed = ApeFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
    let mut tag = parsed.ape().cloned().unwrap_or_default();
    apply_ape_extra_tags(&mut tag, updates)?;
    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::write(&temporary, core)?;
        tag.save_to_path(&temporary, WriteOptions::new())?;
        let candidate = fs::read(&temporary)?;
        if ape_audio_core(&candidate) != Some(core) {
            return Err(ApiError::MediaSafety(
                "Monkey audio core changed during extra-tag write".to_string(),
            ));
        }
        read_track_metadata(&temporary)?;
        if candidate == original {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Write canonical APEv2 metadata after the exact tag-free Monkey audio core.
/// Trailing ID3v1 is intentionally removed, matching Electron characterization.
pub fn write_ape_atomic(path: &Path, patch: &TrackPatch) -> Result<TrackWriteOutcome, ApiError> {
    let original_bytes = fs::read(path)?;
    let original_core = ape_audio_core(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid Monkey audio boundary".to_string()))?;
    let before = read_track_metadata(path)?;
    let mut file = File::open(path)?;
    let parsed = ApeFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
    let mut tag = parsed.ape().cloned().unwrap_or_default();
    apply_ape_patch(&mut tag, patch)?;

    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::write(&temporary, original_core)?;
        tag.save_to_path(&temporary, WriteOptions::new())?;
        let candidate_bytes = fs::read(&temporary)?;
        let candidate_core = ape_audio_core(&candidate_bytes).ok_or_else(|| {
            ApiError::MediaSafety("invalid written Monkey audio boundary".to_string())
        })?;
        if candidate_core != original_core {
            return Err(ApiError::MediaSafety(
                "Monkey audio core changed during metadata write".to_string(),
            ));
        }
        let after = read_track_metadata(&temporary)?;
        if candidate_bytes == original_bytes && same_metadata(before, after) {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Write WAV ID3 metadata through a validated sibling. RIFF chunk layout may
/// change, but every PCM `data` payload must remain exact.
pub fn write_wav_atomic(path: &Path, patch: &TrackPatch) -> Result<TrackWriteOutcome, ApiError> {
    let original_bytes = fs::read(path)?;
    let original_audio = wav_data_payloads(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid WAV chunk structure".to_string()))?;
    let before = read_track_metadata(path)?;
    let mut file = File::open(path)?;
    let parsed = WavFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
    let mut tag = parsed.id3v2().cloned().unwrap_or_default();
    preserve_omitted_list(&mut tag, path, "ARTISTS", &patch.artists);
    preserve_omitted_list(&mut tag, path, "ALBUMARTISTS", &patch.album_artists);
    apply_patch(&mut tag, patch);

    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::copy(path, &temporary)?;
        tag.save_to_path(&temporary, WriteOptions::new())?;
        let candidate_bytes = fs::read(&temporary)?;
        let candidate_audio = wav_data_payloads(&candidate_bytes).ok_or_else(|| {
            ApiError::MediaSafety("invalid written WAV chunk structure".to_string())
        })?;
        if candidate_audio != original_audio {
            return Err(ApiError::MediaSafety(
                "WAV data payload changed during metadata write".to_string(),
            ));
        }
        let after = read_track_metadata(&temporary)?;
        if same_metadata(before, after) {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Write M4A/MP4 ilst metadata through a validated sibling. Container atom
/// offsets may change, but every top-level `mdat` payload must remain exact.
pub fn write_mp4_atomic(path: &Path, patch: &TrackPatch) -> Result<TrackWriteOutcome, ApiError> {
    let original_bytes = fs::read(path)?;
    let original_media = mp4_mdat_payloads(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid MP4 atom structure".to_string()))?;
    let before = read_track_metadata(path)?;
    let mut file = File::open(path)?;
    let mut parsed = Mp4File::read_from(&mut file, ParseOptions::new().read_properties(false))?;
    let ilst = parsed
        .ilst_mut()
        .ok_or_else(|| ApiError::MediaSafety("MP4 has no ilst metadata atom".to_string()))?;
    apply_mp4_patch(ilst, patch);

    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::copy(path, &temporary)?;
        parsed.save_to_path(&temporary, WriteOptions::new())?;
        let candidate_bytes = fs::read(&temporary)?;
        let candidate_media = mp4_mdat_payloads(&candidate_bytes).ok_or_else(|| {
            ApiError::MediaSafety("invalid written MP4 atom structure".to_string())
        })?;
        if candidate_media != original_media {
            return Err(ApiError::MediaSafety(
                "MP4 mdat payload changed during metadata write".to_string(),
            ));
        }
        let after = read_track_metadata(&temporary)?;
        if same_metadata(before, after) {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Write OGG Vorbis or true Opus through a validated sibling. Page layout and
/// CRC may change, but every logical encoded-audio packet must remain exact.
pub fn write_ogg_atomic(path: &Path, patch: &TrackPatch) -> Result<TrackWriteOutcome, ApiError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let header_packets = if extension == "opus" { 2 } else { 3 };
    let original_bytes = fs::read(path)?;
    let original_audio = ogg_audio_packets(&original_bytes, header_packets)
        .ok_or_else(|| ApiError::MediaSafety("invalid OGG packet structure".to_string()))?;
    let before = read_track_metadata(path)?;
    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::copy(path, &temporary)?;
        let mut file = File::open(path)?;
        let options = ParseOptions::new().read_properties(false);
        if extension == "opus" {
            let mut parsed = OpusFile::read_from(&mut file, options)?;
            apply_vorbis_patch(parsed.vorbis_comments_mut(), patch);
            parsed.save_to_path(&temporary, WriteOptions::new())?;
        } else {
            let mut parsed = VorbisFile::read_from(&mut file, options)?;
            apply_vorbis_patch(parsed.vorbis_comments_mut(), patch);
            parsed.save_to_path(&temporary, WriteOptions::new())?;
        }
        let candidate_bytes = fs::read(&temporary)?;
        let candidate_audio =
            ogg_audio_packets(&candidate_bytes, header_packets).ok_or_else(|| {
                ApiError::MediaSafety("invalid written OGG packet structure".to_string())
            })?;
        if candidate_audio != original_audio {
            return Err(ApiError::MediaSafety(
                "OGG audio packets changed during metadata write".to_string(),
            ));
        }
        let after = read_track_metadata(&temporary)?;
        if same_metadata_ignoring_container_size(before, after) {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Write one FLAC through a validated sibling file. Unknown comments and
/// pictures remain owned by Lofty's format-specific `FlacFile` representation.
pub fn write_flac_atomic(path: &Path, patch: &TrackPatch) -> Result<TrackWriteOutcome, ApiError> {
    let original_bytes = fs::read(path)?;
    let (prepared, repairs) = prepare_flac_source(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid FLAC metadata boundary".to_string()))?;
    let original_payload = flac_audio_payload(&prepared)
        .ok_or_else(|| ApiError::MediaSafety("invalid prepared FLAC boundary".to_string()))?;
    let original_audio_offset = prepared.len() - original_payload.len();
    let original_payload = original_payload.to_vec();
    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::write(&temporary, &prepared)?;
        let before = read_track_metadata(&temporary)?;
        let flac = read_flac(&temporary)?;
        let mut comments = flac.vorbis_comments().cloned().unwrap_or_default();
        apply_vorbis_patch(&mut comments, patch);
        comments.save_to_path(&temporary, WriteOptions::new())?;
        let candidate_bytes = fs::read(&temporary)?;
        let candidate_payload = flac_audio_payload(&candidate_bytes).ok_or_else(|| {
            ApiError::MediaSafety("invalid written FLAC metadata boundary".to_string())
        })?;
        if candidate_payload != original_payload {
            return Err(ApiError::MediaSafety(
                "FLAC audio payload changed during metadata write".to_string(),
            ));
        }
        if !repairs.force_full_rewrite {
            if let Some(repacked) =
                repack_flac_metadata(&candidate_bytes, original_audio_offset, &original_payload)
            {
                fs::write(&temporary, repacked)?;
            }
        }
        let after = read_track_metadata(&temporary)?;
        if !repairs.any() && same_metadata(before, after) {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Write one MP3 through a validated sibling file. The original path is not
/// touched until tag readback and MPEG payload equality both pass.
pub fn write_mp3_atomic(path: &Path, patch: &TrackPatch) -> Result<TrackWriteOutcome, ApiError> {
    let original_bytes = fs::read(path)?;
    let original_payload = mpeg_payload(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid ID3v2 boundary".to_string()))?;
    let before = read_track_metadata(path)?;
    let mut tag = read_id3v2(path)?;
    preserve_omitted_list(&mut tag, path, "ARTISTS", &patch.artists);
    preserve_omitted_list(&mut tag, path, "ALBUMARTISTS", &patch.album_artists);
    apply_patch(&mut tag, patch);

    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::copy(path, &temporary)?;
        tag.save_to_path(&temporary, WriteOptions::new())?;

        let candidate_bytes = fs::read(&temporary)?;
        let candidate_payload = mpeg_payload(&candidate_bytes)
            .ok_or_else(|| ApiError::MediaSafety("invalid written ID3v2 boundary".to_string()))?;
        if candidate_payload != original_payload {
            return Err(ApiError::MediaSafety(
                "MP3 audio payload changed during metadata write".to_string(),
            ));
        }

        let after = read_track_metadata(&temporary)?;
        if same_metadata(before, after) {
            return Ok(TrackWriteOutcome::Skipped);
        }
        replace_file_atomic(&temporary, path)?;
        Ok(TrackWriteOutcome::Replaced)
    })();

    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_flac(path: &Path) -> Result<FlacFile, ApiError> {
    let mut file = File::open(path)?;
    Ok(FlacFile::read_from(
        &mut file,
        ParseOptions::new().read_properties(false),
    )?)
}

fn read_id3v2(path: &Path) -> Result<Id3v2Tag, ApiError> {
    let mut file = File::open(path)?;
    let parsed = MpegFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
    Ok(parsed.id3v2().cloned().unwrap_or_default())
}

fn apply_ape_patch(tag: &mut ApeTag, patch: &TrackPatch) -> Result<(), ApiError> {
    apply_ape_text(tag, "TITLE", &patch.title)?;
    apply_ape_text(tag, "ALBUM", &patch.album)?;
    apply_ape_text(tag, "ALBUM ARTIST", &patch.album_artist)?;
    apply_ape_text(tag, "DATE", &patch.year)?;
    apply_ape_text(tag, "GENRE", &patch.genre)?;
    apply_ape_text(tag, "COMPOSER", &patch.composer)?;
    apply_ape_text(tag, "COMMENT", &patch.comment)?;
    apply_ape_text(tag, "DESCRIPTION", &patch.description)?;
    apply_ape_text(tag, "LYRICS", &patch.lyrics)?;
    apply_ape_merged_list(tag, "ARTIST", &patch.artist, &patch.artists)?;
    apply_ape_merged_list(
        tag,
        "ALBUM ARTIST",
        &patch.album_artist,
        &patch.album_artists,
    )?;
    apply_ape_bool(tag, "COMPILATION", &patch.compilation)?;
    apply_ape_provider(tag, "MUSICBRAINZ_TRACKID", &patch.musicbrainz_track_id)?;
    apply_ape_provider(tag, "MUSICBRAINZ_ALBUMID", &patch.musicbrainz_album_id)?;
    apply_ape_provider(tag, "MUSICBRAINZ_ARTISTID", &patch.musicbrainz_artist_id)?;
    apply_ape_provider(tag, "DISCOGS_ARTIST_ID", &patch.discogs_artist_id)?;
    apply_ape_provider(tag, "DISCOGS_RELEASE_ID", &patch.discogs_release_id)?;
    apply_ape_position(tag, "TRACK", &patch.track_number, &patch.track_total)?;
    apply_ape_position(tag, "DISC", &patch.disc_number, &patch.disc_total)?;
    Ok(())
}

fn apply_ape_text(tag: &mut ApeTag, key: &str, patch: &Patch<String>) -> Result<(), ApiError> {
    match patch {
        Patch::Omitted => {}
        Patch::Null => tag.remove(key),
        Patch::Value(value) if value.is_empty() => tag.remove(key),
        Patch::Value(value) => tag.insert(ApeItem::new(
            key.to_string(),
            ItemValue::Text(value.clone()),
        )?),
    }
    Ok(())
}

fn apply_ape_provider(
    tag: &mut ApeTag,
    canonical_key: &str,
    patch: &Patch<String>,
) -> Result<(), ApiError> {
    if matches!(patch, Patch::Omitted) {
        return Ok(());
    }
    let canonical = normalize_provider_key(canonical_key);
    let aliases = (&*tag)
        .into_iter()
        .filter(|item| normalize_provider_key(item.key()) == canonical)
        .map(|item| item.key().to_string())
        .collect::<Vec<_>>();
    for alias in aliases {
        tag.remove(&alias);
    }
    if let Patch::Value(value) = patch {
        if !value.is_empty() {
            tag.insert(ApeItem::new(
                canonical_key.to_string(),
                ItemValue::Text(value.clone()),
            )?);
        }
    }
    Ok(())
}

fn apply_ape_merged_list(
    tag: &mut ApeTag,
    key: &str,
    primary: &Patch<String>,
    list: &Patch<StringList>,
) -> Result<(), ApiError> {
    if matches!(primary, Patch::Omitted) && matches!(list, Patch::Omitted) {
        return Ok(());
    }
    let mut values = Vec::new();
    if let Patch::Value(value) = primary {
        if !value.is_empty() {
            values.push(value.clone());
        }
    }
    if let Patch::Value(list) = list {
        for value in list.normalized() {
            if !values.contains(&value) {
                values.push(value);
            }
        }
    }
    tag.remove(key);
    if !values.is_empty() {
        tag.insert(ApeItem::new(
            key.to_string(),
            ItemValue::Text(values.join("\0")),
        )?);
    }
    Ok(())
}

fn apply_ape_bool(tag: &mut ApeTag, key: &str, patch: &Patch<bool>) -> Result<(), ApiError> {
    match patch {
        Patch::Omitted => {}
        Patch::Null => tag.remove(key),
        Patch::Value(value) => tag.insert(ApeItem::new(
            key.to_string(),
            ItemValue::Text(if *value { "1" } else { "0" }.to_string()),
        )?),
    }
    Ok(())
}

fn apply_ape_position(
    tag: &mut ApeTag,
    key: &str,
    number: &Patch<u32>,
    total: &Patch<u32>,
) -> Result<(), ApiError> {
    match number {
        Patch::Omitted => {}
        Patch::Null => tag.remove(key),
        Patch::Value(number) => {
            let value = match total {
                Patch::Value(total) => format!("{number}/{total}"),
                _ => number.to_string(),
            };
            tag.insert(ApeItem::new(key.to_string(), ItemValue::Text(value))?);
        }
    }
    Ok(())
}

fn apply_mp4_patch(tag: &mut Ilst, patch: &TrackPatch) {
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"\xa9nam"), &patch.title);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"\xa9ART"), &patch.artist);
    apply_mp4_freeform_list(tag, "ARTISTS", &patch.artists);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"\xa9alb"), &patch.album);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"aART"), &patch.album_artist);
    apply_mp4_freeform_list(tag, "ALBUMARTISTS", &patch.album_artists);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"\xa9day"), &patch.year);
    apply_mp4_number_pair(tag, patch);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"\xa9gen"), &patch.genre);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"\xa9wrt"), &patch.composer);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"\xa9cmt"), &patch.comment);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"desc"), &patch.description);
    apply_mp4_text(tag, AtomIdent::Fourcc(*b"\xa9lyr"), &patch.lyrics);
    match patch.compilation {
        Patch::Omitted => {}
        Patch::Null => drop(tag.remove(&AtomIdent::Fourcc(*b"cpil"))),
        Patch::Value(value) => tag.set_flag(AtomIdent::Fourcc(*b"cpil"), value),
    }
    apply_mp4_freeform(tag, "MusicBrainz Track Id", &patch.musicbrainz_track_id);
    apply_mp4_freeform(tag, "MusicBrainz Album Id", &patch.musicbrainz_album_id);
    apply_mp4_freeform(tag, "MusicBrainz Artist Id", &patch.musicbrainz_artist_id);
    apply_mp4_freeform(tag, "Discogs Artist Id", &patch.discogs_artist_id);
    apply_mp4_freeform(tag, "Discogs Release Id", &patch.discogs_release_id);
}

fn apply_mp4_text(tag: &mut Ilst, ident: AtomIdent<'static>, patch: &Patch<String>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => drop(tag.remove(&ident)),
        Patch::Value(value) => tag.replace_atom(Atom::new(ident, AtomData::UTF8(value.clone()))),
    }
}

fn mp4_freeform(name: &str) -> AtomIdent<'static> {
    AtomIdent::Freeform {
        mean: Cow::Borrowed("com.apple.iTunes"),
        name: Cow::Owned(name.to_string()),
    }
}

fn apply_mp4_freeform(tag: &mut Ilst, name: &str, patch: &Patch<String>) {
    apply_mp4_text(tag, mp4_freeform(name), patch);
}

fn apply_mp4_freeform_list(tag: &mut Ilst, name: &str, patch: &Patch<StringList>) {
    let ident = mp4_freeform(name);
    match patch {
        Patch::Omitted => {}
        Patch::Null => drop(tag.remove(&ident)),
        Patch::Value(values) => {
            let data = values
                .normalized()
                .into_iter()
                .map(AtomData::UTF8)
                .collect::<Vec<_>>();
            if let Some(atom) = Atom::from_collection(ident.clone(), data) {
                tag.replace_atom(atom);
            } else {
                drop(tag.remove(&ident));
            }
        }
    }
}

fn apply_mp4_number_pair(tag: &mut Ilst, patch: &TrackPatch) {
    match patch.track_number {
        Patch::Omitted => {}
        Patch::Null => tag.remove_track(),
        Patch::Value(value) => tag.set_track(value),
    }
    match patch.track_total {
        Patch::Omitted => {}
        Patch::Null => tag.remove_track_total(),
        Patch::Value(value) => tag.set_track_total(value),
    }
    match patch.disc_number {
        Patch::Omitted => {}
        Patch::Null => tag.remove_disk(),
        Patch::Value(value) => tag.set_disk(value),
    }
    match patch.disc_total {
        Patch::Omitted => {}
        Patch::Null => tag.remove_disk_total(),
        Patch::Value(value) => tag.set_disk_total(value),
    }
}

fn apply_vorbis_patch(tag: &mut lofty::ogg::VorbisComments, patch: &TrackPatch) {
    apply_vorbis_string(tag, "TITLE", &patch.title);
    apply_vorbis_string(tag, "ARTIST", &patch.artist);
    apply_vorbis_list(tag, "ARTISTS", &patch.artists);
    apply_vorbis_string(tag, "ALBUM", &patch.album);
    apply_vorbis_string(tag, "ALBUMARTIST", &patch.album_artist);
    if matches!(patch.album_artists, Patch::Omitted) {
        match &patch.album_artist {
            Patch::Omitted => {}
            Patch::Null => drop(tag.remove("ALBUMARTISTS")),
            Patch::Value(value) => tag.insert("ALBUMARTISTS".to_string(), value.clone()),
        }
    } else {
        apply_vorbis_list(tag, "ALBUMARTISTS", &patch.album_artists);
    }
    apply_vorbis_string(tag, "DATE", &patch.year);
    apply_vorbis_number(tag, "TRACKNUMBER", &patch.track_number);
    apply_vorbis_number(tag, "TRACKTOTAL", &patch.track_total);
    apply_vorbis_number(tag, "DISCNUMBER", &patch.disc_number);
    apply_vorbis_number(tag, "DISCTOTAL", &patch.disc_total);
    apply_vorbis_string(tag, "GENRE", &patch.genre);
    apply_vorbis_string(tag, "COMPOSER", &patch.composer);
    apply_vorbis_string(tag, "COMMENT", &patch.comment);
    apply_vorbis_string(tag, "DESCRIPTION", &patch.description);
    apply_vorbis_string(tag, "LYRICS", &patch.lyrics);
    apply_vorbis_bool(tag, "COMPILATION", &patch.compilation);
    apply_vorbis_provider(tag, "MUSICBRAINZ_TRACKID", &patch.musicbrainz_track_id);
    apply_vorbis_provider(tag, "MUSICBRAINZ_ALBUMID", &patch.musicbrainz_album_id);
    apply_vorbis_provider(tag, "MUSICBRAINZ_ARTISTID", &patch.musicbrainz_artist_id);
    apply_vorbis_provider(tag, "DISCOGS_ARTIST_ID", &patch.discogs_artist_id);
    apply_vorbis_provider(tag, "DISCOGS_RELEASE_ID", &patch.discogs_release_id);
}

fn apply_vorbis_string(tag: &mut lofty::ogg::VorbisComments, key: &str, patch: &Patch<String>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => drop(tag.remove(key)),
        Patch::Value(value) => tag.insert(key.to_string(), value.clone()),
    }
}

fn apply_vorbis_list(tag: &mut lofty::ogg::VorbisComments, key: &str, patch: &Patch<StringList>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => drop(tag.remove(key)),
        Patch::Value(values) => {
            drop(tag.remove(key));
            for value in values.normalized() {
                tag.push(key.to_string(), value);
            }
        }
    }
}

fn apply_vorbis_number(tag: &mut lofty::ogg::VorbisComments, key: &str, patch: &Patch<u32>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => drop(tag.remove(key)),
        Patch::Value(value) => tag.insert(key.to_string(), value.to_string()),
    }
}

fn apply_vorbis_provider(
    tag: &mut lofty::ogg::VorbisComments,
    canonical_key: &str,
    patch: &Patch<String>,
) {
    if matches!(patch, Patch::Omitted) {
        return;
    }
    let canonical = normalize_provider_key(canonical_key);
    let aliases = tag
        .items()
        .filter(|(key, _)| normalize_provider_key(key) == canonical)
        .map(|(key, _)| key.to_string())
        .collect::<Vec<_>>();
    for alias in aliases {
        drop(tag.remove(&alias));
    }
    if let Patch::Value(value) = patch {
        if !value.is_empty() {
            tag.insert(canonical_key.to_string(), value.clone());
        }
    }
}

fn normalize_provider_key(key: &str) -> String {
    let key = if key
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("TXXX:"))
    {
        &key[5..]
    } else {
        key
    };
    let normalized = key
        .chars()
        .filter(|character| !matches!(character, ' ' | '_' | '-'))
        .collect::<String>()
        .to_ascii_uppercase();
    normalized
        .strip_prefix("MUSICBRAINS")
        .map_or(normalized.clone(), |suffix| format!("MUSICBRAINZ{suffix}"))
}

fn apply_vorbis_bool(tag: &mut lofty::ogg::VorbisComments, key: &str, patch: &Patch<bool>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => drop(tag.remove(key)),
        Patch::Value(value) => {
            tag.insert(key.to_string(), if *value { "1" } else { "0" }.to_string())
        }
    }
}

fn apply_patch(tag: &mut Id3v2Tag, patch: &TrackPatch) {
    match &patch.title {
        Patch::Omitted => {}
        Patch::Null => tag.remove_title(),
        Patch::Value(value) => tag.set_title(value.clone()),
    }
    match &patch.artist {
        Patch::Omitted => {}
        Patch::Null => tag.remove_artist(),
        Patch::Value(value) => tag.set_artist(value.clone()),
    }
    match &patch.album {
        Patch::Omitted => {}
        Patch::Null => tag.remove_album(),
        Patch::Value(value) => tag.set_album(value.clone()),
    }
    apply_text_frame(tag, "TPE2", &patch.album_artist);
    apply_year(tag, &patch.year);
    apply_text_frame(tag, "TCOM", &patch.composer);
    match &patch.genre {
        Patch::Omitted => {}
        Patch::Null => tag.remove_genre(),
        Patch::Value(value) => tag.set_genre(value.clone()),
    }
    match &patch.comment {
        Patch::Omitted => {}
        Patch::Null => tag.remove_comment(),
        Patch::Value(value) => tag.set_comment(value.clone()),
    }
    match patch.track_number {
        Patch::Omitted => {}
        Patch::Null => tag.remove_track(),
        Patch::Value(value) => tag.set_track(value),
    }
    match patch.track_total {
        Patch::Omitted => {}
        Patch::Null => tag.remove_track_total(),
        Patch::Value(value) => tag.set_track_total(value),
    }
    match patch.disc_number {
        Patch::Omitted => {}
        Patch::Null => tag.remove_disk(),
        Patch::Value(value) => tag.set_disk(value),
    }
    match patch.disc_total {
        Patch::Omitted => {}
        Patch::Null => tag.remove_disk_total(),
        Patch::Value(value) => tag.set_disk_total(value),
    }
    apply_list(tag, "ARTISTS", &patch.artists);
    apply_list(tag, "ALBUMARTISTS", &patch.album_artists);
    apply_user_text(tag, "DESCRIPTION", &patch.description);
    apply_user_text(tag, "MusicBrainz Track Id", &patch.musicbrainz_track_id);
    apply_user_text(tag, "MusicBrainz Album Id", &patch.musicbrainz_album_id);
    apply_user_text(tag, "MusicBrainz Artist Id", &patch.musicbrainz_artist_id);
    apply_user_text(tag, "Discogs Artist Id", &patch.discogs_artist_id);
    apply_user_text(tag, "Discogs Release Id", &patch.discogs_release_id);
    apply_compilation(tag, &patch.compilation);
    apply_lyrics(tag, &patch.lyrics);
}

fn frame_id(id: &'static str) -> FrameId<'static> {
    FrameId::Valid(Cow::Borrowed(id))
}

fn apply_year(tag: &mut Id3v2Tag, patch: &Patch<String>) {
    if matches!(patch, Patch::Omitted) {
        return;
    }
    drop(tag.remove(&frame_id("TYER")));
    drop(tag.remove(&frame_id("TDRC")));
    if let Patch::Value(value) = patch {
        tag.insert(Frame::Text(TextInformationFrame::new(
            frame_id("TDRC"),
            TextEncoding::UTF8,
            value.clone(),
        )));
    }
}

fn apply_text_frame(tag: &mut Id3v2Tag, id: &'static str, patch: &Patch<String>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => {
            drop(tag.remove(&frame_id(id)));
        }
        Patch::Value(value) => {
            tag.insert(Frame::Text(TextInformationFrame::new(
                frame_id(id),
                TextEncoding::UTF8,
                value.clone(),
            )));
        }
    }
}

fn apply_user_text(tag: &mut Id3v2Tag, description: &str, patch: &Patch<String>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => {
            tag.remove_user_text(description);
        }
        Patch::Value(value) if value.is_empty() => {
            tag.remove_user_text(description);
        }
        Patch::Value(value) => {
            tag.insert_user_text(description.to_string(), value.clone());
        }
    }
}

fn preserve_omitted_list(
    tag: &mut Id3v2Tag,
    path: &Path,
    description: &str,
    patch: &Patch<StringList>,
) {
    if !matches!(patch, Patch::Omitted) {
        return;
    }
    let values = id3_user_text_values(path, description);
    if !values.is_empty() {
        tag.remove_user_text(description);
        tag.insert_user_text(description.to_string(), values.join(";"));
    }
}

fn apply_list(tag: &mut Id3v2Tag, description: &str, patch: &Patch<StringList>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => {
            tag.remove_user_text(description);
        }
        Patch::Value(values) => {
            let values = values.normalized();
            if values.is_empty() {
                tag.remove_user_text(description);
            } else {
                tag.insert_user_text(description.to_string(), values.join(";"));
            }
        }
    }
}

fn apply_compilation(tag: &mut Id3v2Tag, patch: &Patch<bool>) {
    match patch {
        Patch::Omitted => {}
        Patch::Null => {
            tag.remove_user_text("COMPILATION");
        }
        Patch::Value(value) => {
            tag.insert_user_text(
                "COMPILATION".to_string(),
                if *value { "1" } else { "0" }.to_string(),
            );
        }
    }
}

fn apply_lyrics(tag: &mut Id3v2Tag, patch: &Patch<String>) {
    if matches!(patch, Patch::Omitted) {
        return;
    }
    drop(tag.remove(&frame_id("USLT")));
    if let Patch::Value(value) = patch {
        if !value.is_empty() {
            tag.insert(Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(
                TextEncoding::UTF8,
                *b"eng",
                "",
                value.clone(),
            )));
        }
    }
}

fn same_metadata(before: TrackData, mut after: TrackData) -> bool {
    after.path.clone_from(&before.path);
    after.size_bytes = before.size_bytes;
    before == after
}

fn same_metadata_ignoring_container_size(before: TrackData, mut after: TrackData) -> bool {
    after.path.clone_from(&before.path);
    after.size_bytes = before.size_bytes;
    after.bitrate = before.bitrate;
    before == after
}

fn ape_audio_core(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 52 || bytes.get(..4)? != b"MAC " {
        return None;
    }
    let fields = [8_usize, 12, 16, 20, 24, 32];
    let mut audio_end = 0_usize;
    for offset in fields {
        let value = u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize;
        audio_end = audio_end.checked_add(value)?;
    }
    (audio_end > 0).then(|| bytes.get(..audio_end)).flatten()
}

fn wav_data_payloads(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    if bytes.len() < 12 || bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut payloads = Vec::new();
    let mut offset = 12_usize;
    while offset.checked_add(8)? <= bytes.len() {
        let id = bytes.get(offset..offset + 4)?;
        let size = u32::from_le_bytes(bytes.get(offset + 4..offset + 8)?.try_into().ok()?) as usize;
        let data_start = offset.checked_add(8)?;
        let data_end = data_start.checked_add(size)?;
        if data_end > bytes.len() {
            return None;
        }
        if id == b"data" {
            payloads.push(bytes.get(data_start..data_end)?.to_vec());
        }
        offset = data_end.checked_add(size % 2)?;
    }
    (offset == bytes.len() && !payloads.is_empty()).then_some(payloads)
}

fn mp4_mdat_payloads(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut payloads = Vec::new();
    let mut offset = 0_usize;
    while offset.checked_add(8)? <= bytes.len() {
        let size32 = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?);
        let kind = bytes.get(offset + 4..offset + 8)?;
        let (header, size) = if size32 == 1 {
            (
                16_usize,
                usize::try_from(u64::from_be_bytes(
                    bytes.get(offset + 8..offset + 16)?.try_into().ok()?,
                ))
                .ok()?,
            )
        } else if size32 == 0 {
            (8_usize, bytes.len().checked_sub(offset)?)
        } else {
            (8_usize, size32 as usize)
        };
        if size < header {
            return None;
        }
        let end = offset.checked_add(size)?;
        if end > bytes.len() {
            return None;
        }
        if kind == b"mdat" {
            payloads.push(bytes.get(offset + header..end)?.to_vec());
        }
        offset = end;
    }
    (offset == bytes.len() && !payloads.is_empty()).then_some(payloads)
}

fn ogg_audio_packets(bytes: &[u8], header_packets: usize) -> Option<Vec<Vec<u8>>> {
    let mut packets = Vec::new();
    let mut packet = Vec::new();
    let mut offset = 0_usize;
    while offset.checked_add(27)? <= bytes.len() {
        if bytes.get(offset..offset + 4)? != b"OggS" {
            return None;
        }
        let segment_count = usize::from(*bytes.get(offset + 26)?);
        let table_start = offset.checked_add(27)?;
        let data_start = table_start.checked_add(segment_count)?;
        let table = bytes.get(table_start..data_start)?;
        let mut data_offset = data_start;
        for segment in table {
            let length = usize::from(*segment);
            let next = data_offset.checked_add(length)?;
            packet.extend_from_slice(bytes.get(data_offset..next)?);
            data_offset = next;
            if length < 255 {
                packets.push(std::mem::take(&mut packet));
            }
        }
        offset = data_offset;
    }
    if offset != bytes.len() || !packet.is_empty() || packets.len() < header_packets {
        return None;
    }
    Some(packets.into_iter().skip(header_packets).collect())
}

#[derive(Debug, Default)]
struct FlacRepairs {
    trailing_ape: bool,
    ghost_vorbis: bool,
    duplicate_vorbis: bool,
    force_full_rewrite: bool,
}

impl FlacRepairs {
    fn any(&self) -> bool {
        self.trailing_ape || self.ghost_vorbis || self.duplicate_vorbis
    }
}

fn prepare_flac_source(original: &[u8]) -> Option<(Vec<u8>, FlacRepairs)> {
    let (without_ape, trailing_ape) = strip_trailing_apev2(original)?;
    let mut prepared = without_ape.to_vec();
    let audio_offset = flac_audio_offset(&prepared)?;
    let ghost_vorbis = neutralize_ghost_vorbis(&mut prepared, audio_offset);
    let duplicate_vorbis = flac_metadata_types(&prepared)?
        .into_iter()
        .filter(|block_type| *block_type == 4)
        .count()
        > 1;
    Some((
        prepared,
        FlacRepairs {
            trailing_ape,
            ghost_vorbis,
            duplicate_vorbis,
            force_full_rewrite: trailing_ape || ghost_vorbis,
        },
    ))
}

fn strip_trailing_apev2(bytes: &[u8]) -> Option<(&[u8], bool)> {
    if bytes.len() < 32 || bytes.get(bytes.len() - 32..bytes.len() - 24)? != b"APETAGEX" {
        return Some((bytes, false));
    }
    let footer = bytes.len() - 32;
    let tag_size =
        u32::from_le_bytes(bytes.get(footer + 12..footer + 16)?.try_into().ok()?) as usize;
    if tag_size < 32 || tag_size > bytes.len() {
        return None;
    }
    let mut start = bytes.len().checked_sub(tag_size)?;
    if start >= 32 && bytes.get(start - 32..start - 24) == Some(b"APETAGEX") {
        let flags = u32::from_le_bytes(bytes.get(start - 12..start - 8)?.try_into().ok()?);
        if flags & 0x2000_0000 != 0 {
            start -= 32;
        }
    }
    Some((bytes.get(..start)?, true))
}

fn neutralize_ghost_vorbis(bytes: &mut [u8], audio_offset: usize) -> bool {
    const VENDOR: &[u8] = b"auto-tagger";
    let mut search = audio_offset;
    let mut found = false;
    while search <= bytes.len().saturating_sub(VENDOR.len()) {
        let Some(relative) = bytes[search..]
            .windows(VENDOR.len())
            .position(|window| window == VENDOR)
        else {
            break;
        };
        let position = search + relative;
        if position >= 4 {
            let claimed =
                u32::from_le_bytes(bytes[position - 4..position].try_into().unwrap_or_default());
            if claimed as usize == VENDOR.len() {
                bytes[position - 4..position].fill(0);
                found = true;
            }
        }
        search = position + 1;
    }
    found
}

fn flac_metadata_types(bytes: &[u8]) -> Option<Vec<u8>> {
    let marker = bytes.windows(4).position(|window| window == b"fLaC")?;
    let mut offset = marker.checked_add(4)?;
    let mut types = Vec::new();
    loop {
        let header = bytes.get(offset..offset.checked_add(4)?)?;
        let last = header[0] & 0x80 != 0;
        types.push(header[0] & 0x7f);
        let length =
            (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        offset = offset.checked_add(4)?.checked_add(length)?;
        if offset > bytes.len() {
            return None;
        }
        if last {
            return Some(types);
        }
    }
}

fn flac_audio_offset(bytes: &[u8]) -> Option<usize> {
    let payload = flac_audio_payload(bytes)?;
    bytes.len().checked_sub(payload.len())
}

fn repack_flac_metadata(
    candidate: &[u8],
    target_audio_offset: usize,
    original_payload: &[u8],
) -> Option<Vec<u8>> {
    let marker = candidate.windows(4).position(|window| window == b"fLaC")?;
    let metadata_start = marker.checked_add(4)?;
    let available = target_audio_offset.checked_sub(metadata_start)?;
    let mut blocks = Vec::new();
    let mut saw_vorbis = false;
    let mut offset = metadata_start;
    loop {
        let header_end = offset.checked_add(4)?;
        let header = candidate.get(offset..header_end)?;
        let last = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7f;
        let length =
            (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        let data_start = header_end;
        let data_end = data_start.checked_add(length)?;
        let data = candidate.get(data_start..data_end)?;
        if block_type == 4 {
            if !saw_vorbis {
                blocks.push((block_type, data));
                saw_vorbis = true;
            }
        } else if block_type != 1 {
            blocks.push((block_type, data));
        }
        offset = data_end;
        if last {
            break;
        }
    }
    if blocks.is_empty() {
        return None;
    }
    let required = blocks.iter().try_fold(0_usize, |sum, (_, data)| {
        sum.checked_add(data.len().checked_add(4)?)
    })?;
    let leftover = available.checked_sub(required)?;
    if (1..4).contains(&leftover) || leftover.saturating_sub(4) > 0x00ff_ffff {
        return None;
    }

    let capacity = target_audio_offset.checked_add(original_payload.len())?;
    let mut output = Vec::with_capacity(capacity);
    output.extend_from_slice(candidate.get(..metadata_start)?);
    let has_padding = leftover >= 4;
    for (index, (block_type, data)) in blocks.iter().enumerate() {
        let last = !has_padding && index + 1 == blocks.len();
        push_flac_block(&mut output, *block_type, data, last)?;
    }
    if has_padding {
        let padding = vec![0_u8; leftover - 4];
        push_flac_block(&mut output, 1, &padding, true)?;
    }
    if output.len() != target_audio_offset {
        return None;
    }
    output.extend_from_slice(original_payload);
    Some(output)
}

fn push_flac_block(output: &mut Vec<u8>, block_type: u8, data: &[u8], last: bool) -> Option<()> {
    if data.len() > 0x00ff_ffff {
        return None;
    }
    output.push((if last { 0x80 } else { 0 }) | (block_type & 0x7f));
    output.push(((data.len() >> 16) & 0xff) as u8);
    output.push(((data.len() >> 8) & 0xff) as u8);
    output.push((data.len() & 0xff) as u8);
    output.extend_from_slice(data);
    Some(())
}

fn flac_audio_payload(bytes: &[u8]) -> Option<&[u8]> {
    let marker = bytes.windows(4).position(|window| window == b"fLaC")?;
    let mut offset = marker.checked_add(4)?;
    loop {
        let header_end = offset.checked_add(4)?;
        let header = bytes.get(offset..header_end)?;
        let last = header[0] & 0x80 != 0;
        let length =
            (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        offset = header_end.checked_add(length)?;
        if offset > bytes.len() {
            return None;
        }
        if last {
            return bytes.get(offset..);
        }
    }
}

fn mpeg_payload(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.get(..3) != Some(b"ID3") {
        return Some(bytes);
    }
    let header = bytes.get(..10)?;
    if header[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return None;
    }
    let size = ((header[6] as usize) << 21)
        | ((header[7] as usize) << 14)
        | ((header[8] as usize) << 7)
        | header[9] as usize;
    bytes.get(10_usize.checked_add(size)?..)
}

#[cfg(not(windows))]
fn replace_file_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers that live
    // through the call; flags request an atomic replacement of an existing file.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn sibling_temp_path(path: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("track");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("tmp");
    path.with_file_name(format!(
        ".{name}.auto-tagger-{}-{sequence}.tmp.{extension}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::id3::v2::BinaryFrame;

    fn media_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus")
            .join(name)
            .canonicalize()
            .unwrap()
    }

    fn fixture() -> PathBuf {
        media_fixture("minimal.mp3")
    }

    fn copy_fixture() -> (PathBuf, PathBuf) {
        copy_to_temp(&fixture(), "track.mp3")
    }

    fn writer_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/writer-corpus")
            .join(name)
            .canonicalize()
            .unwrap()
    }

    fn copy_flac_fixture() -> (PathBuf, PathBuf) {
        copy_to_temp(&writer_fixture("padded.flac"), "track.flac")
    }

    fn copy_ogg_fixture(name: &str) -> (PathBuf, PathBuf) {
        copy_to_temp(&writer_fixture(name), name)
    }

    fn copy_to_temp(source: &Path, name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-write-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(name);
        fs::copy(source, &path).unwrap();
        (root, path)
    }

    #[test]
    fn tri_state_deserialization_distinguishes_missing_null_and_value() {
        let omitted: TrackPatch = serde_json::from_value(serde_json::json!({})).unwrap();
        let null: TrackPatch = serde_json::from_value(serde_json::json!({"title": null})).unwrap();
        let value: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Changed"})).unwrap();
        assert_eq!(omitted.title, Patch::Omitted);
        assert_eq!(null.title, Patch::Null);
        assert_eq!(value.title, Patch::Value("Changed".to_string()));
    }

    #[test]
    fn identical_patch_is_true_noop_and_preserves_all_bytes() {
        let (root, path) = copy_fixture();
        let before = fs::read(&path).unwrap();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Corpus MP3"})).unwrap();
        assert_eq!(
            write_mp3_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Skipped
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn title_update_preserves_mpeg_payload_and_reads_back() {
        let (root, path) = copy_fixture();
        let before = fs::read(&path).unwrap();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Changed title"})).unwrap();
        assert_eq!(
            write_mp3_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        let after = fs::read(&path).unwrap();
        assert_eq!(mpeg_payload(&before), mpeg_payload(&after));
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Changed title")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_null_clears_while_omitted_preserves() {
        let (root, path) = copy_fixture();
        assert_eq!(
            write_mp3_atomic(&path, &TrackPatch::default()).unwrap(),
            TrackWriteOutcome::Skipped
        );
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Corpus MP3")
        );
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({"title": null})).unwrap();
        write_mp3_atomic(&path, &patch).unwrap();
        assert_eq!(read_track_metadata(&path).unwrap().title, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rich_patch_matches_normalized_electron_readback() {
        let (root, path) = copy_fixture();
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Replacement",
            "artist": "Replacement Artist",
            "artists": ["Primary", "Guest"],
            "album": "Replacement Album",
            "albumArtist": "Replacement Album Artist",
            "albumArtists": ["Replacement Album Artist", "Album Guest"],
            "year": "2030",
            "trackNumber": 7,
            "trackTotal": 9,
            "discNumber": 2,
            "discTotal": 3,
            "genre": "Jazz",
            "composer": "Replacement Composer",
            "comment": "Replacement Comment",
            "description": "Replacement description",
            "lyrics": "Replacement lyrics",
            "compilation": false,
            "musicbrainzTrackId": "replacement-mb-track",
            "musicbrainzAlbumId": "replacement-mb-album",
            "musicbrainzArtistId": "replacement-mb-artist",
            "discogsArtistId": "replacement-discogs-artist",
            "discogsReleaseId": "replacement-discogs-release"
        }))
        .unwrap();
        assert_eq!(
            write_mp3_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title.as_deref(), Some("Replacement"));
        assert_eq!(track.artist.as_deref(), Some("Replacement Artist"));
        assert_eq!(track.artists, ["Primary", "Guest"]);
        assert_eq!(track.album.as_deref(), Some("Replacement Album"));
        assert_eq!(
            track.album_artist.as_deref(),
            Some("Replacement Album Artist")
        );
        assert_eq!(track.year.as_deref(), Some("2030"));
        assert_eq!((track.track_number, track.track_total), (Some(7), Some(9)));
        assert_eq!((track.disc_number, track.disc_total), (Some(2), Some(3)));
        assert_eq!(track.genre.as_deref(), Some("Jazz"));
        assert_eq!(track.composer.as_deref(), Some("Replacement Composer"));
        assert_eq!(track.comment.as_deref(), Some("Replacement Comment"));
        assert_eq!(
            track.description.as_deref(),
            Some("Replacement description")
        );
        assert_eq!(track.lyrics.as_deref(), Some("Replacement lyrics"));
        assert_eq!(
            track.musicbrainz_track_id.as_deref(),
            Some("replacement-mb-track")
        );
        assert_eq!(
            track.musicbrainz_album_id.as_deref(),
            Some("replacement-mb-album")
        );
        assert_eq!(
            track.musicbrainz_artist_id.as_deref(),
            Some("replacement-mb-artist")
        );
        assert_eq!(
            track.discogs_artist_id.as_deref(),
            Some("replacement-discogs-artist")
        );
        assert_eq!(
            track.discogs_release_id.as_deref(),
            Some("replacement-discogs-release")
        );
        assert!(track.has_cover);
        assert_eq!(
            id3_user_text_values(&path, "ALBUMARTISTS"),
            ["Replacement Album Artist", "Album Guest"]
        );
        assert_eq!(
            read_id3v2(&path).unwrap().get_user_text("COMPILATION"),
            Some("0")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_user_text_survives_a_changed_patch() {
        let (root, path) = copy_fixture();
        let mut tag = read_id3v2(&path).unwrap();
        tag.insert_user_text("UNRELATED".to_string(), "keep-me".to_string());
        tag.save_to_path(&path, WriteOptions::new()).unwrap();

        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Changed title"})).unwrap();
        write_mp3_atomic(&path, &patch).unwrap();
        let tag = read_id3v2(&path).unwrap();
        assert_eq!(tag.get_user_text("UNRELATED"), Some("keep-me"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_binary_frame_survives_a_changed_patch() {
        let (root, path) = copy_fixture();
        let unknown_id = frame_id("XABC");
        let mut tag = read_id3v2(&path).unwrap();
        tag.insert(Frame::Binary(BinaryFrame::new(
            unknown_id.clone(),
            vec![1, 2, 3, 4],
        )));
        tag.save_to_path(&path, WriteOptions::new()).unwrap();

        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Changed title"})).unwrap();
        write_mp3_atomic(&path, &patch).unwrap();
        assert!(read_id3v2(&path).unwrap().get(&unknown_id).is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ape_rich_update_preserves_audio_removes_id3v1_and_noops() {
        let (root, path) = copy_to_temp(&media_fixture("ape-id3v1-fallback.ape"), "track.ape");
        let original_core = ape_audio_core(&fs::read(&path).unwrap()).unwrap().to_vec();
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Replacement APE",
            "artist": "Primary",
            "artists": ["Primary", "Guest"],
            "album": "Replacement Album",
            "trackNumber": 7,
            "trackTotal": 9,
            "discNumber": 2,
            "discTotal": 3,
            "musicbrainzAlbumId": "replacement-mb-album",
            "discogsReleaseId": "replacement-discogs-release"
        }))
        .unwrap();
        assert_eq!(
            write_ape_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        let after = fs::read(&path).unwrap();
        assert_eq!(ape_audio_core(&after).unwrap(), original_core);
        assert_ne!(
            after.get(after.len().saturating_sub(128)..after.len().saturating_sub(125)),
            Some(&b"TAG"[..])
        );
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title.as_deref(), Some("Replacement APE"));
        assert_eq!(track.artist.as_deref(), Some("Primary"));
        assert_eq!(track.artists, ["Primary", "Guest"]);
        assert_eq!(track.album.as_deref(), Some("Replacement Album"));
        assert_eq!((track.track_number, track.track_total), (Some(7), Some(9)));
        assert_eq!((track.disc_number, track.disc_total), (Some(2), Some(3)));
        assert_eq!(
            track.musicbrainz_album_id.as_deref(),
            Some("replacement-mb-album")
        );
        assert_eq!(
            track.discogs_release_id.as_deref(),
            Some("replacement-discogs-release")
        );

        let before_noop = fs::read(&path).unwrap();
        assert_eq!(
            write_ape_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Skipped
        );
        assert_eq!(fs::read(&path).unwrap(), before_noop);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ape_null_clears_and_unknown_item_survives() {
        let (root, path) = copy_to_temp(&media_fixture("ape-id3v1-fallback.ape"), "track.ape");
        let mut source = File::open(&path).unwrap();
        let parsed =
            ApeFile::read_from(&mut source, ParseOptions::new().read_properties(false)).unwrap();
        let mut tag = parsed.ape().cloned().unwrap_or_default();
        tag.insert(
            ApeItem::new(
                "UNRELATED".to_string(),
                ItemValue::Text("keep-me".to_string()),
            )
            .unwrap(),
        );
        fs::write(&path, ape_audio_core(&fs::read(&path).unwrap()).unwrap()).unwrap();
        tag.save_to_path(&path, WriteOptions::new()).unwrap();

        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": null, "artist": "Changed"}))
                .unwrap();
        write_ape_atomic(&path, &patch).unwrap();
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title, None);
        assert_eq!(track.artist.as_deref(), Some("Changed"));
        let mut source = File::open(&path).unwrap();
        let parsed =
            ApeFile::read_from(&mut source, ParseOptions::new().read_properties(false)).unwrap();
        assert_eq!(
            parsed
                .ape()
                .unwrap()
                .get("UNRELATED")
                .unwrap()
                .text_values()
                .unwrap()
                .next(),
            Some("keep-me")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wav_add_update_noop_and_null_preserve_pcm() {
        let (root, path) = copy_to_temp(&media_fixture("minimal.wav"), "track.wav");
        let original_audio = wav_data_payloads(&fs::read(&path).unwrap()).unwrap();
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Replacement WAV",
            "artist": "Replacement Artist",
            "trackNumber": 7,
            "trackTotal": 9,
            "musicbrainzAlbumId": "replacement-mb-album",
            "discogsReleaseId": "replacement-discogs-release"
        }))
        .unwrap();
        assert_eq!(
            write_wav_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        assert_eq!(
            wav_data_payloads(&fs::read(&path).unwrap()).unwrap(),
            original_audio
        );
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title.as_deref(), Some("Replacement WAV"));
        assert_eq!(track.artist.as_deref(), Some("Replacement Artist"));
        assert_eq!((track.track_number, track.track_total), (Some(7), Some(9)));
        assert_eq!(
            track.musicbrainz_album_id.as_deref(),
            Some("replacement-mb-album")
        );
        assert_eq!(
            track.discogs_release_id.as_deref(),
            Some("replacement-discogs-release")
        );

        let before_noop = fs::read(&path).unwrap();
        assert_eq!(
            write_wav_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Skipped
        );
        assert_eq!(fs::read(&path).unwrap(), before_noop);

        let clear: TrackPatch = serde_json::from_value(serde_json::json!({"title": null})).unwrap();
        write_wav_atomic(&path, &clear).unwrap();
        assert_eq!(read_track_metadata(&path).unwrap().title, None);
        assert_eq!(
            wav_data_payloads(&fs::read(&path).unwrap()).unwrap(),
            original_audio
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp4_identical_patch_is_true_noop() {
        let (root, path) = copy_to_temp(&media_fixture("minimal.m4a"), "track.m4a");
        let before = fs::read(&path).unwrap();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Corpus Encoded"})).unwrap();
        assert_eq!(
            write_mp4_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Skipped
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp4_rich_patch_preserves_mdat_and_reads_back_validly() {
        for name in ["minimal.m4a", "minimal.mp4"] {
            let (root, path) = copy_to_temp(&media_fixture(name), name);
            let before = mp4_mdat_payloads(&fs::read(&path).unwrap()).unwrap();
            let patch: TrackPatch = serde_json::from_value(serde_json::json!({
                "title": "Replacement MP4",
                "artist": "Replacement Artist",
                "artists": ["Primary", "Guest"],
                "albumArtist": "Replacement Album Artist",
                "trackNumber": 7,
                "trackTotal": 9,
                "discNumber": 2,
                "discTotal": 3,
                "description": "Replacement description",
                "lyrics": "Replacement lyrics",
                "compilation": true,
                "musicbrainzAlbumId": "replacement-mb-album",
                "discogsReleaseId": "replacement-discogs-release"
            }))
            .unwrap();
            assert_eq!(
                write_mp4_atomic(&path, &patch).unwrap(),
                TrackWriteOutcome::Replaced
            );
            assert_eq!(
                mp4_mdat_payloads(&fs::read(&path).unwrap()).unwrap(),
                before
            );
            let track = read_track_metadata(&path).unwrap();
            assert_eq!(track.title.as_deref(), Some("Replacement MP4"));
            assert_eq!(track.artist.as_deref(), Some("Replacement Artist"));
            assert_eq!(track.artists, ["Primary", "Guest"]);
            assert_eq!(
                track.album_artist.as_deref(),
                Some("Replacement Album Artist")
            );
            assert_eq!((track.track_number, track.track_total), (Some(7), Some(9)));
            assert_eq!((track.disc_number, track.disc_total), (Some(2), Some(3)));
            assert_eq!(
                track.description.as_deref(),
                Some("Replacement description")
            );
            assert_eq!(track.lyrics.as_deref(), Some("Replacement lyrics"));
            assert_eq!(track.compilation, Some(true));
            assert_eq!(
                track.musicbrainz_album_id.as_deref(),
                Some("replacement-mb-album")
            );
            assert_eq!(
                track.discogs_release_id.as_deref(),
                Some("replacement-discogs-release")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn mp4_unknown_freeform_atom_survives_changed_patch() {
        let (root, path) = copy_to_temp(&media_fixture("minimal.m4a"), "track.m4a");
        let unknown = mp4_freeform("UNRELATED");
        let mut source = File::open(&path).unwrap();
        let mut parsed =
            Mp4File::read_from(&mut source, ParseOptions::new().read_properties(false)).unwrap();
        parsed.ilst_mut().unwrap().replace_atom(Atom::new(
            unknown.clone(),
            AtomData::UTF8("keep-me".to_string()),
        ));
        parsed.save_to_path(&path, WriteOptions::new()).unwrap();

        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Changed"})).unwrap();
        write_mp4_atomic(&path, &patch).unwrap();
        let mut source = File::open(&path).unwrap();
        let parsed =
            Mp4File::read_from(&mut source, ParseOptions::new().read_properties(false)).unwrap();
        assert!(parsed.ilst().unwrap().get(&unknown).is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ogg_identical_patch_is_true_noop() {
        let (root, path) = copy_ogg_fixture("vorbis.ogg");
        let before = fs::read(&path).unwrap();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Corpus Encoded"})).unwrap();
        assert_eq!(
            write_ogg_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Skipped
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ogg_rich_patch_preserves_logical_audio_packets() {
        let (root, path) = copy_ogg_fixture("vorbis.ogg");
        let before = ogg_audio_packets(&fs::read(&path).unwrap(), 3).unwrap();
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Replacement OGG",
            "artist": "Replacement Artist",
            "trackNumber": 7,
            "trackTotal": 9,
            "discogsReleaseId": "replacement-discogs-release"
        }))
        .unwrap();
        assert_eq!(
            write_ogg_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        assert_eq!(
            ogg_audio_packets(&fs::read(&path).unwrap(), 3).unwrap(),
            before
        );
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title.as_deref(), Some("Replacement OGG"));
        assert_eq!(track.artist.as_deref(), Some("Replacement Artist"));
        assert_eq!((track.track_number, track.track_total), (Some(7), Some(9)));
        assert_eq!(
            track.discogs_release_id.as_deref(),
            Some("replacement-discogs-release")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn true_opus_patch_updates_tags_and_preserves_audio_packets() {
        let (root, path) = copy_ogg_fixture("opus.opus");
        let before = ogg_audio_packets(&fs::read(&path).unwrap(), 2).unwrap();
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Replacement Opus",
            "artists": ["Primary", "Guest"],
            "musicbrainzTrackId": "replacement-mb-track"
        }))
        .unwrap();
        assert_eq!(
            write_ogg_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        assert_eq!(
            ogg_audio_packets(&fs::read(&path).unwrap(), 2).unwrap(),
            before
        );
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title.as_deref(), Some("Replacement Opus"));
        assert_eq!(track.artists, ["Primary", "Guest"]);
        assert_eq!(
            track.musicbrainz_track_id.as_deref(),
            Some("replacement-mb-track")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_identical_patch_is_true_noop() {
        let (root, path) = copy_flac_fixture();
        let before = fs::read(&path).unwrap();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Corpus Encoded"})).unwrap();
        assert_eq!(
            write_flac_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Skipped
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_rich_patch_preserves_padded_boundary_and_audio() {
        let (root, path) = copy_flac_fixture();
        let before = fs::read(&path).unwrap();
        let before_payload = flac_audio_payload(&before).unwrap();
        let before_offset = before.len() - before_payload.len();
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Replacement FLAC title",
            "artist": "Replacement Artist",
            "trackNumber": 7,
            "trackTotal": 9,
            "discNumber": 2,
            "discTotal": 3,
            "musicbrainzAlbumId": "replacement-mb-album",
            "discogsReleaseId": "replacement-discogs-release"
        }))
        .unwrap();
        assert_eq!(
            write_flac_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        let after = fs::read(&path).unwrap();
        let after_payload = flac_audio_payload(&after).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(after.len() - after_payload.len(), before_offset);
        assert_eq!(after_payload, before_payload);
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title.as_deref(), Some("Replacement FLAC title"));
        assert_eq!(track.artist.as_deref(), Some("Replacement Artist"));
        assert_eq!((track.track_number, track.track_total), (Some(7), Some(9)));
        assert_eq!((track.disc_number, track.disc_total), (Some(2), Some(3)));
        assert_eq!(
            track.musicbrainz_album_id.as_deref(),
            Some("replacement-mb-album")
        );
        assert_eq!(
            track.discogs_release_id.as_deref(),
            Some("replacement-discogs-release")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_null_clears_and_unknown_comment_survives() {
        let (root, path) = copy_flac_fixture();
        let mut flac = read_flac(&path).unwrap();
        flac.vorbis_comments_mut()
            .unwrap()
            .insert("UNRELATED".to_string(), "keep-me".to_string());
        flac.save_to_path(&path, WriteOptions::new()).unwrap();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": null, "artist": "Changed"}))
                .unwrap();
        write_flac_atomic(&path, &patch).unwrap();
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title, None);
        assert_eq!(track.artist.as_deref(), Some("Changed"));
        assert_eq!(
            read_flac(&path)
                .unwrap()
                .vorbis_comments()
                .unwrap()
                .get("UNRELATED"),
            Some("keep-me")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_trailing_ape_is_removed_with_exact_audio() {
        let (root, path) = copy_to_temp(&writer_fixture("flac-trailing-ape.flac"), "edge.flac");
        let before = fs::read(&path).unwrap();
        let (prepared, repairs) = prepare_flac_source(&before).unwrap();
        assert!(repairs.trailing_ape);
        let expected_audio = flac_audio_payload(&prepared).unwrap().to_vec();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"album": "Correct Album"})).unwrap();
        assert_eq!(
            write_flac_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        let after = fs::read(&path).unwrap();
        assert!(!after.windows(8).any(|window| window == b"APETAGEX"));
        assert_eq!(flac_audio_payload(&after).unwrap(), expected_audio);
        assert_eq!(
            read_track_metadata(&path).unwrap().album.as_deref(),
            Some("Correct Album")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_ghost_vorbis_is_neutralized_with_only_length_word_changed() {
        let (root, path) = copy_to_temp(&writer_fixture("flac-ghost-vc.flac"), "edge.flac");
        let before = fs::read(&path).unwrap();
        let (prepared, repairs) = prepare_flac_source(&before).unwrap();
        assert!(repairs.ghost_vorbis);
        let expected_audio = flac_audio_payload(&prepared).unwrap().to_vec();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "RealTitle"})).unwrap();
        write_flac_atomic(&path, &patch).unwrap();
        let after = fs::read(&path).unwrap();
        assert!(after.len() > before.len());
        assert_eq!(flac_audio_payload(&after).unwrap(), expected_audio);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_duplicate_vorbis_is_collapsed_at_same_audio_boundary() {
        let (root, path) = copy_to_temp(&writer_fixture("flac-duplicate-vc.flac"), "edge.flac");
        let before = fs::read(&path).unwrap();
        let before_audio = flac_audio_payload(&before).unwrap().to_vec();
        let before_offset = flac_audio_offset(&before).unwrap();
        assert_eq!(
            flac_metadata_types(&before)
                .unwrap()
                .iter()
                .filter(|kind| **kind == 4)
                .count(),
            2
        );
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Canonical Title"})).unwrap();
        write_flac_atomic(&path, &patch).unwrap();
        let after = fs::read(&path).unwrap();
        assert_eq!(
            flac_metadata_types(&after)
                .unwrap()
                .iter()
                .filter(|kind| **kind == 4)
                .count(),
            1
        );
        assert_eq!(flac_audio_offset(&after).unwrap(), before_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), before_audio);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_without_vorbis_block_creates_one_at_same_audio_boundary() {
        let (root, path) = copy_to_temp(&writer_fixture("flac-bare.flac"), "edge.flac");
        let before = fs::read(&path).unwrap();
        let before_audio = flac_audio_payload(&before).unwrap().to_vec();
        let before_offset = flac_audio_offset(&before).unwrap();
        assert!(!flac_metadata_types(&before).unwrap().contains(&4));
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Fresh Title"})).unwrap();
        write_flac_atomic(&path, &patch).unwrap();
        let after = fs::read(&path).unwrap();
        assert_eq!(
            flac_metadata_types(&after)
                .unwrap()
                .iter()
                .filter(|kind| **kind == 4)
                .count(),
            1
        );
        assert_eq!(flac_audio_offset(&after).unwrap(), before_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), before_audio);
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Fresh Title")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_insufficient_padding_grows_metadata_without_changing_audio() {
        let (root, path) = copy_to_temp(
            &writer_fixture("flac-insufficient-padding.flac"),
            "edge.flac",
        );
        let before = fs::read(&path).unwrap();
        let before_audio = flac_audio_payload(&before).unwrap().to_vec();
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Expanded",
            "lyrics": "lyrics".repeat(500)
        }))
        .unwrap();
        write_flac_atomic(&path, &patch).unwrap();
        let after = fs::read(&path).unwrap();
        assert!(after.len() > before.len());
        assert_eq!(flac_audio_payload(&after).unwrap(), before_audio);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn queued_single_track_write_runs_the_atomic_core() {
        let (root, path) = copy_fixture();
        let queue = WriteQueue::default();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Queued title"})).unwrap();
        write_track_queued(&queue, path.clone(), patch)
            .await
            .unwrap();
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Queued title")
        );
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn queued_flac_write_runs_the_atomic_core() {
        let (root, path) = copy_flac_fixture();
        let queue = WriteQueue::default();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Queued FLAC title"})).unwrap();
        write_track_queued(&queue, path.clone(), patch)
            .await
            .unwrap();
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Queued FLAC title")
        );
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    fn extra_payload_snapshot(path: &Path) -> Vec<Vec<u8>> {
        let bytes = fs::read(path).unwrap();
        match path.extension().and_then(|value| value.to_str()).unwrap() {
            "mp3" => vec![mpeg_payload(&bytes).unwrap().to_vec()],
            "flac" => vec![flac_audio_payload(&bytes).unwrap().to_vec()],
            "ogg" => ogg_audio_packets(&bytes, 3).unwrap(),
            "opus" => ogg_audio_packets(&bytes, 2).unwrap(),
            "wav" => wav_data_payloads(&bytes).unwrap(),
            "ape" => vec![ape_audio_core(&bytes).unwrap().to_vec()],
            _ => unreachable!(),
        }
    }

    #[test]
    fn extra_tag_writes_replace_extras_preserve_standard_fields_and_audio() {
        let cases = [
            (media_fixture("minimal.mp3"), "track.mp3"),
            (writer_fixture("padded.flac"), "track.flac"),
            (writer_fixture("vorbis.ogg"), "track.ogg"),
            (writer_fixture("opus.opus"), "track.opus"),
            (media_fixture("minimal.wav"), "track.wav"),
            (media_fixture("ape-id3v1-fallback.ape"), "track.ape"),
        ];
        for (fixture, name) in cases {
            let (root, path) = copy_to_temp(&fixture, name);
            let title = read_track_metadata(&path).unwrap().title;
            let audio = extra_payload_snapshot(&path);
            write_extra_tags_dispatch(
                &path,
                &[
                    ExtraTagUpdate {
                        key: " mood ".to_string(),
                        value: " Bright ".to_string(),
                    },
                    ExtraTagUpdate {
                        key: "BARCODE".to_string(),
                        value: "111".to_string(),
                    },
                    ExtraTagUpdate {
                        key: "BARCODE".to_string(),
                        value: "222".to_string(),
                    },
                    ExtraTagUpdate {
                        key: "ARTISTS".to_string(),
                        value: "One".to_string(),
                    },
                    ExtraTagUpdate {
                        key: "ARTISTS".to_string(),
                        value: "Two".to_string(),
                    },
                    ExtraTagUpdate {
                        key: "TITLE".to_string(),
                        value: "Must not replace".to_string(),
                    },
                ],
            )
            .unwrap();
            assert_eq!(extra_payload_snapshot(&path), audio, "{name}");
            assert_eq!(read_track_metadata(&path).unwrap().title, title, "{name}");
            let rows = crate::commands::tracks::read_extra_tags(&path);
            assert!(
                rows.iter()
                    .any(|row| row.key.eq_ignore_ascii_case("MOOD") && row.value == "Bright"),
                "{name}"
            );
            assert!(
                rows.iter()
                    .any(|row| row.key == "BARCODE" && row.value == "111"),
                "{name}"
            );
            assert!(
                rows.iter()
                    .any(|row| row.key == "BARCODE" && row.value == "222"),
                "{name}"
            );
            assert_eq!(
                rows.iter().filter(|row| row.key == "ARTISTS").count(),
                2,
                "{name}"
            );
            assert!(!rows.iter().any(|row| row.key == "TITLE"), "{name}");

            write_extra_tags_dispatch(
                &path,
                &[ExtraTagUpdate {
                    key: "MOOD".to_string(),
                    value: "Calm".to_string(),
                }],
            )
            .unwrap();
            let rows = crate::commands::tracks::read_extra_tags(&path);
            assert!(
                rows.iter()
                    .any(|row| row.key.eq_ignore_ascii_case("MOOD") && row.value == "Calm"),
                "{name}"
            );
            assert!(
                !rows
                    .iter()
                    .any(|row| row.key == "BARCODE" || row.key == "ARTISTS"),
                "{name}"
            );
            write_extra_tags_dispatch(&path, &[]).unwrap();
            let cleared = crate::commands::tracks::read_extra_tags(&path);
            assert!(
                !cleared.iter().any(|row| {
                    row.key.eq_ignore_ascii_case("MOOD")
                        || row.key == "BARCODE"
                        || row.key == "ARTISTS"
                }),
                "{name}"
            );
            assert_eq!(extra_payload_snapshot(&path), audio, "{name}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn batch_extra_tags_writes_all_supported_formats() {
        let (root, mp3) = copy_to_temp(&media_fixture("minimal.mp3"), "first.mp3");
        let flac = root.join("second.flac");
        fs::copy(writer_fixture("padded.flac"), &flac).unwrap();
        let updates = vec![
            ExtraTagBatchUpdate {
                path: mp3.to_string_lossy().into_owned(),
                tags: vec![ExtraTagUpdate {
                    key: "MOOD".to_string(),
                    value: "Bright".to_string(),
                }],
            },
            ExtraTagBatchUpdate {
                path: flac.to_string_lossy().into_owned(),
                tags: vec![ExtraTagUpdate {
                    key: "MOOD".to_string(),
                    value: "Calm".to_string(),
                }],
            },
        ];
        let queue = WriteQueue::default();
        batch_write_extra_tags_queued(&queue, updates)
            .await
            .unwrap();
        assert!(!queue.is_active());
        assert!(crate::commands::tracks::read_extra_tags(&mp3)
            .iter()
            .any(|row| row.value == "Bright"));
        assert!(crate::commands::tracks::read_extra_tags(&flac)
            .iter()
            .any(|row| row.value == "Calm"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn batch_extra_tags_aggregates_failures_and_continues() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-extra-batch-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let bad = root.join("bad.mp3");
        fs::write(&bad, b"bad").unwrap();
        let good = root.join("good.flac");
        fs::copy(writer_fixture("padded.flac"), &good).unwrap();
        let updates = vec![
            ExtraTagBatchUpdate {
                path: bad.to_string_lossy().into_owned(),
                tags: Vec::new(),
            },
            ExtraTagBatchUpdate {
                path: good.to_string_lossy().into_owned(),
                tags: vec![ExtraTagUpdate {
                    key: "MOOD".to_string(),
                    value: "Written".to_string(),
                }],
            },
        ];
        let error = batch_write_extra_tags_queued(&WriteQueue::default(), updates)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Batch extra-tag write failed for 1 file(s)"));
        assert!(error.to_string().contains("bad.mp3"));
        assert!(crate::commands::tracks::read_extra_tags(&good)
            .iter()
            .any(|row| row.value == "Written"));
        assert_eq!(fs::read(&bad).unwrap(), b"bad");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extra_tag_normalization_canonicalizes_and_deduplicates() {
        let rows = normalized_extra_tags(&[
            ExtraTagUpdate {
                key: " COMM ".to_string(),
                value: " note ".to_string(),
            },
            ExtraTagUpdate {
                key: "MusicBrains Album Id".to_string(),
                value: "mb".to_string(),
            },
            ExtraTagUpdate {
                key: "MUSICBRAINZ_ALBUMID".to_string(),
                value: "mb".to_string(),
            },
            ExtraTagUpdate {
                key: "GENRE".to_string(),
                value: "Rock".to_string(),
            },
            ExtraTagUpdate {
                key: "EMPTY".to_string(),
                value: " ".to_string(),
            },
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "COMMENT");
        assert_eq!(rows[0].value, "note");
        assert_eq!(rows[1].key, "MUSICBRAINZ_ALBUMID");
    }

    #[tokio::test]
    async fn extra_tag_queue_rejects_unsupported_without_mutation() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-extra-unsupported-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("track.m4a");
        fs::write(&path, b"untouched").unwrap();
        let error = write_extra_tags_queued(&WriteQueue::default(), path.clone(), Vec::new())
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Extra tag editing is not supported for .m4a"));
        assert_eq!(fs::read(&path).unwrap(), b"untouched");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_exists_matches_files_directories_and_missing_paths() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-exists-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("track.mp3");
        fs::write(&file, b"x").unwrap();
        assert!(file_exists(file.to_string_lossy().into_owned()));
        assert!(file_exists(root.to_string_lossy().into_owned()));
        assert!(!file_exists(
            root.join("missing").to_string_lossy().into_owned()
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn delete_files_returns_ordered_per_path_results_and_continues() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-delete-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first.mp3");
        let second = root.join("second.flac");
        let missing = root.join("missing.ogg");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let queue = WriteQueue::default();
        let results = delete_files_queued(
            &queue,
            vec![
                first.to_string_lossy().into_owned(),
                missing.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
                root.to_string_lossy().into_owned(),
            ],
        )
        .await;
        assert_eq!(results.len(), 5);
        assert!(results[0].success);
        assert!(!results[1].success);
        assert!(results[2].success);
        assert!(!results[3].success);
        assert!(!results[4].success);
        assert!(results
            .iter()
            .skip(1)
            .filter(|result| !result.success)
            .all(|result| result
                .error
                .as_deref()
                .is_some_and(|error| !error.is_empty())));
        assert!(!first.exists());
        assert!(!second.exists());
        assert!(root.exists());
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rename_creates_nested_parent_and_returns_new_path_metadata() {
        let (root, source) = copy_to_temp(&media_fixture("minimal.mp3"), "source.mp3");
        let target = root.join("nested").join("renamed.mp3");
        let queue = WriteQueue::default();
        let track = rename_track_queued(&queue, source.clone(), target.clone())
            .await
            .unwrap();
        assert!(!source.exists());
        assert!(target.exists());
        assert_eq!(track.path, target.to_string_lossy());
        assert_eq!(track.title.as_deref(), Some("Corpus MP3"));
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rename_matches_unix_collision_replacement_semantics() {
        let (root, source) = copy_to_temp(&media_fixture("minimal.mp3"), "source.mp3");
        let target = root.join("target.mp3");
        fs::write(&target, b"old target").unwrap();
        rename_track_queued(&WriteQueue::default(), source, target.clone())
            .await
            .unwrap();
        assert_eq!(
            read_track_metadata(&target).unwrap().title.as_deref(),
            Some("Corpus MP3")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rename_uses_normal_filesystem_parent_traversal_resolution() {
        let (root, source) = copy_to_temp(&media_fixture("minimal.mp3"), "source.mp3");
        let target = root.join("created").join("..").join("resolved.mp3");
        let track = rename_track_queued(&WriteQueue::default(), source, target.clone())
            .await
            .unwrap();
        assert_eq!(track.path, target.to_string_lossy());
        assert!(root.join("resolved.mp3").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rename_failure_keeps_source_bytes() {
        let (root, source) = copy_to_temp(&media_fixture("minimal.mp3"), "source.mp3");
        let before = fs::read(&source).unwrap();
        let target = root.join("target-dir");
        fs::create_dir_all(&target).unwrap();
        assert!(
            rename_track_queued(&WriteQueue::default(), source.clone(), target)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&source).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn batch_write_is_sequential_and_supports_empty_batches() {
        let (root, first) = copy_to_temp(&media_fixture("minimal.mp3"), "first.mp3");
        let second = root.join("second.mp3");
        fs::copy(media_fixture("minimal.mp3"), &second).unwrap();
        let updates = vec![
            TrackUpdate {
                path: first.to_string_lossy().into_owned(),
                fields: serde_json::from_value(serde_json::json!({"title": "First batch title"}))
                    .unwrap(),
            },
            TrackUpdate {
                path: second.to_string_lossy().into_owned(),
                fields: serde_json::from_value(serde_json::json!({"title": "Second batch title"}))
                    .unwrap(),
            },
        ];
        let queue = WriteQueue::default();
        batch_write_queued(&queue, updates).await.unwrap();
        assert_eq!(
            read_track_metadata(&first).unwrap().title.as_deref(),
            Some("First batch title")
        );
        assert_eq!(
            read_track_metadata(&second).unwrap().title.as_deref(),
            Some("Second batch title")
        );
        batch_write_queued(&queue, Vec::new()).await.unwrap();
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn batch_write_stops_at_first_error_after_prior_commits() {
        let (root, first) = copy_to_temp(&media_fixture("minimal.mp3"), "first.mp3");
        let unsupported = root.join("second.xyz");
        fs::write(&unsupported, b"untouched").unwrap();
        let updates = vec![
            TrackUpdate {
                path: first.to_string_lossy().into_owned(),
                fields: serde_json::from_value(serde_json::json!({"title": "Committed first"}))
                    .unwrap(),
            },
            TrackUpdate {
                path: unsupported.to_string_lossy().into_owned(),
                fields: TrackPatch::default(),
            },
        ];
        let error = batch_write_queued(&WriteQueue::default(), updates)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("other than MP3/FLAC/OGG/Opus/M4A/MP4/WAV/APE"));
        assert_eq!(
            read_track_metadata(&first).unwrap().title.as_deref(),
            Some("Committed first")
        );
        assert_eq!(fs::read(&unsupported).unwrap(), b"untouched");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn queued_ape_write_runs_the_atomic_core() {
        let (root, path) = copy_to_temp(&media_fixture("ape-id3v1-fallback.ape"), "track.ape");
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Queued APE title"})).unwrap();
        write_track_queued(&WriteQueue::default(), path.clone(), patch)
            .await
            .unwrap();
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Queued APE title")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn queued_wav_write_runs_the_atomic_core() {
        let (root, path) = copy_to_temp(&media_fixture("minimal.wav"), "track.wav");
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Queued WAV title"})).unwrap();
        write_track_queued(&WriteQueue::default(), path.clone(), patch)
            .await
            .unwrap();
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Queued WAV title")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn aiff_write_returns_electron_unsupported_error() {
        let path = media_fixture("minimal.aiff");
        let error = write_track_queued(&WriteQueue::default(), path, TrackPatch::default())
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "AIFF metadata writing is not supported");
    }

    #[tokio::test]
    async fn queued_mp4_write_runs_the_atomic_core() {
        let (root, path) = copy_to_temp(&media_fixture("minimal.mp4"), "track.mp4");
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Queued MP4 title"})).unwrap();
        write_track_queued(&WriteQueue::default(), path.clone(), patch)
            .await
            .unwrap();
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Queued MP4 title")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn queued_true_opus_write_runs_the_atomic_core() {
        let (root, path) = copy_ogg_fixture("opus.opus");
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Queued Opus title"})).unwrap();
        write_track_queued(&WriteQueue::default(), path.clone(), patch)
            .await
            .unwrap();
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Queued Opus title")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn queued_unsupported_write_fails_loudly_without_touching_file() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-pending-write-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("pending.xyz");
        fs::write(&path, b"unchanged").unwrap();
        let error = write_track_queued(&WriteQueue::default(), path.clone(), TrackPatch::default())
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("other than MP3/FLAC/OGG/Opus/M4A/MP4/WAV/APE"));
        assert_eq!(fs::read(&path).unwrap(), b"unchanged");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_ape_failure_leaves_original_untouched() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-bad-ape-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("bad.ape");
        fs::write(&path, b"not an ape").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(write_ape_atomic(&path, &TrackPatch::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_wav_failure_leaves_original_untouched() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-bad-wav-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("bad.wav");
        fs::write(&path, b"not a wav").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(write_wav_atomic(&path, &TrackPatch::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_mp4_failure_leaves_original_untouched() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-bad-mp4-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("bad.mp4");
        fs::write(&path, b"not an mp4").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(write_mp4_atomic(&path, &TrackPatch::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_ogg_failure_leaves_original_untouched() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-bad-ogg-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("bad.ogg");
        fs::write(&path, b"not an ogg").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(write_ogg_atomic(&path, &TrackPatch::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_flac_failure_leaves_original_untouched() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-bad-flac-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("bad.flac");
        fs::write(&path, b"not a flac").unwrap();
        let before = fs::read(&path).unwrap();
        assert!(write_flac_atomic(&path, &TrackPatch::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_mp3_failure_leaves_original_untouched() {
        let root = std::env::temp_dir().join(format!(
            "auto-tagger-bad-mp3-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("bad.mp3");
        fs::write(&path, b"not an mp3").unwrap();
        let before = fs::read(&path).unwrap();
        let result = write_mp3_atomic(&path, &TrackPatch::default());
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }
}
