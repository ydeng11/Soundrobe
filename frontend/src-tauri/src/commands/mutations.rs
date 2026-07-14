//! Media mutation core. Commands and queue integration land only after each
//! format's pure writer passes differential and payload-safety tests.

use crate::commands::tracks::{id3_user_text_values, read_track_metadata, TrackData};
use crate::error::ApiError;
use crate::state::write_queue::WriteQueue;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, FrameId, Id3v2Tag, TextInformationFrame, UnsynchronizedTextFrame};
use lofty::mpeg::MpegFile;
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::tag::{Accessor, TagExt};
use lofty::TextEncoding;
use serde::{Deserialize, Deserializer};
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

async fn write_track_queued(
    queue: &WriteQueue,
    path: PathBuf,
    patch: TrackPatch,
) -> Result<(), ApiError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("mp3" | "flac" | "ogg" | "opus")) {
        return Err(ApiError::NotImplemented(
            "track:write for formats other than MP3/FLAC/OGG/Opus",
        ));
    }
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || match extension.as_deref() {
                Some("mp3") => write_mp3_atomic(&path, &patch),
                Some("flac") => write_flac_atomic(&path, &patch),
                _ => write_ogg_atomic(&path, &patch),
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await?;
    Ok(())
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
    let original_payload = flac_audio_payload(&original_bytes)
        .ok_or_else(|| ApiError::MediaSafety("invalid FLAC metadata boundary".to_string()))?;
    let original_audio_offset = original_bytes.len() - original_payload.len();
    let before = read_track_metadata(path)?;
    let mut flac = read_flac(path)?;
    let comments = flac
        .vorbis_comments_mut()
        .ok_or_else(|| ApiError::MediaSafety("FLAC has no Vorbis comment block".to_string()))?;
    apply_vorbis_patch(comments, patch);

    let temporary = sibling_temp_path(path);
    let result = (|| {
        fs::copy(path, &temporary)?;
        flac.save_to_path(&temporary, WriteOptions::new())?;
        let candidate_bytes = fs::read(&temporary)?;
        let candidate_payload = flac_audio_payload(&candidate_bytes).ok_or_else(|| {
            ApiError::MediaSafety("invalid written FLAC metadata boundary".to_string())
        })?;
        if candidate_payload != original_payload {
            return Err(ApiError::MediaSafety(
                "FLAC audio payload changed during metadata write".to_string(),
            ));
        }
        if let Some(repacked) =
            repack_flac_metadata(&candidate_bytes, original_audio_offset, original_payload)
        {
            fs::write(&temporary, repacked)?;
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

fn repack_flac_metadata(
    candidate: &[u8],
    target_audio_offset: usize,
    original_payload: &[u8],
) -> Option<Vec<u8>> {
    let marker = candidate.windows(4).position(|window| window == b"fLaC")?;
    let metadata_start = marker.checked_add(4)?;
    let available = target_audio_offset.checked_sub(metadata_start)?;
    let mut blocks = Vec::new();
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
        if block_type != 1 {
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

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.mp3")
            .canonicalize()
            .unwrap()
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
        let path = root.join("pending.m4a");
        fs::write(&path, b"unchanged").unwrap();
        let error = write_track_queued(&WriteQueue::default(), path.clone(), TrackPatch::default())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("other than MP3/FLAC/OGG/Opus"));
        assert_eq!(fs::read(&path).unwrap(), b"unchanged");
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
