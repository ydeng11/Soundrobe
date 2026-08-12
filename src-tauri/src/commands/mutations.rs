//! Media mutation core. Commands and queue integration land only after each
//! format's pure writer passes differential and payload-safety tests.

use crate::commands::tracks::{
    id3_user_text_values, read_track_metadata, strip_wav_padding, unreadable_track_data, TrackData,
};
use crate::error::ApiError;
use crate::state::write_queue::WriteQueue;
use lofty::ape::{ApeFile, ApeItem, ApeTag};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, FrameId, Id3v2Tag, TextInformationFrame, UnsynchronizedTextFrame};
use lofty::iff::wav::WavFile;
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::{OggPictureStorage, OpusFile, VorbisFile};
use lofty::probe::Probe;
use lofty::tag::TagType;
use lofty::tag::{Accessor, ItemValue, TagExt};
use lofty::TextEncoding;
use memchr::memmem::Finder;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

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

/// Per-track failure reported in a batch write result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackWriteFailure {
    pub path: String,
    pub error: String,
}

/// Structured batch write result returned to the renderer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchWriteResult {
    pub tracks: Vec<TrackData>,
    pub failures: Vec<TrackWriteFailure>,
}

/// Progress event emitted from batch track writes so the renderer can show
/// a determinate progress bar (completed / total) instead of an indeterminate
/// "breath light" animation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackWriteEvent {
    pub current: u64,
    pub total: u64,
    pub message: String,
}

/// One phase of the volume write-probe diagnostic.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteProbePhase {
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
    pub os_error_code: Option<i32>,
}

/// Aggregate result of the volume write-probe diagnostic.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteProbeResult {
    pub path: String,
    pub phases: Vec<WriteProbePhase>,
    pub all_successful: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackWriteOutcome {
    Skipped,
    Replaced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtraTagWriteStrategy {
    InPlace,
    FullRewrite,
    Skipped,
}

impl ExtraTagWriteStrategy {
    fn as_str(self) -> &'static str {
        match self {
            Self::InPlace => "in_place",
            Self::FullRewrite => "full_rewrite",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExtraTagWriteReport {
    outcome: TrackWriteOutcome,
    strategy: ExtraTagWriteStrategy,
    metadata_bytes_read: u64,
    metadata_bytes_written: u64,
}

impl ExtraTagWriteReport {
    fn from_full_rewrite(outcome: TrackWriteOutcome) -> Self {
        Self {
            outcome,
            strategy: if outcome == TrackWriteOutcome::Skipped {
                ExtraTagWriteStrategy::Skipped
            } else {
                ExtraTagWriteStrategy::FullRewrite
            },
            metadata_bytes_read: 0,
            metadata_bytes_written: 0,
        }
    }
}

/// Maximum number of tracks to write in one sequential sub-batch within a
/// single folder worker. Albums larger than this are split into chunks so
/// memory stays bounded and each chunk acts as a natural checkpoint.
pub(crate) const SUBBATCH_SIZE: usize = 20;
const DEFAULT_FOLDER_WRITE_CONCURRENCY: usize = 4;

fn effective_write_concurrency(configured: Option<usize>) -> usize {
    configured
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_FOLDER_WRITE_CONCURRENCY)
}

/// Group a flat list of track updates by their parent album folder.
///
/// The partition key is `Path::parent()` — the directory containing the track
/// file. Track paths from the renderer are already absolute, so no
/// `fs::canonicalize` syscall is needed.
pub(crate) fn group_by_folder(updates: Vec<TrackUpdate>) -> HashMap<PathBuf, Vec<TrackUpdate>> {
    let mut groups: HashMap<PathBuf, Vec<TrackUpdate>> = HashMap::new();
    for update in updates {
        let parent = Path::new(&update.path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        groups.entry(parent).or_default().push(update);
    }
    groups
}

#[tauri::command]
pub async fn track_write(
    path: String,
    fields: TrackPatch,
    queue: State<'_, WriteQueue>,
) -> Result<TrackData, ApiError> {
    write_track_with_readback(&queue, PathBuf::from(path), fields).await
}

#[tauri::command]
pub async fn tracks_batch_write(
    app: tauri::AppHandle,
    updates: Vec<TrackUpdate>,
    queue: State<'_, WriteQueue>,
) -> Result<BatchWriteResult, ApiError> {
    batch_write_with_readback(&queue, updates, Some(app)).await
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

/// Helper: record a single probe phase outcome.
fn probe_phase(name: &str, result: &std::io::Result<()>) -> WriteProbePhase {
    match result {
        Ok(_) => WriteProbePhase {
            name: name.to_string(),
            success: true,
            error: None,
            os_error_code: None,
        },
        Err(e) => WriteProbePhase {
            name: name.to_string(),
            success: false,
            error: Some(e.to_string()),
            os_error_code: e.raw_os_error(),
        },
    }
}

/// Diagnose why writes to a given path or its parent directory may be failing.
/// Creates and cleans up temp files but never modifies the target.
#[tauri::command]
pub async fn volume_probe_write(path: String) -> WriteProbeResult {
    let target = PathBuf::from(&path);
    let parent = target.parent().unwrap_or(&target);
    let mut phases: Vec<WriteProbePhase> = Vec::new();

    // 1. Can we stat the target?
    phases.push(probe_phase(
        "stat_target",
        &fs::metadata(&target).map(|_| ()),
    ));

    // 2. Can we read the target?
    let read_result = fs::read(&target);
    phases.push(match &read_result {
        Ok(_) => WriteProbePhase {
            name: "read_target".to_string(),
            success: true,
            error: None,
            os_error_code: None,
        },
        Err(e) => WriteProbePhase {
            name: "read_target".to_string(),
            success: false,
            error: Some(e.to_string()),
            os_error_code: e.raw_os_error(),
        },
    });

    // 3. Can we stat the parent directory?
    phases.push(probe_phase(
        "parent_exists",
        &fs::metadata(parent).map(|_| ()),
    ));

    // 4. Create a sibling temp file and probe each I/O phase
    let temp = sibling_temp_path(&target);
    let alt_temp = sibling_temp_path(&target); // second distinct temp for rename-over test

    // 4a. Write to temp file (simulates creating the sibling temp)
    let original_len = read_result.as_ref().ok().map(|b| b.len());
    let write_result = (|| -> std::io::Result<()> {
        let mut dst = File::create(&temp)?;
        if let Some(len) = original_len {
            // Write some data so the file isn't empty
            let data = vec![0u8; len.min(4096)];
            use std::io::Write;
            dst.write_all(&data)?;
        }
        Ok(())
    })();
    phases.push(probe_phase("sibling_temp_create_write", &write_result));

    // 4b. Sync the temp file via a write handle (matching copy_file_data).
    let sync_result = (|| -> std::io::Result<()> {
        let f = fs::OpenOptions::new().write(true).open(&temp)?;
        f.sync_all()?;
        Ok(())
    })();
    phases.push(probe_phase("sibling_temp_sync", &sync_result));

    // 4c. Rename within the same directory (temp -> alt_temp)
    let rename_result = (|| -> std::io::Result<()> {
        if alt_temp.exists() {
            fs::remove_file(&alt_temp)?;
        }
        fs::rename(&temp, &alt_temp)?;
        Ok(())
    })();
    phases.push(probe_phase("sibling_temp_rename", &rename_result));

    // 4d. Rename over existing (simulates atomic replacement)
    let rename_over_result = (|| -> std::io::Result<()> {
        // Re-create temp, then rename alt_temp over it
        File::create(&temp)?;
        fs::rename(&alt_temp, &temp)?;
        Ok(())
    })();
    phases.push(probe_phase(
        "sibling_temp_rename_over_existing",
        &rename_over_result,
    ));

    // 4e. Remove temp
    let remove_result = (|| -> std::io::Result<()> {
        if temp.exists() {
            fs::remove_file(&temp)?;
        }
        if alt_temp.exists() {
            fs::remove_file(&alt_temp)?;
        }
        Ok(())
    })();
    phases.push(probe_phase("sibling_temp_remove", &remove_result));

    // Clean up any leftover temp files
    let _ = fs::remove_file(&temp);
    let _ = fs::remove_file(&alt_temp);

    let all_successful = phases.iter().all(|p| p.success);
    WriteProbeResult {
        path,
        phases,
        all_successful,
    }
}

/// Run the real `write_track_dispatch` on a COPY of the target file.
/// The original is never touched. The copy is removed after diagnostics.
/// Reports the exact `TrackWriteOutcome`, a human-readable error if the
/// write itself fails, and before/after metadata for the requested field.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealWriteProbeResult {
    pub path: String,
    pub outcome: String, // "Skipped", "Replaced", or "Error"
    pub error: Option<String>,
    pub os_error_code: Option<i32>,
    pub before_field: Option<String>,
    pub after_field: Option<String>,
    pub copy_removed: bool,
}

/// Diagnostic: copies `path` to a sibling `.probe-test.flac`, runs the real
/// `write_track_dispatch` with the given JSON field patch on the copy, reads
/// back before/after metadata, cleans up, and reports everything.
#[tauri::command]
pub async fn volume_probe_write_real(
    path: String,
    patch_json: serde_json::Value,
) -> RealWriteProbeResult {
    let target = PathBuf::from(&path);
    let patch: TrackPatch = match serde_json::from_value(patch_json) {
        Ok(p) => p,
        Err(e) => {
            return RealWriteProbeResult {
                path,
                outcome: "Error".to_string(),
                error: Some(format!("patch deserialization: {e}")),
                os_error_code: None,
                before_field: None,
                after_field: None,
                copy_removed: true,
            };
        }
    };

    // Read original metadata before copy (for before_field)
    let before_meta = read_track_metadata(&target).ok();
    let before_field = before_meta
        .as_ref()
        .and_then(|t| t.album_artist.as_deref())
        .map(|s| s.to_string());

    // Create a sibling copy (e.g. "05 靠近一點.probe-test.flac")
    let copy_path = target.with_file_name(format!(
        "{}.probe-test.{}",
        target
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("track"),
        target
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("flac")
    ));

    // Ensure the copy is clean
    let _ = fs::remove_file(&copy_path);

    let copy_result = fs::copy(&target, &copy_path);
    if let Err(e) = copy_result {
        return RealWriteProbeResult {
            path,
            outcome: "Error".to_string(),
            error: Some(format!("copy: {e}")),
            os_error_code: e.raw_os_error(),
            before_field,
            after_field: None,
            copy_removed: false,
        };
    }

    tracing::info!(
        path = %path,
        patch_album_artist = ?patch.album_artist,
        patch_artist = ?patch.artist,
        "volume probe real: running write_track_dispatch"
    );

    // Run the real writer on the copy
    let write_result = write_track_dispatch(&copy_path, &patch);
    tracing::info!(
        path = %path,
        write_result = ?write_result,
        "volume probe real: write complete"
    );

    let after_field = match &write_result {
        Ok(_) => read_track_metadata(&copy_path)
            .ok()
            .and_then(|t| t.album_artist),
        Err(_) => None,
    };
    let (outcome, error) = match write_result {
        Ok(TrackWriteOutcome::Skipped) => ("Skipped".to_string(), None),
        Ok(TrackWriteOutcome::Replaced) => ("Replaced".to_string(), None),
        Err(e) => ("Error".to_string(), Some(e.to_string())),
    };
    // Clean up
    let copy_removed = fs::remove_file(&copy_path).is_ok();

    RealWriteProbeResult {
        path,
        outcome,
        error,
        // os_error_code is always None here; WriteProbePhase (the raw-I/O probe)
        // has it for I/O-origin errors, but write-path errors carry it in `error`.
        os_error_code: None,
        before_field,
        after_field,
        copy_removed,
    }
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

pub(crate) async fn rename_track_queued(
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

pub(crate) async fn write_track_queued(
    queue: &WriteQueue,
    path: PathBuf,
    patch: TrackPatch,
) -> Result<(), ApiError> {
    validated_track_extension(&path)?;
    let display_path = path.to_string_lossy().to_string();
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                let write_start = std::time::Instant::now();
                let result = write_track_dispatch(&path, &patch);
                let elapsed = write_start.elapsed();
                match &result {
                    Ok(_) => tracing::debug!(
                        path = %display_path,
                        elapsed_us = elapsed.as_micros(),
                        "track write done"
                    ),
                    Err(e) => tracing::warn!(
                        path = %display_path,
                        elapsed_us = elapsed.as_micros(),
                        error = %e,
                        "track write failed"
                    ),
                }
                result
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await?;
    Ok(())
}

/// Remove all embedded cover art pictures from a single audio track file.
/// Uses lofty's unified `Probe` + `TaggedFile` API to handle all formats.
pub(crate) fn remove_embedded_cover_at(path: &Path) -> Result<(), ApiError> {
    let mut tagged_file = Probe::open(path)
        .map_err(|e| ApiError::WriteTask(format!("Failed to open track for cover removal: {e}")))?
        .options(ParseOptions::new().read_properties(false))
        .read()
        .map_err(|e| ApiError::WriteTask(format!("Failed to read track for cover removal: {e}")))?;

    // Collect tag types first to avoid borrow conflicts with tag_mut.
    let tag_types: Vec<TagType> = tagged_file.tags().iter().map(|t| t.tag_type()).collect();
    let mut removed_any = false;
    for tag_type in &tag_types {
        if let Some(tag) = tagged_file.tag_mut(*tag_type) {
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
                removed_any = true;
            }
        }
    }

    if !removed_any {
        return Err(ApiError::Message(
            "No embedded cover art found in this track".into(),
        ));
    }

    tagged_file
        .save_to_path(path, WriteOptions::new())
        .map_err(|e| {
            ApiError::WriteTask(format!("Failed to save track after cover removal: {e}"))
        })?;

    Ok(())
}

/// Remove embedded cover art from a single track, queued through the global write lock.
pub(crate) async fn remove_embedded_cover_queued(
    queue: &WriteQueue,
    path: PathBuf,
) -> Result<(), ApiError> {
    let display_path = path.to_string_lossy().to_string();
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                let write_start = std::time::Instant::now();
                let result = remove_embedded_cover_at(&path);
                let elapsed = write_start.elapsed();
                match &result {
                    Ok(_) => tracing::debug!(
                        path = %display_path,
                        elapsed_us = elapsed.as_micros(),
                        "embedded cover removed"
                    ),
                    Err(e) => tracing::warn!(
                        path = %display_path,
                        elapsed_us = elapsed.as_micros(),
                        error = %e,
                        "embedded cover removal failed"
                    ),
                }
                result
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await?;
    Ok(())
}

/// Shared accumulator for batch write results across folder workers.
#[derive(Default)]
struct BatchAccumulator {
    successes: Vec<String>,
    failures: Vec<TrackWriteFailure>,
}

pub(crate) type TrackWriteProgress = Arc<dyn Fn(u64, u64) + Send + Sync>;

#[derive(Debug)]
pub(crate) struct ExclusiveBatchWriteResult {
    pub successes: Vec<String>,
    pub failures: Vec<TrackWriteFailure>,
}

async fn batch_write_grouped(
    queue: Option<WriteQueue>,
    updates: Vec<TrackUpdate>,
    progress: Option<TrackWriteProgress>,
    accum: &Arc<Mutex<BatchAccumulator>>,
) -> Result<(), ApiError> {
    let total = updates.len() as u64;
    // 1. Partition by folder (Path::parent() — no syscall needed)
    let folder_groups = group_by_folder(updates);

    if folder_groups.is_empty() {
        return Ok(());
    }

    let completed = Arc::new(Mutex::new(0u64));

    // 2. Spawn one task per folder (concurrent across folders).
    //    Queued callers acquire only their folder-scoped lock via
    //    run_for_folder; callers already inside run_exclusive skip recursive
    //    locking. Writes to different album folders proceed in parallel.
    //    Within a folder, tracks are written in sub-batches of SUBBATCH_SIZE
    //    (sequential within the folder worker) to keep memory bounded.
    //
    //    Cap concurrent folder workers at 4 by default. Controlled local and
    //    SMB benchmarks both improved through four workers after per-file I/O
    //    amplification was removed. The user can override
    //    via `write_concurrency` in ~/.soundrobe/config.yaml or the
    //    AUTO_TAG_WRITE_CONCURRENCY environment variable (e.g. 8 for
    //    local NVMe).
    let max_concurrency = effective_write_concurrency(
        crate::state::config::resolve_write_concurrency(&dirs::home_dir().unwrap_or_default()),
    );
    let io_quota = Arc::new(tokio::sync::Semaphore::new(max_concurrency));
    let mut handles = Vec::new();
    for (folder, folder_updates) in folder_groups {
        let q = queue.clone();
        let accum = Arc::clone(accum);
        let progress = progress.clone();
        let completed = Arc::clone(&completed);
        // acquire_owned only errors if the semaphore is dropped, which never
        // happens since io_quota lives until all spawned tasks complete.
        let permit = io_quota.clone().acquire_owned().await.unwrap();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let operation = async move {
                tokio::task::spawn_blocking(move || {
                    for batch in folder_updates.chunks(SUBBATCH_SIZE) {
                        for update in batch {
                            let write_start = std::time::Instant::now();
                            let path_str = update.path.clone();
                            match write_track_dispatch(Path::new(&update.path), &update.fields) {
                                Ok(_) => {
                                    tracing::debug!(
                                        path = %update.path,
                                        elapsed_s = write_start.elapsed().as_secs_f64(),
                                        "batch track write done"
                                    );
                                    let mut acc = accum.lock().expect("accum lock poisoned");
                                    acc.successes.push(path_str);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        path = %update.path,
                                        elapsed_us = write_start.elapsed().as_micros(),
                                        error = %e,
                                        "batch track write failed, continuing"
                                    );
                                    accum.lock().expect("accum lock poisoned").failures.push(
                                        TrackWriteFailure {
                                            path: path_str,
                                            error: e.to_string(),
                                        },
                                    );
                                }
                            }
                            let mut completed = completed.lock().expect("progress lock poisoned");
                            *completed += 1;
                            if let Some(progress) = &progress {
                                progress(*completed, total);
                            }
                        }
                    }
                    Ok::<(), ApiError>(())
                })
                .await
                .map_err(|e| ApiError::WriteTask(e.to_string()))?
            };
            if let Some(q) = q {
                q.run_for_folder(&folder, operation).await
            } else {
                operation.await
            }
        }));
    }

    // 3. Reduce: wait for all folders. Folders that already committed
    //    succeeded before an error in another folder was detected.
    //    This matches Electron's non-transactional semantics — no rollback.
    join_folder_workers(handles).await?;

    Ok(())
}

async fn join_folder_workers(
    handles: Vec<tokio::task::JoinHandle<Result<(), ApiError>>>,
) -> Result<(), ApiError> {
    let mut first_error = None;
    for handle in handles {
        let result = match handle.await {
            Ok(result) => result,
            Err(error) => Err(ApiError::WriteTask(error.to_string())),
        };
        if first_error.is_none() {
            if let Err(error) = result {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn batch_write_queued(
    queue: &WriteQueue,
    updates: Vec<TrackUpdate>,
    progress_tracker: Option<(tauri::AppHandle, u64)>,
    accum: &Arc<Mutex<BatchAccumulator>>,
) -> Result<(), ApiError> {
    let progress = progress_tracker.map(|(app, _)| {
        Arc::new(move |current, total| {
            let _ = app.emit(
                "tracks:write-event",
                TrackWriteEvent {
                    current,
                    total,
                    message: format!("Writing {current}/{total}"),
                },
            );
        }) as TrackWriteProgress
    });
    batch_write_grouped(Some(queue.clone()), updates, progress, accum).await?;

    // After all folder workers complete, check whether anything succeeded.
    let acc = accum.lock().expect("accum lock poisoned");
    if acc.successes.is_empty() {
        return Err(ApiError::Message(format!(
            "All {} write(s) failed. First error: {}",
            acc.failures.len(),
            acc.failures.first().map_or("unknown", |f| &f.error)
        )));
    }
    // Partial success: log a warning but return Ok so readback proceeds
    if !acc.failures.is_empty() {
        tracing::warn!(
            "batch write completed with {}/{} failures",
            acc.failures.len(),
            acc.successes.len() + acc.failures.len()
        );
    }
    Ok(())
}

/// Execute the regular folder-grouped writer while the caller holds
/// `WriteQueue::run_exclusive`. Queue locks are intentionally skipped to avoid
/// recursive coordination-lock acquisition; this function still serializes
/// same-folder writes and applies configured cross-folder concurrency.
pub(crate) async fn batch_write_with_exclusive_queue_held(
    updates: Vec<TrackUpdate>,
    progress: Option<TrackWriteProgress>,
) -> Result<ExclusiveBatchWriteResult, ApiError> {
    let accum = Arc::new(Mutex::new(BatchAccumulator::default()));
    batch_write_grouped(None, updates, progress, &accum).await?;
    let mut accum = accum.lock().expect("accum lock poisoned");
    Ok(ExclusiveBatchWriteResult {
        successes: std::mem::take(&mut accum.successes),
        failures: std::mem::take(&mut accum.failures),
    })
}

fn read_track_with_fallback(path: &Path) -> Result<TrackData, ApiError> {
    read_track_metadata(path).or_else(|_| {
        let size = fs::metadata(path)?.len();
        let title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        Ok(unreadable_track_data(path, size, title))
    })
}

async fn write_track_with_readback(
    queue: &WriteQueue,
    path: PathBuf,
    patch: TrackPatch,
) -> Result<TrackData, ApiError> {
    write_track_queued(queue, path.clone(), patch).await?;
    read_track_with_fallback(&path)
}

async fn batch_write_with_readback(
    queue: &WriteQueue,
    updates: Vec<TrackUpdate>,
    app: Option<tauri::AppHandle>,
) -> Result<BatchWriteResult, ApiError> {
    // Preserve input paths before updates is moved into batch_write_queued
    let input_paths: Vec<String> = updates.iter().map(|u| u.path.clone()).collect();
    let total = updates.len() as u64;
    let accum = Arc::new(Mutex::new(BatchAccumulator::default()));
    batch_write_queued(queue, updates, app.map(|a| (a, total)), &accum).await?;
    let mut acc = accum.lock().expect("accum lock poisoned");
    let successes: std::collections::HashSet<String> = acc.successes.drain(..).collect();
    let failures = acc.failures.clone();
    drop(acc);
    // Separate readback failures so they don't silently drop from the result.
    // Read back in input order so result.tracks matches the update sequence.
    let mut tracks: Vec<TrackData> = Vec::new();
    let mut all_failures: Vec<TrackWriteFailure> = failures;
    for path in &input_paths {
        if !successes.contains(path) {
            continue; // already recorded as a write failure
        }
        match read_track_with_fallback(Path::new(path)) {
            Ok(track) => tracks.push(track),
            Err(e) => all_failures.push(TrackWriteFailure {
                path: path.clone(),
                error: format!("{e}"),
            }),
        }
    }
    Ok(BatchWriteResult {
        tracks,
        failures: all_failures,
    })
}

pub(crate) async fn write_extra_tags_queued(
    queue: &WriteQueue,
    path: PathBuf,
    tags: Vec<ExtraTagUpdate>,
) -> Result<(), ApiError> {
    validate_extra_tag_extension(&path)?;
    let display_path = path.to_string_lossy().to_string();
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                let write_start = std::time::Instant::now();
                let result = write_extra_tags_dispatch(&path, &tags);
                let elapsed = write_start.elapsed();
                match &result {
                    Ok(report) => tracing::debug!(
                        path = %display_path,
                        strategy = report.strategy.as_str(),
                        metadata_bytes_read = report.metadata_bytes_read,
                        metadata_bytes_written = report.metadata_bytes_written,
                        elapsed_us = elapsed.as_micros(),
                        "extra tags write done"
                    ),
                    Err(e) => tracing::warn!(
                        path = %display_path,
                        elapsed_us = elapsed.as_micros(),
                        error = %e,
                        "extra tags write failed"
                    ),
                }
                result
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await?;
    Ok(())
}

pub(crate) async fn write_extra_tags_with_exclusive_queue_held(
    path: PathBuf,
    tags: Vec<ExtraTagUpdate>,
) -> Result<(), ApiError> {
    validate_extra_tag_extension(&path)?;
    tokio::task::spawn_blocking(move || write_extra_tags_dispatch(&path, &tags))
        .await
        .map_err(|error| ApiError::WriteTask(error.to_string()))??;
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
                    let write_start = std::time::Instant::now();
                    match write_extra_tags_dispatch(Path::new(&update.path), &update.tags) {
                        Ok(report) => tracing::debug!(
                            path = %update.path,
                            strategy = report.strategy.as_str(),
                            metadata_bytes_read = report.metadata_bytes_read,
                            metadata_bytes_written = report.metadata_bytes_written,
                            elapsed_us = write_start.elapsed().as_micros(),
                            "batch extra tags write done"
                        ),
                        Err(error) => {
                            tracing::warn!(
                                path = %update.path,
                                elapsed_us = write_start.elapsed().as_micros(),
                                error = %error,
                                "batch extra tags write failed"
                            );
                            failures.push(format!("{}: {error}", update.path));
                        }
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
) -> Result<ExtraTagWriteReport, ApiError> {
    let outcome = match validate_extra_tag_extension(path)?.as_str() {
        "mp3" => write_id3_extra_tags_atomic(path, tags, false),
        "wav" => write_id3_extra_tags_atomic(path, tags, true),
        "flac" => return write_flac_extra_tags_atomic(path, tags),
        "ogg" | "opus" => write_ogg_extra_tags_atomic(path, tags),
        _ => write_ape_extra_tags_atomic(path, tags),
    }?;
    Ok(ExtraTagWriteReport::from_full_rewrite(outcome))
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

const FLAC_GHOST_PROBE_BYTES: usize = 64 * 1024;
const FLAC_FAST_PATH_MAX_PREFIX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug)]
struct FlacMetadataBlock {
    block_type: u8,
    data: Vec<u8>,
}

#[derive(Debug)]
struct FlacPrefixLayout {
    prefix: Vec<u8>,
    marker_offset: usize,
    audio_offset: usize,
    file_len: u64,
    blocks: Vec<FlacMetadataBlock>,
    requires_full_rewrite: bool,
    bytes_read: u64,
}

fn try_flac_extra_tags_inplace(
    path: &Path,
    updates: &[ExtraTagUpdate],
) -> Result<Option<ExtraTagWriteReport>, ApiError> {
    try_flac_prefix_update(path, |comments| apply_vorbis_extra_tags(comments, updates))
}

fn try_flac_canonical_prefix_update(
    path: &Path,
    patch: &TrackPatch,
) -> Result<Option<ExtraTagWriteReport>, ApiError> {
    try_flac_prefix_update(path, |comments| apply_vorbis_patch(comments, patch))
}

fn try_flac_prefix_update<F>(path: &Path, apply: F) -> Result<Option<ExtraTagWriteReport>, ApiError>
where
    F: FnOnce(&mut lofty::ogg::VorbisComments),
{
    let Some(layout) = read_flac_prefix_layout(path)? else {
        return Ok(None);
    };
    if layout.requires_full_rewrite {
        return Ok(None);
    }

    let Some(mut comments) = read_flac_prefix_comments(&layout.prefix) else {
        return Ok(None);
    };
    if !comments.pictures().is_empty() {
        return Ok(None);
    }
    let before_vendor = comments.vendor().to_string();
    let before_items = comments
        .items()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<Vec<_>>();
    apply(&mut comments);
    let expected_vendor = comments.vendor().to_string();
    let expected_items = comments
        .items()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<Vec<_>>();

    if before_vendor == expected_vendor && vorbis_items_equivalent(&before_items, &expected_items) {
        return Ok(Some(ExtraTagWriteReport {
            outcome: TrackWriteOutcome::Skipped,
            strategy: ExtraTagWriteStrategy::Skipped,
            metadata_bytes_read: layout.bytes_read,
            metadata_bytes_written: 0,
        }));
    }

    let encoded_comments = encode_flac_vorbis_comments(&comments)?;
    let Some(candidate) = rebuild_flac_prefix(&layout, &encoded_comments) else {
        return Ok(None);
    };
    let Some(candidate_comments) = read_flac_prefix_comments(&candidate) else {
        return Ok(None);
    };
    if !vorbis_comments_match(&candidate_comments, &expected_vendor, &expected_items) {
        return Ok(None);
    }

    let verify_vendor = expected_vendor;
    let verify_items = expected_items;
    commit_flac_metadata_prefix(
        path,
        &layout.prefix,
        &candidate,
        layout.file_len,
        move |written_prefix| {
            let parsed = read_flac_prefix_comments(written_prefix).ok_or_else(|| {
                ApiError::MediaSafety(
                    "FLAC metadata prefix is unreadable after in-place write".to_string(),
                )
            })?;
            if !vorbis_comments_match(&parsed, &verify_vendor, &verify_items) {
                return Err(ApiError::MediaSafety(
                    "FLAC extra tags failed in-place readback".to_string(),
                ));
            }
            Ok(())
        },
    )?;

    Ok(Some(ExtraTagWriteReport {
        outcome: TrackWriteOutcome::Replaced,
        strategy: ExtraTagWriteStrategy::InPlace,
        metadata_bytes_read: layout.bytes_read.saturating_add(candidate.len() as u64),
        metadata_bytes_written: candidate.len() as u64,
    }))
}

fn vorbis_items_equivalent(left: &[(String, String)], right: &[(String, String)]) -> bool {
    fn normalized(items: &[(String, String)]) -> HashMap<String, Vec<String>> {
        let mut grouped = HashMap::new();
        for (key, value) in items {
            grouped
                .entry(key.to_ascii_uppercase())
                .or_insert_with(Vec::new)
                .push(value.clone());
        }
        grouped
    }
    normalized(left) == normalized(right)
}

fn read_flac_prefix_layout(path: &Path) -> Result<Option<FlacPrefixLayout>, ApiError> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < 10 {
        return Ok(None);
    }

    let mut header = [0_u8; 10];
    file.read_exact(&mut header)?;
    let marker_offset = if &header[..4] == b"fLaC" {
        0
    } else if &header[..3] == b"ID3" {
        let Some(id3_size) = synchsafe_u32(&header[6..10]) else {
            return Ok(None);
        };
        let footer_size = if header[5] & 0x10 != 0 { 10_u64 } else { 0 };
        let marker = 10_u64
            .checked_add(u64::from(id3_size))
            .and_then(|value| value.checked_add(footer_size));
        let Some(marker) = marker.and_then(|value| usize::try_from(value).ok()) else {
            return Ok(None);
        };
        if marker as u64 + 4 > file_len {
            return Ok(None);
        }
        file.seek(SeekFrom::Start(marker as u64))?;
        let mut signature = [0_u8; 4];
        file.read_exact(&mut signature)?;
        if &signature != b"fLaC" {
            return Ok(None);
        }
        marker
    } else {
        return Ok(None);
    };

    let metadata_start = marker_offset
        .checked_add(4)
        .ok_or_else(|| ApiError::MediaSafety("FLAC metadata offset overflow".to_string()))?;
    let mut prefix = vec![0_u8; metadata_start];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut prefix)?;
    let mut blocks = Vec::new();
    let mut vorbis_count = 0_usize;
    let mut vorbis_valid = true;

    loop {
        let mut block_header = [0_u8; 4];
        file.read_exact(&mut block_header)?;
        let last = block_header[0] & 0x80 != 0;
        let block_type = block_header[0] & 0x7f;
        let block_len = (usize::from(block_header[1]) << 16)
            | (usize::from(block_header[2]) << 8)
            | usize::from(block_header[3]);
        let next_len = prefix
            .len()
            .checked_add(4)
            .and_then(|value| value.checked_add(block_len));
        if next_len
            .is_none_or(|value| value > FLAC_FAST_PATH_MAX_PREFIX_BYTES || value as u64 > file_len)
        {
            return Ok(None);
        }
        let mut data = vec![0_u8; block_len];
        file.read_exact(&mut data)?;
        prefix.extend_from_slice(&block_header);
        prefix.extend_from_slice(&data);
        if block_type == 4 {
            vorbis_count += 1;
            vorbis_valid &= valid_flac_vorbis_comments(&data);
        }
        blocks.push(FlacMetadataBlock { block_type, data });
        if last {
            break;
        }
    }

    if blocks
        .first()
        .is_none_or(|block| block.block_type != 0 || block.data.len() != 34)
    {
        return Ok(None);
    }

    let audio_offset = prefix.len();
    let mut bytes_read = 10_u64.saturating_add(prefix.len() as u64);
    let trailing_ape = if file_len >= 32 {
        file.seek(SeekFrom::End(-32))?;
        let mut footer = [0_u8; 32];
        file.read_exact(&mut footer)?;
        bytes_read = bytes_read.saturating_add(footer.len() as u64);
        &footer[..8] == b"APETAGEX"
    } else {
        false
    };

    let probe_len = usize::try_from(file_len.saturating_sub(audio_offset as u64))
        .unwrap_or(usize::MAX)
        .min(FLAC_GHOST_PROBE_BYTES);
    let ghost_vorbis = if probe_len > 0 {
        file.seek(SeekFrom::Start(audio_offset as u64))?;
        let mut probe = vec![0_u8; probe_len];
        file.read_exact(&mut probe)?;
        bytes_read = bytes_read.saturating_add(probe.len() as u64);
        neutralize_ghost_vorbis(&mut probe, 0)
    } else {
        false
    };

    Ok(Some(FlacPrefixLayout {
        prefix,
        marker_offset,
        audio_offset,
        file_len,
        blocks,
        requires_full_rewrite: vorbis_count > 1 || !vorbis_valid || trailing_ape || ghost_vorbis,
        bytes_read,
    }))
}

fn valid_flac_vorbis_comments(data: &[u8]) -> bool {
    fn take_u32(data: &mut &[u8]) -> Option<usize> {
        let value = u32::from_le_bytes(data.get(..4)?.try_into().ok()?) as usize;
        *data = data.get(4..)?;
        Some(value)
    }

    let mut remaining = data;
    let Some(vendor_len) = take_u32(&mut remaining) else {
        return false;
    };
    let Some(vendor) = remaining.get(..vendor_len) else {
        return false;
    };
    if std::str::from_utf8(vendor).is_err() {
        return false;
    }
    remaining = &remaining[vendor_len..];

    let Some(item_count) = take_u32(&mut remaining) else {
        return false;
    };
    for _ in 0..item_count {
        let Some(item_len) = take_u32(&mut remaining) else {
            return false;
        };
        let Some(item) = remaining.get(..item_len) else {
            return false;
        };
        let Some(separator) = item.iter().position(|byte| *byte == b'=') else {
            return false;
        };
        let key = &item[..separator];
        if key.is_empty()
            || key
                .iter()
                .any(|byte| !(0x20..=0x7d).contains(byte) || *byte == b'=')
            || std::str::from_utf8(&item[separator + 1..]).is_err()
        {
            return false;
        }
        remaining = &remaining[item_len..];
    }
    remaining.is_empty()
}

fn synchsafe_u32(bytes: &[u8]) -> Option<u32> {
    let [a, b, c, d]: [u8; 4] = bytes.try_into().ok()?;
    if [a, b, c, d].iter().any(|byte| byte & 0x80 != 0) {
        return None;
    }
    Some((u32::from(a) << 21) | (u32::from(b) << 14) | (u32::from(c) << 7) | u32::from(d))
}

fn read_flac_prefix_comments(prefix: &[u8]) -> Option<lofty::ogg::VorbisComments> {
    let mut cursor = Cursor::new(prefix);
    FlacFile::read_from(&mut cursor, ParseOptions::new().read_properties(false))
        .ok()
        .map(|flac| flac.vorbis_comments().cloned().unwrap_or_default())
}

fn encode_flac_vorbis_comments(comments: &lofty::ogg::VorbisComments) -> Result<Vec<u8>, ApiError> {
    let vendor = comments.vendor().as_bytes();
    let vendor_len = u32::try_from(vendor.len())
        .map_err(|_| ApiError::MediaSafety("FLAC Vorbis vendor string is too large".to_string()))?;
    let items = comments
        .items()
        .filter(|(_, value)| !value.is_empty())
        .collect::<Vec<_>>();
    let item_count = u32::try_from(items.len())
        .map_err(|_| ApiError::MediaSafety("too many FLAC Vorbis comments".to_string()))?;

    let mut encoded = Vec::new();
    encoded.extend_from_slice(&vendor_len.to_le_bytes());
    encoded.extend_from_slice(vendor);
    encoded.extend_from_slice(&item_count.to_le_bytes());
    for (key, value) in items {
        let item = format!("{key}={value}");
        let item_len = u32::try_from(item.len())
            .map_err(|_| ApiError::MediaSafety("FLAC Vorbis item is too large".to_string()))?;
        encoded.extend_from_slice(&item_len.to_le_bytes());
        encoded.extend_from_slice(item.as_bytes());
    }
    if encoded.len() > 0x00ff_ffff {
        return Err(ApiError::MediaSafety(
            "FLAC Vorbis comment block is too large".to_string(),
        ));
    }
    Ok(encoded)
}

fn rebuild_flac_prefix(layout: &FlacPrefixLayout, comments: &[u8]) -> Option<Vec<u8>> {
    let metadata_start = layout.marker_offset.checked_add(4)?;
    let available = layout.audio_offset.checked_sub(metadata_start)?;
    let mut blocks = Vec::new();
    let mut inserted_comments = false;
    for block in &layout.blocks {
        match block.block_type {
            1 => {}
            4 if !inserted_comments => {
                blocks.push((4_u8, comments));
                inserted_comments = true;
            }
            4 => return None,
            _ => blocks.push((block.block_type, block.data.as_slice())),
        }
    }
    if !inserted_comments {
        let insert_at = usize::from(
            blocks
                .first()
                .is_some_and(|(block_type, _)| *block_type == 0),
        );
        blocks.insert(insert_at, (4_u8, comments));
    }

    let required = blocks.iter().try_fold(0_usize, |sum, (_, data)| {
        sum.checked_add(data.len().checked_add(4)?)
    })?;
    let leftover = available.checked_sub(required)?;
    if (1..4).contains(&leftover) || leftover.saturating_sub(4) > 0x00ff_ffff {
        return None;
    }

    let mut output = Vec::with_capacity(layout.audio_offset);
    output.extend_from_slice(layout.prefix.get(..metadata_start)?);
    let has_padding = leftover >= 4;
    for (index, (block_type, data)) in blocks.iter().enumerate() {
        let last = !has_padding && index + 1 == blocks.len();
        push_flac_block(&mut output, *block_type, data, last)?;
    }
    if has_padding {
        let padding = vec![0_u8; leftover - 4];
        push_flac_block(&mut output, 1, &padding, true)?;
    }
    (output.len() == layout.audio_offset).then_some(output)
}

fn vorbis_comments_match(
    comments: &lofty::ogg::VorbisComments,
    expected_vendor: &str,
    expected_items: &[(String, String)],
) -> bool {
    comments.vendor() == expected_vendor
        && comments
            .items()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .eq(expected_items.iter().cloned())
}

fn commit_flac_metadata_prefix<F>(
    path: &Path,
    original_prefix: &[u8],
    candidate_prefix: &[u8],
    expected_file_len: u64,
    verify: F,
) -> Result<(), ApiError>
where
    F: FnOnce(&[u8]) -> Result<(), ApiError>,
{
    let write_result = (|| -> Result<(), ApiError> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|error| {
                ApiError::WriteTask(format!("open FLAC for in-place write: {error}"))
            })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| ApiError::WriteTask(format!("seek FLAC metadata prefix: {error}")))?;
        file.write_all(candidate_prefix)
            .map_err(|error| ApiError::WriteTask(format!("write FLAC metadata prefix: {error}")))?;
        file.sync_all()
            .map_err(|error| ApiError::WriteTask(format!("sync FLAC metadata prefix: {error}")))?;

        if fs::metadata(path)?.len() != expected_file_len {
            return Err(ApiError::MediaSafety(
                "FLAC size changed during in-place metadata write".to_string(),
            ));
        }
        let mut written_prefix = vec![0_u8; candidate_prefix.len()];
        File::open(path)?.read_exact(&mut written_prefix)?;
        if written_prefix != candidate_prefix {
            return Err(ApiError::MediaSafety(
                "FLAC metadata prefix differs after in-place write".to_string(),
            ));
        }
        verify(&written_prefix)
    })();

    if let Err(error) = write_result {
        let restore_result = (|| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new().write(true).open(path)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(original_prefix)?;
            file.sync_all()
        })();
        return match restore_result {
            Ok(()) => Err(error),
            Err(restore_error) => Err(ApiError::WriteTask(format!(
                "{error}; restoring original FLAC metadata also failed: {restore_error}"
            ))),
        };
    }
    Ok(())
}

fn write_id3_extra_tags_atomic(
    path: &Path,
    updates: &[ExtraTagUpdate],
    wav: bool,
) -> Result<TrackWriteOutcome, ApiError> {
    let mut original = fs::read(path)?;
    if wav {
        fix_wav_orphan_tail(&mut original);
    }
    let mut tag = if wav {
        WavFile::read_from(
            &mut Cursor::new(original.as_slice()),
            ParseOptions::new().read_properties(false),
        )?
        .id3v2()
        .cloned()
        .unwrap_or_default()
    } else {
        let mut file = File::open(path)?;
        MpegFile::read_from(&mut file, ParseOptions::new().read_properties(false))?
            .id3v2()
            .cloned()
            .unwrap_or_default()
    };
    apply_id3_extra_tags(&mut tag, updates);
    normalize_empty_id3_picture_descriptions(&mut tag);
    let original_wav_ranges = if wav {
        Some(
            wav_data_ranges(&original)
                .ok_or_else(|| ApiError::MediaSafety("invalid WAV chunk structure".to_string()))?,
        )
    } else {
        None
    };
    let temporary = sibling_temp_path(path);
    let result = (|| {
        write_loaded_file_data(&original, &temporary)?;
        tag.save_to_path(&temporary, WriteOptions::new())?;
        let candidate = fs::read(&temporary)?;
        let payload_equal = if wav {
            let candidate_ranges = wav_data_ranges(&candidate).ok_or_else(|| {
                ApiError::MediaSafety("invalid written WAV chunk structure".to_string())
            })?;
            byte_ranges_match(
                &original,
                original_wav_ranges
                    .as_deref()
                    .expect("WAV ranges initialized above"),
                &mut Cursor::new(candidate.as_slice()),
                &candidate_ranges
                    .iter()
                    .map(|range| range.start as u64..range.end as u64)
                    .collect::<Vec<_>>(),
            )?
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
) -> Result<ExtraTagWriteReport, ApiError> {
    if let Some(report) = try_flac_extra_tags_inplace(path, updates)? {
        return Ok(report);
    }

    let original = fs::read(path)?;
    let (prepared, repairs) = prepare_flac_source(&original)
        .ok_or_else(|| ApiError::MediaSafety("invalid FLAC metadata boundary".to_string()))?;
    let payload = flac_audio_payload(&prepared).ok_or_else(|| {
        ApiError::MediaSafety("invalid prepared FLAC metadata boundary".to_string())
    })?;
    let target_offset = prepared.len() - payload.len();
    let payload = payload.to_vec();

    // Fallback: full rewrite via local scratch → atomic rename.
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
    result.map(ExtraTagWriteReport::from_full_rewrite)
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
    let original_audio = ogg_audio_packet_ranges(&original, header_packets)
        .ok_or_else(|| ApiError::MediaSafety("invalid OGG packet structure".to_string()))?;
    let temporary = sibling_temp_path(path);
    let result = (|| {
        write_loaded_file_data(&original, &temporary)?;
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
        let candidate_audio =
            ogg_audio_packet_ranges(&candidate, header_packets).ok_or_else(|| {
                ApiError::MediaSafety("invalid written OGG packet structure".to_string())
            })?;
        if !payload_range_groups_equal(&original, &original_audio, &candidate, &candidate_audio) {
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

/// Try to apply an APE tag update in-place by truncating the file to the
/// audio core and appending the new tag.  For remote volumes this reduces
/// the write pass from a full file to just the kilobyte-scale tag.
fn try_ape_inplace_update(
    path: &Path,
    original_bytes: &[u8],
    patch: &TrackPatch,
) -> Result<Option<TrackWriteOutcome>, ApiError> {
    let original_core = ape_audio_core(original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid Monkey audio boundary".to_string()))?;
    // Keep a copy of the original tag region for restoration on failure.
    let original_tag_bytes = &original_bytes[original_core.len()..];
    let before = read_track_metadata(path)?;

    // Build the new tag locally on a scratch file.
    let mut file = File::open(path)?;
    let parsed = ApeFile::read_from(&mut file, ParseOptions::new().read_properties(false))?;
    let mut tag = parsed.ape().cloned().unwrap_or_default();
    apply_ape_patch(&mut tag, patch)?;

    let scratch = sibling_temp_path(path);
    fs::write(&scratch, original_core)?;
    tag.save_to_path(&scratch, WriteOptions::new())?;
    let candidate = fs::read(&scratch)?;
    let _ = fs::remove_file(&scratch);

    // Verify audio core unchanged.
    let candidate_core = ape_audio_core(&candidate).ok_or_else(|| {
        ApiError::MediaSafety("invalid written Monkey audio boundary".to_string())
    })?;
    if candidate_core != original_core {
        return Err(ApiError::MediaSafety(
            "Monkey audio core changed during metadata write".to_string(),
        ));
    }

    let after = read_track_metadata_from_bytes(&candidate)?;
    if same_metadata(before, after) {
        return Ok(Some(TrackWriteOutcome::Skipped));
    }

    // In-place update: truncate to core length, then seek to end and append
    // the new tag.  Without the seek the write cursor is at 0, overwriting the
    // start of the audio core.
    //
    // **Durability note:** this is NOT crash-atomic.  A power loss between
    // set_len and the append will leave the file truncated.  We restore the
    // original tag on every detected write/verify failure, but mid-write
    // crashes cannot be recovered.
    use std::io::Seek;
    let tag_bytes = &candidate[candidate_core.len()..];
    let write_result = (|| -> Result<(), ApiError> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| ApiError::WriteTask(format!("open for APE in-place: {e}")))?;
        f.set_len(original_core.len() as u64)
            .map_err(|e| ApiError::WriteTask(format!("APE truncate: {e}")))?;
        f.seek(std::io::SeekFrom::End(0))
            .map_err(|e| ApiError::WriteTask(format!("APE seek: {e}")))?;
        f.write_all(tag_bytes)
            .map_err(|e| ApiError::WriteTask(format!("APE tag write: {e}")))?;
        f.sync_all()
            .map_err(|e| ApiError::WriteTask(format!("APE tag fsync: {e}")))?;

        // Verify.
        let verify_bytes = fs::read(path)?;
        if ape_audio_core(&verify_bytes) != Some(original_core) {
            return Err(ApiError::MediaSafety(
                "APE audio core changed during in-place write".to_string(),
            ));
        }
        Ok(())
    })();

    match write_result {
        Ok(()) => Ok(Some(TrackWriteOutcome::Replaced)),
        Err(e) => {
            // Restore original tag on any failure after mutation.
            if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
                let _ = f.set_len(original_core.len() as u64);
                let _ = f.seek(std::io::SeekFrom::End(0));
                let _ = f.write_all(original_tag_bytes);
                let _ = f.sync_all();
            }
            Err(e)
        }
    }
}

/// Wrap `read_track_metadata` to accept in-memory bytes for APE validation.
/// Preserves the file extension of `source_path` so Lofty picks the right parser.
fn read_track_metadata_from_bytes(bytes: &[u8]) -> Result<TrackData, ApiError> {
    // Write to a temporary file and read it back — Lofty doesn't accept bytes.
    // We use the raw path bytes so the extension (.ape) is preserved.
    let tmp_name = format!(
        ".ape-verify-{}.ape",
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let tmp = std::env::temp_dir().join(&tmp_name);
    fs::write(&tmp, bytes)?;
    let result = read_track_metadata(&tmp);
    let _ = fs::remove_file(&tmp);
    result
}

/// Write canonical APEv2 metadata after the exact tag-free Monkey audio core.
/// Trailing ID3v1 is intentionally removed, matching Electron characterization.
pub fn write_ape_atomic(path: &Path, patch: &TrackPatch) -> Result<TrackWriteOutcome, ApiError> {
    let original_bytes = fs::read(path)?;

    // Fast path: in-place APE tag update (truncate + append).
    if let Some(outcome) = try_ape_inplace_update(path, &original_bytes, patch)? {
        return Ok(outcome);
    }

    // Fallback: full rewrite via local scratch → SMB staging → atomic rename.
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
    // Standard WAV metadata reads are seekable and bounded. Reading this
    // before the owned source keeps the common path to one full source pass;
    // compatibility layouts retain the established owned-reader fallback.
    let before = read_track_metadata(path)?;
    let mut original_bytes = fs::read(path)?;
    fix_wav_orphan_tail(&mut original_bytes);
    let original_audio = wav_data_ranges(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid WAV chunk structure".to_string()))?;
    // Strip verified all-zero terminal padding before any Lofty
    // write operation so the FourCC warning does not fire on
    // subsequent reads or during the write pipeline.
    strip_wav_padding(&mut original_bytes);

    // NOTE: WAV writes the ID3v2 tag inside a RIFF chunk, not at offset 0 as
    // MP3 does.  An in-place path would need to locate the `id3 ` chunk and
    // replace its data + size fields in the RIFF tree — more complex than the
    // current offset-0 write approach.  Skipped for now; only MP3 uses the
    // shared try_id3v2_inplace_update fast path.

    // Fallback: full rewrite via local scratch + atomic rename.
    let parsed = WavFile::read_from(
        &mut Cursor::new(original_bytes.as_slice()),
        ParseOptions::new().read_properties(false),
    )?;
    let mut tag = parsed.id3v2().cloned().unwrap_or_default();
    preserve_omitted_list_from_tag(&mut tag, "ARTISTS", &patch.artists);
    preserve_omitted_list_from_tag(&mut tag, "ALBUMARTISTS", &patch.album_artists);
    apply_patch(&mut tag, patch);

    let temporary = sibling_temp_path(path);
    let result = (|| {
        // Strip the garbled LIST INFO chunk and trailing null-byte padding
        // before saving, so the ID3v2 values are authoritative after write
        // (no stale INAM/IART/IPRD) and subsequent reads don't emit
        // "invalid FourCC" warnings from Lofty's IFF chunk parser.
        let mut staging = File::create(&temporary)
            .map_err(|e| ApiError::Io(std::io::Error::other(format!("create WAV staging: {e}"))))?;
        write_wav_without_list_info(&original_bytes, &mut staging)
            .map_err(|e| ApiError::Io(std::io::Error::other(format!("write WAV staging: {e}"))))?;
        drop(staging);
        tag.save_to_path(&temporary, WriteOptions::new())?;
        let mut candidate = File::open(&temporary)?;
        let candidate_len = candidate.metadata()?.len();
        if !wav_payloads_match(
            &original_bytes,
            &original_audio,
            &mut candidate,
            candidate_len,
        )? {
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
    let original_media = mp4_mdat_payload_ranges(&original_bytes)
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
        write_loaded_file_data(&original_bytes, &temporary)?;
        parsed.save_to_path(&temporary, WriteOptions::new())?;
        let candidate_bytes = fs::read(&temporary)?;
        let candidate_media = mp4_mdat_payload_ranges(&candidate_bytes).ok_or_else(|| {
            ApiError::MediaSafety("invalid written MP4 atom structure".to_string())
        })?;
        if !payload_ranges_equal(
            &original_bytes,
            &original_media,
            &candidate_bytes,
            &candidate_media,
        ) {
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
    let original_audio = ogg_audio_packet_ranges(&original_bytes, header_packets)
        .ok_or_else(|| ApiError::MediaSafety("invalid OGG packet structure".to_string()))?;
    let before = read_track_metadata(path)?;
    let temporary = sibling_temp_path(path);
    let result = (|| {
        write_loaded_file_data(&original_bytes, &temporary)?;
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
        let candidate_audio = ogg_audio_packet_ranges(&candidate_bytes, header_packets)
            .ok_or_else(|| {
                ApiError::MediaSafety("invalid written OGG packet structure".to_string())
            })?;
        if !payload_range_groups_equal(
            &original_bytes,
            &original_audio,
            &candidate_bytes,
            &candidate_audio,
        ) {
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
    if let Some(report) = try_flac_canonical_prefix_update(path, patch)? {
        tracing::debug!(
            path = %path.display(),
            strategy = report.strategy.as_str(),
            metadata_bytes_read = report.metadata_bytes_read,
            metadata_bytes_written = report.metadata_bytes_written,
            "FLAC canonical write done"
        );
        return Ok(report.outcome);
    }

    let original_bytes = fs::read(path)?;
    let (prepared, repairs) = prepare_flac_source(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid FLAC metadata boundary".to_string()))?;
    let original_payload = flac_audio_payload(&prepared)
        .ok_or_else(|| ApiError::MediaSafety("invalid prepared FLAC boundary".to_string()))?;
    let original_audio_offset = prepared.len() - original_payload.len();
    let original_payload = original_payload.to_vec();

    // Fallback: full rewrite via local scratch → SMB sibling staging →
    // atomic rename (used when metadata won't fit in the existing area
    // or structural repairs are required).
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
    if let Ok(outcome) = result.as_ref() {
        tracing::debug!(
            path = %path.display(),
            strategy = if *outcome == TrackWriteOutcome::Skipped {
                ExtraTagWriteStrategy::Skipped.as_str()
            } else {
                ExtraTagWriteStrategy::FullRewrite.as_str()
            },
            source_bytes_read = original_bytes.len(),
            file_bytes_written = if *outcome == TrackWriteOutcome::Replaced {
                fs::metadata(path).map(|metadata| metadata.len()).unwrap_or_default()
            } else {
                0
            },
            "FLAC canonical write done"
        );
    }
    result
}

/// Try to apply an ID3v2 tag update in-place when the new tag (plus any
/// padding Lofty adds) fits within the original ID3v2 region before the
/// audio data.  Both MP3 and WAV share this approach since Lofty
/// represents both formats' ID3v2 tags via `Id3v2Tag`.
///
/// Currently disabled for MP3 (edge case with Lofty save-to-scratch)
/// and WAV (RIFF chunk structure); re-enable after those are resolved.
#[allow(unused)]
fn try_id3v2_inplace_update(
    path: &Path,
    original_bytes: &[u8],
    payload_offset: usize,
    tag: &mut Id3v2Tag,
    patch: &TrackPatch,
    payload_ok: &dyn Fn(&[u8], &[u8]) -> bool,
) -> Result<Option<TrackWriteOutcome>, ApiError> {
    let original_payload = original_bytes
        .get(payload_offset..)
        .ok_or_else(|| ApiError::MediaSafety("invalid ID3v2 boundary".to_string()))?;
    let before = read_track_metadata(path)?;
    preserve_omitted_list(tag, path, "ARTISTS", &patch.artists);
    preserve_omitted_list(tag, path, "ALBUMARTISTS", &patch.album_artists);
    apply_patch(tag, patch);

    // Build the new file locally by copying the original file and letting
    // Lofty modify the tag.  Since Lofty's save_to_path expects the file
    // to already have the target format's structure (MPEG sync word etc.),
    // we copy the entire original and then measure the tag size change.
    let scratch = sibling_temp_path(path);
    copy_file_data(path, &scratch)?;
    tag.save_to_path(&scratch, WriteOptions::new())?;
    let candidate = fs::read(&scratch)?;
    let after = read_track_metadata(&scratch)?;
    let _ = fs::remove_file(&scratch);

    if same_metadata(before, after) {
        return Ok(Some(TrackWriteOutcome::Skipped));
    }

    // The candidate file now has the new tag.  Compute the new tag size
    // by subtracting the audio payload length from the total length.
    let new_tag_size = candidate.len() - original_payload.len();

    if new_tag_size > payload_offset {
        return Ok(None); // doesn't fit — fall back
    }

    // Verify audio payload is unchanged.
    if !payload_ok(&candidate, original_payload) {
        return Err(ApiError::MediaSafety(
            "audio payload changed during ID3v2 metadata write".to_string(),
        ));
    }

    // In-place write: overwrite the ID3v2 tag at offset 0.
    let tag_bytes = &candidate[..new_tag_size];
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
        f.write_all(tag_bytes)?;
        // Zero-fill any remaining space in the original tag region.
        if new_tag_size < payload_offset {
            let padding = vec![0u8; payload_offset - new_tag_size];
            f.write_all(&padding)?;
        }
        f.sync_all()?;
    }

    // Final verification.
    let verify = fs::read(path)?;
    if !payload_ok(&verify, original_payload) {
        return Err(ApiError::MediaSafety(
            "audio payload changed during in-place ID3v2 write".to_string(),
        ));
    }

    Ok(Some(TrackWriteOutcome::Replaced))
}

/// Write one MP3 through a validated sibling file. The original path is not
/// touched until tag readback and MPEG payload equality both pass.
pub fn write_mp3_atomic(path: &Path, patch: &TrackPatch) -> Result<TrackWriteOutcome, ApiError> {
    let original_bytes = fs::read(path)?;
    let original_payload = mpeg_payload(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid ID3v2 boundary".to_string()))?;
    let _payload_offset = original_bytes.len() - original_payload.len();

    // NOTE: MP3 in-place fast path is disabled until the interaction between
    // Lofty's save_to_path and the pure-audio scratch file is resolved.
    // Tracked by the 3 failing tests: unknown_user_text, unknown_binary_frame,
    // renderer_write_contract. Re-enable after fixing Lofty save-to-scratch.
    //
    // let mut tag = read_id3v2(path)?;
    // if let Some(outcome) = try_id3v2_inplace_update( ...

    // Fallback: full rewrite via local scratch + atomic rename.
    let before = read_track_metadata(path)?;
    let mut tag = read_id3v2(path)?;
    preserve_omitted_list(&mut tag, path, "ARTISTS", &patch.artists);
    preserve_omitted_list(&mut tag, path, "ALBUMARTISTS", &patch.album_artists);
    apply_patch(&mut tag, patch);
    normalize_empty_id3_picture_descriptions(&mut tag);

    let temporary = sibling_temp_path(path);
    let result = (|| {
        write_loaded_file_data(&original_bytes, &temporary)?;
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

/// Lofty serializes a missing APIC description as one zero byte regardless of
/// its declared encoding. For UTF-16 pictures that creates a tag Lofty cannot
/// read back. An empty description has no encoding-dependent content, so use
/// UTF-8 while preserving the picture, frame flags, and all non-picture frames.
fn normalize_empty_id3_picture_descriptions(tag: &mut Id3v2Tag) {
    let pictures = tag.remove(&frame_id("APIC")).collect::<Vec<_>>();
    for mut frame in pictures {
        if let Frame::Picture(picture) = &mut frame {
            if picture.picture.description().is_none()
                && matches!(
                    picture.encoding,
                    TextEncoding::UTF16 | TextEncoding::UTF16BE
                )
            {
                picture.encoding = TextEncoding::UTF8;
            }
        }
        tag.insert(frame);
    }
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
    // ALBUMARTIST / ALBUM ARTIST: Lofty maps both variants to ItemKey::AlbumArtist.
    // When inserting or clearing, remove ALL known variants so no stale entry
    // survives to shadow the new value during readback.
    match &patch.album_artist {
        Patch::Omitted => {}
        Patch::Null => {
            let _ = tag.remove("ALBUMARTIST");
            let _ = tag.remove("ALBUM ARTIST");
        }
        Patch::Value(value) => {
            let _ = tag.remove("ALBUMARTIST");
            let _ = tag.remove("ALBUM ARTIST");
            tag.insert("ALBUMARTIST".to_string(), value.clone());
        }
    }
    // ALBUMARTISTS / ALBUM ARTISTS: same variant removal logic.
    if matches!(patch.album_artists, Patch::Omitted) {
        match &patch.album_artist {
            Patch::Omitted => {}
            Patch::Null => {
                let _ = tag.remove("ALBUMARTISTS");
                let _ = tag.remove("ALBUM ARTISTS");
            }
            Patch::Value(value) => {
                let _ = tag.remove("ALBUMARTISTS");
                let _ = tag.remove("ALBUM ARTISTS");
                tag.insert("ALBUMARTISTS".to_string(), value.clone());
            }
        }
    } else {
        // Clear both variants so no stale "ALBUM ARTISTS" (space) entry survives.
        let _ = tag.remove("ALBUMARTISTS");
        let _ = tag.remove("ALBUM ARTISTS");
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
    let album_artist_active = !patch.album_artist.is_omitted();
    apply_text_frame(tag, "TPE2", &patch.album_artist);
    // Remove legacy TXXX frames that Lofty maps to ItemKey::AlbumArtist
    // ("ALBUM ARTIST", "ALBUMARTIST"). These shadow TPE2 during readback
    // via first_string() because they appear earlier in the frame order.
    if album_artist_active {
        for desc in ["ALBUM ARTIST", "ALBUMARTIST"] {
            tag.remove_user_text(desc);
        }
    }
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

/// Preserve plural values already present in a parsed ID3 tag. WAV writers
/// have the complete source buffer and tag in hand, so rereading the source
/// through `id3_user_text_values` would transfer the PCM payload needlessly.
fn preserve_omitted_list_from_tag(
    tag: &mut Id3v2Tag,
    description: &str,
    patch: &Patch<StringList>,
) {
    if !matches!(patch, Patch::Omitted) {
        return;
    }

    let mut values = Vec::new();
    let mut descriptions = Vec::new();
    for frame in &*tag {
        if let Frame::UserText(frame) = frame {
            if frame.description.eq_ignore_ascii_case(description) {
                descriptions.push(frame.description.to_string());
                values.extend(
                    frame
                        .content
                        .split(';')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned),
                );
            }
        }
    }
    if values.is_empty() {
        return;
    }

    descriptions.sort_unstable();
    descriptions.dedup();
    for existing in descriptions {
        while tag.remove_user_text(&existing).is_some() {}
    }
    tag.insert_user_text(description.to_string(), values.join(";"));
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
    after.bitrate = before.bitrate;
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

/// If the last declared WAV chunk (typically `data`) is followed by orphan
/// bytes up to the RIFF end, expand the data chunk to include them.
///
/// Some WAV files have a few leftover PCM bytes after the declared data chunk
/// end.  These are counted in the RIFF total size but not in any chunk's
/// header, making `wav_data_payloads` reject them.  This function adopts them
/// into the data chunk so the file is self-consistent.
///
/// Guards:
/// - Valid RIFF/WAVE header
/// - PCM fmt with known block_align
/// - A single `data` chunk as the last valid chunk
/// - Orphan tail is bounded (<= 4096 bytes)
/// - Orphan tail is block-aligned
/// - Truncated (incomplete) data is never repaired
fn fix_wav_orphan_tail(data: &mut [u8]) -> bool {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return false;
    }

    let mut offset = 12_usize;
    let mut last_chunk_end = 0_usize;
    let mut data_chunk_pos: Option<usize> = None;
    let mut fmt_block_align: Option<u16> = None;

    while let Some(end) = offset.checked_add(8) {
        if end > data.len() {
            break;
        }
        let id = &data[offset..offset + 4];

        // Stop at null FourCC (verified trailing padding)
        if *id == [0u8; 4] {
            break;
        }

        let size = u32::from_le_bytes(
            data.get(offset + 4..offset + 8)
                .and_then(|s| <[u8; 4]>::try_from(s).ok())
                .unwrap_or([0; 4]),
        ) as usize;
        let chunk_total = 8 + size + (size & 1);
        let Some(next) = offset.checked_add(chunk_total) else {
            return false;
        };

        if next > data.len() {
            // This chunk overflows — everything from `offset` onward is orphan
            break;
        }

        if id == b"data" {
            data_chunk_pos = Some(offset);
        }
        if id == b"fmt " && size >= 16 {
            // block_align is at offset 20 within the fmt chunk
            let ba_bytes: [u8; 2] = data
                .get(offset + 20..offset + 22)
                .and_then(|s| <[u8; 2]>::try_from(s).ok())
                .unwrap_or([0; 2]);
            fmt_block_align = Some(u16::from_le_bytes(ba_bytes));
        }

        last_chunk_end = next;
        offset = next;
    }

    let orphan_start = last_chunk_end;
    let Some(orphan_len) = data.len().checked_sub(orphan_start) else {
        return false;
    };
    if orphan_len == 0 {
        return false;
    }

    // Only repair when the data chunk was the last valid chunk
    let data_pos = match data_chunk_pos {
        Some(p) => p,
        None => return false,
    };
    let old_data_size = match data
        .get(data_pos + 4..data_pos + 8)
        .and_then(|s| <[u8; 4]>::try_from(s).ok())
    {
        Some(s) => u32::from_le_bytes(s) as usize,
        None => return false,
    };
    let data_chunk_end = data_pos + 8 + old_data_size + (old_data_size & 1);
    if data_chunk_end != orphan_start {
        return false;
    }

    // Bounded orphan tail (4 KiB)
    const MAX_ORPHAN_TAIL: usize = 4096;
    if orphan_len > MAX_ORPHAN_TAIL {
        return false;
    }

    // Block-aligned (only when fmt block_align is available; default to 1)
    let block_align = fmt_block_align.unwrap_or(1) as usize;
    if orphan_len % block_align != 0 {
        return false;
    }

    // All checks pass: expand the data chunk to absorb the orphan bytes
    let new_data_size = old_data_size + orphan_len;
    data[data_pos + 4..data_pos + 8].copy_from_slice(&(new_data_size as u32).to_le_bytes());

    // Ensure RIFF total size is consistent
    let riff_len = (data.len() as u32).wrapping_sub(8).to_le_bytes();
    data[4..8].copy_from_slice(&riff_len);

    true
}

#[derive(Clone, Copy)]
struct WavChunk {
    id: [u8; 4],
    start: usize,
    data_start: usize,
    data_end: usize,
    end: usize,
}

fn wav_chunks(bytes: &[u8]) -> Option<Vec<WavChunk>> {
    if bytes.len() < 12 || bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut chunks = Vec::new();
    let mut offset = 12_usize;
    while offset.checked_add(8)? <= bytes.len() {
        let id: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        let size = u32::from_le_bytes(bytes.get(offset + 4..offset + 8)?.try_into().ok()?) as usize;
        let data_start = offset.checked_add(8)?;
        let data_end = data_start.checked_add(size)?;
        if data_end > bytes.len() {
            return None;
        }
        let end = data_end.checked_add(size % 2)?;
        if end > bytes.len() {
            return None;
        }
        chunks.push(WavChunk {
            id,
            start: offset,
            data_start,
            data_end,
            end,
        });
        offset = end;
    }
    (offset == bytes.len()).then_some(chunks)
}

fn wav_data_ranges(bytes: &[u8]) -> Option<Vec<Range<usize>>> {
    let ranges: Vec<_> = wav_chunks(bytes)?
        .into_iter()
        .filter(|chunk| chunk.id == *b"data")
        .map(|chunk| chunk.data_start..chunk.data_end)
        .collect();
    (!ranges.is_empty()).then_some(ranges)
}

#[cfg(test)]
fn wav_data_payloads(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    wav_data_ranges(bytes).map(|ranges| {
        ranges
            .into_iter()
            .map(|range| bytes[range].to_vec())
            .collect()
    })
}

fn wav_data_ranges_from_reader<R: Read + Seek>(
    reader: &mut R,
    file_len: u64,
) -> Result<Option<Vec<Range<u64>>>, ApiError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut riff = [0_u8; 12];
    if reader.read_exact(&mut riff).is_err() || &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" {
        return Ok(None);
    }

    let mut ranges = Vec::new();
    let mut offset = 12_u64;
    while offset < file_len {
        if file_len - offset < 8 {
            return Ok(None);
        }
        reader.seek(SeekFrom::Start(offset))?;
        let mut header = [0_u8; 8];
        reader.read_exact(&mut header)?;
        let size = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap()));
        let Some(data_start) = offset.checked_add(8) else {
            return Ok(None);
        };
        let Some(data_end) = data_start.checked_add(size) else {
            return Ok(None);
        };
        let Some(next) = data_end.checked_add(size & 1) else {
            return Ok(None);
        };
        if next > file_len {
            return Ok(None);
        }
        if &header[..4] == b"data" {
            ranges.push(data_start..data_end);
        }
        offset = next;
    }

    Ok((offset == file_len && !ranges.is_empty()).then_some(ranges))
}

/// Compare corresponding byte ranges without materializing either payload.
/// The container-specific scanners remain responsible for locating ranges;
/// this bounded comparer is reusable by WAV and other contiguous payloads.
fn byte_ranges_match<R: Read + Seek>(
    original: &[u8],
    original_ranges: &[Range<usize>],
    candidate: &mut R,
    candidate_ranges: &[Range<u64>],
) -> Result<bool, ApiError> {
    if original_ranges.len() != candidate_ranges.len() {
        return Ok(false);
    }

    let mut buffer = vec![0_u8; 64 * 1024];
    for (original_range, candidate_range) in original_ranges.iter().zip(candidate_ranges) {
        let original_len = original_range.end.saturating_sub(original_range.start) as u64;
        if original_len != candidate_range.end.saturating_sub(candidate_range.start) {
            return Ok(false);
        }
        candidate.seek(SeekFrom::Start(candidate_range.start))?;
        let mut original_offset = original_range.start;
        while original_offset < original_range.end {
            let count = buffer.len().min(original_range.end - original_offset);
            candidate.read_exact(&mut buffer[..count])?;
            if buffer[..count] != original[original_offset..original_offset + count] {
                return Ok(false);
            }
            original_offset += count;
        }
    }
    Ok(true)
}

fn wav_payloads_match<R: Read + Seek>(
    original: &[u8],
    original_ranges: &[Range<usize>],
    candidate: &mut R,
    candidate_len: u64,
) -> Result<bool, ApiError> {
    let Some(candidate_ranges) = wav_data_ranges_from_reader(candidate, candidate_len)? else {
        return Ok(false);
    };
    byte_ranges_match(original, original_ranges, candidate, &candidate_ranges)
}

/// Strip the RIFF `LIST` chunk from a WAV byte buffer, returning a new
/// buffer with the same audio payload but no LIST INFO metadata.
/// The RIFF total size in the header is updated accordingly.
#[cfg(test)]
fn strip_wav_list_chunk(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    if write_wav_without_list_info(bytes, &mut out).is_err() {
        return bytes.to_vec();
    }
    out
}

/// Write a valid RIFF/WAVE source while omitting only LIST/INFO metadata.
/// Source chunks are borrowed and emitted directly, avoiding a file-sized
/// cleaned buffer alongside the already owned source.
fn write_wav_without_list_info<W: Write>(bytes: &[u8], writer: &mut W) -> std::io::Result<()> {
    let chunks = wav_chunks(bytes)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid WAV"))?;
    let kept_len = chunks
        .iter()
        .filter(|chunk| !wav_chunk_is_list_info(bytes, chunk))
        .try_fold(12_usize, |len, chunk| {
            len.checked_add(chunk.end - chunk.start)
        })
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "WAV too large"))?;
    let riff_len = u32::try_from(kept_len.saturating_sub(8))
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "WAV too large"))?;
    let mut header: [u8; 12] = bytes[..12].try_into().unwrap();
    header[4..8].copy_from_slice(&riff_len.to_le_bytes());
    writer.write_all(&header)?;
    for chunk in chunks
        .iter()
        .filter(|chunk| !wav_chunk_is_list_info(bytes, chunk))
    {
        writer.write_all(&bytes[chunk.start..chunk.end])?;
    }
    Ok(())
}

fn wav_chunk_is_list_info(bytes: &[u8], chunk: &WavChunk) -> bool {
    chunk.id == *b"LIST"
        && chunk
            .data_start
            .checked_add(4)
            .and_then(|end| bytes.get(chunk.data_start..end))
            == Some(b"INFO")
}

fn mp4_mdat_payload_ranges(bytes: &[u8]) -> Option<Vec<Range<usize>>> {
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
            let start = offset.checked_add(header)?;
            bytes.get(start..end)?;
            payloads.push(start..end);
        }
        offset = end;
    }
    (offset == bytes.len() && !payloads.is_empty()).then_some(payloads)
}

#[cfg(test)]
fn mp4_mdat_payloads(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    mp4_mdat_payload_ranges(bytes).map(|payloads| {
        payloads
            .into_iter()
            .map(|range| bytes[range].to_vec())
            .collect()
    })
}

fn ogg_audio_packet_ranges(bytes: &[u8], header_packets: usize) -> Option<Vec<Vec<Range<usize>>>> {
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
            bytes.get(data_offset..next)?;
            if length > 0 {
                packet.push(data_offset..next);
            }
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

#[cfg(test)]
fn ogg_audio_packets(bytes: &[u8], header_packets: usize) -> Option<Vec<Vec<u8>>> {
    ogg_audio_packet_ranges(bytes, header_packets).map(|packets| {
        packets
            .into_iter()
            .map(|ranges| {
                ranges
                    .into_iter()
                    .flat_map(|range| bytes[range].iter().copied())
                    .collect()
            })
            .collect()
    })
}

fn payload_range_groups_equal(
    left: &[u8],
    left_groups: &[Vec<Range<usize>>],
    right: &[u8],
    right_groups: &[Vec<Range<usize>>],
) -> bool {
    if left_groups.len() != right_groups.len() {
        return false;
    }

    left_groups
        .iter()
        .zip(right_groups)
        .all(|(left_ranges, right_ranges)| {
            if left_ranges
                .iter()
                .any(|range| left.get(range.clone()).is_none())
                || right_ranges
                    .iter()
                    .any(|range| right.get(range.clone()).is_none())
            {
                return false;
            }
            left_ranges
                .iter()
                .flat_map(|range| left[range.clone()].iter())
                .eq(right_ranges
                    .iter()
                    .flat_map(|range| right[range.clone()].iter()))
        })
}

fn payload_ranges_equal(
    left: &[u8],
    left_ranges: &[Range<usize>],
    right: &[u8],
    right_ranges: &[Range<usize>],
) -> bool {
    left_ranges.len() == right_ranges.len()
        && left_ranges
            .iter()
            .zip(right_ranges)
            .all(|(left_range, right_range)| {
                left.get(left_range.clone()) == right.get(right_range.clone())
            })
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
    let mut found = false;
    for vendor in [b"soundrobe".as_slice(), b"auto-tagger".as_slice()] {
        let finder = Finder::new(vendor);
        let mut search = audio_offset;
        while search < bytes.len() {
            let Some(relative) = finder.find(&bytes[search..]) else {
                break;
            };
            let position = search + relative;
            if position >= 4 {
                let claimed = u32::from_le_bytes(
                    bytes[position - 4..position].try_into().unwrap_or_default(),
                );
                if claimed as usize == vendor.len() {
                    bytes[position - 4..position].fill(0);
                    found = true;
                }
            }
            search = position + 1;
        }
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
    // Fast path: same-filesystem rename (atomic on Unix).
    match fs::rename(source, destination) {
        Ok(()) => return Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            // Cross-filesystem rename — fall through to copy+rename.
        }
        Err(e) => return Err(e),
    }

    // Staging temp goes in the SAME directory as `destination` so the
    // subsequent rename stays within a single filesystem.
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("track");
    let ext = destination
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tmp");
    let staging =
        destination.with_file_name(format!(".{name}.soundrobe-{pid}-{sequence}.tmp.{ext}"));

    // 1. Copy validated result to staging temp on the target filesystem.
    // 2. Atomic-rename staging over the original.
    // 3. Only then remove the local scratch (the validated result).
    //    If anything fails between copy and rename, the original is untouched
    //    and we can retry.
    if let Err(e) = copy_file_data(source, &staging) {
        let _ = fs::remove_file(&staging);
        return Err(e);
    }
    match fs::rename(&staging, destination) {
        Ok(()) => {
            let _ = fs::remove_file(source);
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&staging);
            Err(e)
        }
    }
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
    // through the call.  Cross-volume moves are handled by the Unix-style
    // copy-to-sibling-temp fallback above (same logic, no MOVEFILE_COPY_ALLOWED
    // which is non-atomic).  This path assumes source and destination are on
    // the same volume so the rename is genuinely atomic.
    //
    // When source and destination ARE on different volumes this will fail
    // with ERROR_NOT_SAME_DEVICE, which is acceptable — the caller's stack
    // should handle the error or use the cross-filesystem fallback.
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

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Copy file data bytes only, without extended attributes or ACLs.
/// macOS `fs::copy` wraps `copyfile()` which preserves xattrs/ACLs via
/// `COPYFILE_ALL`. On SMB volumes the xattr copy fails with `EACCES`,
/// so we use an explicit read/write that transfers only the data fork.
fn copy_file_data(source: &Path, destination: &Path) -> std::io::Result<u64> {
    let mut src = File::open(source)?;
    let mut dst = File::create(destination)?;
    let n = std::io::copy(&mut src, &mut dst)?;
    dst.sync_all()?;
    Ok(n)
}

/// Stage bytes already loaded for payload validation without reopening the
/// source path. This avoids a second remote-volume read before local mutation.
fn write_loaded_file_data(bytes: &[u8], destination: &Path) -> std::io::Result<()> {
    let mut file = File::create(destination)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Returns a temporary file path suitable for atomic replacement of `path`.
///
/// * **Local filesystem** — sibling temp in the same directory (rename stays
///   within the same filesystem, guaranteeing atomicity).
/// * **Remote filesystem** (SMB/NFS/etc.) — local scratch file under the
///   system temp directory.  All intermediate writes, Lofty saves, repacking,
///   and validation happen locally.  The final `replace_file_atomic` then
///   copies the result once to a sibling temp on the remote filesystem and
///   renames it over the original, reducing SMB traffic from (3 writes +
///   2 reads) to (1 read + 1 write).
fn sibling_temp_path(path: &Path) -> PathBuf {
    let is_remote = on_different_filesystem(path);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("track");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("tmp");
    if is_remote {
        std::env::temp_dir().join(format!(
            ".{name}.soundrobe-{pid}-{sequence}.tmp.{extension}"
        ))
    } else {
        path.with_file_name(format!(
            ".{name}.soundrobe-{pid}-{sequence}.tmp.{extension}"
        ))
    }
}

/// Returns `true` when `path` and the system temp directory live on different
/// physical filesystems (e.g. local SSD vs. SMB/NFS mount).  On such volumes,
/// all intermediate read/write/validate work uses a local scratch file so that
/// metadata operations don't amplify SMB traffic.  The final validated result
/// is then copied once to a sibling temp on the target filesystem for an atomic
/// rename commit.
///
/// On Windows the check is skipped (rename is handled by `MoveFileExW` with
/// copy-allowed flags, so the optimization is harmless to omit).
fn on_different_filesystem(path: &Path) -> bool {
    #[cfg(unix)]
    {
        let path_meta = match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return false,
        };
        let temp_meta = match fs::metadata(std::env::temp_dir()) {
            Ok(m) => m,
            Err(_) => return false,
        };
        path_meta.dev() != temp_meta.dev()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::id3::v2::BinaryFrame;
    use lofty::picture::{MimeType, Picture, PictureInformation, PictureType};

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
            "soundrobe-write-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(name);
        fs::copy(source, &path).unwrap();
        (root, path)
    }

    // ------------------------------------------------------------------
    // neutralize_ghost_vorbis unit tests
    // ------------------------------------------------------------------

    #[test]
    fn ghost_no_match_returns_false() {
        let mut buf = b"audio data with nothing matching".to_vec();
        assert!(!neutralize_ghost_vorbis(&mut buf, 0));
    }

    #[test]
    fn ghost_match_before_audio_offset_ignored() {
        // Vendor string appears before audio_offset -> should not be touched
        let vendor = b"soundrobe";
        let prefix = [0u8, 0, 0, 9]; // valid length prefix = 9
        let mut buf = Vec::new();
        buf.extend_from_slice(&prefix);
        buf.extend_from_slice(vendor);
        buf.extend_from_slice(b"audio_payload");
        let audio_offset = (buf.len() - b"audio_payload".len()) as usize;
        assert!(!neutralize_ghost_vorbis(&mut buf, audio_offset));
        // Prefix should still be intact
        assert_eq!(&buf[0..4], &[0u8, 0, 0, 9]);
    }

    #[test]
    fn ghost_valid_prefixed_match_neutralized() {
        let vendor = b"soundrobe"; // length 9
        let len_prefix = 9u32.to_le_bytes(); // [9, 0, 0, 0]
        let mut buf = Vec::new();
        buf.extend_from_slice(b"fLaC");
        // Add some metadata headers to simulate a small FLAC header
        // Last-metadata-block flag + block-type STREAMINFO (0) + length 34
        buf.extend_from_slice(&[0x80, 0, 0, 34]);
        buf.extend_from_slice(&[0u8; 34]); // STREAMINFO
                                           // Now audio payload containing a valid prefixed vendor string
                                           // We need the 4-byte length prefix + vendor string in the audio region
        let audio_off = buf.len();
        buf.extend_from_slice(&len_prefix); // valid length prefix = 9
        buf.extend_from_slice(vendor);
        buf.extend_from_slice(b"more audio data");

        assert!(neutralize_ghost_vorbis(&mut buf, audio_off));
        // The 4-byte prefix should now be zeroed out
        assert_eq!(&buf[audio_off..audio_off + 4], &[0u8, 0, 0, 0]);
        // Vendor string itself is untouched
        assert_eq!(&buf[audio_off + 4..audio_off + 4 + vendor.len()], vendor);
    }

    #[test]
    fn ghost_invalid_prefix_not_touched() {
        let vendor = b"soundrobe";
        // Wrong length prefix (10 instead of 9)
        let wrong_prefix = 10u32.to_le_bytes();
        let mut buf = Vec::new();
        buf.extend_from_slice(b"fLaC");
        buf.extend_from_slice(&[0x80, 0, 0, 34]);
        buf.extend_from_slice(&[0u8; 34]);
        let audio_off = buf.len();
        buf.extend_from_slice(&wrong_prefix);
        buf.extend_from_slice(vendor);
        buf.extend_from_slice(b"more audio");

        assert!(!neutralize_ghost_vorbis(&mut buf, audio_off));
        // Prefix should be intact
        assert_eq!(&buf[audio_off..audio_off + 4], &wrong_prefix);
    }

    #[test]
    fn ghost_multiple_matches_all_neutralized() {
        let vendor = b"auto-tagger"; // length 11
        let len_prefix = 11u32.to_le_bytes();
        let mut buf = Vec::new();
        buf.extend_from_slice(b"fLaC");
        buf.extend_from_slice(&[0x80, 0, 0, 34]);
        buf.extend_from_slice(&[0u8; 34]);
        let audio_off = buf.len();
        // Two valid prefixed matches
        buf.extend_from_slice(&len_prefix);
        buf.extend_from_slice(vendor);
        buf.extend_from_slice(b"some filler ");
        buf.extend_from_slice(&len_prefix);
        buf.extend_from_slice(vendor);

        assert!(neutralize_ghost_vorbis(&mut buf, audio_off));
        // Both 4-byte prefixes should be zeroed
        let first = audio_off;
        let second = audio_off + 4 + vendor.len() + b"some filler ".len();
        assert_eq!(&buf[first..first + 4], &[0u8, 0, 0, 0]);
        assert_eq!(&buf[second..second + 4], &[0u8, 0, 0, 0]);
    }

    #[test]
    fn ghost_overlapping_matches_handled() {
        // Create a scenario where two vendor strings overlap
        // "soundrobe" is 9 bytes; we'll place "soundr" + "oundrobe" such that
        // find() skips correctly after neutralizing
        let vendor = b"soundrobe";
        let len_prefix = 9u32.to_le_bytes();
        let mut buf = Vec::new();
        buf.extend_from_slice(b"fLaC");
        buf.extend_from_slice(&[0x80, 0, 0, 34]);
        buf.extend_from_slice(&[0u8; 34]);
        let audio_off = buf.len();
        // Place two prefixed matches close together
        buf.extend_from_slice(&len_prefix);
        buf.extend_from_slice(vendor);
        buf.extend_from_slice(&len_prefix);
        buf.extend_from_slice(vendor);
        buf.extend_from_slice(b"trailing");

        assert!(neutralize_ghost_vorbis(&mut buf, audio_off));
        // Both prefixes zeroed
        assert_eq!(&buf[audio_off..audio_off + 4], &[0u8, 0, 0, 0]);
        let second = audio_off + 4 + vendor.len();
        assert_eq!(&buf[second..second + 4], &[0u8, 0, 0, 0]);
    }

    #[test]
    fn ghost_audio_payload_outside_prefix_not_touched() {
        let vendor = b"soundrobe";
        let len_prefix = 9u32.to_le_bytes();
        let mut buf = Vec::new();
        buf.extend_from_slice(b"fLaC");
        buf.extend_from_slice(&[0x80, 0, 0, 34]);
        buf.extend_from_slice(&[0u8; 34]);
        let audio_off = buf.len();
        buf.extend_from_slice(&len_prefix);
        buf.extend_from_slice(vendor);
        // The rest of the audio payload
        let audio_start = buf.len();
        buf.extend_from_slice(b"important audio data that must survive intact");

        let payload_snapshot = buf[audio_start..].to_vec();
        neutralize_ghost_vorbis(&mut buf, audio_off);
        // Only the 4 prefix bytes should have changed; everything else intact
        assert_eq!(&buf[audio_start..], payload_snapshot.as_slice());
    }

    // ------------------------------------------------------------------
    // Existing tests
    // ------------------------------------------------------------------

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
    fn camelcase_deserialization_maps_album_artist() {
        // Verify rename_all = "camelCase" maps albumArtist -> album_artist
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "albumArtist": "Diagnostic Test"
        }))
        .unwrap();
        assert_eq!(
            patch.album_artist,
            Patch::Value("Diagnostic Test".to_string())
        );
    }

    /// Regression: when the FLAC file has album artist stored under the Vorbis key
    /// "ALBUM ARTIST" (with space) instead of "ALBUMARTIST" (no space), writing
    /// a new album artist via write_flac_atomic must clear both variants so the
    /// stale space-variant does not shadow the new value on readback.
    #[test]
    fn flac_album_artist_clears_both_vorbis_key_variants() {
        let (root, path) = copy_to_temp(&writer_fixture("flac-bare.flac"), "test.flac");
        let before_bytes = fs::read(&path).unwrap();
        let before_audio = flac_audio_payload(&before_bytes).unwrap().to_vec();

        // Step 1: Seed the file with ALBUM ARTIST (with space) = "old"
        {
            let mut comments = lofty::ogg::VorbisComments::new();
            comments.push("ALBUM ARTIST".to_string(), "old".to_string());
            comments.save_to_path(&path, WriteOptions::new()).unwrap();
        }
        // Confirm the file has ALBUM ARTIST (space) and NOT ALBUMARTIST (no space).
        // Lofty stores raw key-value pairs; "ALBUM ARTIST" and "ALBUMARTIST" are
        // distinct keys at the VorbisComments level (they differ at the space
        // character). The normalization to ItemKey::AlbumArtist only happens when
        // converting to the format-agnostic Tag abstraction.
        {
            let flac = read_flac(&path).unwrap();
            let comments = flac.vorbis_comments().unwrap();
            assert_eq!(
                comments.get("ALBUM ARTIST"),
                Some("old"),
                "seeded with ALBUM ARTIST (space)"
            );
            assert_eq!(
                comments.get("ALBUMARTIST"),
                None,
                "file only has ALBUM ARTIST (space), not ALBUMARTIST (no space)"
            );
        }

        // Step 2: Apply the album-artist patch
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "albumArtist": "new"
        }))
        .unwrap();
        assert_eq!(
            write_flac_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );

        // Step 3: Confirm readback returns the new value
        let meta = read_track_metadata(&path).unwrap();
        assert_eq!(meta.album_artist.as_deref(), Some("new"));

        // Step 4: Confirm stale ALBUM ARTIST (space) is gone and canonical key holds new value
        let flac = read_flac(&path).unwrap();
        let comments = flac.vorbis_comments().unwrap();
        assert_eq!(
            comments.get("ALBUM ARTIST"),
            None,
            "stale ALBUM ARTIST (space) must be cleared"
        );
        assert_eq!(
            comments.get("ALBUMARTIST"),
            Some("new"),
            "canonical ALBUMARTIST (no space) must hold new value"
        );

        // Step 5: Audio payload unchanged
        let after_bytes = fs::read(&path).unwrap();
        assert_eq!(flac_audio_payload(&after_bytes).unwrap(), before_audio);

        fs::remove_dir_all(root).unwrap();
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

    fn install_legacy_utf16_empty_picture(path: &Path) {
        let mut tag = read_id3v2(&path).unwrap();
        let picture = Picture::unchecked(vec![0xff, 0xd8, 0xff, 0xd9])
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Jpeg)
            .description("")
            .build();
        tag.insert(Frame::Picture(lofty::id3::v2::AttachedPictureFrame::new(
            TextEncoding::UTF16,
            picture,
        )));
        tag.save_to_path(&path, WriteOptions::new().use_id3v23(true))
            .unwrap();
        read_track_metadata(&path).unwrap();
    }

    #[test]
    fn legacy_id3v23_utf16_empty_picture_remains_writable() {
        let (root, path) = copy_fixture();
        install_legacy_utf16_empty_picture(&path);
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Changed title"})).unwrap();
        let before = fs::read(&path).unwrap();

        assert_eq!(
            write_mp3_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("Changed title")
        );
        assert_eq!(
            mpeg_payload(&before),
            mpeg_payload(&fs::read(&path).unwrap())
        );
        let written_tag = read_id3v2(&path).unwrap();
        let written_picture = (&written_tag)
            .into_iter()
            .find_map(|frame| match frame {
                Frame::Picture(picture) if picture.picture.data() == [0xff, 0xd8, 0xff, 0xd9] => {
                    Some(picture)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(written_picture.picture.data(), [0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(written_picture.encoding, TextEncoding::UTF8);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_id3v23_utf16_empty_picture_accepts_extra_tag_updates() {
        let (root, path) = copy_fixture();
        install_legacy_utf16_empty_picture(&path);
        let before = fs::read(&path).unwrap();

        assert_eq!(
            write_id3_extra_tags_atomic(
                &path,
                &[ExtraTagUpdate {
                    key: "MOOD".to_string(),
                    value: "Bright".to_string(),
                }],
                false,
            )
            .unwrap(),
            TrackWriteOutcome::Replaced
        );
        assert_eq!(
            read_id3v2(&path).unwrap().get_user_text("MOOD"),
            Some("Bright")
        );
        assert_eq!(
            mpeg_payload(&before),
            mpeg_payload(&fs::read(&path).unwrap())
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Regression: TXXX "ALBUM ARTIST" / "ALBUMARTIST" must not shadow TPE2.
    /// Lofty maps these descriptions to ItemKey::AlbumArtist and they appear
    /// before TPE2 in frame order, so first_string() returns the stale value
    /// and same_metadata skips the write.
    #[test]
    fn album_artist_patch_removes_legacy_txxx_aliases() {
        let (root, path) = copy_fixture();

        // Install legacy aliases into a fresh copy of minimal.mp3
        fn install_aliases(path: &Path) {
            let mut tag = read_id3v2(path).unwrap();
            let val = Patch::Value("Legacy Artist".to_string());
            apply_text_frame(&mut tag, "TPE2", &val);
            tag.insert_user_text("ALBUM ARTIST".to_string(), "Legacy Artist".to_string());
            tag.insert_user_text("ALBUMARTIST".to_string(), "Legacy Artist".to_string());
            // Unrelated TXXX with underscore — must survive
            tag.insert_user_text("ALBUM_ARTIST".to_string(), "Preserved".to_string());
            tag.save_to_path(path, WriteOptions::new()).unwrap();
        }
        install_aliases(&path);
        let before = read_track_metadata(&path).unwrap();
        assert_eq!(before.album_artist.as_deref(), Some("Legacy Artist"));

        // Patch albumArtist to a different value
        let sentinel = "Sentinel Artist";
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "albumArtist": sentinel
        }))
        .unwrap();
        let before_bytes = fs::read(&path).unwrap();

        assert_eq!(
            write_mp3_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Replaced
        );

        // Readback reflects the new value
        let after = read_track_metadata(&path).unwrap();
        assert_eq!(after.album_artist.as_deref(), Some(sentinel));
        assert_eq!(
            read_id3v2(&path).unwrap().get_user_text("ALBUM ARTIST"),
            None
        );
        assert_eq!(
            read_id3v2(&path).unwrap().get_user_text("ALBUMARTIST"),
            None
        );
        // Unrelated ALBUM_ARTIST (underscore) survives
        assert_eq!(
            read_id3v2(&path).unwrap().get_user_text("ALBUM_ARTIST"),
            Some("Preserved")
        );

        // MPEG payload unchanged
        let after_bytes = fs::read(&path).unwrap();
        assert_eq!(mpeg_payload(&before_bytes), mpeg_payload(&after_bytes));
        fs::remove_dir_all(root).unwrap();
    }

    /// Clearing album_artist (null) also removes legacy TXXX aliases.
    #[test]
    fn album_artist_null_clears_legacy_txxx_aliases() {
        let (root, path) = copy_fixture();

        let mut tag = read_id3v2(&path).unwrap();
        let val = Patch::Value("Existing".to_string());
        apply_text_frame(&mut tag, "TPE2", &val);
        tag.insert_user_text("ALBUM ARTIST".to_string(), "Existing".to_string());
        tag.insert_user_text("ALBUMARTIST".to_string(), "Existing".to_string());
        tag.save_to_path(&path, WriteOptions::new()).unwrap();
        assert_eq!(
            read_track_metadata(&path).unwrap().album_artist.as_deref(),
            Some("Existing")
        );

        // Null the album artist
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "albumArtist": null
        }))
        .unwrap();
        write_mp3_atomic(&path, &patch).unwrap();

        let after = read_track_metadata(&path).unwrap();
        assert_eq!(after.album_artist, None);
        // Aliases are gone too
        assert_eq!(
            read_id3v2(&path).unwrap().get_user_text("ALBUM ARTIST"),
            None
        );
        assert_eq!(
            read_id3v2(&path).unwrap().get_user_text("ALBUMARTIST"),
            None
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
        let result = write_ape_atomic(&path, &patch);
        assert_eq!(result.unwrap(), TrackWriteOutcome::Replaced);
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

    /// Intent: payload validation must describe PCM by borrowed byte ranges so
    /// a large WAV write does not clone the complete audio payload into a
    /// second heap allocation before staging begins.
    #[test]
    fn wav_payload_ranges_borrow_large_pcm() {
        let mut wav = fs::read(media_fixture("minimal.wav")).unwrap();
        let data = wav_data_ranges(&wav).unwrap();
        let original_payload = wav[data[0].clone()].as_ptr();

        assert_eq!(data.len(), 1);
        assert_eq!(
            original_payload,
            wav.as_ptr().wrapping_add(data[0].start),
            "the PCM payload must remain a borrowed range into the source buffer"
        );

        let before = wav[data[0].start];
        wav[data[0].start] ^= 0xff;
        assert_eq!(wav[data[0].clone()][0], before ^ 0xff);
    }

    /// Intent: exact payload verification should stream the candidate in
    /// bounded blocks and ignore metadata-only differences.
    #[test]
    fn wav_payload_comparison_is_exact_and_metadata_independent() {
        let original = fs::read(media_fixture("minimal.wav")).unwrap();
        let ranges = wav_data_ranges(&original).unwrap();
        let mut metadata_only_change = original.clone();
        metadata_only_change[20] ^= 0x01;
        let metadata_len = metadata_only_change.len() as u64;
        assert!(wav_payloads_match(
            &original,
            &ranges,
            &mut Cursor::new(metadata_only_change),
            metadata_len,
        )
        .unwrap());

        let mut audio_change = original.clone();
        audio_change[ranges[0].start] ^= 0xff;
        let audio_len = audio_change.len() as u64;
        assert!(!wav_payloads_match(
            &original,
            &ranges,
            &mut Cursor::new(audio_change),
            audio_len,
        )
        .unwrap());
    }

    /// Intent: LIST/INFO cleanup must be streamable without materializing a
    /// second file-sized buffer, while unrelated LIST chunks and PCM survive.
    #[test]
    fn wav_list_info_cleanup_streams_only_kept_chunks() {
        let original = fs::read(media_fixture("synthetic-list-id3.wav")).unwrap();
        let original_ranges = wav_data_ranges(&original).unwrap();
        let mut cleaned = Vec::new();
        write_wav_without_list_info(&original, &mut cleaned).unwrap();

        assert!(cleaned.len() < original.len());
        assert!(!cleaned.windows(4).any(|bytes| bytes == b"INFO"));
        assert!(cleaned.windows(4).any(|bytes| bytes == b"adtl"));
        let cleaned_ranges = wav_data_ranges(&cleaned).unwrap();
        assert_eq!(cleaned_ranges.len(), original_ranges.len());
        for (before, after) in original_ranges.iter().zip(&cleaned_ranges) {
            assert_eq!(&original[before.clone()], &cleaned[after.clone()]);
        }
    }

    /// Intent: applying an unrelated WAV patch must preserve plural ID3
    /// identities from the already parsed tag, without rereading the source.
    #[test]
    fn wav_write_preserves_omitted_plural_fields_from_parsed_tag() {
        let (root, path) = copy_to_temp(&media_fixture("minimal.wav"), "track.wav");
        let mut source = File::open(&path).unwrap();
        let parsed =
            WavFile::read_from(&mut source, ParseOptions::new().read_properties(false)).unwrap();
        let mut tag = parsed.id3v2().cloned().unwrap_or_default();
        tag.insert_user_text("ARTISTS".to_string(), "Primary; Guest".to_string());
        tag.insert_user_text(
            "ALBUMARTISTS".to_string(),
            "Album primary; Album guest".to_string(),
        );
        tag.save_to_path(&path, WriteOptions::new()).unwrap();

        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Changed"})).unwrap();
        write_wav_atomic(&path, &patch).unwrap();

        assert_eq!(id3_user_text_values(&path, "ARTISTS"), ["Primary", "Guest"]);
        assert_eq!(
            id3_user_text_values(&path, "ALBUMARTISTS"),
            ["Album primary", "Album guest"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// WAV with both garbled LIST INFO and correct ID3v2 must prefer the
    /// ID3v2 values on read and strip the stale LIST chunk on write.
    #[test]
    fn wav_read_prefers_id3v2_over_list_info_and_write_strips_list() {
        let path = media_fixture("synthetic-list-id3.wav");

        // Verify read prefers ID3v2 values over garbled LIST INFO.
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(
            track.title.as_deref(),
            Some("从今以后"),
            "title should come from ID3v2, not LIST INAM"
        );
        assert_eq!(
            track.artist.as_deref(),
            Some("信乐团"),
            "artist should come from ID3v2, not LIST IART"
        );
        assert_eq!(
            track.album.as_deref(),
            Some("挑信"),
            "album should come from ID3v2, not LIST IPRD"
        );

        // Verify write strips the LIST INFO chunk but preserves non-INFO
        // LIST chunks (e.g. adtl cue labels).
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"title": "Updated After Write"})).unwrap();
        let original_audio = wav_data_payloads(&fs::read(&path).unwrap()).unwrap();
        {
            let (root, tmp) = copy_to_temp(&path, "synthetic-list-id3.wav");
            write_wav_atomic(&tmp, &patch).unwrap();
            assert_eq!(
                wav_data_payloads(&fs::read(&tmp).unwrap()).unwrap(),
                original_audio
            );
            // After write, LIST INFO must be absent, but LIST adtl must survive.
            let written = fs::read(&tmp).unwrap();
            let mut saw_adtl = false;
            let mut saw_info = false;
            let mut off = 12_usize;
            while off + 8 <= written.len() {
                let id = &written[off..off + 4];
                if id == b"LIST" && off + 12 <= written.len() {
                    match &written[off + 8..off + 12] {
                        b"INFO" => saw_info = true,
                        b"adtl" => saw_adtl = true,
                        _ => {}
                    }
                }
                let size =
                    u32::from_le_bytes(written[off + 4..off + 8].try_into().unwrap()) as usize;
                off += 8 + size + (size % 2);
            }
            assert!(!saw_info, "LIST INFO must be stripped on write");
            assert!(saw_adtl, "LIST adtl must survive on write");
            fs::remove_dir_all(root).unwrap();
        }

        // Verify strip_wav_list_chunk directly (INFO only, not adtl).
        let bytes = fs::read(&path).unwrap();
        assert!(
            bytes.windows(4).any(|w| w == b"adtl"),
            "fixture must have adtl chunk"
        );
        let cleaned = strip_wav_list_chunk(&bytes);
        assert!(
            cleaned.len() < bytes.len(),
            "stripped file should be smaller"
        );
        let mut saw_adtl = false;
        let mut saw_info = false;
        let mut off = 12_usize;
        while off + 8 <= cleaned.len() {
            let id = &cleaned[off..off + 4];
            if id == b"LIST" && off + 12 <= cleaned.len() {
                match &cleaned[off + 8..off + 12] {
                    b"INFO" => saw_info = true,
                    b"adtl" => saw_adtl = true,
                    _ => {}
                }
            }
            let size = u32::from_le_bytes(cleaned[off + 4..off + 8].try_into().unwrap()) as usize;
            off += 8 + size + (size % 2);
        }
        assert!(!saw_info, "strip_wav_list_chunk must remove LIST INFO");
        assert!(saw_adtl, "strip_wav_list_chunk must preserve LIST adtl");
    }

    /// Regression: a WAV whose RIFF total accounts for orphan bytes after
    /// the last declared `data` chunk must be repairable via
    /// `fix_wav_orphan_tail` so `write_wav_atomic` can validate and write.
    ///
    /// The repair expands the data chunk to absorb the orphan bytes (they
    /// become part of the audio payload).  The test verifies:
    /// - `wav_data_payloads` (strict) rejects the unrepaired buffer
    /// - `fix_wav_orphan_tail` succeeds and expands the payload by 8 bytes
    /// - The expanded payload ends with the exact orphan tail
    /// - `write_wav_atomic` writes and preserves the expanded payload
    /// - Truncated-data and non-block-aligned cases are correctly rejected
    #[test]
    fn fix_wav_orphan_tail_absorbs_block_aligned_tail() {
        let (root, path) = copy_to_temp(&media_fixture("minimal.wav"), "track.wav");
        let original_clean = fs::read(&path).unwrap();

        // Build a junked WAV: append 8 bytes (block-aligned for 16-bit mono
        // — block_align=2 from fmt) after the data chunk, update RIFF total.
        let orphan_bytes: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00];
        let mut junked = original_clean.clone();
        junked.extend_from_slice(&orphan_bytes);
        let riff_len = (junked.len() as u32).wrapping_sub(8).to_le_bytes();
        junked[4..8].copy_from_slice(&riff_len);
        fs::write(&path, &junked).unwrap();
        let original_data_len = original_clean.len() - 44; // data payload size

        // 1) Strict wav_data_payloads rejects the unrepaired buffer
        assert!(
            wav_data_payloads(&junked).is_none(),
            "strict wav_data_payloads must reject orphan-tail WAV"
        );

        // 2) fix_wav_orphan_tail succeeds
        let mut repaired = junked.clone();
        assert!(fix_wav_orphan_tail(&mut repaired));

        // 3) Expanded payload is 8 bytes longer and ends with orphan bytes
        let payloads = wav_data_payloads(&repaired).unwrap();
        assert_eq!(payloads.len(), 1);
        assert_eq!(
            payloads[0].len(),
            original_data_len + orphan_bytes.len(),
            "data payload must grow by orphan size"
        );
        assert_eq!(
            &payloads[0][payloads[0].len() - orphan_bytes.len()..],
            &orphan_bytes[..],
            "expanded payload must end with the orphan bytes"
        );

        // 4) strip_wav_padding on the repaired buffer does nothing
        //    (the orphan bytes are now inside the data chunk, not padding)
        let mut for_padding = repaired.clone();
        strip_wav_padding(&mut for_padding);
        assert_eq!(for_padding.len(), repaired.len());

        // 5) write_wav_atomic writes and preserves the expanded payload
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "After Repair Write",
        }))
        .unwrap();
        fs::write(&path, &junked).unwrap(); // restore junked original
        let outcome = write_wav_atomic(&path, &patch).unwrap();
        assert_eq!(outcome, TrackWriteOutcome::Replaced);
        let written = fs::read(&path).unwrap();
        let written_payloads = wav_data_payloads(&written).unwrap();
        assert_eq!(
            written_payloads[0], payloads[0],
            "expanded data payload must be preserved after write"
        );
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title.as_deref(), Some("After Repair Write"));

        // 6) Second identical write → Skipped
        assert_eq!(
            write_wav_atomic(&path, &patch).unwrap(),
            TrackWriteOutcome::Skipped
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// `fix_wav_orphan_tail` must NOT repair truncated data (incomplete
    /// data chunk) because the orphan bytes would be interpreted as part
    /// of the incomplete chunk, not as a trailing tail.
    #[test]
    fn fix_wav_orphan_tail_rejects_truncated_data() {
        let mut bytes = fs::read(&media_fixture("minimal.wav")).unwrap();
        // Truncate in the middle of the data chunk (after first 100 bytes).
        bytes.truncate(44 + 100);
        assert!(!fix_wav_orphan_tail(&mut bytes));
    }

    /// `fix_wav_orphan_tail` must NOT repair a non-block-aligned tail.
    #[test]
    fn fix_wav_orphan_tail_rejects_non_aligned_tail() {
        let (root, path) = copy_to_temp(&media_fixture("minimal.wav"), "track.wav");
        let mut bytes = fs::read(&path).unwrap();
        // Append 7 bytes (not block-aligned for block_align=2).
        bytes.extend_from_slice(&[0xFFu8; 7]);
        let riff_len = (bytes.len() as u32).wrapping_sub(8).to_le_bytes();
        bytes[4..8].copy_from_slice(&riff_len);
        assert!(!fix_wav_orphan_tail(&mut bytes));
        fs::remove_dir_all(root).unwrap();
    }

    /// `fix_wav_orphan_tail` must NOT repair a malformed chunk before data.
    #[test]
    fn fix_wav_orphan_tail_rejects_malformed_chunk_before_data() {
        let mut bytes = fs::read(&media_fixture("minimal.wav")).unwrap();
        // Corrupt the fmt chunk: set an impossible size that overflows.
        bytes[16..20].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
        assert!(!fix_wav_orphan_tail(&mut bytes));
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

    /// Intent: MP4 safety validation should describe `mdat` with borrowed
    /// ranges, avoiding a second allocation proportional to encoded audio.
    #[test]
    fn mp4_payload_ranges_borrow_mdat_bytes() {
        let bytes = fs::read(media_fixture("minimal.m4a")).unwrap();
        let groups = mp4_mdat_payload_ranges(&bytes).unwrap();
        let range = groups.first().unwrap();

        assert_eq!(
            bytes[range.clone()].as_ptr(),
            bytes.as_ptr().wrapping_add(range.start)
        );
        assert!(payload_ranges_equal(&bytes, &groups, &bytes, &groups));
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

    /// Intent: OGG/Opus safety validation must compare logical packets exactly
    /// without cloning all encoded audio, while ignoring container-only bytes.
    #[test]
    fn ogg_packet_range_comparison_is_exact_and_container_independent() {
        let original = fs::read(writer_fixture("vorbis.ogg")).unwrap();
        let ranges = ogg_audio_packet_ranges(&original, 3).unwrap();
        let first_audio = ranges.first().and_then(|packet| packet.first()).unwrap();
        assert_eq!(
            original[first_audio.clone()].as_ptr(),
            original.as_ptr().wrapping_add(first_audio.start)
        );

        let mut container_only = original.clone();
        container_only[22] ^= 0xff; // Ogg page checksum, outside packet data.
        let container_ranges = ogg_audio_packet_ranges(&container_only, 3).unwrap();
        assert!(payload_range_groups_equal(
            &original,
            &ranges,
            &container_only,
            &container_ranges,
        ));

        let mut audio_change = original.clone();
        audio_change[first_audio.start] ^= 0xff;
        let changed_ranges = ogg_audio_packet_ranges(&audio_change, 3).unwrap();
        assert!(!payload_range_groups_equal(
            &original,
            &ranges,
            &audio_change,
            &changed_ranges,
        ));
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
        let outcome = write_flac_atomic(&path, &patch).unwrap();
        assert_eq!(outcome, TrackWriteOutcome::Skipped);
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

    // ------------------------------------------------------------------
    // FLAC in-place fast-path tests
    // ------------------------------------------------------------------

    #[test]
    fn flac_canonical_prefix_update_is_bounded_by_metadata_not_audio() {
        let (root, path) = copy_to_temp(
            &writer_fixture("padded.flac"),
            "canonical-prefix-bounded.flac",
        );
        let before = fs::read(&path).unwrap();
        let before_audio = flac_audio_payload(&before).unwrap().to_vec();
        let before_offset = flac_audio_offset(&before).unwrap();
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"album": "Bounded Prefix Album"})).unwrap();

        let report = try_flac_canonical_prefix_update(&path, &patch)
            .unwrap()
            .expect("padded FLAC should use the prefix-only canonical writer");

        assert_eq!(report.outcome, TrackWriteOutcome::Replaced);
        assert_eq!(report.strategy, ExtraTagWriteStrategy::InPlace);
        assert!(
            report.metadata_bytes_read <= (before_offset * 2 + FLAC_GHOST_PROBE_BYTES + 42) as u64,
            "canonical fast path must not read the audio payload"
        );
        assert_eq!(report.metadata_bytes_written, before_offset as u64);
        let after = fs::read(&path).unwrap();
        assert_eq!(flac_audio_offset(&after).unwrap(), before_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), before_audio);
        assert_eq!(
            read_track_metadata(&path).unwrap().album.as_deref(),
            Some("Bounded Prefix Album")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_inplace_fits_with_padding() {
        // padded.flac has ample PADDING — the in-place fast path should succeed.
        let (root, path) = copy_to_temp(&writer_fixture("padded.flac"), "inplace-padded.flac");
        let before_audio = flac_audio_payload(&fs::read(&path).unwrap())
            .unwrap()
            .to_vec();
        let before_offset = flac_audio_offset(&fs::read(&path).unwrap()).unwrap();

        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"album": "In-place Album"})).unwrap();
        let outcome = write_flac_atomic(&path, &patch).unwrap();
        assert_eq!(outcome, TrackWriteOutcome::Replaced);

        // Audio payload and offset must be identical to before.
        let after = fs::read(&path).unwrap();
        assert_eq!(flac_audio_offset(&after).unwrap(), before_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), before_audio);

        // Verify the metadata was actually written.
        let meta = read_track_metadata(&path).unwrap();
        assert_eq!(meta.album.as_deref(), Some("In-place Album"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_inplace_falls_back_when_metadata_grows() {
        // Use flac-bare.flac which has no PADDING — any metadata addition
        // exceeds the available space and forces a full rewrite.
        let (root, path) = copy_to_temp(&writer_fixture("flac-bare.flac"), "inplace-bare.flac");
        let before_audio = flac_audio_payload(&fs::read(&path).unwrap())
            .unwrap()
            .to_vec();
        let before_offset = flac_audio_offset(&fs::read(&path).unwrap()).unwrap();

        // Add a long title that won't fit in the tiny bare FLAC.
        let patch: TrackPatch = serde_json::from_value(
            serde_json::json!({"title": "A Very Long Title That Exceeds The Available Padding Space In This Minimal File"}),
        ).unwrap();
        let outcome = write_flac_atomic(&path, &patch).unwrap();
        assert_eq!(outcome, TrackWriteOutcome::Replaced);

        // Even though the in-place path was skipped, the fallback must
        // still produce a valid file with identical audio.
        let after = fs::read(&path).unwrap();
        assert_eq!(flac_audio_offset(&after).unwrap(), before_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), before_audio);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_inplace_extra_tags_fits_with_padding() {
        let (root, path) =
            copy_to_temp(&writer_fixture("padded.flac"), "inplace-extra-padded.flac");
        let before_audio = flac_audio_payload(&fs::read(&path).unwrap())
            .unwrap()
            .to_vec();
        let before_offset = flac_audio_offset(&fs::read(&path).unwrap()).unwrap();

        let updates = vec![ExtraTagUpdate {
            key: "CUSTOM_KEY".to_string(),
            value: "custom_value".to_string(),
        }];
        let report = write_flac_extra_tags_atomic(&path, &updates).unwrap();
        assert_eq!(report.outcome, TrackWriteOutcome::Replaced);
        assert_eq!(report.strategy, ExtraTagWriteStrategy::InPlace);
        assert!(
            report.metadata_bytes_read <= (before_offset * 2 + FLAC_GHOST_PROBE_BYTES + 42) as u64
        );
        assert_eq!(report.metadata_bytes_written, before_offset as u64);

        let after = fs::read(&path).unwrap();
        assert_eq!(flac_audio_offset(&after).unwrap(), before_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), before_audio);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_extra_tags_shrink_into_zero_padding_uses_prefix_path() {
        let (root, path) = copy_to_temp(&writer_fixture("padded.flac"), "extra-zero-padding.flac");

        let mut flac = read_flac(&path).unwrap();
        flac.vorbis_comments_mut()
            .unwrap()
            .push("CUSTOM_LONG".to_string(), "x".repeat(256));
        flac.save_to_path(&path, WriteOptions::new()).unwrap();

        let seeded = fs::read(&path).unwrap();
        let seeded_audio = flac_audio_payload(&seeded).unwrap().to_vec();
        let seeded_offset = flac_audio_offset(&seeded).unwrap();
        let padding_len = read_flac_prefix_layout(&path)
            .unwrap()
            .unwrap()
            .blocks
            .into_iter()
            .find(|block| block.block_type == 1)
            .map(|block| block.data.len())
            .unwrap();
        let zero_padding_offset = seeded_offset - padding_len;
        let zero_padded =
            repack_flac_metadata(&seeded, zero_padding_offset, &seeded_audio).unwrap();
        fs::write(&path, zero_padded).unwrap();

        let before = fs::read(&path).unwrap();
        let before_offset = flac_audio_offset(&before).unwrap();
        assert_eq!(
            read_flac_prefix_layout(&path)
                .unwrap()
                .unwrap()
                .blocks
                .into_iter()
                .find(|block| block.block_type == 1)
                .unwrap()
                .data
                .len(),
            0
        );

        let report = write_flac_extra_tags_atomic(
            &path,
            &[ExtraTagUpdate {
                key: "MOOD".to_string(),
                value: "Calm".to_string(),
            }],
        )
        .unwrap();
        assert_eq!(report.strategy, ExtraTagWriteStrategy::InPlace);
        assert_eq!(report.outcome, TrackWriteOutcome::Replaced);

        let after = fs::read(&path).unwrap();
        assert_eq!(flac_audio_offset(&after).unwrap(), before_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), seeded_audio);
        assert!(crate::commands::tracks::read_extra_tags(&path)
            .iter()
            .any(|row| row.key == "MOOD" && row.value == "Calm"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_extra_tags_growth_without_space_falls_back() {
        let (root, path) =
            copy_to_temp(&writer_fixture("padded.flac"), "extra-growth-fallback.flac");
        let before = fs::read(&path).unwrap();
        let before_audio = flac_audio_payload(&before).unwrap().to_vec();
        let before_offset = flac_audio_offset(&before).unwrap();
        let no_padding =
            repack_flac_metadata(&before, before_offset - 8192, &before_audio).unwrap();
        fs::write(&path, no_padding).unwrap();

        let report = write_flac_extra_tags_atomic(
            &path,
            &[ExtraTagUpdate {
                key: "VERY_LARGE".to_string(),
                value: "y".repeat(32 * 1024),
            }],
        )
        .unwrap();
        assert_eq!(report.strategy, ExtraTagWriteStrategy::FullRewrite);
        assert_eq!(report.outcome, TrackWriteOutcome::Replaced);
        assert_eq!(
            flac_audio_payload(&fs::read(&path).unwrap()).unwrap(),
            before_audio
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_extra_tags_identical_update_skips_without_writing() {
        let (root, path) = copy_to_temp(&writer_fixture("padded.flac"), "extra-noop-prefix.flac");
        let updates = [ExtraTagUpdate {
            key: "MOOD".to_string(),
            value: "Calm".to_string(),
        }];
        write_flac_extra_tags_atomic(&path, &updates).unwrap();
        let before = fs::read(&path).unwrap();

        let report = write_flac_extra_tags_atomic(&path, &updates).unwrap();
        assert_eq!(report.outcome, TrackWriteOutcome::Skipped);
        assert_eq!(report.strategy, ExtraTagWriteStrategy::Skipped);
        assert_eq!(report.metadata_bytes_written, 0);
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_extra_tags_reordered_repeated_values_are_not_skipped() {
        let (root, path) = copy_to_temp(
            &writer_fixture("padded.flac"),
            "extra-reordered-artists.flac",
        );
        let original_order = [
            ExtraTagUpdate {
                key: "ARTISTS".to_string(),
                value: "First Artist".to_string(),
            },
            ExtraTagUpdate {
                key: "ARTISTS".to_string(),
                value: "Second Artist".to_string(),
            },
        ];
        write_flac_extra_tags_atomic(&path, &original_order).unwrap();

        let reversed_order = [
            ExtraTagUpdate {
                key: "ARTISTS".to_string(),
                value: "Second Artist".to_string(),
            },
            ExtraTagUpdate {
                key: "ARTISTS".to_string(),
                value: "First Artist".to_string(),
            },
        ];
        let report = write_flac_extra_tags_atomic(&path, &reversed_order).unwrap();

        assert_eq!(report.outcome, TrackWriteOutcome::Replaced);
        assert_eq!(report.strategy, ExtraTagWriteStrategy::InPlace);
        let artists = crate::commands::tracks::read_extra_tags(&path)
            .into_iter()
            .filter(|row| row.key == "ARTISTS")
            .map(|row| row.value)
            .collect::<Vec<_>>();
        assert_eq!(artists, ["Second Artist", "First Artist"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_extra_tags_bare_file_uses_existing_padding() {
        let (root, path) =
            copy_to_temp(&writer_fixture("flac-bare.flac"), "extra-bare-prefix.flac");
        assert!(read_flac(&path).unwrap().vorbis_comments().is_none());
        let before = fs::read(&path).unwrap();
        let before_audio = flac_audio_payload(&before).unwrap().to_vec();
        let before_offset = flac_audio_offset(&before).unwrap();

        let report = write_flac_extra_tags_atomic(
            &path,
            &[ExtraTagUpdate {
                key: "MOOD".to_string(),
                value: "Calm".to_string(),
            }],
        )
        .unwrap();
        assert_eq!(report.strategy, ExtraTagWriteStrategy::InPlace);
        let after = fs::read(&path).unwrap();
        assert_eq!(flac_audio_offset(&after).unwrap(), before_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), before_audio);
        assert!(crate::commands::tracks::read_extra_tags(&path)
            .iter()
            .any(|row| row.key == "MOOD" && row.value == "Calm"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_extra_tags_preserve_picture_and_non_comment_blocks() {
        let (root, path) =
            copy_to_temp(&writer_fixture("padded.flac"), "extra-picture-prefix.flac");
        let mut flac = read_flac(&path).unwrap();
        flac.insert_picture(
            Picture::unchecked(vec![0x89, b'P', b'N', b'G'])
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
            Some(PictureInformation::default()),
        )
        .unwrap();
        flac.save_to_path(&path, WriteOptions::new()).unwrap();

        let before_layout = read_flac_prefix_layout(&path).unwrap().unwrap();
        let before_blocks = before_layout
            .blocks
            .iter()
            .filter(|block| !matches!(block.block_type, 1 | 4))
            .map(|block| (block.block_type, block.data.clone()))
            .collect::<Vec<_>>();
        let before_audio = flac_audio_payload(&fs::read(&path).unwrap())
            .unwrap()
            .to_vec();

        let report = write_flac_extra_tags_atomic(
            &path,
            &[
                ExtraTagUpdate {
                    key: "BARCODE".to_string(),
                    value: "111".to_string(),
                },
                ExtraTagUpdate {
                    key: "BARCODE".to_string(),
                    value: "222".to_string(),
                },
                ExtraTagUpdate {
                    key: "TITLE".to_string(),
                    value: "must remain editor-owned".to_string(),
                },
            ],
        )
        .unwrap();
        assert_eq!(report.strategy, ExtraTagWriteStrategy::InPlace);

        let after_layout = read_flac_prefix_layout(&path).unwrap().unwrap();
        let after_blocks = after_layout
            .blocks
            .iter()
            .filter(|block| !matches!(block.block_type, 1 | 4))
            .map(|block| (block.block_type, block.data.clone()))
            .collect::<Vec<_>>();
        assert_eq!(after_blocks, before_blocks);
        assert_eq!(
            flac_audio_payload(&fs::read(&path).unwrap()).unwrap(),
            before_audio
        );
        assert_eq!(read_flac(&path).unwrap().pictures().len(), 1);
        let rows = crate::commands::tracks::read_extra_tags(&path);
        assert_eq!(rows.iter().filter(|row| row.key == "BARCODE").count(), 2);
        assert!(!rows.iter().any(|row| row.key == "TITLE"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_extra_tags_preserve_picture_stored_in_vorbis_comments() {
        let (root, path) =
            copy_to_temp(&writer_fixture("padded.flac"), "extra-comment-picture.flac");
        let layout = read_flac_prefix_layout(&path).unwrap().unwrap();
        let mut comments = read_flac_prefix_comments(&layout.prefix).unwrap();
        let picture = Picture::unchecked(vec![0x89, b'P', b'N', b'G'])
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Png)
            .build();
        let encoded_picture =
            String::from_utf8(picture.as_flac_bytes(PictureInformation::default(), true)).unwrap();
        comments.push("METADATA_BLOCK_PICTURE".to_string(), encoded_picture);
        let encoded_comments = encode_flac_vorbis_comments(&comments).unwrap();
        let seeded_prefix = rebuild_flac_prefix(&layout, &encoded_comments).unwrap();
        commit_flac_metadata_prefix(
            &path,
            &layout.prefix,
            &seeded_prefix,
            layout.file_len,
            |_| Ok(()),
        )
        .unwrap();

        let seeded = read_flac(&path).unwrap();
        assert_eq!(seeded.vorbis_comments().unwrap().pictures().len(), 1);
        assert!(seeded.pictures().is_empty());

        let updates = [ExtraTagUpdate {
            key: "MOOD".to_string(),
            value: "Calm".to_string(),
        }];
        assert!(try_flac_extra_tags_inplace(&path, &updates)
            .unwrap()
            .is_none());
        let report = write_flac_extra_tags_atomic(&path, &updates).unwrap();

        assert_eq!(report.strategy, ExtraTagWriteStrategy::FullRewrite);
        let written = read_flac(&path).unwrap();
        assert_eq!(
            written.pictures().len()
                + written
                    .vorbis_comments()
                    .map(|tag| tag.pictures().len())
                    .unwrap_or_default(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_extra_tags_repair_cases_bypass_prefix_path() {
        for name in [
            "flac-duplicate-vc.flac",
            "flac-trailing-ape.flac",
            "flac-ghost-vc.flac",
        ] {
            let (root, path) = copy_to_temp(&writer_fixture(name), name);
            let attempt = try_flac_extra_tags_inplace(
                &path,
                &[ExtraTagUpdate {
                    key: "MOOD".to_string(),
                    value: "Calm".to_string(),
                }],
            )
            .unwrap();
            assert!(attempt.is_none(), "{name} must use the repair path");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn flac_prefix_commit_restores_original_after_verification_failure() {
        let (root, path) =
            copy_to_temp(&writer_fixture("padded.flac"), "extra-restore-prefix.flac");
        let original_file = fs::read(&path).unwrap();
        let layout = read_flac_prefix_layout(&path).unwrap().unwrap();
        let mut candidate = layout.prefix.clone();
        candidate[layout.marker_offset + 8] ^= 0x01;

        let error =
            commit_flac_metadata_prefix(&path, &layout.prefix, &candidate, layout.file_len, |_| {
                Err(ApiError::MediaSafety(
                    "injected verification failure".to_string(),
                ))
            })
            .unwrap_err();
        assert!(error.to_string().contains("injected verification failure"));
        assert_eq!(fs::read(&path).unwrap(), original_file);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_flac_extra_tags_never_use_prefix_path() {
        let updates = [ExtraTagUpdate {
            key: "MOOD".to_string(),
            value: "Calm".to_string(),
        }];

        let (repair_root, repair_path) = copy_to_temp(
            &media_fixture("malformed-vorbis-length.flac"),
            "malformed-vorbis-length.flac",
        );
        assert!(try_flac_extra_tags_inplace(&repair_path, &updates)
            .unwrap()
            .is_none());
        assert_eq!(
            write_flac_extra_tags_atomic(&repair_path, &updates)
                .unwrap()
                .strategy,
            ExtraTagWriteStrategy::FullRewrite
        );
        fs::remove_dir_all(repair_root).unwrap();

        let (bad_root, bad_path) = copy_to_temp(
            &media_fixture("malformed-truncated.flac"),
            "malformed-truncated.flac",
        );
        let before = fs::read(&bad_path).unwrap();
        assert!(try_flac_extra_tags_inplace(&bad_path, &updates)
            .unwrap()
            .is_none());
        assert!(write_flac_extra_tags_atomic(&bad_path, &updates).is_err());
        assert_eq!(fs::read(&bad_path).unwrap(), before);
        fs::remove_dir_all(bad_root).unwrap();
    }

    #[test]
    fn flac_inplace_restores_metadata_on_payload_mismatch() {
        // Verify that after a successful in-place write, the file is valid
        // and audio is preserved.  (True payload-corruption injection would
        // require a mock layer; this test at least exercises the write path
        // and confirms the restoration buffer is populated.)
        let (root, path) = copy_to_temp(&writer_fixture("padded.flac"), "inplace-restore.flac");
        let original_bytes = fs::read(&path).unwrap();
        let original_audio = flac_audio_payload(&original_bytes).unwrap().to_vec();
        let original_offset = flac_audio_offset(&original_bytes).unwrap();

        // Write a change that triggers the in-place fast path (metadata fits
        // within existing padding).
        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"album": "Restored Album"})).unwrap();
        let outcome = write_flac_atomic(&path, &patch).unwrap();
        assert_eq!(outcome, TrackWriteOutcome::Replaced);

        // Audio must be intact.
        let after = fs::read(&path).unwrap();
        assert_eq!(flac_audio_offset(&after).unwrap(), original_offset);
        assert_eq!(flac_audio_payload(&after).unwrap(), original_audio);

        // Metadata must reflect the change.
        let meta = read_track_metadata(&path).unwrap();
        assert_eq!(meta.album.as_deref(), Some("Restored Album"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn flac_inplace_noop_patch_returns_replaced_or_skipped() {
        // A no-op patch may produce Replaced because Lofty's VORBIS_COMMENT
        // serialization rewrites the block (padding/encoding can differ) even
        // when the logical metadata values are identical.  The important
        // invariant is that the file stays valid and audio is preserved.
        let (root, path) = copy_to_temp(&writer_fixture("padded.flac"), "inplace-noop.flac");
        let before_audio = flac_audio_payload(&fs::read(&path).unwrap())
            .unwrap()
            .to_vec();
        let before_offset = flac_audio_offset(&fs::read(&path).unwrap()).unwrap();

        let patch: TrackPatch =
            serde_json::from_value(serde_json::json!({"album": "Corpus Album"})).unwrap();
        let outcome = write_flac_atomic(&path, &patch).unwrap();
        // Accept either outcome — both preserve audio integrity.
        assert!(matches!(
            outcome,
            TrackWriteOutcome::Replaced | TrackWriteOutcome::Skipped
        ));

        let after = fs::read(&path).unwrap();
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
    async fn renderer_write_contract_returns_fresh_single_and_batch_track_data() {
        let (root, first) = copy_to_temp(&media_fixture("minimal.mp3"), "first.mp3");
        let second = root.join("second.mp3");
        fs::copy(media_fixture("minimal.mp3"), &second).unwrap();
        let queue = WriteQueue::default();

        let single = write_track_with_readback(
            &queue,
            first.clone(),
            serde_json::from_value(serde_json::json!({"title": "Single readback"})).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(single.path, first.to_string_lossy());
        assert_eq!(single.title.as_deref(), Some("Single readback"));

        let batch = batch_write_with_readback(
            &queue,
            vec![
                TrackUpdate {
                    path: first.to_string_lossy().into_owned(),
                    fields: serde_json::from_value(serde_json::json!({
                        "title": "First readback",
                        "genre": "Jazz"
                    }))
                    .unwrap(),
                },
                TrackUpdate {
                    path: second.to_string_lossy().into_owned(),
                    fields: serde_json::from_value(serde_json::json!({
                        "title": "Second readback",
                        "genre": "Jazz"
                    }))
                    .unwrap(),
                },
            ],
            None,
        )
        .await
        .unwrap();
        assert_eq!(batch.tracks.len(), 2);
        assert!(batch.failures.is_empty());
        assert_eq!(batch.tracks[0].path, first.to_string_lossy());
        assert_eq!(batch.tracks[0].title.as_deref(), Some("First readback"));
        assert_eq!(batch.tracks[0].genre.as_deref(), Some("Jazz"));
        assert_eq!(batch.tracks[1].path, second.to_string_lossy());
        assert_eq!(batch.tracks[1].title.as_deref(), Some("Second readback"));
        assert_eq!(batch.tracks[1].genre.as_deref(), Some("Jazz"));
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
            "soundrobe-extra-batch-{}",
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
            "soundrobe-extra-unsupported-{}",
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
            "soundrobe-exists-{}",
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
            "soundrobe-delete-{}",
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
        let accum = Arc::new(Mutex::new(BatchAccumulator::default()));
        batch_write_queued(&queue, updates, None, &accum)
            .await
            .unwrap();
        assert_eq!(
            read_track_metadata(&first).unwrap().title.as_deref(),
            Some("First batch title")
        );
        assert_eq!(
            read_track_metadata(&second).unwrap().title.as_deref(),
            Some("Second batch title")
        );
        batch_write_queued(&queue, Vec::new(), None, &accum)
            .await
            .unwrap();
        assert!(!queue.is_active());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn batch_write_continues_after_per_track_failure() {
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
        let accum = Arc::new(Mutex::new(BatchAccumulator::default()));
        batch_write_queued(&WriteQueue::default(), updates, None, &accum)
            .await
            .unwrap();
        // The first track was committed; the second failed (unsupported format)
        // but the batch continues and returns Ok because at least one succeeded.
        let acc = accum.lock().expect("accum lock poisoned");
        assert_eq!(acc.successes.len(), 1);
        assert_eq!(acc.failures.len(), 1);
        assert!(acc.failures[0].error.contains("other than"));
        drop(acc);
        assert_eq!(
            read_track_metadata(&first).unwrap().title.as_deref(),
            Some("Committed first")
        );
        assert_eq!(fs::read(&unsupported).unwrap(), b"untouched");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn exclusive_batch_progress_counts_failed_attempts() {
        let (root, first) = copy_to_temp(&media_fixture("minimal.mp3"), "first.mp3");
        let unsupported = root.join("second.xyz");
        fs::write(&unsupported, b"untouched").unwrap();
        let updates = vec![
            TrackUpdate {
                path: first.to_string_lossy().into_owned(),
                fields: serde_json::from_value(serde_json::json!({"title": "Committed"})).unwrap(),
            },
            TrackUpdate {
                path: unsupported.to_string_lossy().into_owned(),
                fields: TrackPatch::default(),
            },
        ];
        let observed = Arc::new(Mutex::new(Vec::new()));
        let observed_for_progress = Arc::clone(&observed);
        let progress = Arc::new(move |current: u64, total: u64| {
            observed_for_progress
                .lock()
                .expect("progress lock poisoned")
                .push((current, total));
        }) as TrackWriteProgress;

        let result = batch_write_with_exclusive_queue_held(updates, Some(progress))
            .await
            .unwrap();

        assert_eq!(result.successes.len(), 1);
        assert_eq!(result.failures.len(), 1);
        let mut observed = observed.lock().expect("progress lock poisoned").clone();
        observed.sort_unstable();
        assert_eq!(observed, vec![(1, 2), (2, 2)]);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn folder_worker_reducer_waits_for_every_handle_before_returning_error() {
        let release = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let first = tokio::spawn(async {
            Err::<(), ApiError>(ApiError::WriteTask("first failure".into()))
        });
        let release_worker = Arc::clone(&release);
        let completed_worker = Arc::clone(&completed);
        let second = tokio::spawn(async move {
            release_worker.notified().await;
            completed_worker.store(true, Ordering::Release);
            Ok(())
        });

        let reducer = tokio::spawn(join_folder_workers(vec![first, second]));
        tokio::task::yield_now().await;
        assert!(
            !reducer.is_finished(),
            "an early worker error must not detach later folder writes"
        );

        release.notify_one();
        let error = reducer.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("first failure"));
        assert!(completed.load(Ordering::Acquire));
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
            "soundrobe-pending-write-{}",
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
            "soundrobe-bad-ape-{}",
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
            "soundrobe-bad-wav-{}",
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
            "soundrobe-bad-mp4-{}",
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
            "soundrobe-bad-ogg-{}",
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
            "soundrobe-bad-flac-{}",
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
            "soundrobe-bad-mp3-{}",
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

    // --- group_by_folder ---

    #[test]
    fn group_by_folder_groups_tracks_by_parent() {
        let updates = vec![
            TrackUpdate {
                path: "/albums/A/01-flac.flac".to_string(),
                fields: TrackPatch::default(),
            },
            TrackUpdate {
                path: "/albums/A/02-flac.flac".to_string(),
                fields: TrackPatch::default(),
            },
            TrackUpdate {
                path: "/albums/B/01-flac.flac".to_string(),
                fields: TrackPatch::default(),
            },
        ];
        let groups = group_by_folder(updates);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.get(Path::new("/albums/A")).map(|v| v.len()), Some(2));
        assert_eq!(groups.get(Path::new("/albums/B")).map(|v| v.len()), Some(1));
    }

    #[test]
    fn group_by_folder_handles_single_track() {
        let updates = vec![TrackUpdate {
            path: "/single/track.flac".to_string(),
            fields: TrackPatch::default(),
        }];
        let groups = group_by_folder(updates);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups.get(Path::new("/single")).map(|v| v.len()), Some(1));
    }

    /// Intent: the measured cross-folder default should use four workers while
    /// preserving an explicit user override for slower storage.
    #[test]
    fn effective_write_concurrency_uses_measured_default_and_override() {
        assert_eq!(effective_write_concurrency(None), 4);
        assert_eq!(effective_write_concurrency(Some(0)), 4);
        assert_eq!(effective_write_concurrency(Some(1)), 1);
        assert_eq!(effective_write_concurrency(Some(8)), 8);
    }

    #[test]
    fn group_by_folder_handles_root_path() {
        let updates = vec![TrackUpdate {
            path: "track.flac".to_string(),
            fields: TrackPatch::default(),
        }];
        let groups = group_by_folder(updates);
        assert_eq!(groups.len(), 1);
        // A path without a parent gets grouped under "" (empty path)
        assert!(groups.contains_key(Path::new("")));
    }

    // --- per-folder isolation (batch writes to two folders) ---

    /// Helper: copy a writer-corpus FLAC fixture to a specific path.
    fn copy_flac_to(path: &Path) {
        let (_root, fixture) = copy_flac_fixture();
        fs::copy(&fixture, path).unwrap();
        fs::remove_dir_all(fixture.parent().unwrap()).unwrap();
    }

    /// Write tracks in two different album folders concurrently and verify
    /// both succeed. This exercises the per-folder concurrent write path.
    #[tokio::test]
    async fn batch_write_to_two_folders_both_succeed() {
        let root = std::env::temp_dir().join(format!(
            "soundrobe-two-folder-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let album_a = root.join("AlbumA");
        let album_b = root.join("AlbumB");
        fs::create_dir_all(&album_a).unwrap();
        fs::create_dir_all(&album_b).unwrap();

        let track_a = album_a.join("01.flac");
        let track_b = album_b.join("01.flac");
        copy_flac_to(&track_a);
        copy_flac_to(&track_b);

        let queue = WriteQueue::default();
        let updates = vec![
            TrackUpdate {
                path: track_a.to_string_lossy().into_owned(),
                fields: serde_json::from_value(serde_json::json!({
                    "title": "Album A Track",
                    "artist": "Artist A"
                }))
                .unwrap(),
            },
            TrackUpdate {
                path: track_b.to_string_lossy().into_owned(),
                fields: serde_json::from_value(serde_json::json!({
                    "title": "Album B Track",
                    "artist": "Artist B"
                }))
                .unwrap(),
            },
        ];

        let result = batch_write_with_readback(&queue, updates, None)
            .await
            .unwrap();
        assert_eq!(result.tracks.len(), 2);
        assert!(result.failures.is_empty());
        assert_eq!(result.tracks[0].title.as_deref(), Some("Album A Track"));
        assert_eq!(result.tracks[0].artist.as_deref(), Some("Artist A"));
        assert_eq!(result.tracks[1].title.as_deref(), Some("Album B Track"));
        assert_eq!(result.tracks[1].artist.as_deref(), Some("Artist B"));

        fs::remove_dir_all(root).unwrap();
    }

    // --- no duplicate file writes ---

    /// Two TrackUpdate entries pointing to the same file path are serialised
    /// (same folder → same folder lock → sequential). The last write wins.
    #[tokio::test]
    async fn batch_write_same_path_is_serialised_last_write_wins() {
        let root = std::env::temp_dir().join(format!(
            "soundrobe-same-path-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();

        let track_path = root.join("same.flac");
        copy_flac_to(&track_path);
        let path_str = track_path.to_string_lossy().into_owned();

        let queue = WriteQueue::default();
        let updates = vec![
            TrackUpdate {
                path: path_str.clone(),
                fields: serde_json::from_value(serde_json::json!({
                    "title": "First Write",
                    "artist": "First Artist"
                }))
                .unwrap(),
            },
            TrackUpdate {
                path: path_str.clone(),
                fields: serde_json::from_value(serde_json::json!({
                    "title": "Second Write",
                    "artist": "Second Artist"
                }))
                .unwrap(),
            },
        ];

        let result = batch_write_with_readback(&queue, updates, None)
            .await
            .unwrap();
        assert_eq!(result.tracks.len(), 2);
        assert!(result.failures.is_empty());
        // Both entries point to the same file; after both writes complete
        // sequentially (same-folder lock), the on-disk state is the second
        // write. Both readbacks return the current on-disk state.
        assert_eq!(result.tracks[0].title.as_deref(), Some("Second Write"));
        assert_eq!(result.tracks[1].title.as_deref(), Some("Second Write"));

        // Verify on-disk: re-read the file, should have second write's values
        let on_disk = read_track_metadata(&track_path).unwrap();
        assert_eq!(on_disk.title.as_deref(), Some("Second Write"));
        assert_eq!(on_disk.artist.as_deref(), Some("Second Artist"));

        fs::remove_dir_all(root).unwrap();
    }

    // --- large-album sub-batching ---

    /// More than 20 tracks in one folder verifies the SUBBATCH_SIZE chunking
    /// boundary. All writes must succeed and read back in order.
    #[tokio::test]
    async fn batch_write_large_album_exceeds_subbatch_size() {
        let root = std::env::temp_dir().join(format!(
            "soundrobe-large-album-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();

        let track_count = 25; // > SUBBATCH_SIZE (20)
        let mut updates = Vec::with_capacity(track_count);
        for i in 0..track_count {
            let track_path = root.join(format!("{:02}.flac", i + 1));
            copy_flac_to(&track_path);
            updates.push(TrackUpdate {
                path: track_path.to_string_lossy().into_owned(),
                fields: serde_json::from_value(serde_json::json!({
                    "title": format!("Track {}", i + 1),
                    "trackNumber": (i + 1) as u32
                }))
                .unwrap(),
            });
        }

        let queue = WriteQueue::default();
        let result = batch_write_with_readback(&queue, updates, None)
            .await
            .unwrap();
        assert_eq!(result.tracks.len(), track_count);
        assert!(result.failures.is_empty());
        for (i, track) in result.tracks.iter().enumerate() {
            assert_eq!(
                track.title.as_deref(),
                Some(format!("Track {}", i + 1).as_str())
            );
            assert_eq!(track.track_number, Some((i + 1) as u32));
        }

        fs::remove_dir_all(root).unwrap();
    }
}
