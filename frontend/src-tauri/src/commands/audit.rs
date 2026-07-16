//! Audit runner and cancellation commands.

use crate::state::audit::AuditState;
use crate::{
    commands::{
        library::is_audio_file,
        mutations::{write_track_dispatch, Patch, StringList, TrackPatch},
        tracks::read_track_metadata,
    },
    error::ApiError,
    state::write_queue::WriteQueue,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tauri::State;
use unicode_normalization::UnicodeNormalization;

const DETERMINISTIC_CONFIDENCE: f64 = 0.98;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditTrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub album_artists: Vec<String>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    pub disc_total: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditCorrectedFields {
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub title: Patch<String>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub artist: Patch<String>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub artists: Patch<Vec<String>>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub album: Patch<String>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub album_artist: Patch<String>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub album_artists: Patch<Vec<String>>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub year: Patch<String>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub genre: Patch<String>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub track_number: Patch<u32>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub track_total: Patch<u32>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub disc_number: Patch<u32>,
    #[serde(default, skip_serializing_if = "Patch::is_omitted")]
    pub disc_total: Patch<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditAlbumResult {
    pub album_path: String,
    pub results: Vec<AuditFinding>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditApplyFixesSummary {
    pub fixed: usize,
    pub album_results: Vec<AuditAlbumResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditStatus {
    Correct,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFinding {
    pub index: usize,
    pub field: String,
    pub status: AuditStatus,
    pub message: String,
    pub suggestion: Option<String>,
    pub corrected: Option<AuditCorrectedFields>,
    pub source: String,
    pub confidence: f64,
    pub auto_fix_eligible: bool,
    pub auto_fixed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditReviewTarget {
    pub index: usize,
    pub field: String,
    pub current: String,
    pub expected: Option<String>,
    pub evidence: String,
    pub reason: String,
}

#[derive(Default)]
struct FilenameFacts {
    title: Option<String>,
    artist: Option<String>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
}

fn regex(pattern: &'static str) -> Arc<Regex> {
    static REGEXES: OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, Arc<Regex>>>,
    > = OnceLock::new();
    let regexes = REGEXES.get_or_init(Default::default);
    let mut regexes = regexes.lock().unwrap_or_else(|poisoned| {
        tracing::error!("audit regex cache mutex poisoned");
        poisoned.into_inner()
    });
    regexes
        .entry(pattern)
        .or_insert_with(|| Arc::new(Regex::new(pattern).expect("valid audit regex")))
        .clone()
}

fn compact(value: Option<&str>) -> String {
    value
        .unwrap_or_default()
        .nfkc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn comparable(value: Option<&str>) -> String {
    let compacted = compact(value).to_lowercase();
    let without_punctuation = compacted
        .chars()
        .filter(|character| !"._:：'\"`()[]{}《》「」【】".contains(*character))
        .collect::<String>();
    regex(r"[‐\-‒–—―]+")
        .replace_all(&without_punctuation, "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn same_text(left: Option<&str>, right: Option<&str>) -> bool {
    comparable(left) == comparable(right)
}

fn same_string_list(left: &[String], right: &[String]) -> bool {
    let normalize = |values: &[String]| {
        values
            .iter()
            .map(|value| comparable(Some(value)))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
    };
    normalize(left) == normalize(right)
}

fn split_artist_names(value: &str) -> Vec<String> {
    let marked =
        value
            .chars()
            .collect::<Vec<_>>()
            .windows(3)
            .fold(value.to_string(), |current, window| {
                if is_cjk(window[0]) && window[1] == '.' && is_cjk(window[2]) {
                    current.replace(
                        &window.iter().collect::<String>(),
                        &format!("{}|{}", window[0], window[2]),
                    )
                } else {
                    current
                }
            });
    let delimiter = regex(
        r"(?i)\s+(?:feat\.?|ft\.?|featuring)\s+|\s*[&/;,]\s*|\s*[＋+]\s*|\s*[、，；]\s*|\s*[·‧|]\s*",
    );
    let mut seen = HashSet::new();
    delimiter
        .split(&marked)
        .filter_map(|part| {
            let artist = part.trim();
            let key = artist.to_lowercase();
            if artist.is_empty() || !seen.insert(key) {
                None
            } else {
                Some(artist.to_string())
            }
        })
        .collect()
}

fn is_cjk(character: char) -> bool {
    matches!(character, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}')
}

fn parse_filename_facts(filename: &str) -> FilenameFacts {
    let mut stem = filename
        .rsplit_once('.')
        .map_or(filename, |(stem, _)| stem)
        .trim()
        .to_string();
    let mut facts = FilenameFacts::default();

    if let Some(captures) =
        regex(r"^(\d{1,2})[-_](\d{1,3})(?:\s*[.\-_\s]+\s*|\s+)(.+)$").captures(&stem)
    {
        facts.disc_number = captures[1].parse().ok();
        facts.track_number = captures[2].parse().ok();
        stem = captures[3].trim().to_string();
    } else if let Some(captures) = regex(r"^(\d{1,3})(?:\s*[.)_-]\s*|\s+)(.+)$").captures(&stem) {
        facts.track_number = captures[1].parse().ok();
        stem = captures[2].trim().to_string();
    }

    if let Some(separator) = regex(r"\s[-–—]\s")
        .find(&stem)
        .filter(|matched| matched.start() > 0)
    {
        facts.artist = Some(stem[..separator.start()].trim().to_string());
        facts.title = Some(stem[separator.end()..].trim().to_string());
    } else if !stem.is_empty() {
        facts.title = Some(stem);
    }
    facts
}

fn extract_clear_year(album_hint: &str) -> Option<String> {
    let captures = regex(r"^((?:19|20)\d{2})[-.]")
        .captures(album_hint)
        .or_else(|| regex(r"[《（\[]\s*((?:19|20)\d{2})\s*[-.]").captures(album_hint))
        .or_else(|| regex(r"[\[(（]\s*((?:19|20)\d{2})\s*[\])）]").captures(album_hint))
        .or_else(|| regex(r"(?:^|[^\d])((?:19|20)\d{2})(?:[^\d]|$)").captures(album_hint));
    captures.map(|capture| capture[1].to_string())
}

fn clean_album_folder_name(album_hint: &str, artist_hint: Option<&str>) -> String {
    let mut cleaned = album_hint.trim().to_string();
    if let Some(bookmarked) = regex(r"《([^》]+)》").captures(&cleaned) {
        cleaned = bookmarked[1].to_string();
    }
    cleaned = regex(r"^\d{4}\s*[-—]\s*").replace(&cleaned, "").to_string();
    cleaned = regex(r"^\d{4}(?:[-.]\d{2}(?:[-.]\d{2})?)?\s*")
        .replace(&cleaned, "")
        .to_string();
    cleaned = regex(r"(?i)\s*\[?(?:flac|mp3|wav|aac|ogg|m4a|wma|ape)(?:\s*分轨)?\]?\s*$")
        .replace(&cleaned, "")
        .to_string();
    if let Some(artist) = artist_hint {
        let prefix = format!(
            r"(?i)^{}\s*(?:[-—–._:：]+|\s{{2,}})\s*",
            regex::escape(artist.trim())
        );
        if let Ok(prefix) = Regex::new(&prefix) {
            cleaned = prefix.replace(&cleaned, "").to_string();
        }
    }
    cleaned.trim().to_string()
}

fn make_finding(
    index: usize,
    field: &str,
    status: AuditStatus,
    message: String,
    corrected: Option<AuditCorrectedFields>,
    suggestion: Option<String>,
    auto_fix_eligible: bool,
) -> AuditFinding {
    AuditFinding {
        index,
        field: field.to_string(),
        status,
        message,
        suggestion,
        corrected,
        source: "deterministic".to_string(),
        confidence: DETERMINISTIC_CONFIDENCE,
        auto_fix_eligible,
        auto_fixed: false,
    }
}

fn add_unique(findings: &mut Vec<AuditFinding>, finding: AuditFinding) {
    if !findings
        .iter()
        .any(|existing| existing.index == finding.index && existing.field == finding.field)
    {
        findings.push(finding);
    }
}

pub fn build_deterministic_audit_findings(
    artist_hint: Option<&str>,
    album_hint: Option<&str>,
    tracks: &[AuditTrackMeta],
    filenames: &[String],
    disc_folder_hint: Option<&str>,
) -> Vec<AuditFinding> {
    let mut findings = Vec::new();
    let expected_album = album_hint
        .map(|value| clean_album_folder_name(value, artist_hint))
        .unwrap_or_default();
    let expected_year = album_hint.and_then(extract_clear_year);
    let expected_album_artist = compact(artist_hint);
    let expected_album_artists = split_artist_names(&expected_album_artist);
    let folder_disc_number = disc_folder_hint
        .or(album_hint)
        .and_then(|value| regex(r"(?i)^(?:cd|disc|disk)\s*(\d{1,2})$").captures(value.trim()))
        .and_then(|captures| captures[1].parse::<u32>().ok());
    let total_tracks = (tracks.len() > 1).then_some(tracks.len() as u32);

    for (index, track) in tracks.iter().enumerate() {
        let filename = filenames.get(index).map(String::as_str).unwrap_or_default();
        let facts = parse_filename_facts(filename);

        if !expected_album.is_empty() && !same_text(track.album.as_deref(), Some(&expected_album)) {
            let corrected = AuditCorrectedFields {
                album: Patch::Value(expected_album.clone()),
                ..Default::default()
            };
            add_unique(
                &mut findings,
                make_finding(
                    index,
                    "album",
                    AuditStatus::Error,
                    format!("Album tag does not match album folder \"{expected_album}\"."),
                    Some(corrected),
                    Some(expected_album.clone()),
                    true,
                ),
            );
        }

        let missing_obvious_album_artist = track.album_artist.is_none()
            && same_text(track.artist.as_deref(), Some(&expected_album_artist));
        if !expected_album_artist.is_empty() && !missing_obvious_album_artist {
            if !same_text(track.album_artist.as_deref(), Some(&expected_album_artist)) {
                let corrected = AuditCorrectedFields {
                    album_artist: Patch::Value(expected_album_artist.clone()),
                    ..Default::default()
                };
                add_unique(&mut findings, make_finding(index, "albumArtist", AuditStatus::Error,
                    format!("Album artist does not match artist folder \"{expected_album_artist}\"."), Some(corrected), Some(expected_album_artist.clone()), true));
            }
            if !track.album_artists.is_empty()
                && !expected_album_artists.is_empty()
                && !same_string_list(&track.album_artists, &expected_album_artists)
            {
                let corrected = AuditCorrectedFields {
                    album_artists: Patch::Value(expected_album_artists.clone()),
                    ..Default::default()
                };
                add_unique(&mut findings, make_finding(index, "albumArtists", AuditStatus::Error,
                    format!("Album artists list does not match artist folder \"{expected_album_artist}\"."), Some(corrected), Some(expected_album_artists.join(", ")), true));
            }
        }

        if let Some(title) = facts
            .title
            .clone()
            .filter(|title| !same_text(track.title.as_deref(), Some(title)))
        {
            let corrected = AuditCorrectedFields {
                title: Patch::Value(title.clone()),
                ..Default::default()
            };
            add_unique(
                &mut findings,
                make_finding(
                    index,
                    "title",
                    AuditStatus::Error,
                    format!("Title does not match filename \"{filename}\"."),
                    Some(corrected),
                    Some(title),
                    true,
                ),
            );
        }
        if let Some(artist) = facts
            .artist
            .clone()
            .filter(|artist| !same_text(track.artist.as_deref(), Some(artist)))
        {
            let corrected = AuditCorrectedFields {
                artist: Patch::Value(artist.clone()),
                ..Default::default()
            };
            add_unique(
                &mut findings,
                make_finding(
                    index,
                    "artist",
                    AuditStatus::Error,
                    format!("Artist does not match filename artist \"{artist}\"."),
                    Some(corrected),
                    Some(artist),
                    true,
                ),
            );
        }

        let source_artist = facts.artist.as_deref().or(track.artist.as_deref());
        if let Some(source_artist) = source_artist {
            let expected = split_artist_names(source_artist);
            let characters = source_artist.chars().collect::<Vec<_>>();
            let high_confidence = regex(r"(?i)\s+(?:feat\.?|ft\.?|featuring)\s+|[&;；、，＋+·‧]")
                .is_match(source_artist)
                || characters
                    .windows(3)
                    .any(|window| is_cjk(window[0]) && window[1] == '.' && is_cjk(window[2]));
            if expected.len() >= 2 && !same_string_list(&track.artists, &expected) {
                let replaceable = track.artists.is_empty()
                    || (track.artists.len() == 1
                        && same_text(
                            track.artists.first().map(String::as_str),
                            Some(source_artist),
                        ))
                    || (facts.artist.is_some()
                        && track.artists.len() == 1
                        && same_text(
                            track.artists.first().map(String::as_str),
                            track.artist.as_deref(),
                        ));
                if high_confidence && replaceable {
                    let corrected = AuditCorrectedFields {
                        artists: Patch::Value(expected.clone()),
                        ..Default::default()
                    };
                    add_unique(
                        &mut findings,
                        make_finding(
                            index,
                            "artists",
                            AuditStatus::Error,
                            format!(
                                "Artists should be split into separate values: {}",
                                expected.join(", ")
                            ),
                            Some(corrected),
                            Some(expected.join(", ")),
                            true,
                        ),
                    );
                } else {
                    add_unique(
                        &mut findings,
                        make_finding(
                            index,
                            "artists",
                            AuditStatus::Warning,
                            format!("Artists may need manual splitting: {source_artist}"),
                            None,
                            Some(expected.join(", ")),
                            false,
                        ),
                    );
                }
            }
        }

        if let Some(year) = expected_year
            .clone()
            .filter(|year| !same_text(track.year.as_deref(), Some(year)))
        {
            let corrected = AuditCorrectedFields {
                year: Patch::Value(year.clone()),
                ..Default::default()
            };
            add_unique(
                &mut findings,
                make_finding(
                    index,
                    "year",
                    AuditStatus::Error,
                    format!("Year tag does not match clear folder year \"{year}\"."),
                    Some(corrected),
                    Some(year),
                    true,
                ),
            );
        }

        if let Some(number) = facts.track_number {
            if track.track_number != Some(number) {
                let corrected = AuditCorrectedFields {
                    track_number: Patch::Value(number),
                    track_total: total_tracks.map_or(Patch::Omitted, Patch::Value),
                    ..Default::default()
                };
                add_unique(
                    &mut findings,
                    make_finding(
                        index,
                        "trackNumber",
                        AuditStatus::Error,
                        format!("Track number does not match filename number {number}."),
                        Some(corrected),
                        Some(number.to_string()),
                        true,
                    ),
                );
            } else if let Some(total) =
                total_tracks.filter(|total| track.track_total != Some(*total))
            {
                let corrected = AuditCorrectedFields {
                    track_total: Patch::Value(total),
                    ..Default::default()
                };
                add_unique(
                    &mut findings,
                    make_finding(
                        index,
                        "trackTotal",
                        AuditStatus::Error,
                        format!("Track total should be {total} for this album."),
                        Some(corrected),
                        Some(total.to_string()),
                        true,
                    ),
                );
            }
        }

        if let Some(number) = facts.disc_number.or(folder_disc_number) {
            if track.disc_number != Some(number) {
                let corrected = AuditCorrectedFields {
                    disc_number: Patch::Value(number),
                    ..Default::default()
                };
                add_unique(
                    &mut findings,
                    make_finding(
                        index,
                        "discNumber",
                        AuditStatus::Error,
                        format!(
                            "Disc number does not match filename or disc folder number {number}."
                        ),
                        Some(corrected),
                        Some(number.to_string()),
                        true,
                    ),
                );
            }
        }
    }
    findings
}

pub fn build_llm_review_targets(
    tracks: &[AuditTrackMeta],
    findings: &[AuditFinding],
) -> Vec<AuditReviewTarget> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for finding in findings.iter().filter(|finding| !finding.auto_fix_eligible) {
        if !seen.insert((finding.index, finding.field.clone())) {
            continue;
        }
        targets.push(AuditReviewTarget {
            index: finding.index,
            field: finding.field.clone(),
            current: track_field_value(&tracks[finding.index], &finding.field),
            expected: finding.suggestion.clone(),
            evidence: finding.message.clone(),
            reason: "Deterministic audit found an ambiguous issue that needs semantic review."
                .into(),
        });
    }
    let genres = tracks
        .iter()
        .filter_map(|track| track.genre.as_deref())
        .map(|value| comparable(Some(value)))
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();
    let differing_genres = genres.len() > 1;
    for (index, track) in tracks.iter().enumerate() {
        if seen.contains(&(index, "genre".to_string()))
            || (track.genre.is_some() && !differing_genres)
        {
            continue;
        }
        targets.push(AuditReviewTarget {
            index,
            field: "genre".into(),
            current: track.genre.clone().unwrap_or_default(),
            expected: None,
            evidence: track.genre.as_ref().map_or_else(|| "Genre tag is empty.".into(), |genre| format!("Genre \"{genre}\" differs from other album genre tags.")),
            reason: "Genre is semantic and should be reviewed by the LLM rather than deterministic rules.".into(),
        });
    }
    targets
}

fn track_field_value(track: &AuditTrackMeta, field: &str) -> String {
    match field {
        "title" => track.title.clone(),
        "artist" => track.artist.clone(),
        "album" => track.album.clone(),
        "albumArtist" => track.album_artist.clone(),
        "year" => track.year.clone(),
        "genre" => track.genre.clone(),
        "artists" => Some(track.artists.join(", ")),
        "albumArtists" => Some(track.album_artists.join(", ")),
        "trackNumber" => track.track_number.map(|value| value.to_string()),
        "trackTotal" => track.track_total.map(|value| value.to_string()),
        "discNumber" => track.disc_number.map(|value| value.to_string()),
        "discTotal" => track.disc_total.map(|value| value.to_string()),
        _ => None,
    }
    .unwrap_or_default()
}

pub fn collect_audio_files_for_audit(album_path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(album_path) else {
        return Vec::new();
    };
    let mut files = entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_file())
                .map(|_| entry.path())
        })
        .filter(|path| is_audio_file(path))
        .collect::<Vec<_>>();
    files.sort();
    files
}

pub fn discover_album_dirs(library_path: &Path) -> Vec<PathBuf> {
    let mut albums = Vec::new();
    let mut add_if_album = |path: PathBuf| {
        if !collect_audio_files_for_audit(&path).is_empty() && !albums.contains(&path) {
            albums.push(path);
            true
        } else {
            false
        }
    };
    add_if_album(library_path.to_path_buf());

    let Ok(entries) = fs::read_dir(library_path) else {
        return albums;
    };
    let mut top_level = entries
        .flatten()
        .filter(|entry| {
            !entry.file_name().to_string_lossy().starts_with('.')
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    top_level.sort();

    for directory in top_level {
        if add_if_album(directory.clone()) {
            continue;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten().filter(|entry| {
            !entry.file_name().to_string_lossy().starts_with('.')
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
        }) {
            add_if_album(entry.path());
        }
    }
    albums
}

pub fn audit_album_deterministic(
    album_path: &Path,
    cancelled: &AtomicBool,
) -> Result<Vec<AuditFinding>, ApiError> {
    let audio_files = collect_audio_files_for_audit(album_path);
    if audio_files.is_empty() {
        return Ok(Vec::new());
    }
    check_audit_cancelled(cancelled)?;

    let filenames = audio_files
        .iter()
        .map(|path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    let tracks = audio_files
        .iter()
        .map(|path| {
            check_audit_cancelled(cancelled)?;
            Ok(read_track_metadata(path).ok().map(track_meta_from_data))
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    if !tracks
        .iter()
        .flatten()
        .any(|track| track.title.is_some() || track.artist.is_some())
    {
        return Ok(Vec::new());
    }

    let folder_name = album_path.file_name().unwrap_or_default().to_string_lossy();
    let disc_hint = regex(r"(?i)^(?:cd|disc|disk)\s*\d{1,2}$")
        .is_match(folder_name.trim())
        .then(|| folder_name.into_owned());
    let metadata_album_path = if disc_hint.is_some() {
        album_path.parent().unwrap_or(album_path)
    } else {
        album_path
    };
    let artist_hint = metadata_album_path
        .parent()
        .and_then(Path::file_name)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let album_hint = metadata_album_path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let valid_tracks = tracks
        .into_iter()
        .map(|track| track.unwrap_or_default())
        .collect::<Vec<_>>();
    let mut findings = build_deterministic_audit_findings(
        Some(&artist_hint),
        Some(&album_hint),
        &valid_tracks,
        &filenames,
        disc_hint.as_deref(),
    );
    let existing = findings
        .iter()
        .map(|finding| (finding.index, finding.field.clone()))
        .collect::<HashSet<_>>();
    findings.extend(
        build_llm_review_targets(&valid_tracks, &findings)
            .into_iter()
            .filter(|target| !existing.contains(&(target.index, target.field.clone())))
            .map(|target| AuditFinding {
                index: target.index,
                field: target.field,
                status: AuditStatus::Warning,
                message: target.evidence,
                suggestion: target.expected,
                corrected: None,
                source: "deterministic".into(),
                confidence: 0.5,
                auto_fix_eligible: false,
                auto_fixed: false,
            }),
    );
    Ok(findings)
}

fn track_meta_from_data(track: crate::commands::tracks::TrackData) -> AuditTrackMeta {
    AuditTrackMeta {
        title: track.title,
        artist: track.artist,
        artists: track.artists,
        album: track.album,
        album_artist: track.album_artist,
        album_artists: track.album_artists,
        year: track.year,
        genre: track.genre,
        track_number: track.track_number,
        track_total: track.track_total,
        disc_number: track.disc_number,
        disc_total: track.disc_total,
    }
}

fn check_audit_cancelled(cancelled: &AtomicBool) -> Result<(), ApiError> {
    if cancelled.load(Ordering::Acquire) {
        Err(ApiError::Message("Audit cancelled".into()))
    } else {
        Ok(())
    }
}

struct AuditWriteJob {
    path: PathBuf,
    fields: TrackPatch,
    finding_indices: Vec<usize>,
}

pub async fn apply_audit_fixes_for_album_results(
    queue: &WriteQueue,
    mut album_results: Vec<AuditAlbumResult>,
) -> Result<AuditApplyFixesSummary, ApiError> {
    let mut fixed = 0;
    for album_result in &mut album_results {
        fixed += apply_album_fixes(queue, album_result).await?;
    }
    Ok(AuditApplyFixesSummary {
        fixed,
        album_results,
    })
}

async fn apply_album_fixes(
    queue: &WriteQueue,
    album_result: &mut AuditAlbumResult,
) -> Result<usize, ApiError> {
    let audio_files = collect_audio_files_for_audit(Path::new(&album_result.album_path));
    let mut jobs: Vec<AuditWriteJob> = Vec::new();
    for (finding_index, finding) in album_result.results.iter_mut().enumerate() {
        finding.auto_fixed = false;
        if !finding.auto_fix_eligible || finding.field == "path" {
            continue;
        }
        let Some(path) = audio_files.get(finding.index) else {
            continue;
        };
        let Some(fields) = audit_write_fields(finding) else {
            continue;
        };
        if let Some(job) = jobs.iter_mut().find(|job| job.path == *path) {
            merge_track_patch(&mut job.fields, fields);
            job.finding_indices.push(finding_index);
        } else {
            jobs.push(AuditWriteJob {
                path: path.clone(),
                fields,
                finding_indices: vec![finding_index],
            });
        }
    }
    if jobs.is_empty() {
        return Ok(0);
    }

    let writes = jobs
        .iter()
        .map(|job| (job.path.clone(), job.fields.clone()))
        .collect::<Vec<_>>();
    let successes = queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                writes
                    .into_iter()
                    .map(
                        |(path, fields)| match write_track_dispatch(&path, &fields) {
                            Ok(_) => true,
                            Err(error) => {
                                tracing::warn!(
                                    path = %path.display(),
                                    %error,
                                    "audit fix write failed"
                                );
                                false
                            }
                        },
                    )
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))
        })
        .await?;

    for (job, success) in jobs.iter().zip(&successes) {
        for finding_index in &job.finding_indices {
            album_result.results[*finding_index].auto_fixed = *success;
        }
    }
    Ok(successes.into_iter().filter(|success| *success).count())
}

fn audit_write_fields(finding: &AuditFinding) -> Option<TrackPatch> {
    if let Some(corrected) = &finding.corrected {
        let fields = TrackPatch {
            title: corrected.title.clone(),
            artist: corrected.artist.clone(),
            artists: list_patch(&corrected.artists),
            album: corrected.album.clone(),
            album_artist: corrected.album_artist.clone(),
            album_artists: list_patch(&corrected.album_artists),
            year: corrected.year.clone(),
            genre: corrected.genre.clone(),
            track_number: corrected.track_number.clone(),
            track_total: corrected.track_total.clone(),
            disc_number: corrected.disc_number.clone(),
            disc_total: corrected.disc_total.clone(),
            ..Default::default()
        };
        return track_patch_has_field(&fields).then_some(fields);
    }

    let suggestion = finding.suggestion.as_ref()?;
    let mut fields = TrackPatch::default();
    match finding.field.as_str() {
        "title" => fields.title = Patch::Value(suggestion.clone()),
        "artist" => fields.artist = Patch::Value(suggestion.clone()),
        "artists" => fields.artists = Patch::Value(StringList::One(suggestion.clone())),
        "album" => fields.album = Patch::Value(suggestion.clone()),
        "album_artist" | "albumArtist" => {
            fields.album_artist = Patch::Value(suggestion.clone());
        }
        "albumArtists" => {
            fields.album_artists = Patch::Value(StringList::One(suggestion.clone()));
        }
        "year" => fields.year = Patch::Value(suggestion.clone()),
        "genre" => fields.genre = Patch::Value(suggestion.clone()),
        "trackNumber" => fields.track_number = parsed_number_patch(suggestion),
        "trackTotal" => fields.track_total = parsed_number_patch(suggestion),
        "discNumber" => fields.disc_number = parsed_number_patch(suggestion),
        "discTotal" => fields.disc_total = parsed_number_patch(suggestion),
        _ => return None,
    }
    track_patch_has_field(&fields).then_some(fields)
}

fn list_patch(values: &Patch<Vec<String>>) -> Patch<StringList> {
    match values {
        Patch::Omitted => Patch::Omitted,
        Patch::Null => Patch::Null,
        Patch::Value(values) => Patch::Value(StringList::Many(values.clone())),
    }
}

fn parsed_number_patch(value: &str) -> Patch<u32> {
    value.parse().map_or(Patch::Omitted, Patch::Value)
}

fn track_patch_has_field(fields: &TrackPatch) -> bool {
    !fields.title.is_omitted()
        || !fields.artist.is_omitted()
        || !fields.artists.is_omitted()
        || !fields.album.is_omitted()
        || !fields.album_artist.is_omitted()
        || !fields.album_artists.is_omitted()
        || !fields.year.is_omitted()
        || !fields.genre.is_omitted()
        || !fields.track_number.is_omitted()
        || !fields.track_total.is_omitted()
        || !fields.disc_number.is_omitted()
        || !fields.disc_total.is_omitted()
}

fn merge_track_patch(target: &mut TrackPatch, incoming: TrackPatch) {
    macro_rules! merge {
        ($field:ident) => {
            if !incoming.$field.is_omitted() {
                target.$field = incoming.$field;
            }
        };
    }
    merge!(title);
    merge!(artist);
    merge!(artists);
    merge!(album);
    merge!(album_artist);
    merge!(album_artists);
    merge!(year);
    merge!(genre);
    merge!(track_number);
    merge!(track_total);
    merge!(disc_number);
    merge!(disc_total);
}

#[tauri::command]
pub async fn audit_apply_fixes(
    album_results: Vec<AuditAlbumResult>,
    queue: State<'_, WriteQueue>,
) -> Result<AuditApplyFixesSummary, ApiError> {
    apply_audit_fixes_for_album_results(&queue, album_results).await
}

#[tauri::command]
pub fn audit_cancel(state: State<'_, AuditState>) {
    state.cancel();
}

#[cfg(test)]
mod deterministic_contract_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    fn base_track() -> AuditTrackMeta {
        AuditTrackMeta {
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            artists: vec!["Artist".into()],
            album: Some("Album".into()),
            album_artist: Some("Artist".into()),
            album_artists: vec!["Artist".into()],
            year: Some("2020".into()),
            genre: Some("Pop".into()),
            track_number: Some(1),
            track_total: Some(2),
            disc_number: None,
            disc_total: None,
        }
    }

    #[test]
    fn clear_mismatches_become_fix_plans_but_ambiguous_artist_lists_do_not() {
        let tracks = vec![AuditTrackMeta {
            title: Some("Wrong".into()),
            artist: Some("AC/DC".into()),
            artists: vec![],
            album: Some("Wrong Album".into()),
            album_artist: Some("Artist".into()),
            album_artists: vec!["Artist".into()],
            year: Some("2019".into()),
            genre: Some("Pop".into()),
            track_number: Some(9),
            track_total: None,
            disc_number: None,
            disc_total: None,
        }];

        let findings = build_deterministic_audit_findings(
            Some("Artist"),
            Some("2020 - Album"),
            &tracks,
            &["01. AC/DC - Song.flac".into()],
            None,
        );

        assert!(findings.iter().any(|finding| {
            finding.field == "title"
                && finding.auto_fix_eligible
                && finding
                    .corrected
                    .as_ref()
                    .and_then(|value| value.title.value().map(String::as_str))
                    == Some("Song")
        }));
        assert!(findings.iter().any(|finding| {
            finding.field == "album"
                && finding.auto_fix_eligible
                && finding
                    .corrected
                    .as_ref()
                    .and_then(|value| value.album.value().map(String::as_str))
                    == Some("Album")
        }));
        assert!(findings.iter().any(|finding| {
            finding.field == "artists"
                && finding.status == AuditStatus::Warning
                && !finding.auto_fix_eligible
                && finding.corrected.is_none()
        }));
    }

    #[test]
    fn matching_core_tags_need_no_deterministic_or_semantic_review() {
        let mut second = base_track();
        second.title = Some("Second Song".into());
        second.track_number = Some(2);
        let tracks = vec![base_track(), second];
        let findings = build_deterministic_audit_findings(
            Some("Artist"),
            Some("2020 - Album"),
            &tracks,
            &["01. Song.flac".into(), "02. Second Song.flac".into()],
            None,
        );

        assert!(findings.is_empty());
        assert!(build_llm_review_targets(&tracks, &findings).is_empty());
    }

    #[test]
    fn missing_or_inconsistent_genres_are_semantic_review_only() {
        let mut missing = base_track();
        missing.genre = None;
        let targets = build_llm_review_targets(&[missing], &[]);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].field, "genre");
        assert_eq!(targets[0].current, "");

        let mut rock = base_track();
        rock.genre = Some("Rock".into());
        let targets = build_llm_review_targets(&[base_track(), rock], &[]);
        assert_eq!(targets.len(), 2);
        assert!(targets.iter().all(|target| target.field == "genre"));
    }

    #[test]
    fn clear_artist_and_disc_delimiters_are_safe_fix_plans() {
        let mut track = base_track();
        track.artist = Some("A & B".into());
        track.artists.clear();
        track.disc_number = None;
        let findings = build_deterministic_audit_findings(
            Some("Artist"),
            Some("Album"),
            &[track],
            &["02-01 A & B - Song.flac".into()],
            Some("Disc 2"),
        );

        assert!(findings.iter().any(|finding| {
            finding.field == "artists"
                && finding.auto_fix_eligible
                && finding
                    .corrected
                    .as_ref()
                    .and_then(|value| value.artists.value())
                    == Some(&vec!["A".to_string(), "B".to_string()])
        }));
        assert!(findings.iter().any(|finding| {
            finding.field == "discNumber"
                && finding
                    .corrected
                    .as_ref()
                    .and_then(|value| value.disc_number.value().copied())
                    == Some(2)
        }));
    }

    #[test]
    fn obvious_single_artist_album_does_not_force_missing_album_artist() {
        let mut track = base_track();
        track.album_artist = None;
        track.album_artists.clear();
        let findings = build_deterministic_audit_findings(
            Some("Artist"),
            Some("Album"),
            &[track],
            &["01. Song.flac".into()],
            None,
        );

        assert!(!findings
            .iter()
            .any(|finding| matches!(finding.field.as_str(), "albumArtist" | "albumArtists")));
    }

    #[test]
    fn discovers_flat_direct_and_nested_album_layouts() {
        let root = temp_dir("audit-discovery");
        let direct = root.join("Direct Album");
        let nested = root.join("Artist").join("Nested Album");
        fs::create_dir_all(&direct).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("root.mp3"), b"data").unwrap();
        fs::write(direct.join("one.flac"), b"data").unwrap();
        fs::write(nested.join("two.ogg"), b"data").unwrap();

        let discovered = discover_album_dirs(&root);
        assert!(discovered.contains(&root));
        assert!(discovered.contains(&direct));
        assert!(discovered.contains(&nested));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn single_album_core_reads_real_tags_and_never_writes_before_approval() {
        let root = temp_dir("audit-album");
        let album = root.join("Artist").join("2020 - Album");
        fs::create_dir_all(&album).unwrap();
        let path = album.join("01. Song.mp3");
        fs::copy(media_fixture("minimal.mp3"), &path).unwrap();
        let patch: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Wrong",
            "artist": "Artist",
            "artists": ["Artist"],
            "album": "Wrong Album",
            "albumArtist": "Artist",
            "year": "2019",
            "trackNumber": 9,
            "genre": null
        }))
        .unwrap();
        write_track_dispatch(&path, &patch).unwrap();
        let before = fs::read(&path).unwrap();

        let findings = audit_album_deterministic(&album, &AtomicBool::new(false)).unwrap();

        assert!(findings.iter().any(|finding| {
            finding.field == "title"
                && finding
                    .corrected
                    .as_ref()
                    .and_then(|value| value.title.value().map(String::as_str))
                    == Some("Song")
        }));
        assert!(findings
            .iter()
            .any(|finding| finding.field == "genre" && !finding.auto_fix_eligible));
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn approval_applies_only_eligible_fixes_and_preserves_omitted_fields() {
        let root = temp_dir("audit-apply");
        let album = root.join("Artist").join("Album");
        fs::create_dir_all(&album).unwrap();
        let path = album.join("01. Song.mp3");
        fs::copy(media_fixture("minimal.mp3"), &path).unwrap();
        let initial: TrackPatch = serde_json::from_value(serde_json::json!({
            "title": "Wrong",
            "artist": "Keep Artist",
            "album": "Keep Album",
            "genre": "Remove Me"
        }))
        .unwrap();
        write_track_dispatch(&path, &initial).unwrap();
        let results: Vec<AuditFinding> = serde_json::from_value(serde_json::json!([
            {
                "index": 0,
                "field": "title",
                "status": "error",
                "message": "Title mismatch",
                "corrected": { "title": "Song" },
                "source": "deterministic",
                "confidence": 0.98,
                "autoFixEligible": true,
                "autoFixed": false
            },
            {
                "index": 0,
                "field": "genre",
                "status": "error",
                "message": "Clear genre",
                "corrected": { "genre": null },
                "source": "llm",
                "confidence": 0.95,
                "autoFixEligible": true,
                "autoFixed": false
            },
            {
                "index": 0,
                "field": "album",
                "status": "warning",
                "message": "Do not apply",
                "corrected": { "album": "Wrongly Applied" },
                "source": "llm",
                "confidence": 0.5,
                "autoFixEligible": false,
                "autoFixed": false
            }
        ]))
        .unwrap();

        let summary = apply_audit_fixes_for_album_results(
            &WriteQueue::default(),
            vec![AuditAlbumResult {
                album_path: album.to_string_lossy().into_owned(),
                results,
            }],
        )
        .await
        .unwrap();

        assert_eq!(summary.fixed, 1);
        assert!(summary.album_results[0].results[0].auto_fixed);
        assert!(summary.album_results[0].results[1].auto_fixed);
        assert!(!summary.album_results[0].results[2].auto_fixed);
        let track = read_track_metadata(&path).unwrap();
        assert_eq!(track.title.as_deref(), Some("Song"));
        assert_eq!(track.artist.as_deref(), Some("Keep Artist"));
        assert_eq!(track.album.as_deref(), Some("Keep Album"));
        assert_eq!(track.genre, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn approval_marks_only_successful_file_jobs_and_continues_after_failure() {
        let root = temp_dir("audit-apply-partial");
        let album = root.join("Artist").join("Album");
        fs::create_dir_all(&album).unwrap();
        let good = album.join("01. Good.mp3");
        let bad = album.join("02. Bad.mp3");
        fs::copy(media_fixture("minimal.mp3"), &good).unwrap();
        fs::write(&bad, b"not an mp3").unwrap();
        let results: Vec<AuditFinding> = serde_json::from_value(serde_json::json!([
            {
                "index": 0, "field": "title", "status": "error",
                "message": "good", "corrected": { "title": "Good" },
                "source": "deterministic", "confidence": 0.98,
                "autoFixEligible": true, "autoFixed": false
            },
            {
                "index": 1, "field": "title", "status": "error",
                "message": "bad", "corrected": { "title": "Bad" },
                "source": "deterministic", "confidence": 0.98,
                "autoFixEligible": true, "autoFixed": false
            }
        ]))
        .unwrap();

        let summary = apply_audit_fixes_for_album_results(
            &WriteQueue::default(),
            vec![AuditAlbumResult {
                album_path: album.to_string_lossy().into_owned(),
                results,
            }],
        )
        .await
        .unwrap();

        assert_eq!(summary.fixed, 1);
        assert!(summary.album_results[0].results[0].auto_fixed);
        assert!(!summary.album_results[0].results[1].auto_fixed);
        assert_eq!(
            read_track_metadata(&good).unwrap().title.as_deref(),
            Some("Good")
        );
        assert_eq!(fs::read(&bad).unwrap(), b"not an mp3");
        fs::remove_dir_all(root).unwrap();
    }

    fn media_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus")
            .join(name)
    }

    fn temp_dir(label: &str) -> PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "auto-tagger-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
