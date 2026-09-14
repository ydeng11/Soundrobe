//! Ignored native verification for the live Enya review session.
//!
//! This module deliberately lives behind cfg(test). It exercises the
//! production resolver against disposable copies and leaves only sanitized,
//! audio-free evidence under .planning/debug.

use super::*;
use crate::commands::mutations::{write_track_queued, Patch, TrackPatch};
use crate::state::config::{load_from, ProcessEnv};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const EXPECTED_PROCESSED: usize = 69;
const EXPECTED_APPLIED: usize = 32;
const EXPECTED_REVIEW: usize = 37;
const EXPECTED_REVIEW_FOLDERS: usize = 34;
const EXPECTED_ALL_FOLDERS: usize = 59;

#[derive(Debug, Deserialize)]
struct EnyaManifest {
    #[serde(rename = "sourceRoot")]
    source_root: PathBuf,
    summary: ManifestSummary,
    #[serde(rename = "logicalResults")]
    logical_results: Vec<LogicalReview>,
}

#[derive(Debug, Deserialize)]
struct ManifestSummary {
    processed: usize,
    applied: usize,
    #[serde(rename = "needsReview")]
    needs_review: usize,
    failed: usize,
    cancelled: usize,
    #[serde(rename = "uniqueFolders")]
    unique_folders: usize,
    #[serde(rename = "uniqueNeedsReviewFolders")]
    unique_needs_review_folders: usize,
}

#[derive(Debug, Deserialize)]
struct LogicalReview {
    id: usize,
    outcome: String,
    path: PathBuf,
    detail: String,
    #[serde(default, rename = "retryCount")]
    retry_count: u32,
    #[serde(default)]
    attempts: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderClassification {
    ConfirmedSuccess,
    NeedsReview,
    Incomplete,
    FailedVerification,
}

impl FolderClassification {
    fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmedSuccess => "confirmed_success",
            Self::NeedsReview => "needs_review",
            Self::Incomplete => "incomplete",
            Self::FailedVerification => "failed_verification",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct FileHash {
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
struct HashSetRecord {
    files: BTreeMap<String, FileHash>,
    #[serde(rename = "audioPayloads")]
    audio_payloads: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct ReadbackRecord {
    exact: bool,
    collaborator_preserved: bool,
    mismatches: Vec<String>,
}

fn classify_invocation(
    result: Option<&AutoTagRunResult>,
    resolver_error: bool,
    timed_out: bool,
    provider_incomplete: bool,
    invariants_ok: bool,
) -> FolderClassification {
    if resolver_error || (!invariants_ok && result.is_some_and(|value| value.outcome == AutoTagOutcome::Applied)) {
        return FolderClassification::FailedVerification;
    }
    if timed_out || provider_incomplete {
        return FolderClassification::Incomplete;
    }
    match result.map(|value| value.outcome) {
        Some(AutoTagOutcome::Applied) if invariants_ok => FolderClassification::ConfirmedSuccess,
        Some(AutoTagOutcome::NeedsReview) => FolderClassification::NeedsReview,
        _ => FolderClassification::FailedVerification,
    }
}

fn reconcile_folder_classification(
    cold: FolderClassification,
    warm: FolderClassification,
    identity_consistent: bool,
) -> FolderClassification {
    if !identity_consistent
        || cold == FolderClassification::FailedVerification
        || warm == FolderClassification::FailedVerification
    {
        FolderClassification::FailedVerification
    } else if cold == FolderClassification::ConfirmedSuccess
        || warm == FolderClassification::ConfirmedSuccess
    {
        FolderClassification::ConfirmedSuccess
    } else if cold == FolderClassification::Incomplete || warm == FolderClassification::Incomplete
    {
        FolderClassification::Incomplete
    } else {
        FolderClassification::NeedsReview
    }
}

fn sanitize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let lower = key.to_ascii_lowercase();
                    let sensitive = lower.contains("token")
                        || lower.contains("authorization")
                        || lower.contains("api_key")
                        || lower.contains("apikey")
                        || lower == "key"
                        || lower.ends_with("key")
                        || lower.contains("credential")
                        || lower.contains("secret")
                        || lower.contains("password")
                        || lower.contains("cookie")
                        || lower == "headers"
                        || lower == "config";
                    (
                        key,
                        if sensitive {
                            Value::String("[redacted]".into())
                        } else {
                            sanitize(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::String(value) => {
            let value = value
                .split_once('?')
                .filter(|(base, _)| base.starts_with("http://") || base.starts_with("https://"))
                .map(|(base, _)| format!("{base}?[redacted-query]"))
                .unwrap_or(value);
            Value::String(
                value
                    .replace("Bearer ", "Bearer [redacted]")
                    .replace("bearer ", "bearer [redacted]"),
            )
        }
        other => other,
    }
}

fn sanitize_error(error: impl AsRef<str>) -> String {
    sanitize(Value::String(error.as_ref().to_string()))
        .as_str()
        .unwrap_or("[redacted]")
        .chars()
        .take(400)
        .collect()
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn stream_hash_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok((bytes, format!("{:x}", hasher.finalize())))
}

fn flac_payload(bytes: &[u8]) -> &[u8] {
    let Some(marker) = bytes.windows(4).position(|window| window == b"fLaC") else {
        return bytes;
    };
    let mut offset = marker.saturating_add(4);
    while offset + 4 <= bytes.len() {
        let last = bytes[offset] & 0x80 != 0;
        let size = ((bytes[offset + 1] as usize) << 16)
            | ((bytes[offset + 2] as usize) << 8)
            | bytes[offset + 3] as usize;
        let next = offset.saturating_add(4).saturating_add(size);
        if next > bytes.len() {
            return bytes;
        }
        offset = next;
        if last {
            return &bytes[offset..];
        }
    }
    bytes
}

fn walk_files(current: &Path, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("symlink in fixture: {}", path.display()),
            ));
        }
        if file_type.is_dir() {
            walk_files(&path, output)?;
        } else if file_type.is_file() {
            output.push(path);
        }
    }
    Ok(())
}

fn hash_tree(root: &Path, include_audio_payloads: bool) -> std::io::Result<HashSetRecord> {
    let mut paths = Vec::new();
    walk_files(root, &mut paths)?;
    paths.sort();
    let mut record = HashSetRecord::default();
    for path in paths {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("flac"))
        {
            continue;
        }
        let (bytes, sha256) = stream_hash_file(&path)?;
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        record.files.insert(
            relative.clone(),
            FileHash {
                bytes,
                sha256,
            },
        );
        if include_audio_payloads {
            let bytes = fs::read(&path)?;
            record
                .audio_payloads
                .insert(relative, hash_bytes(flac_payload(&bytes)));
        }
    }
    Ok(record)
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    let mut files = Vec::new();
    walk_files(source, &mut files)?;
    for path in files {
        let relative = path.strip_prefix(source).unwrap();
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(path, target)?;
    }
    Ok(())
}

fn cache_counts(path: &Path) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    let Ok(connection) = Connection::open(path) else {
        return counts;
    };
    for table in ["lookup_cache", "artist_release_cache", "release_detail_cache"] {
        let count = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0);
        counts.insert(table.to_string(), count.max(0) as u64);
    }
    counts
}

fn provider_incomplete(
    result: Option<&AutoTagRunResult>,
    error: Option<&str>,
    timed_out: bool,
) -> bool {
    if timed_out {
        return true;
    }
    let attempts_unavailable = result.is_some_and(|value| {
        value
            .provider_attempts
            .iter()
            .any(|attempt| attempt.status == ProviderAttemptStatus::Unavailable)
    });
    let text = error
        .into_iter()
        .chain(result.into_iter().flat_map(|value| {
            value
                .diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic.as_str())
        }))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    attempts_unavailable
        || ["429", "503", "rate limit", "timed out", "timeout", "detail limit", "unavailable"]
            .iter()
            .any(|needle| text.contains(needle))
}

fn evidence_counts(request: &LookupRequest, candidate: &AlbumCandidate) -> Value {
    let filenames = collect_audio_files(Path::new(&request.path))
        .into_iter()
        .filter_map(|path| Path::new(&path).file_name()?.to_str().map(str::to_string))
        .collect::<Vec<_>>();
    let artist_hints = request
        .artist_hint
        .iter()
        .chain(candidate.artist.iter())
        .chain(candidate.album_artist.iter())
        .cloned()
        .collect::<Vec<_>>();
    let matched = match_remote_candidate_tracks(
        &request.tracks,
        &filenames,
        &candidate.tracks,
        lookup_source_name(candidate.source),
        &artist_hints,
        &[],
    );
    let mut counts = BTreeMap::<String, usize>::new();
    for evidence in matched.evidence.into_iter().flatten() {
        *counts.entry(format!("{evidence:?}")).or_default() += 1;
    }
    json!({
        "counts": counts,
        "matched": matched.stats.matched,
        "local": matched.stats.local,
        "remote": matched.stats.remote,
        "rejections": matched.title_rejections.iter().map(|skip| json!({
            "localIndex": skip.local_index,
            "remoteIndex": skip.remote_index,
            "kind": format!("{:?}", skip.kind),
        })).collect::<Vec<_>>(),
    })
}

fn selected_identity(candidate: &AlbumCandidate) -> Value {
    json!({
        "source": lookup_source_name(candidate.source),
        "musicbrainzAlbumId": candidate.musicbrainz_album_id,
        "discogsReleaseId": candidate.discogs_release_id,
        "album": candidate.album,
        "artist": candidate.artist,
        "year": candidate.year,
    })
}

fn readback(album: &Path, before: &LookupRequest, candidate: &AlbumCandidate) -> ReadbackRecord {
    let mut record = ReadbackRecord {
        exact: true,
        collaborator_preserved: true,
        mismatches: Vec::new(),
    };
    let files = collect_audio_files(album);
    if files.len() != candidate.tracks.len() {
        record.exact = false;
        record.mismatches.push(format!(
            "track count {} != candidate {}",
            files.len(),
            candidate.tracks.len()
        ));
        return record;
    }
    for ((file, local), mapped) in files.iter().zip(&before.tracks).zip(&candidate.tracks) {
        let Ok(read) = crate::commands::tracks::read_track_metadata(Path::new(file)) else {
            record.exact = false;
            record
                .mismatches
                .push(format!("unreadable {}", Path::new(file).display()));
            continue;
        };
        let checks = [
            (
                "title",
                read.title.as_ref() == mapped.title.as_ref().or(local.title.as_ref()),
            ),
            (
                "artist",
                read.artist.as_ref() == mapped.artist.as_ref().or(local.artist.as_ref()),
            ),
            (
                "album",
                read.album.as_ref() == candidate.album.as_ref().or(before.album_hint.as_ref()),
            ),
            (
                "year",
                read.year.as_ref() == candidate.year.as_ref().or(before.year_hint.as_ref()),
            ),
            (
                "trackNumber",
                read.track_number == mapped.track_number.or(local.track_number),
            ),
            (
                "discNumber",
                read.disc_number == mapped.disc_number.or(local.disc_number),
            ),
            (
                "musicbrainzAlbumId",
                read.musicbrainz_album_id == candidate.musicbrainz_album_id,
            ),
            (
                "discogsReleaseId",
                read.discogs_release_id == candidate.discogs_release_id,
            ),
        ];
        for (name, matches) in checks {
            if !matches {
                record.exact = false;
                record
                    .mismatches
                    .push(format!("{name}: {}", Path::new(file).display()));
            }
        }
        let old = split_collaborative_artists(&local.artist, &local.artists);
        let new = split_collaborative_artists(&read.artist, &read.artists);
        if !old.iter().all(|artist| new.contains(artist)) {
            record.collaborator_preserved = false;
            record
                .mismatches
                .push(format!("collaborators: {}", Path::new(file).display()));
        }
    }
    record
}

fn append_jsonl(path: &Path, value: &Value) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(value).unwrap_or_else(|_| "{}".into())
    )
}

fn load_manifest(path: &Path) -> EnyaManifest {
    let text = fs::read_to_string(path).expect("Enya manifest must be readable");
    let manifest: EnyaManifest =
        serde_json::from_str(&text).expect("Enya manifest must be valid JSON");
    assert_eq!(manifest.summary.processed, EXPECTED_PROCESSED);
    assert_eq!(manifest.summary.applied, EXPECTED_APPLIED);
    assert_eq!(manifest.summary.needs_review, EXPECTED_REVIEW);
    assert_eq!(manifest.summary.failed, 0);
    assert_eq!(manifest.summary.cancelled, 0);
    assert_eq!(manifest.summary.unique_folders, EXPECTED_ALL_FOLDERS);
    assert_eq!(
        manifest.summary.unique_needs_review_folders,
        EXPECTED_REVIEW_FOLDERS
    );
    assert_eq!(manifest.logical_results.len(), EXPECTED_REVIEW);
    assert!(manifest
        .logical_results
        .iter()
        .all(|row| row.outcome == "NeedsReview" && !row.path.is_absolute()));
    assert_eq!(
        manifest
            .logical_results
            .iter()
            .map(|row| row.path.clone())
            .collect::<BTreeSet<_>>()
            .len(),
        EXPECTED_REVIEW_FOLDERS
    );
    manifest
}

fn validate_source_path(root: &Path, relative: &Path) -> PathBuf {
    let root = fs::canonicalize(root).expect("Enya source root must exist");
    let path = fs::canonicalize(root.join(relative)).expect("manifest source folder must exist");
    assert!(
        path.starts_with(&root),
        "manifest path escaped Enya root: {}",
        relative.display()
    );
    path
}

fn parse_folder_scope(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn report_markdown(
    artifact_dir: &Path,
    manifest: &EnyaManifest,
    folder_results: &BTreeMap<String, Value>,
) -> std::io::Result<()> {
    let mut counts = BTreeMap::<String, usize>::new();
    for result in folder_results.values() {
        let classification = result
            .get("classification")
            .and_then(Value::as_str)
            .unwrap_or("failed_verification");
        *counts.entry(classification.to_string()).or_default() += 1;
    }
    let guarded_attributions = folder_results
        .values()
        .filter(|result| {
            result
                .get("classification")
                .and_then(Value::as_str)
                == Some(FolderClassification::ConfirmedSuccess.as_str())
                && ["cold", "warm"].iter().any(|phase| {
                    result
                        .get(*phase)
                        .and_then(|value| value.get("selectedMatchEvidence"))
                        .and_then(|value| value.get("counts"))
                        .and_then(|value| value.get("GuardedTitle"))
                        .and_then(Value::as_u64)
                        .is_some_and(|count| count > 0)
                })
        })
        .count();
    let mut causes = BTreeMap::<String, usize>::new();
    for result in folder_results.values() {
        match result.get("classification").and_then(Value::as_str) {
            Some(value) if value == FolderClassification::Incomplete.as_str() => {
                *causes.entry("provider unavailable or detail limit".into()).or_default() += 1;
            }
            Some(value) if value == FolderClassification::NeedsReview.as_str() => {
                *causes.entry("no authoritative provider match".into()).or_default() += 1;
            }
            _ => {}
        }
    }
    let mut text = String::new();
    text.push_str("# Enya native auto-tag verification\n\n");
    text.push_str("The frozen live session contained 69 logical results (32 Applied, 37 Needs Review) across 59 physical folders. The 37 review rows map to 34 unique folders; duplicate logical rows remain visible in results.json.\n\n");
    text.push_str("## Folder classifications\n\n");
    for name in [
        "confirmed_success",
        "needs_review",
        "incomplete",
        "failed_verification",
    ] {
        text.push_str(&format!(
            "- {name}: {}\n",
            counts.get(name).copied().unwrap_or(0)
        ));
    }
    text.push_str("\n## Album results\n\n");
    text.push_str("| Folder | Classification | Cold | Warm | Selected release IDs |\n|---|---|---|---|---|\n");
    for (folder, result) in folder_results {
        let classification = result
            .get("classification")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let cold = result
            .get("cold")
            .map(|_| "recorded")
            .unwrap_or("missing");
        let warm = result
            .get("warm")
            .map(|_| "recorded")
            .unwrap_or("missing");
        let ids = ["cold", "warm"]
            .iter()
            .filter_map(|phase| result.get(*phase)?.get("selectedIdentity")?.get("discogsReleaseId"))
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(
            "| {folder} | {classification} | {cold} | {warm} | {ids} |\n"
        ));
    }
    text.push_str("\n## Attribution\n\n");
    text.push_str("A matcher attribution is reported only when a native selected candidate carries GuardedTitle evidence and prior evidence shows that the same release failed before normalized-title matching. Provider recovery and different-release selection remain separate causes. Matcher-only cache probes are supporting evidence and are not counted as native successes.\n\n");
    text.push_str(&format!("Native GuardedTitle attributions: {guarded_attributions}.\n\n"));
    text.push_str("## Remaining causes\n\n");
    let mut ranked_causes = causes.into_iter().collect::<Vec<_>>();
    ranked_causes.sort_by(|(left_name, left_count), (right_name, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_name.cmp(right_name))
    });
    for (cause, count) in ranked_causes {
        text.push_str(&format!("- {cause}: {count} folder(s)\n"));
    }
    text.push_str(&format!(
        "Manifest rows reconciled: {} logical review rows, {} unique review folders.\n",
        manifest.logical_results.len(),
        folder_results.len()
    ));
    fs::write(artifact_dir.join("report.md"), text)
}

#[test]
fn enya_manifest_reconciles_expected_counts_and_duplicates() {
    // Keep the unit contract independent of retained, local-only debug
    // artifacts. The native smoke still loads the real manifest through its
    // explicit environment variable.
    let path = std::env::temp_dir().join(format!(
        "soundrobe-enya-manifest-{}.json",
        uuid::Uuid::new_v4()
    ));
    let logical_results = (1..=37)
        .map(|id| {
            let folder = if id <= 34 {
                format!("Enya/Review {id}")
            } else {
                format!("Enya/Review {}", id - 3)
            };
            json!({
                "id": id,
                "outcome": "NeedsReview",
                "path": folder,
                "detail": "fixture",
                "retryCount": 0,
                "attempts": if id == 1 { Some(2) } else { None::<u32> },
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "sourceRoot": "/tmp/Enya",
            "summary": {
                "processed": 69,
                "applied": 32,
                "needsReview": 37,
                "failed": 0,
                "cancelled": 0,
                "uniqueFolders": 59,
                "uniqueNeedsReviewFolders": 34,
            },
            "logicalResults": logical_results,
        }))
        .unwrap(),
    )
    .unwrap();
    let manifest = load_manifest(&path);
    assert_eq!(manifest.logical_results[0].id, 1);
    assert!(manifest
        .logical_results
        .iter()
        .any(|row| row.attempts == Some(2)));
    fs::remove_file(path).unwrap();
}

#[test]
fn enya_folder_scope_parser_ignores_blank_lines_and_deduplicates() {
    let scope = parse_folder_scope("first\n\n second \nfirst\n");
    assert_eq!(scope.into_iter().collect::<Vec<_>>(), vec!["first", "second"]);
}

#[test]
fn enya_2016_greatest_hits_disc_suffix_rejects_title_conflict_only_on_cd1() {
    let candidate: AlbumCandidate = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/candidate-8174021.json"
    ))
    .expect("candidate fixture");
    let request_cd1: LookupRequest = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/local-cd1.json"
    ))
    .expect("CD1 request fixture");
    let request_cd2: LookupRequest = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/local-cd2.json"
    ))
    .expect("CD2 request fixture");
    assert_eq!(request_cd1.selected_disc_number, Some(1));
    assert_eq!(candidate_tracks_for_request(&request_cd1, &candidate).len(), 22);
    assert_eq!(
        strict_provider_candidate_credibility(&request_cd1, &candidate).unwrap_err(),
        "provider tracks do not strongly cover every local file: track 12 \"White Is The Winter Night\" -> \"White Is In The Winter Night\": positional_only"
    );
    assert_eq!(request_cd2.selected_disc_number, Some(2));
    assert_eq!(candidate_tracks_for_request(&request_cd2, &candidate).len(), 21);
    assert!(strict_provider_candidate_credibility(&request_cd2, &candidate).is_ok());
}

#[test]
fn enya_2009_greatest_hits_cd2_keeps_title_conflicts_rejected() {
    let request: LookupRequest = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/local-2009-cd2.json"
    ))
    .expect("request fixture");
    let candidate: AlbumCandidate = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/candidate-1969042.json"
    ))
    .expect("candidate fixture");
    assert_eq!(candidate_tracks_for_request(&request, &candidate).len(), 22);
    let credibility = strict_provider_candidate_credibility(&request, &candidate);
    assert_eq!(
        credibility.unwrap_err(),
        "provider tracks do not strongly cover every local file: track 19 \"Afer Ventus\" -> \"After Ventus\": positional_only"
    );
}

#[test]
fn enya_wild_child_maxi_suffix_matches_discogs_release() {
    let request: LookupRequest = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/local-wild-child.json"
    ))
    .expect("request fixture");
    let candidate: AlbumCandidate = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/candidate-544115.json"
    ))
    .expect("candidate fixture");
    assert!(strict_provider_candidate_credibility(&request, &candidate).is_ok());
}

#[test]
fn enya_box_of_dreams_clouds_disc_suffix_matches_discogs_release() {
    let request: LookupRequest = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/local-box-clouds.json"
    ))
    .expect("request fixture");
    let candidate: AlbumCandidate = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/candidate-7832914.json"
    ))
    .expect("candidate fixture");
    assert_eq!(candidate_tracks_for_request(&request, &candidate).len(), 15);
    assert!(strict_provider_candidate_credibility(&request, &candidate).is_ok());
}

#[test]
fn enya_only_time_box_disc_suffix_keeps_disc_mapping_strict() {
    let request: LookupRequest = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/local-only-time-cd2.json"
    ))
    .expect("request fixture");
    let candidate: AlbumCandidate = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/enya-greatest-hits/candidate-15876868.json"
    ))
    .expect("candidate fixture");
    assert_eq!(candidate_tracks_for_request(&request, &candidate).len(), 12);
    let credibility = strict_provider_candidate_credibility(&request, &candidate);
    assert_eq!(
        credibility.unwrap_err(),
        "provider tracks do not strongly cover every local file: track 4 \"The Longships\" -> \"The Longship\": positional_only"
    );
}

#[test]
fn enya_discogs_cd_dvd_candidate_scopes_audio_group_without_flattening() {
    let request = LookupRequest {
        tracks: (1..=22)
            .map(|number| TrackCandidate {
                title: Some(format!("CD track {number}")),
                track_number: Some(number),
                ..TrackCandidate::default()
            })
            .collect(),
        ..LookupRequest::default()
    };
    let cd = (1..=22)
        .map(|number| TrackCandidate {
            title: Some(format!("CD track {number}")),
            track_number: Some(number),
            track_total: Some(22),
            media_type: Some("CD".into()),
            ..TrackCandidate::default()
        })
        .collect::<Vec<_>>();
    let dvd = (1..=16)
        .map(|number| TrackCandidate {
            title: Some(format!("DVD extra {number}")),
            track_number: Some(number),
            track_total: Some(16),
            media_type: Some("DVD".into()),
            ..TrackCandidate::default()
        })
        .collect::<Vec<_>>();
    let candidate = AlbumCandidate {
        tracks: cd.iter().chain(&dvd).cloned().collect(),
        ..AlbumCandidate::default()
    };
    let selected = candidate_tracks_for_request(&request, &candidate);
    assert_eq!(selected.len(), 22);
    assert!(selected
        .iter()
        .all(|track| track.media_type.as_deref() == Some("CD")));

    let mut ambiguous = candidate.clone();
    ambiguous.tracks.extend(cd);
    assert_eq!(candidate_tracks_for_request(&request, &ambiguous).len(), 60);
}

#[test]
fn enya_sanitization_redacts_secrets_and_url_queries_recursively() {
    let value = sanitize(json!({
        "url": "https://example.test/release?id=secret",
        "headers": {"Authorization": "Bearer abc"},
        "nested": [{"discogsToken": "abc", "accessKey": "abc", "title": "Déjà Vu"}],
    }));
    assert_eq!(value["url"], "https://example.test/release?[redacted-query]");
    assert_eq!(value["headers"], "[redacted]");
    assert_eq!(value["nested"][0]["discogsToken"], "[redacted]");
    assert_eq!(value["nested"][0]["accessKey"], "[redacted]");
    assert_eq!(value["nested"][0]["title"], "Déjà Vu");
}

#[test]
fn enya_classification_prioritizes_safety_and_provider_state() {
    assert_eq!(
        classify_invocation(None, false, true, false, false),
        FolderClassification::Incomplete
    );
    assert_eq!(
        classify_invocation(None, true, false, true, false),
        FolderClassification::FailedVerification
    );
    let result = AutoTagRunResult {
        outcome: AutoTagOutcome::NeedsReview,
        authority: None,
        candidate: None,
        written: 0,
        reason_code: None,
        diagnostics: Vec::new(),
        provider_attempts: Vec::new(),
        ai_status: None,
        ai_confidence: None,
        ai_threshold: None,
    };
    assert_eq!(
        classify_invocation(Some(&result), false, false, false, true),
        FolderClassification::NeedsReview
    );
}

#[test]
fn enya_folder_reconciliation_keeps_failed_verification_terminal() {
    assert_eq!(
        reconcile_folder_classification(
            FolderClassification::FailedVerification,
            FolderClassification::ConfirmedSuccess,
            true,
        ),
        FolderClassification::FailedVerification
    );
    assert_eq!(
        reconcile_folder_classification(
            FolderClassification::Incomplete,
            FolderClassification::ConfirmedSuccess,
            true,
        ),
        FolderClassification::ConfirmedSuccess
    );
    assert_eq!(
        reconcile_folder_classification(
            FolderClassification::ConfirmedSuccess,
            FolderClassification::ConfirmedSuccess,
            false,
        ),
        FolderClassification::FailedVerification
    );
}

#[test]
fn enya_flac_payload_hash_ignores_id3_prefix() {
    let mut bytes = b"ID3\0\0\0\0\0\0fLaC".to_vec();
    bytes.extend_from_slice(&[0x80, 0, 0, 0]);
    bytes.extend_from_slice(b"audio");
    assert_eq!(flac_payload(&bytes), b"audio");
}

#[tokio::test]
async fn enya_readback_validation_checks_written_fields_and_collaborators() {
    let root = std::env::temp_dir().join(format!("soundrobe-enya-readback-{}", uuid::Uuid::new_v4()));
    let album = root.join("Enya/readback");
    fs::create_dir_all(&album).expect("create readback fixture");
    let media = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../test/fixtures/tauri/media-corpus/minimal.flac");
    let file = album.join("01 Dr. West.flac");
    fs::copy(media, &file).expect("copy readback fixture");

    let mut before: LookupRequest = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/relapse-deluxe/local.json"
    ))
    .expect("local fixture");
    before.path = album.to_string_lossy().into_owned();
    before.tracks.truncate(1);
    let mut candidate: AlbumCandidate = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/relapse-deluxe/candidate-36441795.json"
    ))
    .expect("candidate fixture");
    candidate.tracks.truncate(1);
    candidate.tracks[0].title = before.tracks[0].title.clone();
    candidate.tracks[0].artist = before.tracks[0].artist.clone();
    candidate.tracks[0].artists = before.tracks[0].artists.clone();
    candidate.tracks[0].disc_number = Some(1);
    candidate.tracks[0].disc_total = Some(1);

    apply_candidate_tags(&album, &candidate, &WriteQueue::default())
        .await
        .expect("apply readback fixture");
    let record = readback(&album, &before, &candidate);
    assert!(record.exact, "{record:?}");
    assert!(record.collaborator_preserved, "{record:?}");
    fs::remove_dir_all(root).expect("remove readback fixture");
}

/// Run all unique Needs Review folders twice against live providers. The test
/// is intentionally ignored: it requires the user's Enya media, a Discogs
/// token, and provider network access.
#[tokio::test]
#[ignore = "requires 34 Enya review folders, live providers, and local config"]
async fn live_enya_review_smoke() {
    let manifest_path = PathBuf::from(
        std::env::var("SOUNDROBE_ENYA_REVIEW_MANIFEST")
            .expect("SOUNDROBE_ENYA_REVIEW_MANIFEST required"),
    );
    let artifact_dir = PathBuf::from(
        std::env::var("SOUNDROBE_ENYA_ARTIFACT_DIR")
            .expect("SOUNDROBE_ENYA_ARTIFACT_DIR required"),
    );
    fs::create_dir_all(&artifact_dir).expect("create Enya artifact directory");
    let manifest = load_manifest(&manifest_path);
    let source_root = fs::canonicalize(&manifest.source_root).expect("source root must exist");
    let mut folders = BTreeMap::<String, PathBuf>::new();
    for row in &manifest.logical_results {
        folders
            .entry(row.path.to_string_lossy().to_string())
            .or_insert_with(|| validate_source_path(&source_root, &row.path));
    }
    let scoped_folders = std::env::var("SOUNDROBE_ENYA_REVIEW_FOLDERS_FILE")
        .ok()
        .map(|path| parse_folder_scope(&fs::read_to_string(path).expect("folder scope readable")));
    if let Some(scope) = &scoped_folders {
        assert!(!scope.is_empty(), "folder scope must not be empty");
        folders.retain(|relative, _| scope.contains(relative));
        assert_eq!(folders.len(), scope.len(), "folder scope contains unknown paths");
    } else {
        assert_eq!(folders.len(), EXPECTED_REVIEW_FOLDERS);
    }

    let command_log = artifact_dir.join("command.log");
    fs::write(&command_log, "live_enya_review_smoke started\n")
        .expect("create command log");

    let source_hashes_before = folders
        .iter()
        .map(|(relative, path)| {
            let mut log = OpenOptions::new().append(true).open(&command_log).unwrap();
            writeln!(log, "hashing source {relative}").unwrap();
            let hashes = hash_tree(path, false).expect("hash source");
            writeln!(log, "hashed source {relative}").unwrap();
            (relative.clone(), hashes)
        })
        .collect::<BTreeMap<_, _>>();
    fs::write(
        artifact_dir.join("source-hashes-before.json"),
        serde_json::to_vec_pretty(&source_hashes_before).unwrap(),
    )
    .expect("write source hashes before");

    let config_path = dirs::home_dir()
        .expect("home directory")
        .join(".soundrobe/config.yaml");
    let config_text = fs::read_to_string(config_path).expect("read Soundrobe config");
    let mut config = load_from(&config_text, &ProcessEnv);
    assert!(
        config.discogs_token.is_some(),
        "Discogs token required for native Enya smoke"
    );
    config.llm_api_key = None;
    config.remote_lookup_enabled = Some(true);
    config.discogs_enabled = Some(true);
    config.lyrics_download_enabled = Some(false);

    let temp_root = PathBuf::from("/private/tmp").join(format!(
        "soundrobe-enya-native-smoke-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&temp_root).expect("create temporary smoke root");
    let cache_path = temp_root.join("cache.db");
    let cache = CacheState::new(temp_root.clone());
    assert!(cache.initialize(Some(cache_path.to_str().unwrap())));
    let providers = ProviderState::new();
    let queue = WriteQueue::default();
    let aliases = temp_root.join("aliases.json");
    let cold_path = artifact_dir.join("cold.jsonl");
    let warm_path = artifact_dir.join("warm.jsonl");
    let _ = fs::remove_file(&cold_path);
    let _ = fs::remove_file(&warm_path);
    let mut folder_results = BTreeMap::<String, Value>::new();

    // Finish the complete cold phase before starting warm runs. This keeps
    // every cold invocation on the empty isolated cache while warm runs reuse
    // the cache populated by the full cold phase.
    for phase in ["cold", "warm"] {
        for (index, (relative, source)) in folders.iter().enumerate() {
            let destination = temp_root
                .join("media")
                .join(format!("{index:02}"))
                .join(phase)
                .join("Enya")
                .join(source.file_name().expect("source folder name"));
            copy_tree(source, &destination).expect("copy complete album folder");
            let copied_before_clear = hash_tree(&destination, false).expect("hash copied album");
            let source_before = source_hashes_before.get(relative).unwrap();
            assert_eq!(&copied_before_clear, source_before, "copy differs from source");
            let audio_files = collect_audio_files(&destination);
            let clear_patch = TrackPatch {
                musicbrainz_album_id: Patch::Null,
                discogs_release_id: Patch::Null,
                ..TrackPatch::default()
            };
            for file in &audio_files {
                write_track_queued(&queue, PathBuf::from(file), clear_patch.clone())
                    .await
                    .expect("clear release IDs on copy");
            }
            let preflight = build_lookup_request(&destination).expect("build preflight request");
            assert!(preflight.musicbrainz_album_id.is_none());
            assert!(preflight.discogs_release_id.is_none());
            let baseline = hash_tree(&destination, true).expect("hash post-clear baseline");
            let cache_before = cache_counts(&cache_path);
            let cancelled = Arc::new(AtomicBool::new(false));
            let report_events = std::cell::RefCell::new(Vec::<Value>::new());
            let run = tokio::time::timeout(
                Duration::from_secs(600),
                resolve_and_apply_album_with_retry_context(
                    &destination,
                    &config,
                    AutoTagServices {
                        providers: &providers,
                        cache: &cache,
                        queue: &queue,
                        alias_file: &aliases,
                    },
                    &cancelled,
                    Some(AutoTagRetryContexts::new(Arc::clone(&cancelled))),
                    |progress, message| {
                        report_events.borrow_mut().push(
                            json!({"kind":"progress", "progress":progress, "message":message}),
                        );
                    },
                    |kind, message, data| {
                        report_events.borrow_mut().push(json!({
                            "kind":kind,
                            "message":message,
                            "data":data
                        }));
                    },
                ),
            )
            .await;
            let report_events = report_events.into_inner();
            let timed_out = run.is_err();
            let (result, resolver_error) = match run {
                Ok(Ok(value)) => (Some(value), None),
                Ok(Err(error)) => (None, Some(sanitize_error(error.to_string()))),
                Err(_) => (None, Some("per-folder timeout after 600 seconds".into())),
            };
            let cache_after = cache_counts(&cache_path);
            let post_hashes = hash_tree(&destination, true).expect("hash post-run copy");
            let audio_payloads_equal = baseline.audio_payloads == post_hashes.audio_payloads;
            let mut readback_record = ReadbackRecord::default();
            let mut evidence = Value::Null;
            let mut selected = Value::Null;
            let mut credibility = false;
            let mut write_count_ok = false;
            if let Some(value) = &result {
                if let Some(candidate) = &value.candidate {
                    selected = selected_identity(candidate);
                    evidence = evidence_counts(&preflight, candidate);
                    credibility = provider_candidate_credibility(&preflight, candidate).is_ok();
                    write_count_ok = value.written == audio_files.len();
                    if value.outcome == AutoTagOutcome::Applied {
                        readback_record = readback(&destination, &preflight, candidate);
                    }
                }
            }
            let invariants_ok = result.as_ref().is_some_and(|value| {
                value.outcome == AutoTagOutcome::Applied
                    && value.authority.is_some_and(|source| source != LookupSource::Llm)
                    && credibility
                    && write_count_ok
                    && readback_record.exact
                    && readback_record.collaborator_preserved
                    && audio_payloads_equal
            });
            let incomplete = provider_incomplete(result.as_ref(), resolver_error.as_deref(), timed_out);
            let no_write_review_ok = result.as_ref().is_some_and(|value| {
                value.outcome == AutoTagOutcome::NeedsReview
                    && value.written == 0
                    && audio_payloads_equal
                    && post_hashes.files == baseline.files
            });
            let classification = classify_invocation(
                result.as_ref(),
                resolver_error.is_some() && !timed_out,
                timed_out,
                incomplete,
                invariants_ok || no_write_review_ok,
            );
            let record = sanitize(json!({
                "folder": relative,
                "phase": phase,
                "sourceTrackCount": audio_files.len(),
                "preflightReleaseIdsAbsent": preflight.musicbrainz_album_id.is_none() && preflight.discogs_release_id.is_none(),
                "cacheCountsBefore": cache_before,
                "cacheCountsAfter": cache_after,
                "result": result,
                "resolverError": resolver_error,
                "timedOut": timed_out,
                "reportEvents": report_events,
                "providerAttempts": result.as_ref().map(|value| &value.provider_attempts),
                "diagnostics": result.as_ref().map(|value| &value.diagnostics),
                "selectedIdentity": selected,
                "selectedMatchEvidence": evidence,
                "readback": readback_record,
                "written": result.as_ref().map(|value| value.written),
                "audioPayloadHashesEqual": audio_payloads_equal,
                "copyFullHashesEqualToBaseline": post_hashes.files == baseline.files,
                "classification": classification.as_str(),
            }));
            append_jsonl(
                if phase == "cold" { &cold_path } else { &warm_path },
                &record,
            )
            .expect("append native invocation record");
            folder_results
                .entry(relative.clone())
                .or_insert_with(|| json!({"path": relative}))
                .as_object_mut()
                .expect("folder result object")
                .insert(phase.to_string(), record);
            let mut log = OpenOptions::new()
                .append(true)
                .open(&command_log)
                .unwrap();
            writeln!(
                log,
                "folder={} phase={} classification={}",
                relative,
                phase,
                classification.as_str()
            )
            .unwrap();
        }
    }

    for result in folder_results.values_mut() {
        let cold = result.get("cold").cloned().unwrap_or(Value::Null);
        let warm = result.get("warm").cloned().unwrap_or(Value::Null);
        let cold_id = cold.get("selectedIdentity").cloned().unwrap_or(Value::Null);
        let warm_id = warm.get("selectedIdentity").cloned().unwrap_or(Value::Null);
        let identity_consistent =
            cold_id == Value::Null || warm_id == Value::Null || cold_id == warm_id;
        let cold_class = cold
            .get("classification")
            .and_then(Value::as_str)
            .unwrap_or("failed_verification");
        let warm_class = warm
            .get("classification")
            .and_then(Value::as_str)
            .unwrap_or("failed_verification");
        let parse_class = |value: &str| match value {
            "confirmed_success" => FolderClassification::ConfirmedSuccess,
            "incomplete" => FolderClassification::Incomplete,
            "needs_review" => FolderClassification::NeedsReview,
            _ => FolderClassification::FailedVerification,
        };
        let final_class = reconcile_folder_classification(
            parse_class(cold_class),
            parse_class(warm_class),
            identity_consistent,
        );
        let object = result.as_object_mut().expect("folder result object");
        object.insert(
            "classification".into(),
            Value::String(final_class.as_str().into()),
        );
        object.insert("identityConsistent".into(), Value::Bool(identity_consistent));
        object.insert(
            "warmRecoveryAfterColdIncomplete".into(),
            Value::Bool(
                cold_class == FolderClassification::Incomplete.as_str()
                    && warm_class == FolderClassification::ConfirmedSuccess.as_str(),
            ),
        );
    }

    let source_hashes_after = folders
        .iter()
        .map(|(relative, path)| (relative.clone(), hash_tree(path, false).expect("hash source after")))
        .collect::<BTreeMap<_, _>>();
    fs::write(
        artifact_dir.join("source-hashes-after.json"),
        serde_json::to_vec_pretty(&source_hashes_after).unwrap(),
    )
    .expect("write source hashes after");
    assert_eq!(source_hashes_before, source_hashes_after, "Enya source changed");

    let logical_rows = manifest
        .logical_results
        .iter()
        .map(|row| {
            let folder = row.path.to_string_lossy().to_string();
            json!({
                "id": row.id,
                "outcome": row.outcome,
                "path": row.path,
                "detail": row.detail,
                "retryCount": row.retry_count,
                "attempts": row.attempts,
                "folderClassification": folder_results.get(&folder).and_then(|value| value.get("classification")),
                "mixedPrior": manifest.logical_results.iter().filter(|other| other.path == row.path).count() > 1,
            })
        })
        .collect::<Vec<_>>();
    let results = sanitize(json!({
        "summary": {
            "logicalRows": manifest.logical_results.len(),
            "scopedFolders": folder_results.len(),
            "uniqueReviewFolders": folder_results.len(),
            "invocations": folder_results.len() * 2,
        },
        "folders": folder_results,
        "logicalRows": logical_rows,
    }));
    fs::write(
        artifact_dir.join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .expect("write reconciled results");
    report_markdown(&artifact_dir, &manifest, &folder_results).expect("write report");
    let mut log = OpenOptions::new().append(true).open(&command_log).unwrap();
    writeln!(
        log,
        "completed folders={} invocations={} sourceHashesUnchanged=true",
        folder_results.len(),
        folder_results.len() * 2
    )
    .unwrap();
    let _ = fs::remove_dir_all(temp_root);
}
