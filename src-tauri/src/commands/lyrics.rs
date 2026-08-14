//! LRCLIB lyrics fetch command.

use crate::commands::mutations::{write_track_dispatch, Patch, TrackPatch};
use crate::commands::tracks::{
    ape_text_values, read_track_metadata, read_track_metadata_without_lyrics,
};
use crate::error::ApiError;
use crate::state::config::{AutoTagConfig, ConfigState};
use crate::state::write_queue::WriteQueue;
use chardetng::EncodingDetector;
use encoding_rs::{Encoding, BIG5, EUC_KR, GB18030, SHIFT_JIS, WINDOWS_1252};
use lofty::config::ParseOptions;
#[cfg(test)]
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame};
use lofty::iff::wav::WavFile;
use lofty::mp4::{AtomData, AtomIdent, Mp4File};
use lofty::mpeg::MpegFile;
use lofty::ogg::{OpusFile, VorbisComments, VorbisFile};
use lofty::tag::ItemKey;
#[cfg(test)]
use lofty::tag::TagExt;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::sync::Semaphore;

pub const DEFAULT_BASE_URL: &str = "https://lrclib.net/api";
const USER_AGENT: &str = concat!(
    "soundrobe/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/ydeng11/Soundrobe)"
);
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "wav", "ogg", "opus", "aiff", "ape",
];

/// Resolve the lyrics API base URL from config, falling back to LRCLIB.
fn resolve_lyrics_base_url(config: &AutoTagConfig) -> String {
    config
        .lyrics_api_url
        .as_ref()
        .filter(|url| !url.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LyricsResponse {
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
    lang: Option<String>,
    #[allow(dead_code)]
    instrumental: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsDocument {
    pub synced_lyrics: Option<String>,
    pub plain_lyrics: String,
    pub language: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LyricsDocumentWire {
    #[serde(default)]
    synced_lyrics: Option<String>,
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    language: Option<String>,
}

impl<'de> Deserialize<'de> for LyricsDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = LyricsDocumentWire::deserialize(deserializer)?;
        let result = Self::from_parts(
            wire.synced_lyrics.as_deref(),
            wire.plain_lyrics.as_deref(),
            wire.language.as_deref().unwrap_or("und"),
        );
        result.map_err(serde::de::Error::custom)
    }
}

impl Deref for LyricsDocument {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.synced_lyrics
            .as_deref()
            .unwrap_or(self.plain_lyrics.as_str())
    }
}

impl LyricsDocument {
    pub fn from_plain(text: &str, language: &str) -> Result<Self, String> {
        let plain_lyrics = validate_lyrics_text(text)?;
        if plain_lyrics.trim().is_empty() {
            return Err("lyrics are empty".to_string());
        }
        Ok(Self {
            synced_lyrics: None,
            language: normalize_language(language, &plain_lyrics),
            plain_lyrics,
        })
    }

    pub fn from_synced(text: &str, language: &str) -> Result<Self, String> {
        let text = validate_lyrics_text(text)?;
        let entries = parse_lrc_entries(&text)?;
        Self::from_timed_lines(entries, language)
    }

    pub fn from_text(text: &str, language: &str) -> Result<Self, String> {
        let text = validate_lyrics_text(text)?;
        if has_lrc_timestamp(&text) {
            Self::from_synced(&text, language)
        } else {
            Self::from_plain(&text, language)
        }
    }

    fn from_parts(
        synced: Option<&str>,
        plain: Option<&str>,
        language: &str,
    ) -> Result<Self, String> {
        if let Some(synced) = synced.filter(|value| !value.trim().is_empty()) {
            return Self::from_synced(synced, language);
        }
        Self::from_plain(plain.unwrap_or_default(), language)
    }

    fn from_timed_lines(entries: Vec<(u32, String)>, language: &str) -> Result<Self, String> {
        if entries.is_empty() {
            return Err("synchronized lyrics contain no timestamped lines".to_string());
        }
        let synced_lyrics = entries
            .iter()
            .map(|(milliseconds, text)| format_lrc_line(*milliseconds, text))
            .collect::<Vec<_>>()
            .join("\n");
        let plain_lyrics = entries
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let plain_lyrics = validate_lyrics_text(&plain_lyrics)?;
        Ok(Self {
            synced_lyrics: Some(synced_lyrics),
            language: normalize_language(language, &plain_lyrics),
            plain_lyrics,
        })
    }

    pub(crate) fn id3_language(&self) -> [u8; 3] {
        self.language.as_bytes().try_into().unwrap_or(*b"und")
    }

    pub(crate) fn timed_lines(&self) -> Result<Vec<(u32, String)>, String> {
        self.synced_lyrics
            .as_deref()
            .map(parse_lrc_entries)
            .transpose()
            .map(Option::unwrap_or_default)
    }
}

fn normalize_language(language: &str, text: &str) -> String {
    let normalized = language.trim().to_ascii_lowercase();
    let supplied = match normalized.as_str() {
        "zh" | "chi" | "zho" => Some("zho"),
        "ja" | "jpn" => Some("jpn"),
        "ko" | "kor" => Some("kor"),
        "en" | "eng" => Some("eng"),
        "und" | "xxx" | "" => None,
        _ if normalized.len() == 3 && normalized.bytes().all(|byte| byte.is_ascii_alphabetic()) => {
            Some(normalized.as_str())
        }
        _ => None,
    };
    if let Some(supplied) = supplied {
        return supplied.to_string();
    }
    if text.chars().any(is_hangul) {
        "kor".to_string()
    } else if text.chars().any(is_kana) {
        "jpn".to_string()
    } else if text.chars().any(is_cjk_ideograph) {
        "zho".to_string()
    } else {
        "und".to_string()
    }
}

fn is_hangul(character: char) -> bool {
    matches!(character as u32, 0x1100..=0x11ff | 0x3130..=0x318f | 0xac00..=0xd7af)
}

fn is_kana(character: char) -> bool {
    matches!(character as u32, 0x3040..=0x30ff | 0x31f0..=0x31ff)
}

fn is_cjk_ideograph(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0x20000..=0x2fa1f
    )
}

fn validate_lyrics_text(text: &str) -> Result<String, String> {
    let normalized = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if normalized.contains('\0') {
        return Err("lyrics contain a NUL character".to_string());
    }
    if normalized.contains('\u{fffd}') {
        return Err("lyrics contain a Unicode replacement character".to_string());
    }
    if normalized
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err("lyrics contain unsupported control characters".to_string());
    }
    if looks_like_reversible_utf8_mojibake(&normalized) {
        return Err("lyrics appear to contain reversible UTF-8 mojibake".to_string());
    }
    Ok(normalized)
}

fn looks_like_reversible_utf8_mojibake(text: &str) -> bool {
    if text.chars().any(is_east_asian_script) {
        return [GB18030, BIG5, SHIFT_JIS, EUC_KR]
            .into_iter()
            .any(|encoding| legacy_round_trip_repairs_mojibake(text, encoding));
    }
    let (bytes, _, had_errors) = WINDOWS_1252.encode(text);
    if had_errors {
        return false;
    }
    std::str::from_utf8(&bytes)
        .is_ok_and(|repaired| repaired != text && repaired.chars().any(is_east_asian_script))
}

fn legacy_round_trip_repairs_mojibake(text: &str, encoding: &'static Encoding) -> bool {
    let (bytes, _, had_errors) = encoding.encode(text);
    if had_errors {
        return false;
    }
    std::str::from_utf8(&bytes).is_ok_and(|repaired| {
        let original_chars = text
            .chars()
            .filter(|character| !character.is_whitespace())
            .count();
        let repaired_chars = repaired
            .chars()
            .filter(|character| !character.is_whitespace())
            .count();
        repaired != text
            && repaired.chars().any(is_east_asian_script)
            && repaired_chars.saturating_mul(3) <= original_chars.saturating_mul(2)
    })
}

fn is_east_asian_script(character: char) -> bool {
    is_hangul(character) || is_kana(character) || is_cjk_ideograph(character)
}

fn has_lrc_timestamp(text: &str) -> bool {
    text.lines().any(|line| {
        line.strip_prefix('[')
            .and_then(|line| line.split_once(']'))
            .is_some_and(|(timestamp, _)| parse_lrc_timestamp(timestamp).is_some())
    })
}

fn parse_lrc_entries(text: &str) -> Result<Vec<(u32, String)>, String> {
    let offset = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("[offset:")
            .and_then(|value| value.strip_suffix(']'))
            .and_then(|value| value.trim().parse::<i64>().ok())
    });
    let offset = offset.unwrap_or(0);
    let mut entries = Vec::new();
    for line in text.lines() {
        let mut remainder = line;
        let mut timestamps = Vec::new();
        while let Some(after_open) = remainder.strip_prefix('[') {
            let Some((candidate, after_close)) = after_open.split_once(']') else {
                break;
            };
            let Some(timestamp) = parse_lrc_timestamp(candidate) else {
                break;
            };
            timestamps.push(timestamp);
            remainder = after_close;
        }
        for timestamp in timestamps {
            let adjusted = i64::from(timestamp).saturating_add(offset).max(0) as u32;
            entries.push((adjusted, remainder.trim_end().to_string()));
        }
    }
    if entries.is_empty() {
        return Err("synchronized lyrics contain no valid LRC timestamps".to_string());
    }
    Ok(entries)
}

fn parse_lrc_timestamp(value: &str) -> Option<u32> {
    let (minutes, seconds) = value.split_once(':')?;
    let minutes = minutes.parse::<u32>().ok()?;
    let seconds = seconds.parse::<f64>().ok()?;
    if !(0.0..60.0).contains(&seconds) || !seconds.is_finite() {
        return None;
    }
    let milliseconds = (seconds * 1_000.0).round() as u32;
    minutes
        .checked_mul(60_000)?
        .checked_add(milliseconds.min(59_999))
}

fn format_lrc_line(milliseconds: u32, text: &str) -> String {
    let minutes = milliseconds / 60_000;
    let seconds = (milliseconds % 60_000) / 1_000;
    let millis = milliseconds % 1_000;
    format!("[{minutes:02}:{seconds:02}.{millis:03}]{text}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LyricsSource {
    Embedded,
    Lrc,
    Txt,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LyricsTrackStatus {
    EmbeddedPreserved,
    Written,
    NoLyrics,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsTrackResult {
    pub path: String,
    pub status: LyricsTrackStatus,
    pub source: Option<LyricsSource>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsBatchReport {
    pub total: usize,
    pub written: usize,
    pub embedded_preserved: usize,
    pub no_lyrics: usize,
    pub unsupported: usize,
    pub failed: usize,
    pub results: Vec<LyricsTrackResult>,
}

#[tauri::command]
pub async fn lyrics_fetch(
    track_name: String,
    artist_name: String,
    album_name: Option<String>,
    duration: Option<f64>,
    config: State<'_, ConfigState>,
) -> Result<Option<LyricsDocument>, ApiError> {
    let base_url = resolve_lyrics_base_url(&config.raw());
    Ok(fetch_lyrics_at(
        &base_url,
        &track_name,
        &artist_name,
        album_name.as_deref(),
        duration,
    )
    .await)
}

#[tauri::command]
pub async fn album_download_lyrics(
    album_path: String,
    config: State<'_, ConfigState>,
    queue: State<'_, WriteQueue>,
) -> Result<LyricsBatchReport, ApiError> {
    let base_url = resolve_lyrics_base_url(&config.raw());
    Ok(download_album_lyrics_at(Path::new(&album_path), &base_url, &queue).await)
}

pub async fn download_album_lyrics_at(
    album_path: &Path,
    base_url: &str,
    queue: &WriteQueue,
) -> LyricsBatchReport {
    apply_album_lyrics_at(album_path, Some(base_url), queue).await
}

/// Collect audio files in a directory (shared helper for lyrics functions).
fn collect_audio_files_in(album_path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(album_path) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() && AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// Resolve missing lyrics for an album, returning only documents that should be written.
///
/// Valid embedded lyrics are preserved and omitted from the returned write jobs. Otherwise this
/// checks local `.lrc` / `.txt` files before the remote API, with up to five concurrent tracks.
pub async fn fetch_album_lyrics(
    album_path: &Path,
    base_url: Option<&str>,
) -> Vec<(PathBuf, LyricsDocument)> {
    resolve_album_lyrics(album_path, base_url)
        .await
        .into_iter()
        .filter_map(|resolution| match resolution.outcome {
            LyricsResolutionOutcome::Found(_, LyricsSource::Embedded) => None,
            LyricsResolutionOutcome::Found(document, _) => Some((resolution.path, document)),
            LyricsResolutionOutcome::Missing
            | LyricsResolutionOutcome::Unsupported(_)
            | LyricsResolutionOutcome::Rejected(_) => None,
        })
        .collect()
}

struct LyricsResolution {
    path: PathBuf,
    outcome: LyricsResolutionOutcome,
}

enum LyricsResolutionOutcome {
    Found(LyricsDocument, LyricsSource),
    Missing,
    Unsupported(String),
    Rejected(String),
}

async fn resolve_album_lyrics(album_path: &Path, base_url: Option<&str>) -> Vec<LyricsResolution> {
    let audio_files = collect_audio_files_in(album_path);
    if audio_files.is_empty() {
        return Vec::new();
    }
    const MAX_CONCURRENT: usize = 5;
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let mut handles = Vec::with_capacity(audio_files.len());
    let mut results = Vec::with_capacity(audio_files.len());

    for path in audio_files {
        let permit = match Arc::clone(&semaphore).acquire_owned().await {
            Ok(p) => p,
            Err(error) => {
                results.push(LyricsResolution {
                    path,
                    outcome: LyricsResolutionOutcome::Rejected(format!(
                        "lyrics worker semaphore closed unexpectedly: {error}"
                    )),
                });
                continue;
            }
        };
        let base_url = base_url.map(|s| s.to_string());
        let join_path = path.clone();

        handles.push((
            join_path,
            tokio::spawn(async move {
                let _permit = permit;
                let read_path = path.clone();
                let metadata = tokio::task::spawn_blocking(move || {
                    read_track_metadata_without_lyrics(&read_path)
                })
                .await;
                let metadata = match metadata {
                    Ok(Ok(metadata)) => metadata,
                    Ok(Err(error)) => {
                        return LyricsResolution {
                            path,
                            outcome: LyricsResolutionOutcome::Rejected(format!(
                                "could not inspect embedded lyrics: {error}"
                            )),
                        };
                    }
                    Err(error) => {
                        return LyricsResolution {
                            path,
                            outcome: LyricsResolutionOutcome::Rejected(error.to_string()),
                        };
                    }
                };
                let embedded_path = path.clone();
                match tokio::task::spawn_blocking(move || read_embedded_lyrics(&embedded_path))
                    .await
                {
                    Ok(Ok(Some(document))) => {
                        return LyricsResolution {
                            path,
                            outcome: LyricsResolutionOutcome::Found(
                                document,
                                LyricsSource::Embedded,
                            ),
                        };
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        return LyricsResolution {
                            path,
                            outcome: LyricsResolutionOutcome::Rejected(format!(
                                "embedded lyrics are malformed or undecodable: {error}"
                            )),
                        };
                    }
                    Err(error) => {
                        return LyricsResolution {
                            path,
                            outcome: LyricsResolutionOutcome::Rejected(format!(
                                "embedded lyrics inspection failed: {error}"
                            )),
                        };
                    }
                }

                if path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("aiff"))
                {
                    return LyricsResolution {
                        path,
                        outcome: LyricsResolutionOutcome::Unsupported(
                            "AIFF lyrics embedding is not supported; no sidecar was modified"
                                .to_string(),
                        ),
                    };
                }

                let local_path = path.clone();
                match tokio::task::spawn_blocking(move || read_local_lyrics(&local_path)).await {
                    Ok(Ok(Some((document, source)))) => {
                        return LyricsResolution {
                            path,
                            outcome: LyricsResolutionOutcome::Found(document, source),
                        };
                    }
                    Ok(Err(error)) => {
                        return LyricsResolution {
                            path,
                            outcome: LyricsResolutionOutcome::Rejected(error),
                        };
                    }
                    Err(error) => {
                        return LyricsResolution {
                            path,
                            outcome: LyricsResolutionOutcome::Rejected(error.to_string()),
                        };
                    }
                    Ok(Ok(None)) => {}
                }

                let lyrics = if let (Some(base_url), Some(title), Some(artist)) =
                    (base_url.as_deref(), metadata.title, metadata.artist)
                {
                    fetch_lyrics_at(
                        base_url,
                        &title,
                        &artist,
                        metadata.album.as_deref(),
                        (metadata.duration > 0.0).then_some(metadata.duration.round()),
                    )
                    .await
                } else {
                    None
                };
                LyricsResolution {
                    path,
                    outcome: lyrics.map_or(LyricsResolutionOutcome::Missing, |document| {
                        LyricsResolutionOutcome::Found(document, LyricsSource::Remote)
                    }),
                }
            }),
        ));
    }

    for (path, handle) in handles {
        match handle.await {
            Ok(resolution) => results.push(resolution),
            Err(error) => results.push(LyricsResolution {
                path,
                outcome: LyricsResolutionOutcome::Rejected(format!(
                    "lyrics worker failed before producing a result: {error}"
                )),
            }),
        }
    }
    results
}

pub async fn apply_album_lyrics_at(
    album_path: &Path,
    base_url: Option<&str>,
    queue: &WriteQueue,
) -> LyricsBatchReport {
    let resolutions = resolve_album_lyrics(album_path, base_url).await;
    let total = resolutions.len();
    if resolutions.is_empty() {
        return LyricsBatchReport::default();
    }
    let album_path_display = album_path.to_string_lossy().into_owned();
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                let mut report = LyricsBatchReport {
                    total,
                    ..LyricsBatchReport::default()
                };
                for resolution in resolutions {
                    let path_text = resolution.path.to_string_lossy().into_owned();
                    match resolution.outcome {
                        LyricsResolutionOutcome::Found(_document, LyricsSource::Embedded) => {
                            report.embedded_preserved += 1;
                            report.results.push(LyricsTrackResult {
                                path: path_text,
                                status: LyricsTrackStatus::EmbeddedPreserved,
                                source: Some(LyricsSource::Embedded),
                                error: None,
                            });
                        }
                        LyricsResolutionOutcome::Found(document, source) => {
                            let patch = TrackPatch {
                                lyrics: Patch::Value(document.clone()),
                                ..TrackPatch::default()
                            };
                            let result =
                                write_track_dispatch(&resolution.path, &patch).and_then(|_| {
                                    let readback = read_track_metadata(&resolution.path)?;
                                    if readback.lyrics.as_ref() == Some(&document) {
                                        Ok(())
                                    } else {
                                        Err(ApiError::MediaSafety(
                                        "exact lyrics readback did not match the requested document"
                                            .to_string(),
                                    ))
                                    }
                                });
                            match result {
                                Ok(()) => {
                                    report.written += 1;
                                    report.results.push(LyricsTrackResult {
                                        path: path_text,
                                        status: LyricsTrackStatus::Written,
                                        source: Some(source),
                                        error: None,
                                    });
                                }
                                Err(error) => {
                                    report.failed += 1;
                                    report.results.push(LyricsTrackResult {
                                        path: path_text,
                                        status: LyricsTrackStatus::Failed,
                                        source: Some(source),
                                        error: Some(error.to_string()),
                                    });
                                }
                            }
                        }
                        LyricsResolutionOutcome::Missing => {
                            report.no_lyrics += 1;
                            report.results.push(LyricsTrackResult {
                                path: path_text,
                                status: LyricsTrackStatus::NoLyrics,
                                source: None,
                                error: None,
                            });
                        }
                        LyricsResolutionOutcome::Unsupported(error) => {
                            report.unsupported += 1;
                            report.results.push(LyricsTrackResult {
                                path: path_text,
                                status: LyricsTrackStatus::Unsupported,
                                source: None,
                                error: Some(error),
                            });
                        }
                        LyricsResolutionOutcome::Rejected(error) => {
                            report.failed += 1;
                            report.results.push(LyricsTrackResult {
                                path: path_text,
                                status: LyricsTrackStatus::Failed,
                                source: None,
                                error: Some(error),
                            });
                        }
                    }
                }
                report
            })
            .await
            .unwrap_or_else(|error| LyricsBatchReport {
                total,
                failed: total,
                results: vec![LyricsTrackResult {
                    path: album_path_display,
                    status: LyricsTrackStatus::Failed,
                    source: None,
                    error: Some(error.to_string()),
                }],
                ..LyricsBatchReport::default()
            })
        })
        .await
}

pub fn read_local_lyrics(
    file_path: &Path,
) -> Result<Option<(LyricsDocument, LyricsSource)>, String> {
    for extension in ["lrc", "txt"] {
        let path = file_path.with_extension(extension);
        match fs::read(&path) {
            Ok(bytes) => {
                let text = decode_lyrics_bytes(&bytes)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                let document = if extension == "lrc" {
                    LyricsDocument::from_text(&text, "und")
                } else {
                    LyricsDocument::from_plain(&text, "und")
                }
                .map_err(|error| format!("{}: {error}", path.display()))?;
                return Ok(Some((
                    document,
                    if extension == "lrc" {
                        LyricsSource::Lrc
                    } else {
                        LyricsSource::Txt
                    },
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
    Ok(None)
}

pub fn decode_lyrics_bytes(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("lyrics file is empty".to_string());
    }
    let decoded = if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        std::str::from_utf8(rest)
            .map(str::to_owned)
            .map_err(|_| "malformed UTF-8 after BOM".to_string())?
    } else if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe, 0x00, 0x00]) {
        decode_utf32(rest, true)?
    } else if let Some(rest) = bytes.strip_prefix(&[0x00, 0x00, 0xfe, 0xff]) {
        decode_utf32(rest, false)?
    } else if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        decode_utf16(rest, true)?
    } else if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        decode_utf16(rest, false)?
    } else if let Some(little_endian) = looks_like_utf32(bytes) {
        decode_utf32(bytes, little_endian)?
    } else if let Some(little_endian) = looks_like_utf16(bytes) {
        decode_utf16(bytes, little_endian)?
    } else if let Ok(utf8) = std::str::from_utf8(bytes) {
        utf8.to_string()
    } else {
        decode_legacy_strict(bytes)?
    };
    validate_lyrics_text(&decoded)
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("malformed UTF-16 byte length".to_string());
    }
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units).map_err(|_| "malformed UTF-16 text".to_string())
}

fn decode_utf32(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if bytes.len() % 4 != 0 {
        return Err("malformed UTF-32 byte length".to_string());
    }
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let value = if little_endian {
                u32::from_le_bytes(chunk.try_into().unwrap())
            } else {
                u32::from_be_bytes(chunk.try_into().unwrap())
            };
            char::from_u32(value).ok_or_else(|| "malformed UTF-32 scalar value".to_string())
        })
        .collect()
}

fn looks_like_utf32(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 8 || bytes.len() % 4 != 0 {
        return None;
    }
    let chunks = bytes.len() / 4;
    let little_score = bytes
        .chunks_exact(4)
        .filter(|chunk| chunk[2] == 0 && chunk[3] == 0)
        .count();
    let big_score = bytes
        .chunks_exact(4)
        .filter(|chunk| chunk[0] == 0 && chunk[1] == 0)
        .count();
    let threshold = (chunks * 3).div_ceil(4);
    if little_score >= threshold && little_score > big_score {
        Some(true)
    } else if big_score >= threshold && big_score > little_score {
        Some(false)
    } else {
        None
    }
}

fn looks_like_utf16(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 4 || bytes.len() % 2 != 0 {
        return None;
    }
    let units = bytes.len() / 2;
    let even_zero = bytes.iter().step_by(2).filter(|byte| **byte == 0).count();
    let odd_zero = bytes
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|byte| **byte == 0)
        .count();
    let threshold = units.div_ceil(3);
    if odd_zero >= threshold && odd_zero > even_zero.saturating_mul(2) {
        Some(true)
    } else if even_zero >= threshold && even_zero > odd_zero.saturating_mul(2) {
        Some(false)
    } else {
        None
    }
}

fn decode_legacy_strict(bytes: &[u8]) -> Result<String, String> {
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let (detected, confident) = detector.guess_assess(None, false);
    if !confident {
        return Err("lyrics encoding is uncertain".to_string());
    }
    let encoding: &'static Encoding = match detected.name() {
        "GBK" | "gb18030" => GB18030,
        "Big5" => BIG5,
        "Shift_JIS" => SHIFT_JIS,
        "EUC-KR" => EUC_KR,
        other => return Err(format!("unsupported or uncertain legacy encoding: {other}")),
    };
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(Cow::into_owned)
        .ok_or_else(|| format!("malformed {} lyrics", encoding.name()))
}

pub(crate) fn read_embedded_lyrics(path: &Path) -> Result<Option<LyricsDocument>, ApiError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let options = ParseOptions::new().read_properties(false);
    match extension.as_str() {
        "mp3" => {
            let mut file = fs::File::open(path)?;
            let parsed = MpegFile::read_from(&mut file, options)?;
            parsed.id3v2().map(id3_lyrics_document).unwrap_or(Ok(None))
        }
        "wav" => {
            let mut file = fs::File::open(path)?;
            let parsed = WavFile::read_from(&mut file, options)?;
            parsed.id3v2().map(id3_lyrics_document).unwrap_or(Ok(None))
        }
        "flac" => {
            let mut file = fs::File::open(path)?;
            let parsed = FlacFile::read_from(&mut file, options)?;
            parsed
                .vorbis_comments()
                .map(vorbis_lyrics_document)
                .unwrap_or(Ok(None))
        }
        "ogg" => {
            let mut file = fs::File::open(path)?;
            let parsed = VorbisFile::read_from(&mut file, options)?;
            vorbis_lyrics_document(parsed.vorbis_comments())
        }
        "opus" => {
            let mut file = fs::File::open(path)?;
            let parsed = OpusFile::read_from(&mut file, options)?;
            vorbis_lyrics_document(parsed.vorbis_comments())
        }
        "m4a" | "mp4" => {
            let mut file = fs::File::open(path)?;
            let parsed = Mp4File::read_from(&mut file, options)?;
            let value = parsed.ilst().and_then(|ilst| {
                ilst.get(&AtomIdent::Fourcc(*b"\xa9lyr")).and_then(|atom| {
                    atom.data().find_map(|data| match data {
                        AtomData::UTF8(value) | AtomData::UTF16(value) => Some(value.as_str()),
                        _ => None,
                    })
                })
            });
            let language = parsed
                .ilst()
                .and_then(|ilst| {
                    ilst.get(&AtomIdent::Freeform {
                        mean: Cow::Borrowed("com.apple.iTunes"),
                        name: Cow::Borrowed("LYRICSLANGUAGE"),
                    })
                })
                .and_then(|atom| {
                    atom.data().find_map(|data| match data {
                        AtomData::UTF8(value) | AtomData::UTF16(value) => Some(value.as_str()),
                        _ => None,
                    })
                })
                .unwrap_or("und");
            value
                .map(|text| document_from_embedded_text(text, language))
                .transpose()
        }
        "ape" => {
            let lyrics = ape_text_values(path, "LYRICS");
            let fallback = ape_text_values(path, "UNSYNCEDLYRICS");
            let language = ape_text_values(path, "LYRICSLANGUAGE");
            document_from_lyrics_values(
                lyrics.first().map(String::as_str),
                fallback.first().map(String::as_str),
                language.first().map(String::as_str).unwrap_or("und"),
            )
        }
        "aiff" => {
            let tagged = lofty::read_from_path(path)?;
            let text = tagged.tags().iter().find_map(|tag| {
                tag.get_string(ItemKey::Lyrics)
                    .or_else(|| tag.get_string(ItemKey::UnsyncLyrics))
            });
            text.map(|text| document_from_embedded_text(text, "und"))
                .transpose()
        }
        _ => Ok(None),
    }
}

fn id3_lyrics_document(tag: &Id3v2Tag) -> Result<Option<LyricsDocument>, ApiError> {
    let mut first_error = None;
    for frame in tag {
        if frame.id_str() != "SYLT" {
            continue;
        }
        if let Frame::Binary(binary) = frame {
            let synchronized = match SynchronizedTextFrame::parse(&binary.data, binary.flags()) {
                Ok(synchronized) => synchronized,
                Err(error) => {
                    first_error.get_or_insert_with(|| error.to_string());
                    continue;
                }
            };
            if synchronized.content_type == SyncTextContentType::Lyrics
                && !synchronized.content.is_empty()
            {
                let language = std::str::from_utf8(&synchronized.language).unwrap_or("und");
                match LyricsDocument::from_timed_lines(synchronized.content, language) {
                    Ok(document) => return Ok(Some(document)),
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
        }
    }

    for frame in tag {
        if let Frame::UnsynchronizedText(frame) = frame {
            let language = std::str::from_utf8(&frame.language).unwrap_or("und");
            match document_from_embedded_text(&frame.content, language) {
                Ok(document) => return Ok(Some(document)),
                Err(error) => {
                    first_error.get_or_insert_with(|| error.to_string());
                }
            }
        }
    }

    for frame in tag {
        if let Frame::UserText(frame) = frame {
            if is_lyrics_alias(&frame.description) && !frame.content.trim().is_empty() {
                match document_from_embedded_text(&frame.content, "und") {
                    Ok(document) => return Ok(Some(document)),
                    Err(error) => {
                        first_error.get_or_insert_with(|| error.to_string());
                    }
                }
            }
        }
    }
    first_error.map_or(Ok(None), |error| Err(ApiError::Message(error)))
}

fn vorbis_lyrics_document(comments: &VorbisComments) -> Result<Option<LyricsDocument>, ApiError> {
    let lyrics = comment_value(comments, "LYRICS");
    let fallback = comment_value(comments, "UNSYNCEDLYRICS");
    let language = comment_value(comments, "LYRICSLANGUAGE").unwrap_or("und");
    document_from_lyrics_values(lyrics, fallback, language)
}

fn comment_value<'a>(comments: &'a VorbisComments, key: &str) -> Option<&'a str> {
    comments
        .items()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

fn document_from_lyrics_values(
    lyrics: Option<&str>,
    fallback: Option<&str>,
    language: &str,
) -> Result<Option<LyricsDocument>, ApiError> {
    if let Some(lyrics) = lyrics {
        match document_from_embedded_text(lyrics, language) {
            Ok(document) => return Ok(Some(document)),
            Err(error) if fallback.is_none() => return Err(error),
            Err(_) => {}
        }
    }
    fallback
        .map(|text| document_from_embedded_text(text, language))
        .transpose()
}

fn document_from_embedded_text(text: &str, language: &str) -> Result<LyricsDocument, ApiError> {
    LyricsDocument::from_text(text, language).map_err(ApiError::Message)
}

pub(crate) fn is_lyrics_alias(value: &str) -> bool {
    matches!(
        value
            .chars()
            .filter(|character| !matches!(character, ' ' | '_' | '-'))
            .collect::<String>()
            .to_ascii_uppercase()
            .as_str(),
        "LYRICS"
            | "USLT"
            | "SYLT"
            | "SYNCEDLYRICS"
            | "SYNCHRONIZEDLYRICS"
            | "SYNCHRONISEDLYRICS"
            | "UNSYNCEDLYRICS"
            | "UNSYNCHRONISEDLYRICS"
            | "LYRICSLANGUAGE"
    )
}

pub async fn fetch_lyrics_at(
    base_url: &str,
    track_name: &str,
    artist_name: &str,
    album_name: Option<&str>,
    duration: Option<f64>,
) -> Option<LyricsDocument> {
    if track_name.is_empty() || artist_name.is_empty() {
        return None;
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(USER_AGENT)
        .build()
        .ok()?;
    let mut query = vec![
        ("track_name", track_name.to_string()),
        ("artist_name", artist_name.to_string()),
    ];
    if let Some(album_name) = album_name.filter(|value| !value.is_empty()) {
        query.push(("album_name", album_name.to_string()));
    }
    if let Some(duration) = duration.filter(|value| *value > 0.0) {
        query.push(("duration", duration.round().to_string()));
    }
    let url = format!("{}/get", base_url.trim_end_matches('/'));
    let response = client.get(url).query(&query).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.json::<LyricsResponse>().await.ok()?;
    let language = body.lang.as_deref().unwrap_or("und");
    if let Some(synced) = body.synced_lyrics.filter(|value| !value.trim().is_empty()) {
        if let Ok(document) = LyricsDocument::from_synced(&synced, language) {
            return Some(document);
        }
    }
    body.plain_lyrics
        .filter(|value| !value.trim().is_empty())
        .and_then(|plain| LyricsDocument::from_plain(&plain, language).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn server(status: &str, body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_string();
        let body = body.to_string();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let _ = send.send(String::from_utf8_lossy(&request[..read]).into_owned());
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (format!("http://{address}/api/"), receive)
    }

    fn temp_root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "soundrobe-lyrics-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn local_lyrics_prefers_lrc_and_decodes_utf16_boms() {
        let root = temp_root();
        let audio = root.join("song.mp3");
        fs::write(&audio, b"").unwrap();
        fs::write(audio.with_extension("txt"), "fallback").unwrap();
        let text = "[00:01]你好";
        let mut utf16 = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(audio.with_extension("lrc"), utf16).unwrap();
        let (document, source) = read_local_lyrics(&audio).unwrap().unwrap();
        assert_eq!(source, LyricsSource::Lrc);
        assert_eq!(document.language, "zho");
        assert_eq!(
            document.synced_lyrics.as_deref(),
            Some("[00:01.000]\u{4f60}\u{597d}")
        );
        let be = text
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            decode_lyrics_bytes(&[vec![0xfe, 0xff], be].concat()).unwrap(),
            text
        );
        let japanese = "これは日本語の歌詞です。音楽と未来を歌います。".repeat(4);
        let (shift_jis, _, _) = encoding_rs::SHIFT_JIS.encode(&japanese);
        assert_eq!(decode_lyrics_bytes(&shift_jis).unwrap(), japanese);
        let traditional = "這是繁體中文歌詞，唱著音樂與未來。".repeat(4);
        let (big5, _, _) = encoding_rs::BIG5.encode(&traditional);
        assert_eq!(decode_lyrics_bytes(&big5).unwrap(), traditional);
        let simplified = "这是简体中文歌词，唱着音乐与未来，包含扩展字𠀀。".repeat(6);
        let (gb18030, _, _) = encoding_rs::GB18030.encode(&simplified);
        assert_eq!(decode_lyrics_bytes(&gb18030).unwrap(), simplified);
        let korean = "이것은 한국어 노래 가사입니다.".repeat(8);
        let (euc_kr, _, _) = encoding_rs::EUC_KR.encode(&korean);
        assert_eq!(decode_lyrics_bytes(&euc_kr).unwrap(), korean);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn strict_decoder_supports_unicode_boms_and_rejects_lossy_text() {
        let text = "[00:01.250]\u{4f60}\u{597d}\r\nnext\rline";
        let utf32le = text
            .chars()
            .flat_map(|character| u32::from(character).to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            decode_lyrics_bytes(&[vec![0xff, 0xfe, 0x00, 0x00], utf32le].concat()).unwrap(),
            "[00:01.250]\u{4f60}\u{597d}\nnext\nline"
        );

        let utf16be = text
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            decode_lyrics_bytes(&utf16be).unwrap(),
            "[00:01.250]\u{4f60}\u{597d}\nnext\nline"
        );
        assert!(decode_lyrics_bytes(&[0xff, 0xfe, 0x00]).is_err());
        assert!(decode_lyrics_bytes(b"bad\0lyrics").is_err());
        assert!(decode_lyrics_bytes("bad \u{fffd} lyrics".as_bytes()).is_err());
        assert!(decode_lyrics_bytes("ä½ å¥½".as_bytes()).is_err());
        assert!(decode_lyrics_bytes("浣犲ソ".as_bytes()).is_err());
        assert!(decode_lyrics_bytes("縺薙ｓ縺ｫ縺｡縺ｯ".as_bytes()).is_err());
        assert!(decode_lyrics_bytes(&[0x81]).is_err());
    }

    #[test]
    fn valid_plain_embedded_fallback_survives_malformed_synced_value() {
        let mut comments = VorbisComments::new();
        comments.insert("LYRICS".to_string(), "bad\0lyrics".to_string());
        comments.insert("UNSYNCEDLYRICS".to_string(), "你好，音乐".to_string());

        assert_eq!(
            vorbis_lyrics_document(&comments).unwrap(),
            Some(LyricsDocument::from_plain("你好，音乐", "zho").unwrap())
        );
    }

    #[test]
    fn malformed_id3_lyric_frame_does_not_hide_a_later_valid_frame() {
        let mut tag = Id3v2Tag::default();
        tag.insert(Frame::Binary(lofty::id3::v2::BinaryFrame::new(
            lofty::id3::v2::FrameId::Valid(Cow::Borrowed("SYLT")),
            vec![0xff],
        )));
        tag.insert(Frame::UnsynchronizedText(
            lofty::id3::v2::UnsynchronizedTextFrame::new(
                lofty::TextEncoding::UTF16,
                *b"zho",
                "",
                "你好，音乐",
            ),
        ));

        assert_eq!(
            id3_lyrics_document(&tag).unwrap(),
            Some(LyricsDocument::from_plain("你好，音乐", "zho").unwrap())
        );
    }

    #[test]
    fn synchronized_lrc_is_canonical_and_derives_plain_fallback() {
        let document = LyricsDocument::from_synced(
            "[ar:Artist]\r\n[00:01.2][00:02.345] first \r\n[01:03]second",
            "zho",
        )
        .unwrap();
        assert_eq!(
            document.synced_lyrics.as_deref(),
            Some("[00:01.200] first\n[00:02.345] first\n[01:03.000]second")
        );
        assert_eq!(document.plain_lyrics, " first\n first\nsecond");
        assert_eq!(document.language, "zho");
    }

    #[test]
    fn canonical_document_deserialization_requires_the_structured_interface() {
        assert!(
            serde_json::from_value::<LyricsDocument>(serde_json::json!("legacy lyrics")).is_err()
        );
        assert_eq!(
            serde_json::from_value::<LyricsDocument>(serde_json::json!({
                "plainLyrics": "你好",
                "language": "zh"
            }))
            .unwrap(),
            LyricsDocument::from_plain("你好", "zho").unwrap()
        );
    }

    #[tokio::test]
    async fn album_download_preserves_embedded_lyrics_and_reports_every_track() {
        let root = temp_root();
        let fixtures =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test/fixtures/tauri/media-corpus");
        let mp3 = root.join("01.mp3");
        let wav = root.join("02.wav");
        fs::copy(fixtures.join("minimal.mp3"), &mp3).unwrap();
        fs::copy(fixtures.join("minimal.wav"), &wav).unwrap();
        let embedded = LyricsDocument::from_plain("Embedded lyrics", "zho").unwrap();
        write_track_dispatch(
            &mp3,
            &TrackPatch {
                lyrics: Patch::Value(embedded.clone()),
                ..TrackPatch::default()
            },
        )
        .unwrap();
        fs::write(
            mp3.with_extension("lrc"),
            "sidecar must not replace embedded",
        )
        .unwrap();
        fs::write(wav.with_extension("txt"), "WAV lyrics").unwrap();
        let auto_tag_jobs = fetch_album_lyrics(&root, None).await;
        assert_eq!(
            auto_tag_jobs.len(),
            1,
            "unexpected auto-tag lyric jobs: {:?}",
            auto_tag_jobs
                .iter()
                .map(|(path, _)| path)
                .collect::<Vec<_>>()
        );
        assert_eq!(auto_tag_jobs[0].0, wav);
        let queue = WriteQueue::default();
        let report = apply_album_lyrics_at(&root, None, &queue).await;
        assert_eq!(report.total, 2);
        assert_eq!(report.embedded_preserved, 1);
        assert_eq!(report.written, 1);
        assert_eq!(read_track_metadata(&mp3).unwrap().lyrics, Some(embedded));
        assert_eq!(
            read_track_metadata(&wav)
                .unwrap()
                .lyrics
                .unwrap()
                .plain_lyrics,
            "WAV lyrics"
        );
        assert!(!queue.is_active());
        let missing =
            download_album_lyrics_at(&root.join("missing"), "http://unused", &queue).await;
        assert_eq!(missing.total, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn album_download_reports_aiff_as_unsupported() {
        let root = temp_root();
        let fixtures =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test/fixtures/tauri/media-corpus");
        let aiff = root.join("01.aiff");
        fs::copy(fixtures.join("minimal.aiff"), &aiff).unwrap();
        let report = apply_album_lyrics_at(&root, None, &WriteQueue::default()).await;
        assert_eq!(report.total, 1, "unexpected results: {:?}", report.results);
        assert_eq!(report.unsupported, 1);
        assert_eq!(report.results[0].status, LyricsTrackStatus::Unsupported);
        assert!(report.results[0].error.as_deref().unwrap().contains("AIFF"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn album_download_preserves_valid_embedded_aiff_lyrics_without_writing() {
        let root = temp_root();
        let fixtures =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test/fixtures/tauri/media-corpus");
        let aiff = root.join("01.aiff");
        fs::copy(fixtures.join("minimal.aiff"), &aiff).unwrap();
        let mut tag = Id3v2Tag::default();
        tag.insert(Frame::UnsynchronizedText(
            lofty::id3::v2::UnsynchronizedTextFrame::new(
                lofty::TextEncoding::UTF16,
                *b"zho",
                "",
                "你好，音乐",
            ),
        ));
        tag.save_to_path(&aiff, WriteOptions::new().use_id3v23(true))
            .unwrap();
        let before = fs::read(&aiff).unwrap();

        let report = apply_album_lyrics_at(&root, None, &WriteQueue::default()).await;

        assert_eq!(report.total, 1);
        assert_eq!(report.embedded_preserved, 1);
        assert_eq!(report.unsupported, 0);
        assert_eq!(fs::read(&aiff).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn synchronized_lyrics_round_trip_every_supported_container() {
        let root = temp_root();
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test/fixtures/tauri");
        let document =
            LyricsDocument::from_synced("[00:01.250]你好\n[00:03.500]音乐", "zho").unwrap();
        let cases = [
            ("media-corpus/minimal.mp3", "track.mp3"),
            ("media-corpus/minimal.wav", "track.wav"),
            ("media-corpus/minimal.flac", "track.flac"),
            ("writer-corpus/vorbis.ogg", "track.ogg"),
            ("writer-corpus/opus.opus", "track.opus"),
            ("media-corpus/minimal.m4a", "track.m4a"),
            ("media-corpus/minimal.mp4", "track.mp4"),
            ("media-corpus/ape-id3v1-fallback.ape", "track.ape"),
        ];
        for (fixture, name) in cases {
            let path = root.join(name);
            fs::copy(fixtures.join(fixture), &path).unwrap();
            write_track_dispatch(
                &path,
                &TrackPatch {
                    lyrics: Patch::Value(document.clone()),
                    ..TrackPatch::default()
                },
            )
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(
                read_track_metadata(&path).unwrap().lyrics,
                Some(document.clone()),
                "{}",
                path.display()
            );
        }

        for name in ["track.mp3", "track.wav"] {
            let path = root.join(name);
            let bytes = fs::read(&path).unwrap();
            if name.ends_with(".mp3") {
                assert_eq!(&bytes[..4], b"ID3\x03", "{name} must use ID3v2.3");
            } else {
                assert!(
                    bytes.windows(4).any(|window| window == b"ID3\x03"),
                    "{name} must use ID3v2.3"
                );
            }
            let tag = if name.ends_with(".mp3") {
                let mut file = fs::File::open(&path).unwrap();
                MpegFile::read_from(&mut file, ParseOptions::new().read_properties(false))
                    .unwrap()
                    .id3v2()
                    .cloned()
                    .unwrap()
            } else {
                let mut file = fs::File::open(&path).unwrap();
                WavFile::read_from(&mut file, ParseOptions::new().read_properties(false))
                    .unwrap()
                    .id3v2()
                    .cloned()
                    .unwrap()
            };
            let uslt = (&tag)
                .into_iter()
                .find_map(|frame| match frame {
                    Frame::UnsynchronizedText(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert_eq!(uslt.encoding, lofty::TextEncoding::UTF16);
            assert_eq!(uslt.language, *b"zho");
            assert_eq!(uslt.content, document.plain_lyrics);
            let sylt = (&tag)
                .into_iter()
                .find_map(|frame| match frame {
                    Frame::Binary(frame) if frame.id().as_str() == "SYLT" => Some(frame),
                    _ => None,
                })
                .unwrap();
            let sylt = SynchronizedTextFrame::parse(&sylt.data, sylt.flags()).unwrap();
            assert_eq!(sylt.language, *b"zho");
            assert_eq!(sylt.timestamp_format, lofty::id3::v2::TimestampFormat::MS);
            assert_eq!(sylt.content_type, SyncTextContentType::Lyrics);

            write_track_dispatch(
                &path,
                &TrackPatch {
                    title: Patch::Value("Later title edit".to_string()),
                    ..TrackPatch::default()
                },
            )
            .unwrap();
            assert_eq!(
                read_track_metadata(&path).unwrap().lyrics,
                Some(document.clone())
            );
            assert!(
                fs::read(&path)
                    .unwrap()
                    .windows(4)
                    .any(|window| window == b"ID3\x03"),
                "{name} must remain ID3v2.3 after a later metadata edit"
            );
        }

        let mut flac_file = fs::File::open(root.join("track.flac")).unwrap();
        let flac = FlacFile::read_from(&mut flac_file, ParseOptions::new().read_properties(false))
            .unwrap();
        let comments = flac.vorbis_comments().unwrap();
        assert_eq!(comments.get("LYRICS"), document.synced_lyrics.as_deref());
        assert_eq!(
            comments.get("UNSYNCEDLYRICS"),
            Some(document.plain_lyrics.as_str())
        );
        assert_eq!(
            ape_text_values(&root.join("track.ape"), "LYRICS").first(),
            document.synced_lyrics.as_ref()
        );
        assert_eq!(
            ape_text_values(&root.join("track.ape"), "UNSYNCEDLYRICS").first(),
            Some(&document.plain_lyrics)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn malformed_higher_priority_sidecar_is_reported_without_writes() {
        let root = temp_root();
        let fixtures =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test/fixtures/tauri/media-corpus");
        let track = root.join("01.wav");
        fs::copy(fixtures.join("minimal.wav"), &track).unwrap();
        let before = fs::read(&track).unwrap();
        fs::write(track.with_extension("lrc"), [0x81]).unwrap();
        fs::write(track.with_extension("txt"), "safe lower-priority lyrics").unwrap();
        let report = apply_album_lyrics_at(&root, None, &WriteQueue::default()).await;
        assert_eq!(report.failed, 1);
        assert_eq!(report.written, 0);
        assert_eq!(fs::read(&track).unwrap(), before);
        assert_eq!(fs::read(track.with_extension("lrc")).unwrap(), [0x81]);
        assert_eq!(
            fs::read_to_string(track.with_extension("txt")).unwrap(),
            "safe lower-priority lyrics"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn prefers_synced_and_sends_encoded_optional_query() {
        let (base, request) = server(
            "200 OK",
            r#"{"plainLyrics":"plain","syncedLyrics":"[00:01]你好","lang":"zh"}"#,
        );
        assert_eq!(
            fetch_lyrics_at(
                &base,
                "Some Song",
                "Some Artist",
                Some("Album"),
                Some(244.6)
            )
            .await,
            Some(LyricsDocument::from_synced("[00:01]你好", "zho").unwrap())
        );
        let request = request.recv().unwrap();
        assert!(request.contains("GET /api/get?"));
        assert!(request.contains("track_name=Some+Song"));
        assert!(request.contains("artist_name=Some+Artist"));
        assert!(request.contains("album_name=Album"));
        assert!(request.contains("duration=245"));
        assert!(request
            .to_ascii_lowercase()
            .contains(concat!("user-agent: soundrobe/", env!("CARGO_PKG_VERSION"))));
    }

    #[tokio::test]
    async fn falls_back_plain_and_contains_all_failure_modes() {
        let (base, _) = server("200 OK", r#"{"plainLyrics":"plain"}"#);
        assert_eq!(
            fetch_lyrics_at(&base, "Track", "Artist", None, None).await,
            Some(LyricsDocument::from_plain("plain", "und").unwrap())
        );
        let (empty, _) = server("200 OK", r#"{"instrumental":true}"#);
        assert_eq!(
            fetch_lyrics_at(&empty, "Track", "Artist", None, None).await,
            None
        );
        let (missing, _) = server("404 Not Found", "{}");
        assert_eq!(
            fetch_lyrics_at(&missing, "Track", "Artist", None, None).await,
            None
        );
        let (invalid, _) = server("200 OK", "not-json");
        assert_eq!(
            fetch_lyrics_at(&invalid, "Track", "Artist", None, None).await,
            None
        );
        assert_eq!(
            fetch_lyrics_at("http://unused", "", "Artist", None, None).await,
            None
        );
        assert_eq!(
            fetch_lyrics_at("http://unused", "Track", "", None, None).await,
            None
        );
    }

    #[test]
    fn resolve_lyrics_base_url_uses_configured_value() {
        let config = AutoTagConfig {
            lyrics_api_url: Some("http://custom.lyrics/api".to_string()),
            ..AutoTagConfig::default()
        };
        assert_eq!(resolve_lyrics_base_url(&config), "http://custom.lyrics/api");
    }

    #[test]
    fn resolve_lyrics_base_url_falls_back_to_default() {
        let config = AutoTagConfig::default();
        assert_eq!(resolve_lyrics_base_url(&config), DEFAULT_BASE_URL);
    }

    #[test]
    fn resolve_lyrics_base_url_rejects_empty_string() {
        let config = AutoTagConfig {
            lyrics_api_url: Some(String::new()),
            ..AutoTagConfig::default()
        };
        assert_eq!(resolve_lyrics_base_url(&config), DEFAULT_BASE_URL);
    }

    #[test]
    fn resolve_lyrics_base_url_rejects_whitespace() {
        let config = AutoTagConfig {
            lyrics_api_url: Some("   ".to_string()),
            ..AutoTagConfig::default()
        };
        assert_eq!(resolve_lyrics_base_url(&config), DEFAULT_BASE_URL);
    }
}
