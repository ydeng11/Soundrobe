//! Metadata-only auto-tag evaluation corpus and native replay controls.
//!
//! The corpus is intentionally separate from the production resolver.  It
//! records the observed library inputs and reviewed answers without embedding
//! audio or provider credentials.  Deterministic tests exercise profile
//! separation and the same credibility/ranking functions used by production;
//! the ignored native test is an opt-in synthetic-media gate.

use super::*;
use crate::commands::mutations::{write_track_queued, Patch, StringList, TrackPatch};
use crate::commands::tracks::TrackData;
use crate::state::config::{load_from, ProcessEnv};
use crate::state::sqlite::CacheState;
use crate::state::write_queue::WriteQueue;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const CORPUS_RELATIVE: &str = "../test/fixtures/tauri/auto-tag-eval/corpus.json";
const EXPECTATIONS_RELATIVE: &str = "../test/fixtures/tauri/auto-tag-eval/expectations.json";
const CANDIDATE_POOLS_RELATIVE: &str = "../test/fixtures/tauri/auto-tag-eval/candidate-pools.json";
const PROFILE_DISCOVERY: &str = "folder_filename";
const PROFILE_ASSISTED: &str = "assisted_without_ids";
const PROFILE_RECOVERY: &str = "tagged_recovery";
const PER_FOLDER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const RUN_TIMEOUT: Duration = Duration::from_secs(8 * 60 * 60);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvalCorpus {
    schema_version: u32,
    corpus_version: String,
    source_root: PathBuf,
    artists: Vec<String>,
    summary: EvalSummary,
    artist_review: HashMap<String, ArtistReview>,
    profiles: HashMap<String, ProfileSpec>,
    provider_snapshots: Vec<Value>,
    cases: Vec<EvalCase>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvalSummary {
    physical_folders: usize,
    tracks: usize,
    cases: usize,
    verified_matchable: usize,
    verified_abstain: usize,
    unverified: usize,
    excluded: usize,
    provisional_gold_artists: usize,
    diagnostic_unverified_artists: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtistReview {
    status: String,
    provenance: String,
    reviewer: Option<String>,
    reviewed_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileSpec {
    purpose: String,
    strips: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvalCase {
    case_id: String,
    artist: String,
    source_relative_folder: String,
    physical_folders: Vec<String>,
    release_group_id: String,
    edition_family: String,
    release_type: String,
    difficulty: String,
    tracks: Vec<Value>,
    input_profiles: HashMap<String, ProfileInput>,
    expectation: Expectation,
    provider_snapshots: Vec<Value>,
    provider_snapshot_status: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileInput {
    tracks: Vec<Value>,
    provider_ids: String,
    tags: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expectation {
    status: String,
    acceptable_edition_ids: Vec<String>,
    rejected_hard_negative_ids: Vec<String>,
    mapping: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectationsFile {
    schema_version: u32,
    corpus_version: String,
    cases: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvalOutcome {
    ConfirmedSuccess,
    WrongMatch,
    SafeAbstention,
    Unresolved,
    Incomplete,
    FailedVerification,
}

impl EvalOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmedSuccess => "confirmed_success",
            Self::WrongMatch => "wrong_match",
            Self::SafeAbstention => "safe_abstention",
            Self::Unresolved => "unresolved",
            Self::Incomplete => "incomplete",
            Self::FailedVerification => "failed_verification",
        }
    }
}

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CORPUS_RELATIVE)
}

fn load_corpus() -> EvalCorpus {
    let path = corpus_path();
    let text = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "read auto-tag evaluation corpus {}: {error}",
            path.display()
        )
    });
    serde_json::from_str(&text).expect("auto-tag evaluation corpus JSON")
}

fn load_expectations() -> ExpectationsFile {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXPECTATIONS_RELATIVE);
    serde_json::from_str(
        &fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("read evaluation expectations {}: {error}", path.display())
        }),
    )
    .expect("evaluation expectations JSON")
}

fn overlay_reviewed_expectations(corpus: &mut EvalCorpus, path: &Path) {
    if !path.exists() {
        return;
    }
    let file: ExpectationsFile = serde_json::from_str(
        &fs::read_to_string(path).unwrap_or_else(|error| {
            panic!("read evaluation expectations {}: {error}", path.display())
        }),
    )
    .unwrap_or_else(|error| panic!("parse evaluation expectations {}: {error}", path.display()));
    assert_eq!(file.schema_version, corpus.schema_version);
    assert_eq!(file.corpus_version, corpus.corpus_version);
    let reviewed = file
        .cases
        .into_iter()
        .filter_map(|value| {
            let case_id = value.get("caseId")?.as_str()?.to_string();
            Some((case_id, value))
        })
        .collect::<HashMap<_, _>>();
    for case in &mut corpus.cases {
        let Some(value) = reviewed.get(&case.case_id) else {
            continue;
        };
        let status = value.get("status").and_then(Value::as_str).unwrap_or("");
        if !matches!(status, "verified_match" | "verified_abstain") {
            continue;
        }
        case.expectation = serde_json::from_value(value.clone()).unwrap_or_else(|error| {
            panic!("parse reviewed expectation for {}: {error}", case.case_id)
        });
    }
}

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn discovery_profile(case: &EvalCase, profile: &str) -> Vec<Value> {
    case.input_profiles
        .get(profile)
        .unwrap_or_else(|| panic!("case {} missing profile {profile}", case.case_id))
        .tracks
        .clone()
}

fn provider_ids(track: &Value) -> &Value {
    track.get("providerIds").unwrap_or(&Value::Null)
}

fn has_tag_evidence(track: &Value) -> bool {
    ["title", "artist", "album", "albumArtist", "year"]
        .iter()
        .any(|field| track.get(*field).is_some_and(|value| !value.is_null()))
}

fn stable_partition(release_group_id: &str) -> &'static str {
    let digest = Sha256::digest(release_group_id.as_bytes());
    if digest[0] < 204 {
        "development"
    } else {
        "holdout"
    }
}

fn provider_candidate_id(candidate: &AlbumCandidate) -> String {
    candidate
        .discogs_release_id
        .as_deref()
        .or(candidate.musicbrainz_album_id.as_deref())
        .unwrap_or("")
        .to_string()
}

fn provider_position_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
}

fn candidate_position_matches(track: &TrackCandidate, expected: &str) -> bool {
    let Some(track_number) = track.track_number else {
        return false;
    };
    if expected == track_number.to_string() {
        return true;
    }
    track.disc_number.is_some_and(|disc_number| {
        expected == format!("{disc_number}-{track_number}")
    })
}

fn normalized_mapping_title(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn mapping_provider_title_matches(track: &TrackCandidate, row: &Value) -> bool {
    let Some(expected) = row.get("providerTitle").and_then(Value::as_str) else {
        return false;
    };
    track.title.as_deref().is_some_and(|actual| {
        normalized_mapping_title(actual) == normalized_mapping_title(expected)
    })
}

fn provider_position_component(value: &str) -> Option<u64> {
    value.rsplit('-').next()?.parse().ok()
}

fn reviewed_mapping_matches(candidate: &AlbumCandidate, mapping: &[Value]) -> bool {
    if mapping.len() != candidate.tracks.len() {
        return false;
    }
    let provider_positions = mapping
        .iter()
        .filter_map(|row| row.get("providerTrack").and_then(provider_position_value))
        .collect::<Vec<_>>();
    let has_duplicate_provider_position = provider_positions.len()
        != provider_positions
            .iter()
            .collect::<BTreeSet<_>>()
            .len();
    let flattened_disc_positions = has_duplicate_provider_position
        && provider_positions
            .iter()
            .filter_map(|value| provider_position_component(value))
            .zip(provider_positions.iter().skip(1))
            .all(|(previous, current)| {
                provider_position_component(current) == Some(previous + 1)
                    || provider_position_component(current) == Some(1)
            });
    let mut local_tracks = BTreeSet::new();
    mapping.iter().all(|row| {
        let Some(local_track) = row
            .get("localTrack")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
        else {
            return false;
        };
        let Some(provider_track) = row
            .get("providerTrack")
            .and_then(provider_position_value)
        else {
            return false;
        };
        local_track > 0
            && local_tracks.insert(local_track)
            && candidate
                .tracks
                .get(local_track - 1)
                .is_some_and(|track| {
                    candidate_position_matches(track, &provider_track)
                        || (flattened_disc_positions
                            && track.disc_number.is_none()
                            && mapping_provider_title_matches(track, row))
                })
    }) && local_tracks.len() == candidate.tracks.len()
}

fn provider_ids_cleared(request: &LookupRequest) -> bool {
    request.musicbrainz_album_id.is_none()
        && request.musicbrainz_artist_id.is_none()
        && request.discogs_release_id.is_none()
        && request.discogs_artist_id.is_none()
        && request
            .tracks
            .iter()
            .all(|track| track.musicbrainz_track_id.is_none())
}

fn candidate_track_count_matches(candidate: &AlbumCandidate, detail_count: usize) -> bool {
    candidate.tracks.len() == detail_count
}

fn select_deterministically(
    request: &LookupRequest,
    mut candidates: Vec<AlbumCandidate>,
) -> Option<AlbumCandidate> {
    // Production selection is deterministic, but sorting here makes the
    // evaluation's frozen candidate pool independent of fixture order too.
    candidates.sort_by_key(provider_candidate_id);
    select_credible_provider_candidate(request, candidates)
}

fn candidate_is_expected(case: &EvalCase, candidate: &AlbumCandidate) -> EvalOutcome {
    let id = provider_candidate_id(candidate);
    if case.expectation.status == "verified_abstain" {
        // A verified abstention is correct only when no candidate is applied.
        // If the resolver writes one, it accepted an unblessed release.
        EvalOutcome::WrongMatch
    } else if case.expectation.status == "verified_match"
        && case
            .expectation
            .acceptable_edition_ids
            .iter()
            .any(|value| value == &id)
        && reviewed_mapping_matches(candidate, &case.expectation.mapping)
    {
        EvalOutcome::ConfirmedSuccess
    } else if case
        .expectation
        .rejected_hard_negative_ids
        .iter()
        .any(|value| value == &id)
    {
        EvalOutcome::WrongMatch
    } else {
        EvalOutcome::Unresolved
    }
}

fn native_classification(
    case: &EvalCase,
    result: Option<&AutoTagRunResult>,
    resolver_error: bool,
    timed_out: bool,
    payload_unchanged: bool,
) -> EvalOutcome {
    if resolver_error || !payload_unchanged {
        return EvalOutcome::FailedVerification;
    }
    if timed_out {
        return EvalOutcome::Incomplete;
    }
    let Some(result) = result else {
        return EvalOutcome::Incomplete;
    };
    if result.authority == Some(LookupSource::Llm) {
        return EvalOutcome::FailedVerification;
    }
    if result
        .provider_attempts
        .iter()
        .any(|attempt| attempt.status == ProviderAttemptStatus::Unavailable)
    {
        return EvalOutcome::Incomplete;
    }
    match result.outcome {
        AutoTagOutcome::Applied => result
            .candidate
            .as_ref()
            .map(|candidate| candidate_is_expected(case, candidate))
            .unwrap_or(EvalOutcome::FailedVerification),
        AutoTagOutcome::NeedsReview => {
            if case.expectation.status == "verified_abstain" {
                EvalOutcome::SafeAbstention
            } else {
                EvalOutcome::Unresolved
            }
        }
    }
}

fn cache_counts(path: &Path) -> BTreeMap<String, u64> {
    let connection = rusqlite::Connection::open(path).expect("open evaluation cache");
    [
        "lookup_cache",
        "artist_release_cache",
        "release_detail_cache",
    ]
    .into_iter()
    .map(|table| {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        (table.to_string(), count.max(0) as u64)
    })
    .collect()
}

fn digest_reader<R: Read>(mut reader: R) -> String {
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer).expect("read media for SHA-256");
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    format!("{:x}", digest.finalize())
}

fn external_sha256(path: &Path) -> Option<String> {
    for (program, args) in [("shasum", vec!["-a", "256"]), ("sha256sum", Vec::new())] {
        let Ok(output) = Command::new(program).args(args).arg(path).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let digest = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .map(str::to_string);
        if digest.as_ref().is_some_and(|value| value.len() == 64) {
            return digest;
        }
    }
    None
}

fn hash_file(path: &Path) -> String {
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() >= 1024 * 1024) {
        if let Some(digest) = external_sha256(path) {
            return digest;
        }
    }
    let file =
        fs::File::open(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    digest_reader(file)
}

fn payload_hash(path: &Path) -> String {
    let mut file =
        fs::File::open(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut magic = [0_u8; 4];
    if file.read_exact(&mut magic).is_err() || &magic != b"fLaC" {
        file.seek(SeekFrom::Start(0))
            .expect("rewind media for SHA-256");
        return digest_reader(file);
    }
    loop {
        let mut header = [0_u8; 4];
        if file.read_exact(&mut header).is_err() {
            file.seek(SeekFrom::Start(0))
                .expect("rewind malformed media");
            return digest_reader(file);
        }
        let last = header[0] & 0x80 != 0;
        let size = ((header[1] as u64) << 16) | ((header[2] as u64) << 8) | header[3] as u64;
        if file.seek(SeekFrom::Current(size as i64)).is_err() {
            file.seek(SeekFrom::Start(0))
                .expect("rewind malformed media");
            return digest_reader(file);
        }
        if last {
            return digest_reader(file);
        }
    }
}

fn relative_audio_key(album: &Path, path: &Path) -> String {
    path.strip_prefix(album)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn hash_map(album: &Path) -> BTreeMap<String, Value> {
    collect_audio_files(album)
        .into_iter()
        .map(|path| {
            let path_ref = Path::new(&path);
            let full_sha256 = hash_file(path_ref);
            let payload_sha256 = payload_hash(path_ref);
            let bytes = fs::metadata(path_ref)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            (
                relative_audio_key(album, path_ref),
                json!({
                    "fullSha256": full_sha256,
                    "payloadSha256": payload_sha256,
                    "bytes": bytes,
                }),
            )
        })
        .collect()
}

fn full_hash_map(album: &Path) -> BTreeMap<String, Value> {
    collect_audio_files(album)
        .into_iter()
        .map(|path| {
            let path_ref = Path::new(&path);
            let full_sha256 = hash_file(path_ref);
            let bytes = fs::metadata(path_ref)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            (
                relative_audio_key(album, path_ref),
                json!({"fullSha256": full_sha256, "bytes": bytes}),
            )
        })
        .collect()
}

fn safe_relative_folder(relative: &str) -> PathBuf {
    let path = PathBuf::from(relative);
    assert!(
        !path.is_absolute(),
        "evaluation folder path must be relative"
    );
    assert!(
        path.components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "evaluation folder path contains traversal: {relative}"
    );
    path
}

fn readback_matches(
    album: &Path,
    result: Option<&AutoTagRunResult>,
    expectation: &Expectation,
) -> bool {
    let Some(result) = result else {
        return false;
    };
    let Some(candidate) = result.candidate.as_ref() else {
        return false;
    };
    let Ok(detail) = crate::commands::tracks::read_album(album) else {
        return false;
    };
    if result.outcome != AutoTagOutcome::Applied
        || result.written != detail.tracks.len()
        || !candidate_track_count_matches(candidate, detail.tracks.len())
        || (expectation.status == "verified_match"
            && !reviewed_mapping_matches(candidate, &expectation.mapping))
    {
        return false;
    }
    let fallback_artist = album
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let expected_album_artist = if candidate.album_artists.is_empty() {
        Some(fallback_artist.to_string())
    } else {
        Some(candidate.album_artists.join(" & "))
    };
    detail
        .tracks
        .iter()
        .zip(&candidate.tracks)
        .all(|(written, expected)| {
            expected
                .title
                .as_deref()
                .is_none_or(|value| written.title.as_deref() == Some(value))
                && expected
                    .artist
                    .as_deref()
                    .is_none_or(|value| written.artist.as_deref() == Some(value))
                && (expected.artists.is_empty() || written.artists == expected.artists)
                && candidate
                    .album
                    .as_deref()
                    .is_none_or(|value| written.album.as_deref() == Some(value))
                && expected_album_artist
                    .as_deref()
                    .is_none_or(|value| written.album_artist.as_deref() == Some(value))
                && (candidate.album_artists.is_empty()
                    || written.album_artists == candidate.album_artists)
                && candidate
                    .year
                    .as_deref()
                    .is_none_or(|value| written.year.as_deref() == Some(value))
                && expected
                    .track_number
                    .is_none_or(|value| written.track_number == Some(value))
                && expected
                    .disc_number
                    .is_none_or(|value| written.disc_number == Some(value))
                && expected
                    .musicbrainz_track_id
                    .as_deref()
                    .is_none_or(|value| written.musicbrainz_track_id.as_deref() == Some(value))
                && candidate
                    .musicbrainz_album_id
                    .as_deref()
                    .is_none_or(|value| written.musicbrainz_album_id.as_deref() == Some(value))
                && candidate
                    .musicbrainz_artist_id
                    .as_deref()
                    .is_none_or(|value| written.musicbrainz_artist_id.as_deref() == Some(value))
                && candidate
                    .discogs_artist_id
                    .as_deref()
                    .is_none_or(|value| written.discogs_artist_id.as_deref() == Some(value))
                && candidate
                    .discogs_release_id
                    .as_deref()
                    .is_none_or(|value| written.discogs_release_id.as_deref() == Some(value))
        })
}

fn selected_track_evidence(result: Option<&AutoTagRunResult>) -> Option<Value> {
    result?.diagnostics.iter().find_map(|diagnostic| {
        (diagnostic.get("stage").and_then(Value::as_str) == Some("selected_track_evidence"))
            .then(|| diagnostic.clone())
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct SyntheticMaterialization {
    track_count: usize,
    bytes: u64,
    peak_bytes: u64,
}

fn synthetic_flac_path(destination: &Path, source_path: &Path, ordinal: usize) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("track");
    let mut name = stem.to_string();
    if ordinal > 0 {
        // Zero-width suffixes keep filename normalization identical while
        // allowing duplicate stems such as Lose Yourself.flac/.wav to coexist.
        name.push_str(&"\u{200b}".repeat(ordinal));
    }
    destination
        .join(source_path.parent().unwrap_or_else(|| Path::new("")))
        .join(format!("{name}.flac"))
}

fn patch_from_track_data(track: &TrackData) -> TrackPatch {
    let mut patch = TrackPatch::default();
    if let Some(value) = track.title.clone() {
        patch.title = Patch::Value(value);
    }
    if let Some(value) = track.artist.clone() {
        patch.artist = Patch::Value(value);
    }
    if !track.artists.is_empty() {
        patch.artists = Patch::Value(StringList::Many(track.artists.clone()));
    }
    if let Some(value) = track.album.clone() {
        patch.album = Patch::Value(value);
    }
    if let Some(value) = track.album_artist.clone() {
        patch.album_artist = Patch::Value(value);
    }
    if !track.album_artists.is_empty() {
        patch.album_artists = Patch::Value(StringList::Many(track.album_artists.clone()));
    }
    if let Some(value) = track.year.clone() {
        patch.year = Patch::Value(value);
    }
    if let Some(value) = track.track_number {
        patch.track_number = Patch::Value(value);
    }
    if let Some(value) = track.track_total {
        patch.track_total = Patch::Value(value);
    }
    if let Some(value) = track.disc_number {
        patch.disc_number = Patch::Value(value);
    }
    if let Some(value) = track.disc_total {
        patch.disc_total = Patch::Value(value);
    }
    if let Some(value) = track.genre.clone() {
        patch.genre = Patch::Value(value);
    }
    if let Some(value) = track.composer.clone() {
        patch.composer = Patch::Value(value);
    }
    if let Some(value) = track.comment.clone() {
        patch.comment = Patch::Value(value);
    }
    if let Some(value) = track.description.clone() {
        patch.description = Patch::Value(value);
    }
    if let Some(value) = track.compilation {
        patch.compilation = Patch::Value(value);
    }
    if let Some(value) = track.musicbrainz_track_id.clone() {
        patch.musicbrainz_track_id = Patch::Value(value);
    }
    if let Some(value) = track.musicbrainz_album_id.clone() {
        patch.musicbrainz_album_id = Patch::Value(value);
    }
    if let Some(value) = track.musicbrainz_artist_id.clone() {
        patch.musicbrainz_artist_id = Patch::Value(value);
    }
    if let Some(value) = track.discogs_artist_id.clone() {
        patch.discogs_artist_id = Patch::Value(value);
    }
    if let Some(value) = track.discogs_release_id.clone() {
        patch.discogs_release_id = Patch::Value(value);
    }
    patch
}

fn generate_synthetic_flac(path: &Path, duration: f64) -> Result<f64, ApiError> {
    let requested = if duration.is_finite() && duration > 0.0 {
        duration
    } else {
        0.0
    };
    let argument = format!("{requested:.6}");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=44100:cl=stereo",
            "-t",
            &argument,
        ])
        .arg(path)
        .status()
        .map_err(|error| {
            ApiError::Message(format!("ffmpeg unavailable for synthetic FLAC: {error}"))
        })?;
    if !status.success() {
        return Err(ApiError::Message(format!(
            "ffmpeg failed to generate synthetic FLAC {}",
            path.display()
        )));
    }
    let actual = crate::commands::tracks::read_track_metadata(path)?.duration;
    let tolerance = if requested > 0.0 {
        0.1_f64.max(requested * 0.001)
    } else {
        0.1
    };
    if (actual - requested).abs() > tolerance {
        return Err(ApiError::Message(format!(
            "synthetic FLAC duration drift: requested {requested:.6}, reader returned {actual:.6}"
        )));
    }
    Ok(actual)
}

async fn materialize_synthetic_case(
    source: &Path,
    destination: &Path,
    queue: &WriteQueue,
) -> Result<SyntheticMaterialization, ApiError> {
    let detail = crate::commands::tracks::read_album(source)?;
    fs::create_dir_all(destination)?;
    let mut materialized = SyntheticMaterialization::default();
    let mut stems = HashMap::<String, usize>::new();
    for track in detail.tracks {
        let source_path = PathBuf::from(&track.path);
        let relative = source_path.strip_prefix(source).map_err(|_| {
            ApiError::Message(format!(
                "track escaped synthetic source: {}",
                source_path.display()
            ))
        })?;
        let stem_key = relative
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("track")
            .to_string();
        let ordinal = stems.entry(stem_key).or_insert(0);
        let output = synthetic_flac_path(destination, relative, *ordinal);
        *ordinal += 1;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        generate_synthetic_flac(&output, track.duration)?;
        write_track_queued(queue, output.clone(), patch_from_track_data(&track)).await?;
        let bytes = fs::metadata(&output)?.len();
        materialized.track_count += 1;
        materialized.bytes += bytes;
        materialized.peak_bytes = materialized.peak_bytes.max(materialized.bytes);
    }
    Ok(materialized)
}

fn assert_lookup_requests_equivalent(real: &LookupRequest, synthetic: &LookupRequest) {
    let mut left = real.clone();
    let mut right = synthetic.clone();
    left.path = "<synthetic-equivalence>".into();
    right.path = "<synthetic-equivalence>".into();
    assert_eq!(left.tracks.len(), right.tracks.len());
    for (real_track, synthetic_track) in left.tracks.iter_mut().zip(&mut right.tracks) {
        let real_duration = real_track.length;
        let synthetic_duration = synthetic_track.length;
        real_track.length = None;
        synthetic_track.length = None;
        synthetic_track.filename = synthetic_track
            .filename
            .take()
            .map(|value| value.replace('\u{200b}', ""));
        assert_eq!(real_track, synthetic_track);
        match (real_duration, synthetic_duration) {
            (Some(real), Some(synthetic)) => assert!(
                (real - synthetic).abs() <= 0.1_f64.max(real.abs() * 0.001),
                "duration changed from {real:.6} to {synthetic:.6}"
            ),
            (real, synthetic) => assert_eq!(real.is_some(), synthetic.is_some()),
        }
    }
    left.tracks.clear();
    right.tracks.clear();
    assert_eq!(left, right);
}

async fn clear_provider_ids(album: &Path, queue: &WriteQueue) -> Result<(), ApiError> {
    for path in collect_audio_files(album) {
        write_track_queued(
            queue,
            PathBuf::from(path),
            TrackPatch {
                musicbrainz_track_id: Patch::Null,
                musicbrainz_album_id: Patch::Null,
                musicbrainz_artist_id: Patch::Null,
                discogs_artist_id: Patch::Null,
                discogs_release_id: Patch::Null,
                ..TrackPatch::default()
            },
        )
        .await?;
    }
    Ok(())
}

async fn apply_profile(album: &Path, profile: &str, queue: &WriteQueue) -> Result<(), ApiError> {
    if profile == PROFILE_RECOVERY {
        return Ok(());
    }
    for path in collect_audio_files(album) {
        let patch = if profile == PROFILE_DISCOVERY {
            TrackPatch {
                title: Patch::Null,
                artist: Patch::Null,
                artists: Patch::Null,
                album: Patch::Null,
                album_artist: Patch::Null,
                album_artists: Patch::Null,
                year: Patch::Null,
                track_number: Patch::Null,
                track_total: Patch::Null,
                disc_number: Patch::Null,
                disc_total: Patch::Null,
                genre: Patch::Null,
                composer: Patch::Null,
                comment: Patch::Null,
                description: Patch::Null,
                compilation: Patch::Null,
                musicbrainz_track_id: Patch::Null,
                musicbrainz_album_id: Patch::Null,
                musicbrainz_artist_id: Patch::Null,
                discogs_artist_id: Patch::Null,
                discogs_release_id: Patch::Null,
                ..TrackPatch::default()
            }
        } else {
            TrackPatch::default()
        };
        if profile == PROFILE_DISCOVERY {
            write_track_queued(queue, PathBuf::from(&path), patch).await?;
        }
    }
    if profile == PROFILE_ASSISTED {
        clear_provider_ids(album, queue).await?;
    }
    Ok(())
}

fn source_path(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    let canonical_root = fs::canonicalize(root).expect("canonical evaluation source root");
    let canonical = fs::canonicalize(&path)
        .unwrap_or_else(|error| panic!("evaluation source path {}: {error}", path.display()));
    assert!(
        canonical.starts_with(&canonical_root),
        "source path escapes corpus root"
    );
    canonical
}

fn verify_frozen_case(case: &EvalCase, source: &Path, hashes: &BTreeMap<String, Value>) {
    for track in &case.tracks {
        let relative = track
            .get("relativePath")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("case {} has a track without relativePath", case.case_id));
        let path = source.join(relative);
        let expected = track
            .get("sourceSha256")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("case {} has a track without sourceSha256", case.case_id));
        assert!(path.is_file(), "frozen input missing: {}", path.display());
        assert_eq!(
            hashes
                .get(relative_audio_key(source, &path).as_str())
                .and_then(|value| value.get("fullSha256"))
                .and_then(Value::as_str),
            Some(expected),
            "frozen input changed: {}",
            path.display()
        );
    }
}

fn sanitise(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sanitise).collect()),
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
                        || lower.contains("secret")
                        || lower.contains("password")
                        || lower.contains("cookie")
                        || lower == "headers"
                        || lower == "config";
                    (
                        key,
                        if sensitive {
                            json!("[redacted]")
                        } else {
                            sanitise(value)
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
            json!(value
                .replace("Bearer ", "Bearer [redacted]")
                .replace("bearer ", "bearer [redacted]"))
        }
        other => other,
    }
}

fn record_classification(record: Option<&Value>) -> Option<&str> {
    record
        .and_then(|value| value.get("classification"))
        .and_then(Value::as_str)
}

fn record_identity(record: Option<&Value>) -> Option<&str> {
    let candidate = record
        .and_then(|value| value.get("native"))?
        .get("candidate")?;
    [
        "discogs_release_id",
        "discogsReleaseId",
        "musicbrainz_albumid",
        "musicbrainz_album_id",
        "musicbrainzAlbumId",
    ]
    .into_iter()
    .find_map(|key| candidate.get(key).and_then(Value::as_str))
}

fn reconciled_classification(cold: Option<&Value>, warm: Option<&Value>) -> &'static str {
    if record_classification(cold) == Some("failed_verification")
        || record_classification(warm) == Some("failed_verification")
    {
        return "failed_verification";
    }
    if record_identity(cold).is_some()
        && record_identity(warm).is_some()
        && record_identity(cold) != record_identity(warm)
    {
        return "failed_verification";
    }
    if record_classification(warm) == Some("confirmed_success") {
        return "confirmed_success";
    }
    if record_classification(cold) == Some("confirmed_success") && warm.is_none() {
        return "confirmed_success";
    }
    if record_classification(cold) == Some("incomplete")
        || record_classification(warm) == Some("incomplete")
        || cold.is_none()
        || warm.is_none()
    {
        return "incomplete";
    }
    if record_classification(cold) == Some("safe_abstention")
        && record_classification(warm) == Some("safe_abstention")
    {
        return "safe_abstention";
    }
    if record_classification(cold) == Some("wrong_match")
        || record_classification(warm) == Some("wrong_match")
    {
        return "wrong_match";
    }
    "unresolved"
}

fn reconcile_results(records: &[Value], corpus: &EvalCorpus, run_id: &str) -> Value {
    let mut by_case = BTreeMap::<String, BTreeMap<String, &Value>>::new();
    for record in records {
        let Some(case_id) = record.get("caseId").and_then(Value::as_str) else {
            continue;
        };
        let Some(phase) = record.get("phase").and_then(Value::as_str) else {
            continue;
        };
        by_case
            .entry(case_id.to_string())
            .or_default()
            .insert(phase.to_string(), record);
    }
    let mut folders = Vec::new();
    let mut logical_rows = Vec::new();
    for case in &corpus.cases {
        let Some(phases) = by_case.get(&case.case_id) else {
            continue;
        };
        let cold = phases.get("cold").copied();
        let warm = phases.get("warm").copied();
        let cold_identity = record_identity(cold).map(str::to_string);
        let warm_identity = record_identity(warm).map(str::to_string);
        let warm_recovery = record_classification(cold) == Some("incomplete")
            && record_classification(warm) == Some("confirmed_success");
        let final_classification = reconciled_classification(cold, warm);
        let selected_identity = warm_identity.clone().or(cold_identity.clone());
        let folder = json!({
            "caseId": case.case_id,
            "artist": case.artist,
            "sourceRelativeFolder": case.source_relative_folder,
            "physicalFolders": case.physical_folders,
            "releaseGroupId": case.release_group_id,
            "editionFamily": case.edition_family,
            "profile": cold
                .or(warm)
                .and_then(|record| record.get("profile"))
                .cloned()
                .unwrap_or(Value::Null),
            "oracleStatus": case.expectation.status,
            "coldClassification": record_classification(cold),
            "warmClassification": record_classification(warm),
            "classification": final_classification,
            "coldIdentity": cold_identity,
            "warmIdentity": warm_identity,
            "selectedIdentity": selected_identity,
            "warmRecovery": warm_recovery,
        });
        folders.push(folder.clone());
        logical_rows.push(json!({
            "logicalRowId": case.case_id,
            "caseId": case.case_id,
            "artist": case.artist,
            "sourceRelativeFolder": case.source_relative_folder,
            "folderOutcome": final_classification,
            "physicalFolders": case.physical_folders,
            "priorExpectation": case.expectation.status,
        }));
    }
    json!({
        "schemaVersion": 1,
        "runId": run_id,
        "profile": records.first().and_then(|record| record.get("profile")).cloned().unwrap_or(Value::Null),
        "invocationCount": records.len(),
        "folderCount": folders.len(),
        "logicalRowCount": logical_rows.len(),
        "folderResults": folders,
        "logicalRows": logical_rows,
        "invocations": records,
    })
}

fn write_report(path: &Path, records: &[Value], corpus: &EvalCorpus, run_id: &str) {
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut dimensions = BTreeMap::<(String, String, String, String, String), (usize, u128)>::new();
    let mut providers = BTreeMap::<String, usize>::new();
    let mut causes = BTreeMap::<String, BTreeSet<String>>::new();
    let mut identities = BTreeMap::<(String, String), String>::new();
    let mut synthetic_tracks = 0_u64;
    let mut synthetic_bytes = 0_u64;
    let mut synthetic_peak_bytes = 0_u64;
    for record in records {
        if let Some(outcome) = record.get("classification").and_then(Value::as_str) {
            *counts.entry(outcome).or_default() += 1;
        }
        let artist = record
            .get("artist")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let profile = record
            .get("profile")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let release_type = record
            .get("releaseType")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let difficulty = record
            .get("difficulty")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let phase = record
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let elapsed = record.get("elapsedMs").and_then(Value::as_u64).unwrap_or(0) as u128;
        let entry = dimensions
            .entry((artist, profile, release_type, difficulty, phase))
            .or_default();
        entry.0 += 1;
        entry.1 += elapsed;
        if let Some(provider) = record.pointer("/native/authority").and_then(Value::as_str) {
            *providers.entry(provider.to_string()).or_default() += 1;
        }
        synthetic_tracks += record
            .get("syntheticTrackCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        synthetic_bytes += record
            .get("syntheticBytes")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        synthetic_peak_bytes = synthetic_peak_bytes.max(
            record
                .get("syntheticPeakBytes")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
        if let (Some(case_id), Some(phase)) = (
            record.get("caseId").and_then(Value::as_str),
            record.get("phase").and_then(Value::as_str),
        ) {
            if let Some(identity) = record_identity(Some(record)) {
                identities.insert(
                    (case_id.to_string(), phase.to_string()),
                    identity.to_string(),
                );
            }
        }
        let provider_unavailable = record
            .pointer("/native/providerAttempts")
            .and_then(Value::as_array)
            .is_some_and(|attempts| {
                attempts.iter().any(|attempt| {
                    attempt.get("status").and_then(Value::as_str) == Some("unavailable")
                })
            });
        let cause = match (
            record.get("classification").and_then(Value::as_str),
            record.get("outcome").and_then(Value::as_str),
        ) {
            (Some("incomplete"), Some("timeout")) => "timed_out",
            (Some("incomplete"), _) if provider_unavailable => "provider_unavailable",
            (Some("incomplete"), _) => "incomplete",
            (Some("failed_verification"), _) => "failed_verification",
            (_, Some("resolver_error")) => "resolver_error",
            (_, Some("needs_review")) => "no_authoritative_match",
            (_, Some("applied")) => "applied",
            _ => "unknown",
        };
        if let Some(case_id) = record.get("caseId").and_then(Value::as_str) {
            causes
                .entry(cause.to_string())
                .or_default()
                .insert(case_id.to_string());
        }
    }
    let reconciled = reconcile_results(records, corpus, run_id);
    let folder_results = reconciled
        .get("folderResults")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut folder_counts = BTreeMap::<&str, usize>::new();
    for folder in &folder_results {
        if let Some(classification) = folder.get("classification").and_then(Value::as_str) {
            *folder_counts.entry(classification).or_default() += 1;
        }
    }
    let mut report = String::new();
    report.push_str("# Auto-tag evaluation report\n\n");
    report.push_str(&format!(
        "Run `{run_id}` against corpus `{}`.\n\n",
        corpus.corpus_version
    ));
    report.push_str("The corpus is metadata-only; unverified cases are excluded from scored precision and coverage.\n\n");
    if records
        .iter()
        .any(|record| record["providerMode"] == "offline_fixtures")
    {
        report.push_str("Provider mode: offline fixtures. These runs establish frozen-response pipeline behavior, not live provider discovery coverage.\n\n");
    }
    report.push_str(&format!(
        "## Synthetic input media\n\n- Mode: `synthetic_flac` (minimal silent FLAC generated per track; original audio was never copied)\n- Generated tracks: {synthetic_tracks}\n- Generated bytes across invocations: {synthetic_bytes} ({} MiB)\n- Peak per-case synthetic bytes before profile setup: {synthetic_peak_bytes} ({} MiB)\n- Real-media malformed-layout and payload-preservation evidence is retained separately in the Enya audit.\n\n",
        synthetic_bytes / (1024 * 1024),
        synthetic_peak_bytes / (1024 * 1024)
    ));
    report.push_str("## Outcomes\n\n| Outcome | Folders | Invocation records |\n|---|---:|---:|\n");
    for outcome in [
        "confirmed_success",
        "wrong_match",
        "safe_abstention",
        "unresolved",
        "incomplete",
        "failed_verification",
    ] {
        report.push_str(&format!(
            "| {outcome} | {} | {} |\n",
            folder_counts.get(outcome).copied().unwrap_or(0),
            counts.get(outcome).copied().unwrap_or(0),
        ));
    }
    let cold_invocations = records
        .iter()
        .filter(|record| record.get("phase").and_then(Value::as_str) == Some("cold"))
        .count();
    let warm_invocations = records
        .iter()
        .filter(|record| record.get("phase").and_then(Value::as_str) == Some("warm"))
        .count();
    let timeout_count = records
        .iter()
        .filter(|record| record.get("outcome").and_then(Value::as_str) == Some("timeout"))
        .count();
    let resolver_error_count = records
        .iter()
        .filter(|record| record.get("outcome").and_then(Value::as_str) == Some("resolver_error"))
        .count();
    let payload_failure_count = records
        .iter()
        .filter(|record| record.get("payloadUnchanged") == Some(&Value::Bool(false)))
        .count();
    let readback_failure_count = records
        .iter()
        .filter(|record| {
            record.get("outcome").and_then(Value::as_str) == Some("applied")
                && record.get("readback") == Some(&Value::Bool(false))
        })
        .count();
    report.push_str(&format!(
        "\n## Verification gates\n\n- Native invocation records: {} (cold {}, warm {}; expected {})\n- Timeout records: {timeout_count}\n- Resolver-error records: {resolver_error_count}\n- Synthetic payload failures: {payload_failure_count}\n- Applied readback failures: {readback_failure_count}\n- Reconciled failed-verification folders: {}\n\n",
        records.len(),
        cold_invocations,
        warm_invocations,
        cold_invocations.max(warm_invocations) * 2,
        folder_counts.get("failed_verification").copied().unwrap_or(0),
    ));
    report.push_str("\n## Artist/profile/release/cache latency\n\n| Artist | Profile | Release type | Difficulty | Cache phase | Invocations | Mean ms |\n|---|---|---|---|---|---:|---:|\n");
    for ((artist, profile, release_type, difficulty, phase), (count, total_ms)) in dimensions {
        report.push_str(&format!(
            "| {artist} | {profile} | {release_type} | {difficulty} | {phase} | {count} | {} |\n",
            total_ms / count as u128
        ));
    }
    report.push_str("\n## Authoritative providers\n\n");
    if providers.is_empty() {
        report.push_str("No authoritative provider candidate was selected.\n");
    } else {
        for (provider, count) in providers {
            report.push_str(&format!("- {provider}: {count}\n"));
        }
    }
    let enya_records = records
        .iter()
        .filter(|record| record.get("artist").and_then(Value::as_str) == Some("Enya"))
        .collect::<Vec<_>>();
    if !enya_records.is_empty() {
        let enya_cases = enya_records
            .iter()
            .filter_map(|record| record.get("caseId").and_then(Value::as_str))
            .collect::<BTreeSet<_>>();
        let enya_guarded_invocations = enya_records
            .iter()
            .filter(|record| {
                record
                    .pointer("/selectedTrackEvidence/evidence")
                    .and_then(Value::as_array)
                    .is_some_and(|evidence| {
                        evidence
                            .iter()
                            .any(|value| value.as_str() == Some("GuardedTitle"))
                    })
            })
            .count();
        let enya_counts = enya_records.iter().fold(
            BTreeMap::<(&str, &str), usize>::new(),
            |mut counts, record| {
                let phase = record
                    .get("phase")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let classification = record
                    .get("classification")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                *counts.entry((phase, classification)).or_default() += 1;
                counts
            },
        );
        report.push_str(&format!(
            "\n## Enya subset\n\n- Cases: {}\n- Cold classifications: incomplete {}, unresolved {}\n- Warm classifications: incomplete {}, unresolved {}\n- GuardedTitle evidence: {} invocation records\n- Attribution remains unscored because these corpus expectations are unverified.\n\n",
            enya_cases.len(),
            enya_counts.get(&("cold", "incomplete")).copied().unwrap_or(0),
            enya_counts.get(&("cold", "unresolved")).copied().unwrap_or(0),
            enya_counts.get(&("warm", "incomplete")).copied().unwrap_or(0),
            enya_counts.get(&("warm", "unresolved")).copied().unwrap_or(0),
            enya_guarded_invocations,
        ));
    }
    report.push_str(&format!("\nScored verified matchable cases: {}. Provisional/unverified cases remain outside precision and coverage.\n\n", corpus.summary.verified_matchable));
    report.push_str("## Album-by-album results\n\n| Artist | Folder | Cold | Warm | Final | Selected release | Warm recovery |\n|---|---|---|---|---|---|---:|\n");
    for folder in &folder_results {
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            folder.get("artist").and_then(Value::as_str).unwrap_or(""),
            folder
                .get("sourceRelativeFolder")
                .and_then(Value::as_str)
                .unwrap_or(""),
            folder
                .get("coldClassification")
                .and_then(Value::as_str)
                .unwrap_or("missing"),
            folder
                .get("warmClassification")
                .and_then(Value::as_str)
                .unwrap_or("missing"),
            folder
                .get("classification")
                .and_then(Value::as_str)
                .unwrap_or("incomplete"),
            folder
                .get("selectedIdentity")
                .and_then(Value::as_str)
                .unwrap_or(""),
            folder
                .get("warmRecovery")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ));
    }
    let mut confirmed_ids = BTreeSet::new();
    for folder in &folder_results {
        if folder.get("classification").and_then(Value::as_str) == Some("confirmed_success") {
            if let Some(identity) = folder.get("selectedIdentity").and_then(Value::as_str) {
                confirmed_ids.insert(identity.to_string());
            }
        }
    }
    report.push_str("\n## Confirmed release IDs\n\n");
    if confirmed_ids.is_empty() {
        report.push_str("No confirmed release IDs.\n");
    } else {
        for identity in confirmed_ids {
            report.push_str(&format!("- {identity}\n"));
        }
    }
    report.push_str("\n## Failure causes (affected folders)\n\n");
    let mut ranked_causes = causes
        .into_iter()
        .map(|(cause, cases)| (cause, cases.len()))
        .collect::<Vec<_>>();
    ranked_causes.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    for (cause, count) in ranked_causes {
        report.push_str(&format!("- {cause}: {count}\n"));
    }
    let mut identity_inconsistent = 0usize;
    let mut warm_recovery = 0usize;
    let case_ids = identities
        .keys()
        .map(|(case_id, _)| case_id.clone())
        .collect::<BTreeSet<_>>();
    for case_id in case_ids {
        let cold = identities.get(&(case_id.clone(), "cold".into()));
        let warm = identities.get(&(case_id.clone(), "warm".into()));
        if cold.is_some() && warm.is_some() && cold != warm {
            identity_inconsistent += 1;
        }
        let cold_incomplete = records.iter().any(|record| {
            record.get("caseId").and_then(Value::as_str) == Some(case_id.as_str())
                && record.get("phase").and_then(Value::as_str) == Some("cold")
                && record.get("classification").and_then(Value::as_str) == Some("incomplete")
        });
        let warm_confirmed = records.iter().any(|record| {
            record.get("caseId").and_then(Value::as_str) == Some(case_id.as_str())
                && record.get("phase").and_then(Value::as_str) == Some("warm")
                && record.get("classification").and_then(Value::as_str) == Some("confirmed_success")
        });
        if cold_incomplete && warm_confirmed {
            warm_recovery += 1;
        }
    }
    report.push_str(&format!("\nCold/warm identity inconsistencies: {identity_inconsistent}. Warm recovery after cold incomplete: {warm_recovery}.\n"));
    report.push_str("## Attribution\n\nA matcher change is credited only when a reviewed case selects an acceptable edition with the new evidence and the prior replay failed on the same case. Provider recovery and a different release remain separate labels.\n");
    fs::write(path, report).expect("write evaluation report");
}

#[test]
fn corpus_inventory_is_complete_and_profiles_are_separated() {
    let corpus = load_corpus();
    let expectations = load_expectations();
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.summary.physical_folders, 321);
    assert_eq!(corpus.summary.tracks, 2_841);
    assert_eq!(corpus.summary.cases, 321);
    assert_eq!(corpus.summary.verified_matchable, 0);
    assert_eq!(corpus.summary.verified_abstain, 0);
    assert_eq!(corpus.summary.unverified, 321);
    assert_eq!(corpus.summary.excluded, 0);
    assert_eq!(corpus.summary.provisional_gold_artists, 5);
    assert_eq!(corpus.summary.diagnostic_unverified_artists, 2);
    assert_eq!(expectations.schema_version, corpus.schema_version);
    assert_eq!(expectations.corpus_version, corpus.corpus_version);
    assert_eq!(expectations.cases.len(), corpus.cases.len());
    assert_eq!(corpus.cases.len(), corpus.summary.cases);
    assert_eq!(corpus.artists.len(), 7);
    assert_eq!(corpus.provider_snapshots.len(), 2);
    assert_eq!(corpus.artist_review.len(), 7);
    assert_eq!(
        corpus
            .artist_review
            .values()
            .filter(|review| review.status == "provisional_gold_unverified")
            .count(),
        5
    );
    assert_eq!(
        corpus
            .artist_review
            .values()
            .filter(|review| review.status == "diagnostic_unverified")
            .count(),
        2
    );
    assert!(corpus
        .artist_review
        .values()
        .all(|review| !review.provenance.is_empty()
            && review.reviewer.is_none()
            && review.reviewed_date.is_none()));
    assert!(corpus
        .profiles
        .values()
        .all(|profile| !profile.purpose.is_empty()));
    let mut physical = BTreeSet::new();
    for case in &corpus.cases {
        assert_eq!(
            case.physical_folders,
            vec![case.source_relative_folder.clone()]
        );
        assert_eq!(case.expectation.status, "unverified");
        assert_eq!(case.provider_snapshot_status, "not_captured");
        assert!(case.provider_snapshots.is_empty());
        assert_eq!(case.difficulty, "unverified");
        assert!(!case.release_type.is_empty());
        assert!(!case.release_group_id.is_empty());
        assert!(!case.edition_family.is_empty());
        assert!(physical.insert(case.source_relative_folder.clone()));
        let clean = discovery_profile(case, PROFILE_DISCOVERY);
        let assisted = discovery_profile(case, PROFILE_ASSISTED);
        let recovery = discovery_profile(case, PROFILE_RECOVERY);
        assert_eq!(
            case.input_profiles[PROFILE_DISCOVERY].provider_ids,
            "stripped"
        );
        assert_eq!(case.input_profiles[PROFILE_DISCOVERY].tags, "stripped");
        assert_eq!(
            case.input_profiles[PROFILE_ASSISTED].provider_ids,
            "stripped"
        );
        assert_eq!(case.input_profiles[PROFILE_ASSISTED].tags, "captured");
        assert_eq!(
            case.input_profiles[PROFILE_RECOVERY].provider_ids,
            "preserved"
        );
        assert_eq!(clean.len(), case.tracks.len());
        assert_eq!(assisted.len(), case.tracks.len());
        assert_eq!(recovery.len(), case.tracks.len());
        for ((clean, assisted), recovery) in clean.iter().zip(&assisted).zip(&recovery) {
            assert!(
                provider_ids(clean).is_null()
                    || provider_ids(clean)
                        .as_object()
                        .is_some_and(|object| object.is_empty())
            );
            assert!(provider_ids(assisted)
                .as_object()
                .is_some_and(|object| object.is_empty()));
            assert_eq!(provider_ids(recovery), recovery.get("providerIds").unwrap());
            assert!(!has_tag_evidence(clean));
            assert!(has_tag_evidence(assisted) || !has_tag_evidence(recovery));
        }
    }
    assert_eq!(physical.len(), 321);
    assert!(corpus.profiles[PROFILE_DISCOVERY]
        .strips
        .iter()
        .any(|v| v == "tags"));
    assert!(corpus.profiles[PROFILE_ASSISTED]
        .strips
        .iter()
        .any(|v| v == "providerIds"));
    assert!(corpus.profiles[PROFILE_RECOVERY].strips.is_empty());
}

#[test]
fn copied_audio_hashes_use_relative_keys() {
    let source = Path::new("/private/tmp/source/Artist/Album");
    let copied = Path::new("/private/tmp/copy/Artist/Album");
    let source_track = source.join("Disc 1/01.flac");
    let copied_track = copied.join("Disc 1/01.flac");
    assert_eq!(relative_audio_key(source, &source_track), "Disc 1/01.flac");
    assert_eq!(
        relative_audio_key(copied, &copied_track),
        relative_audio_key(source, &source_track)
    );
    let mut expected = BTreeMap::new();
    expected.insert("Disc 1/01.flac".to_string(), json!({"sha": "same"}));
    let mut equivalent = expected.clone();
    assert_eq!(expected, equivalent);
    equivalent.insert("Disc 1/01.flac".to_string(), json!({"sha": "changed"}));
    assert_ne!(expected, equivalent);
    equivalent = expected.clone();
    equivalent.remove("Disc 1/01.flac");
    assert_ne!(expected, equivalent);
    equivalent = expected.clone();
    equivalent.insert("Disc 2/01.flac".to_string(), json!({"sha": "same"}));
    assert_ne!(expected, equivalent);
}

#[test]
fn evaluation_copy_path_preserves_source_hierarchy() {
    assert_eq!(
        safe_relative_folder("Enya/A Day Without Rain/Disc 1").to_string_lossy(),
        "Enya/A Day Without Rain/Disc 1"
    );
    assert!(std::panic::catch_unwind(|| safe_relative_folder("../outside")).is_err());
    assert!(std::panic::catch_unwind(|| safe_relative_folder("/absolute")).is_err());
}

#[test]
fn reviewed_expectation_overlay_authorizes_only_reviewed_cases() {
    let mut corpus = load_corpus();
    let case_id = corpus.cases[0].case_id.clone();
    let path = std::env::temp_dir().join(format!(
        "soundrobe-auto-tag-expectations-{}.json",
        uuid::Uuid::new_v4()
    ));
    fs::write(
        &path,
        serde_json::to_string(&json!({
            "schemaVersion": corpus.schema_version,
            "corpusVersion": corpus.corpus_version,
            "cases": [{
                "caseId": case_id,
                "status": "verified_match",
                "acceptableEditionIds": ["reviewed-release"],
                "rejectedHardNegativeIds": [],
                "mapping": [{"localTrack": 1, "providerTrack": 1}]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    overlay_reviewed_expectations(&mut corpus, &path);
    assert_eq!(corpus.cases[0].expectation.status, "verified_match");
    assert_eq!(
        corpus.cases[0].expectation.acceptable_edition_ids,
        vec!["reviewed-release"]
    );
    assert_eq!(corpus.cases[1].expectation.status, "unverified");
    fs::remove_file(path).unwrap();
}

#[test]
fn report_separates_provider_unavailability_from_no_match() {
    let corpus = load_corpus();
    let path = std::env::temp_dir().join(format!(
        "soundrobe-auto-tag-eval-report-{}.md",
        uuid::Uuid::new_v4()
    ));
    let record = json!({
        "caseId": corpus.cases[0].case_id,
        "phase": "cold",
        "profile": PROFILE_DISCOVERY,
        "artist": corpus.cases[0].artist,
        "releaseType": "album",
        "difficulty": "unverified",
        "classification": "incomplete",
        "outcome": "needs_review",
        "elapsedMs": 1,
        "native": {"providerAttempts": [{"status": "unavailable"}]}
    });
    write_report(&path, &[record], &corpus, "report-test");
    let report = fs::read_to_string(&path).unwrap();
    assert!(report.contains("- provider_unavailable: 1"));
    assert!(!report.contains("- no_authoritative_match: 1"));
    fs::remove_file(path).unwrap();
}

#[test]
fn release_groups_have_stable_development_holdout_partitions() {
    let corpus = load_corpus();
    let first = corpus
        .cases
        .iter()
        .map(|case| {
            (
                case.release_group_id.clone(),
                stable_partition(&case.release_group_id),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let second = corpus
        .cases
        .iter()
        .map(|case| {
            (
                case.release_group_id.clone(),
                stable_partition(&case.release_group_id),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(first, second);
    assert!(first.values().any(|partition| *partition == "development"));
    assert!(first.values().any(|partition| *partition == "holdout"));
    let mut families = BTreeMap::<String, &'static str>::new();
    for case in &corpus.cases {
        let partition = stable_partition(&case.release_group_id);
        if let Some(previous) = families.insert(case.edition_family.clone(), partition) {
            assert_eq!(
                previous, partition,
                "edition family split across partitions"
            );
        }
    }
    assert!(
        families.len() < corpus.cases.len(),
        "no physical disc folders were grouped"
    );
}

#[test]
fn candidate_selection_is_independent_of_frozen_pool_order() {
    let request = LookupRequest {
        artist_hint: Some("Artist".into()),
        album_hint: Some("Album".into()),
        tracks: vec![
            TrackCandidate {
                title: Some("One".into()),
                track_number: Some(1),
                length: Some(100.0),
                ..Default::default()
            },
            TrackCandidate {
                title: Some("Two".into()),
                track_number: Some(2),
                length: Some(120.0),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let candidate = |id: &str| AlbumCandidate {
        source: LookupSource::Discogs,
        album: Some("Album".into()),
        artist: Some("Artist".into()),
        album_artist: Some("Artist".into()),
        discogs_release_id: Some(id.into()),
        tracks: request.tracks.clone(),
        ..Default::default()
    };
    let candidates = vec![candidate("b"), candidate("a")];
    let left = select_deterministically(&request, candidates.clone())
        .map(|value| provider_candidate_id(&value));
    let right = select_deterministically(&request, candidates.into_iter().rev().collect())
        .map(|value| provider_candidate_id(&value));
    assert_eq!(left, right);
    assert_eq!(left.as_deref(), Some("a"));
}

#[test]
fn frozen_relapse_pool_replays_credibility_and_complete_mapping() {
    let local_path = fixture_path("../test/fixtures/tauri/relapse-deluxe/local.json");
    let target_path = fixture_path("../test/fixtures/tauri/auto-tag-eval/provider-snapshots/relapse/discogs-release-36441795.json");
    let hard_negative_path = fixture_path("../test/fixtures/tauri/auto-tag-eval/provider-snapshots/relapse/discogs-release-16649340.json");
    let reviewed_truth: Value = serde_json::from_str(
        &fs::read_to_string(fixture_path(
            "../test/fixtures/tauri/relapse-deluxe/reviewed-truth.json",
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(reviewed_truth["status"], "verified_match");
    assert_eq!(reviewed_truth["baseline"]["strongTitleMatches"], 16);
    assert_eq!(reviewed_truth["baseline"]["positionOnlyMatches"], 6);
    assert_eq!(reviewed_truth["postFix"]["strongMatches"], 22);
    assert_eq!(reviewed_truth["postFix"]["positionOnlyMatches"], 0);
    assert_eq!(reviewed_truth["postFix"]["guardedTitleMatches"], 6);
    assert_eq!(reviewed_truth["acceptableEditionIds"], json!(["36441795"]));
    assert_eq!(reviewed_truth["mapping"].as_array().map(Vec::len), Some(22));
    assert_eq!(reviewed_truth["mapping"][20]["providerTrack"], 22);
    assert_eq!(reviewed_truth["mapping"][21]["providerTrack"], 21);
    let pool_manifest: Value =
        serde_json::from_str(&fs::read_to_string(fixture_path(CANDIDATE_POOLS_RELATIVE)).unwrap())
            .unwrap();
    assert_eq!(
        pool_manifest["pools"][0]["replay"]["requireCompleteMapping"],
        true
    );
    let request: LookupRequest =
        serde_json::from_str(&fs::read_to_string(local_path).unwrap()).unwrap();
    let target: AlbumCandidate =
        serde_json::from_str(&fs::read_to_string(target_path).unwrap()).unwrap();
    let hard_negative = discogs_candidate(
        serde_json::from_str::<ProviderAlbum>(&fs::read_to_string(hard_negative_path).unwrap())
            .unwrap(),
    );
    assert!(provider_candidate_credibility(&request, &target).is_ok());
    assert!(provider_candidate_credibility(&request, &hard_negative).is_err());
    let left = select_deterministically(&request, vec![hard_negative.clone(), target.clone()])
        .map(|candidate| provider_candidate_id(&candidate));
    let right = select_deterministically(&request, vec![target, hard_negative])
        .map(|candidate| provider_candidate_id(&candidate));
    assert_eq!(left, Some("36441795".into()));
    assert_eq!(left, right);
}

#[tokio::test]
async fn discogs_snapshot_replay_rejects_unexpected_routes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fixture = fs::read_to_string(fixture_path(
        "../test/fixtures/tauri/relapse-deluxe/release-36441795.json",
    ))
    .unwrap();
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let count = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..count]);
        let first_line = request.lines().next().unwrap_or_default().to_string();
        let allowed = first_line.starts_with("GET /releases/36441795 ");
        send.send((allowed, first_line)).unwrap();
        let (status, body) = if allowed {
            ("200 OK", fixture)
        } else {
            ("404 Not Found", "{}".to_string())
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });

    let client = DiscogsClient::at(ProviderState::new().http(), None, &base);
    let album = client.release_metadata_result("36441795").await.unwrap();

    assert_eq!(album.id, "36441795");
    assert_eq!(album.title, "Relapse (Deluxe)");
    let (allowed, request) = receive.recv().unwrap();
    assert!(allowed, "unexpected fixture request: {request}");
}

#[test]
fn selected_track_evidence_records_guarded_title_matches() {
    let diagnostic = selected_track_match_diagnostic(
        &LookupRequest {
            tracks: vec![TrackCandidate {
                title: Some("Deja Vu".into()),
                length: Some(180.0),
                ..Default::default()
            }],
            ..Default::default()
        },
        &AlbumCandidate {
            source: LookupSource::Discogs,
            tracks: vec![TrackCandidate {
                title: Some("Déjà Vu".into()),
                length: Some(180.0),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    let result = AutoTagRunResult {
        diagnostics: vec![diagnostic],
        ..Default::default()
    };
    let evidence = selected_track_evidence(Some(&result)).unwrap();
    assert_eq!(evidence["evidence"][0], "GuardedTitle");
    assert_eq!(evidence["remoteIndices"][0], 0);
}

#[test]
fn reviewed_expectations_are_required_for_scored_metrics() {
    let corpus = load_corpus();
    assert_eq!(EvalOutcome::Incomplete.as_str(), "incomplete");
    assert_eq!(
        EvalOutcome::FailedVerification.as_str(),
        "failed_verification"
    );
    for case in &corpus.cases {
        let outcome = case.expectation.status.as_str();
        assert!(matches!(
            outcome,
            "unverified" | "verified_match" | "verified_abstain"
        ));
        assert_eq!(
            candidate_is_expected(case, &AlbumCandidate::default()).as_str(),
            "unresolved"
        );
        if outcome == "verified_match" {
            assert!(!case.expectation.acceptable_edition_ids.is_empty());
            assert!(!case.expectation.mapping.is_empty());
        }
        if outcome == "verified_abstain" {
            assert!(case.expectation.acceptable_edition_ids.is_empty());
        }
    }
}

#[test]
fn reviewed_mapping_must_match_the_selected_candidate_positions() {
    let candidate = AlbumCandidate {
        tracks: vec![
            TrackCandidate {
                track_number: Some(2),
                disc_number: Some(1),
                ..TrackCandidate::default()
            },
            TrackCandidate {
                track_number: Some(1),
                disc_number: Some(1),
                ..TrackCandidate::default()
            },
        ],
        ..AlbumCandidate::default()
    };
    let mapping = json!([
        {"localTrack": 1, "providerTrack": "1-2"},
        {"localTrack": 2, "providerTrack": "1-1"}
    ]);
    assert!(reviewed_mapping_matches(&candidate, mapping.as_array().unwrap()));

    let mut case = load_corpus().cases[0].clone();
    case.expectation.status = "verified_match".into();
    case.expectation.acceptable_edition_ids = vec!["reviewed-release".into()];
    case.expectation.mapping = mapping.as_array().unwrap().clone();
    let mut accepted = candidate.clone();
    accepted.discogs_release_id = Some("reviewed-release".into());
    assert_eq!(
        candidate_is_expected(&case, &accepted),
        EvalOutcome::ConfirmedSuccess
    );

    let wrong = json!([
        {"localTrack": 1, "providerTrack": "1-1"},
        {"localTrack": 2, "providerTrack": "1-2"}
    ]);
    assert!(!reviewed_mapping_matches(&candidate, wrong.as_array().unwrap()));
    case.expectation.mapping = wrong.as_array().unwrap().clone();
    assert_eq!(
        candidate_is_expected(&case, &accepted),
        EvalOutcome::Unresolved
    );

    let flattened = AlbumCandidate {
        tracks: vec![
            TrackCandidate {
                track_number: Some(1),
                title: Some("First disc".into()),
                ..TrackCandidate::default()
            },
            TrackCandidate {
                track_number: Some(2),
                title: Some("Second disc".into()),
                ..TrackCandidate::default()
            },
            TrackCandidate {
                track_number: Some(3),
                title: Some("Third disc".into()),
                ..TrackCandidate::default()
            },
            TrackCandidate {
                track_number: Some(4),
                title: Some("Fourth disc".into()),
                ..TrackCandidate::default()
            },
        ],
        ..AlbumCandidate::default()
    };
    let flattened_mapping = json!([
        {"localTrack": 1, "providerTrack": "1", "providerTitle": "First disc"},
        {"localTrack": 2, "providerTrack": "2", "providerTitle": "Second disc"},
        {"localTrack": 3, "providerTrack": "1", "providerTitle": "Third disc"},
        {"localTrack": 4, "providerTrack": "2", "providerTitle": "Fourth disc"}
    ]);
    assert!(reviewed_mapping_matches(
        &flattened,
        flattened_mapping.as_array().unwrap()
    ));
    let flattened_wrong_title = json!([
        {"localTrack": 1, "providerTrack": "1", "providerTitle": "First disc"},
        {"localTrack": 2, "providerTrack": "2", "providerTitle": "Second disc"},
        {"localTrack": 3, "providerTrack": "1", "providerTitle": "Fourth disc"},
        {"localTrack": 4, "providerTrack": "2", "providerTitle": "Third disc"}
    ]);
    assert!(!reviewed_mapping_matches(
        &flattened,
        flattened_wrong_title.as_array().unwrap()
    ));
}

#[test]
fn clean_discovery_requests_have_no_provider_ids() {
    let clean = LookupRequest::default();
    assert!(provider_ids_cleared(&clean));

    let mut with_album_id = clean.clone();
    with_album_id.musicbrainz_album_id = Some("album".into());
    assert!(!provider_ids_cleared(&with_album_id));

    let mut with_artist_id = clean.clone();
    with_artist_id.discogs_artist_id = Some("artist".into());
    assert!(!provider_ids_cleared(&with_artist_id));

    let mut with_track_id = clean;
    with_track_id.tracks = vec![TrackCandidate {
        musicbrainz_track_id: Some("track".into()),
        ..TrackCandidate::default()
    }];
    assert!(!provider_ids_cleared(&with_track_id));
}

#[test]
fn readback_rejects_a_candidate_with_a_different_track_count() {
    let candidate = AlbumCandidate {
        tracks: vec![TrackCandidate::default()],
        ..AlbumCandidate::default()
    };
    assert!(!candidate_track_count_matches(&candidate, 2));
    assert!(candidate_track_count_matches(&candidate, 1));
}

#[test]
fn verified_abstention_rejects_an_applied_candidate() {
    let mut case = load_corpus().cases[0].clone();
    case.expectation.status = "verified_abstain".to_string();
    let candidate = AlbumCandidate {
        source: LookupSource::Discogs,
        discogs_release_id: Some("unblessed-release".into()),
        ..Default::default()
    };
    assert_eq!(
        candidate_is_expected(&case, &candidate),
        EvalOutcome::WrongMatch
    );
}

#[test]
fn native_runner_uses_the_reviewed_timeout_budget() {
    assert_eq!(PER_FOLDER_TIMEOUT.as_secs(), 600);
    assert_eq!(RUN_TIMEOUT.as_secs(), 8 * 60 * 60);
}

#[test]
fn native_results_reconcile_phases_into_folder_and_logical_rows() {
    let corpus = load_corpus();
    let case = &corpus.cases[0];
    let cold = json!({
        "caseId": case.case_id,
        "phase": "cold",
        "profile": PROFILE_DISCOVERY,
        "classification": "incomplete",
        "native": {"candidate": {"discogs_release_id": "release-1"}},
    });
    let warm = json!({
        "caseId": case.case_id,
        "phase": "warm",
        "profile": PROFILE_DISCOVERY,
        "classification": "confirmed_success",
        "native": {"candidate": {"discogs_release_id": "release-1"}},
    });
    let reconciled = reconcile_results(&[cold, warm], &corpus, "reconcile-test");
    assert_eq!(reconciled["folderCount"], 1);
    assert_eq!(reconciled["logicalRowCount"], 1);
    assert_eq!(
        reconciled["folderResults"][0]["classification"],
        "confirmed_success"
    );
    assert_eq!(reconciled["folderResults"][0]["warmRecovery"], true);
    assert_eq!(
        reconciled["logicalRows"][0]["folderOutcome"],
        "confirmed_success"
    );

    let conflicting_warm = json!({
        "caseId": case.case_id,
        "phase": "warm",
        "profile": PROFILE_DISCOVERY,
        "classification": "confirmed_success",
        "native": {"candidate": {"discogs_release_id": "release-2"}},
    });
    let conflicted = reconcile_results(
        &[
            json!({
                "caseId": case.case_id,
                "phase": "cold",
                "profile": PROFILE_DISCOVERY,
                "classification": "confirmed_success",
                "native": {"candidate": {"discogs_release_id": "release-1"}},
            }),
            conflicting_warm,
        ],
        &corpus,
        "reconcile-conflict-test",
    );
    assert_eq!(
        conflicted["folderResults"][0]["classification"],
        "failed_verification"
    );
}

#[test]
fn native_outcome_classification_keeps_safety_and_provider_failures_distinct() {
    let corpus = load_corpus();
    let case = &corpus.cases[0];
    let needs_review = AutoTagRunResult {
        outcome: AutoTagOutcome::NeedsReview,
        authority: None,
        candidate: None,
        written: 0,
        reason_code: Some("no_match".into()),
        diagnostics: Vec::new(),
        provider_attempts: Vec::new(),
        ai_status: None,
        ai_confidence: None,
        ai_threshold: None,
    };
    assert_eq!(
        native_classification(case, Some(&needs_review), false, false, true),
        EvalOutcome::Unresolved
    );
    assert_eq!(
        native_classification(case, Some(&needs_review), false, false, false),
        EvalOutcome::FailedVerification
    );
    assert_eq!(
        native_classification(case, None, true, false, true),
        EvalOutcome::FailedVerification
    );
    assert_eq!(
        native_classification(case, None, false, true, true),
        EvalOutcome::Incomplete
    );
    let unavailable = AutoTagRunResult {
        provider_attempts: vec![ProviderAttempt {
            provider: "discogs",
            status: ProviderAttemptStatus::Unavailable,
            diagnostic: Some("rate limited".into()),
            retry_count: 1,
            retry_after_seconds: Some(60),
        }],
        ..needs_review.clone()
    };
    assert_eq!(
        native_classification(case, Some(&unavailable), false, false, true),
        EvalOutcome::Incomplete
    );
    let llm_applied = AutoTagRunResult {
        outcome: AutoTagOutcome::Applied,
        authority: Some(LookupSource::Llm),
        candidate: Some(AlbumCandidate::default()),
        written: 1,
        ..needs_review
    };
    assert_eq!(
        native_classification(case, Some(&llm_applied), false, false, true),
        EvalOutcome::FailedVerification
    );
}

#[test]
fn evaluation_sanitization_redacts_nested_credentials_and_queries() {
    let value = sanitise(json!({
        "url": "https://example.test/release?token=secret",
        "headers": {"Authorization": "Bearer abc"},
        "nested": [{"apiKey": "key", "safe": "value"}],
    }));
    assert_eq!(
        value["url"],
        "https://example.test/release?[redacted-query]"
    );
    assert_eq!(value["headers"], "[redacted]");
    assert_eq!(value["nested"][0]["apiKey"], "[redacted]");
    assert_eq!(value["nested"][0]["safe"], "value");
}

#[tokio::test]
#[ignore = "requires ffmpeg; generates disposable silent FLAC tracks"]
async fn native_synthetic_flac_duration_and_write_contract() {
    let root = PathBuf::from("/private/tmp").join(format!(
        "soundrobe-auto-tag-eval-synthetic-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let durations = [("short.flac", 1.25_f64), ("long.flac", 7.75_f64)];
    let queue = WriteQueue::default();
    for (name, duration) in durations {
        let path = root.join(name);
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=stereo",
                "-t",
                &duration.to_string(),
            ])
            .arg(&path)
            .status()
            .expect("ffmpeg must be available for synthetic native contract");
        assert!(status.success(), "ffmpeg failed for {}", path.display());
        let read = crate::commands::tracks::read_track_metadata(&path).unwrap();
        assert!(
            (read.duration - duration).abs() <= 0.1,
            "duration drift for {}: {}",
            name,
            read.duration
        );
        let payload_before = payload_hash(&path);
        write_track_queued(
            &queue,
            path.clone(),
            TrackPatch {
                title: Patch::Value("Synthetic".into()),
                artist: Patch::Value("Synthetic Artist".into()),
                album: Patch::Value("Synthetic Album".into()),
                track_number: Patch::Value(1),
                track_total: Patch::Value(1),
                ..TrackPatch::default()
            },
        )
        .await
        .unwrap();
        let readback = crate::commands::tracks::read_track_metadata(&path).unwrap();
        assert_eq!(readback.title.as_deref(), Some("Synthetic"));
        assert_eq!(readback.artist.as_deref(), Some("Synthetic Artist"));
        assert_eq!(readback.album.as_deref(), Some("Synthetic Album"));
        assert_eq!(readback.track_number, Some(1));
        assert_eq!(
            payload_hash(&path),
            payload_before,
            "writer changed synthetic audio payload"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "requires the curated source and ffmpeg; compares disposable synthetic FLAC inputs"]
async fn native_synthetic_flac_lookup_equivalence_uses_production_reader() {
    let corpus = load_corpus();
    let wanted = [
        "Relapse (With Bonus)",
        "Only Time-The Collection",
        "The Very Best Of Enya (Deluxe Edition) (Digibook)",
        "Wild Child",
        "WAV 单曲117首",
        "Hotel California (40th Anniversary Expanded Edition)",
    ];
    let cases = wanted
        .iter()
        .map(|needle| {
            corpus
                .cases
                .iter()
                .find(|case| case.source_relative_folder.contains(needle))
                .unwrap_or_else(|| panic!("corpus is missing equivalence case {needle}"))
        })
        .collect::<Vec<_>>();
    let root = PathBuf::from("/private/tmp").join(format!(
        "soundrobe-auto-tag-eval-equivalence-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let queue = WriteQueue::default();
    let mut equivalence = Vec::new();
    for case in cases {
        let source = source_path(&corpus.source_root, &case.source_relative_folder);
        let real = build_lookup_request(&source).unwrap();
        let destination = root.join(&case.case_id).join(&case.source_relative_folder);
        materialize_synthetic_case(&source, &destination, &queue)
            .await
            .unwrap();
        let synthetic = build_lookup_request(&destination).unwrap();
        assert_lookup_requests_equivalent(&real, &synthetic);
        assert_eq!(real.tracks.len(), synthetic.tracks.len());
        let max_duration_delta = real
            .tracks
            .iter()
            .zip(&synthetic.tracks)
            .filter_map(|(left, right)| left.length.zip(right.length))
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f64, f64::max);

        let relapse = case.source_relative_folder.contains("Relapse (With Bonus)");
        let digibook = case
            .source_relative_folder
            .contains("Very Best Of Enya (Deluxe Edition) (Digibook)");
        if relapse || digibook {
            let fixture = if relapse {
                "../test/fixtures/tauri/relapse-deluxe/release-36441795.json"
            } else {
                "../test/fixtures/tauri/enya-discogs/release-2029801.json"
            };
            let candidate = if relapse {
                serde_json::from_str::<AlbumCandidate>(
                    &fs::read_to_string(fixture_path(
                        "../test/fixtures/tauri/relapse-deluxe/candidate-36441795.json",
                    ))
                    .unwrap(),
                )
                .unwrap()
            } else {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let base = format!("http://{}", listener.local_addr().unwrap());
                let body = fs::read_to_string(fixture_path(fixture)).unwrap();
                let server = thread::spawn(move || {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut request = [0_u8; 4096];
                    assert!(stream.read(&mut request).unwrap() > 0);
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                });
                let client = DiscogsClient::at(ProviderState::new().http(), None, &base);
                let provider = client.release_metadata_result("2029801").await.unwrap();
                server.join().unwrap();
                discogs_candidate(provider)
            };
            let real_candidate = select_deterministically(&real, vec![candidate.clone()]);
            let synthetic_candidate = select_deterministically(&synthetic, vec![candidate.clone()]);
            assert_eq!(
                real_candidate.as_ref().map(provider_candidate_id),
                synthetic_candidate.as_ref().map(provider_candidate_id)
            );
            let real_evidence = selected_track_match_diagnostic(&real, &candidate);
            let synthetic_evidence = selected_track_match_diagnostic(&synthetic, &candidate);
            assert_eq!(real_evidence["evidence"], synthetic_evidence["evidence"]);
            assert_eq!(
                real_evidence["remoteIndices"],
                synthetic_evidence["remoteIndices"]
            );
            assert_eq!(
                real_evidence["isFullOrderedMatch"],
                synthetic_evidence["isFullOrderedMatch"]
            );
            equivalence.push(json!({
                "caseId": case.case_id,
                "sourceRelativeFolder": case.source_relative_folder,
                "trackCount": real.tracks.len(),
                "maxDurationDeltaSeconds": max_duration_delta,
                "selectedIdentityReal": real_candidate.as_ref().map(provider_candidate_id),
                "selectedIdentitySynthetic": synthetic_candidate.as_ref().map(provider_candidate_id),
                "evidenceEqual": real_evidence["evidence"] == synthetic_evidence["evidence"],
                "remoteIndicesEqual": real_evidence["remoteIndices"] == synthetic_evidence["remoteIndices"],
                "orderedMatchEqual": real_evidence["isFullOrderedMatch"] == synthetic_evidence["isFullOrderedMatch"],
            }));
        } else {
            equivalence.push(json!({
                "caseId": case.case_id,
                "sourceRelativeFolder": case.source_relative_folder,
                "trackCount": real.tracks.len(),
                "maxDurationDeltaSeconds": max_duration_delta,
                "requestEqual": true,
            }));
        }
        fs::remove_dir_all(root.join(&case.case_id)).unwrap();
    }
    if let Ok(path) = std::env::var("SOUNDROBE_AUTO_TAG_EVAL_EQUIVALENCE_ARTIFACT") {
        fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "mediaMode": "synthetic_flac",
                "source": "production reader over curated originals",
                "cases": equivalence,
                "allLookupRequestsEquivalent": true,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

/// Opt-in native replay over a filtered corpus. The test is ignored so normal
/// CI never depends on credentials, provider availability, or user media.
#[tokio::test]
#[ignore = "requires explicit corpus source, credentials, and live providers"]
async fn live_auto_tag_eval() {
    let corpus_path = PathBuf::from(
        std::env::var("SOUNDROBE_AUTO_TAG_EVAL_CORPUS")
            .expect("SOUNDROBE_AUTO_TAG_EVAL_CORPUS required"),
    );
    let artifact_dir = PathBuf::from(
        std::env::var("SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR")
            .expect("SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR required"),
    );
    let run_id = std::env::var("SOUNDROBE_AUTO_TAG_EVAL_RUN_ID")
        .unwrap_or_else(|_| format!("native-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&artifact_dir).expect("create evaluation artifact directory");
    let mut corpus: EvalCorpus =
        serde_json::from_str(&fs::read_to_string(&corpus_path).expect("read evaluation corpus"))
            .expect("parse evaluation corpus");
    let expectations_path = std::env::var("SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| fixture_path(EXPECTATIONS_RELATIVE));
    overlay_reviewed_expectations(&mut corpus, &expectations_path);
    let source_root = fs::canonicalize(&corpus.source_root).expect("evaluation source root exists");
    let artist_filter = std::env::var("SOUNDROBE_AUTO_TAG_EVAL_ARTISTS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        });
    let case_filter = std::env::var("SOUNDROBE_AUTO_TAG_EVAL_CASES")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        });
    let profile = std::env::var("SOUNDROBE_AUTO_TAG_EVAL_PROFILE")
        .unwrap_or_else(|_| PROFILE_DISCOVERY.to_string());
    assert!(
        matches!(
            profile.as_str(),
            PROFILE_DISCOVERY | PROFILE_ASSISTED | PROFILE_RECOVERY
        ),
        "unknown evaluation profile {profile}"
    );
    let selected = corpus
        .cases
        .iter()
        .filter(|case| {
            artist_filter
                .as_ref()
                .is_none_or(|values| values.contains(case.artist.as_str()))
        })
        .filter(|case| {
            case_filter
                .as_ref()
                .is_none_or(|values| values.contains(case.case_id.as_str()))
        })
        .collect::<Vec<_>>();
    assert!(!selected.is_empty(), "filters selected no evaluation cases");
    let mock_url = std::env::var("SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL").ok();
    let config_path = dirs::home_dir()
        .expect("home directory")
        .join(".soundrobe/config.yaml");
    let mut config = if mock_url.is_some() {
        load_from("", &crate::state::config::EnvMap::new())
    } else {
        load_from(
            &fs::read_to_string(config_path).expect("read Soundrobe config"),
            &ProcessEnv,
        )
    };
    assert!(
        mock_url.is_some()
            || config
                .discogs_token
                .as_ref()
                .is_some_and(|token| !token.trim().is_empty()),
        "Discogs token required for live evaluation"
    );
    config.llm_api_key = None;
    config.remote_lookup_enabled = Some(true);
    config.discogs_enabled = Some(true);
    config.lyrics_download_enabled = Some(false);
    let temp_root = PathBuf::from("/private/tmp")
        .join(format!("soundrobe-auto-tag-eval-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_root).expect("create evaluation temp root");
    let cache_path = temp_root.join("cache.db");
    let cache = CacheState::new(temp_root.clone());
    assert!(cache.initialize(Some(cache_path.to_str().unwrap())));
    let providers = if let Some(base) = &mock_url {
        config.discogs_token = None;
        offline_eval_providers(base).expect("invalid offline provider service URL")
    } else {
        ProviderState::new()
    };
    let queue = WriteQueue::default();
    let alias_file = temp_root.join("aliases.json");
    let mut records = Vec::new();
    let source_hashes_before = selected
        .iter()
        .map(|case| {
            (
                case.case_id.clone(),
                full_hash_map(&source_path(&source_root, &case.source_relative_folder)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    fs::write(
        artifact_dir.join("source-hashes-before.json"),
        serde_json::to_vec_pretty(&source_hashes_before).unwrap(),
    )
    .unwrap();
    let run_started = Instant::now();
    for phase in ["cold", "warm"] {
        for case in &selected {
            assert!(
                run_started.elapsed() <= RUN_TIMEOUT,
                "native evaluation exceeded eight-hour bound"
            );
            let source = source_path(&source_root, &case.source_relative_folder);
            verify_frozen_case(
                case,
                &source,
                source_hashes_before
                    .get(&case.case_id)
                    .expect("source hash snapshot missing"),
            );
            let destination = temp_root
                .join("media")
                .join(phase)
                .join(&case.case_id)
                .join(safe_relative_folder(&case.source_relative_folder));
            let synthetic = materialize_synthetic_case(&source, &destination, &queue)
                .await
                .expect("materialize synthetic FLAC evaluation folder");
            let copied_before = hash_map(&destination);
            assert_eq!(copied_before.len(), synthetic.track_count);
            apply_profile(&destination, &profile, &queue)
                .await
                .expect("apply evaluation input profile");
            let preflight = build_lookup_request(&destination).expect("build clean lookup request");
            assert!(
                destination
                    .components()
                    .any(|component| component.as_os_str() == case.artist.as_str()),
                "copy hierarchy lost the artist folder component"
            );
            assert!(
                preflight.folder_artist_hint.is_some(),
                "copy hierarchy lost the artist folder hint"
            );
            assert!(
                preflight.folder_album_hint.is_some(),
                "copy hierarchy lost the album folder hint"
            );
            if preflight.selected_disc_number.is_some() {
                assert!(
                    case.source_relative_folder
                        .rsplit('/')
                        .next()
                        .is_some_and(|folder| {
                            let lower = folder.to_ascii_lowercase();
                            lower.contains("disc") || lower.contains("cd")
                        }),
                    "disc hint was inferred from an unexpected source folder"
                );
            }
            if profile != PROFILE_RECOVERY {
                assert!(
                    provider_ids_cleared(&preflight),
                    "discovery profile retained provider IDs"
                );
            }
            let baseline = hash_map(&destination);
            let cache_before = cache_counts(&cache_path);
            let started = Instant::now();
            let cancelled = AtomicBool::new(false);
            let progress_events = RefCell::new(Vec::<Value>::new());
            let report_events = RefCell::new(Vec::<Value>::new());
            let result = tokio::time::timeout(
                PER_FOLDER_TIMEOUT,
                resolve_and_apply_album_with_retry_context(
                    &destination,
                    &config,
                    AutoTagServices {
                        providers: &providers,
                        cache: &cache,
                        queue: &queue,
                        alias_file: &alias_file,
                    },
                    &cancelled,
                    None,
                    |progress, message| {
                        progress_events
                            .borrow_mut()
                            .push(json!({"progress": progress, "message": message}));
                    },
                    |kind, message, data| {
                        report_events
                            .borrow_mut()
                            .push(json!({"kind": kind, "message": message, "data": data}));
                    },
                ),
            )
            .await;
            let elapsed_ms = started.elapsed().as_millis();
            let (outcome, error, native, resolver_error, timed_out) = match result {
                Ok(Ok(value)) => (
                    if value.outcome == AutoTagOutcome::Applied {
                        "applied"
                    } else {
                        "needs_review"
                    },
                    None,
                    Some(value),
                    false,
                    false,
                ),
                Ok(Err(error)) => ("resolver_error", Some(error.to_string()), None, true, false),
                Err(_) => (
                    "timeout",
                    Some("per-folder timeout".into()),
                    None,
                    false,
                    true,
                ),
            };
            assert!(
                !queue.is_active(),
                "evaluation left queued media work active after resolver completion"
            );
            let after = hash_map(&destination);
            let payload_unchanged = after.len() == baseline.len()
                && baseline.iter().all(|(path, before)| {
                    after
                        .get(path)
                        .and_then(|value| value.get("payloadSha256"))
                        .zip(before.get("payloadSha256"))
                        .is_some_and(|(left, right)| left == right)
                });
            let readback = if outcome == "applied" {
                readback_matches(&destination, native.as_ref(), &case.expectation)
            } else {
                native.as_ref().is_none_or(|value| value.written == 0) && after == baseline
            };
            let classification = native_classification(
                case,
                native.as_ref(),
                resolver_error,
                timed_out,
                payload_unchanged && readback,
            );
            let selected_evidence = selected_track_evidence(native.as_ref());
            let record = sanitise(
                json!({"runId":run_id,"phase":phase,"caseId":case.case_id,"artist":case.artist,"releaseType":case.release_type,"difficulty":case.difficulty,"sourceRelativeFolder":case.source_relative_folder,"outcome":outcome,"classification":classification.as_str(),"error":error,"elapsedMs":elapsed_ms,"native":native,"selectedTrackEvidence":selected_evidence,"progressEvents":progress_events.into_inner(),"reportEvents":report_events.into_inner(),"cacheBefore":cache_before,"cacheAfter":cache_counts(&cache_path),"copiedBeforeProfile":copied_before,"profileBaseline":baseline,"after":after,"payloadUnchanged":payload_unchanged,"readback":readback,"profile":profile,"mediaMode":"synthetic_flac","syntheticTrackCount":synthetic.track_count,"syntheticBytes":synthetic.bytes,"syntheticPeakBytes":synthetic.peak_bytes,"oracleStatus":case.expectation.status}),
            );
            let mut record = record;
            record["providerMode"] = json!(if mock_url.is_some() {
                "offline_fixtures"
            } else {
                "live"
            });
            let line = serde_json::to_string(&record).unwrap();
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(artifact_dir.join(format!("{phase}.jsonl")))
                .unwrap()
                .write_all(format!("{line}\n").as_bytes())
                .unwrap();
            records.push(record);
            fs::write(
                artifact_dir.join("checkpoint.json"),
                serde_json::to_vec_pretty(&json!({
                    "runId": run_id,
                    "profile": profile,
                    "mediaMode": "synthetic_flac",
                    "expectedInvocations": selected.len() * 2,
                    "completedInvocations": records.len(),
                    "lastCaseId": case.case_id,
                    "lastPhase": phase,
                }))
                .unwrap(),
            )
            .unwrap();
            fs::remove_dir_all(&destination).expect("remove synthetic evaluation media");
        }
    }
    fs::write(
        artifact_dir.join("results.json"),
        serde_json::to_vec_pretty(&reconcile_results(&records, &corpus, &run_id)).unwrap(),
    )
    .unwrap();
    let source_hashes_after = selected
        .iter()
        .map(|case| {
            (
                case.case_id.clone(),
                full_hash_map(&source_path(&source_root, &case.source_relative_folder)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        source_hashes_before, source_hashes_after,
        "source media changed during evaluation"
    );
    fs::write(
        artifact_dir.join("source-hashes-after.json"),
        serde_json::to_vec_pretty(&source_hashes_after).unwrap(),
    )
    .unwrap();
    write_report(&artifact_dir.join("report.md"), &records, &corpus, &run_id);
    fs::write(
        artifact_dir.join("command.log"),
        format!(
            "status=passed\nrun_id={run_id}\nprofile={profile}\nmedia_mode=synthetic_flac\nselected_cases={}\nphases=2\nper_folder_timeout_seconds={}\nnative_test_passed=1\nnative_test_failed=0\nnative_test_ignored=0\ncommand=cargo test --manifest-path src-tauri/Cargo.toml --lib live_auto_tag_eval -- --ignored --nocapture\n",
            selected.len(),
            PER_FOLDER_TIMEOUT.as_secs()
        ),
    )
    .unwrap();
    fs::remove_dir_all(&temp_root).expect("remove temporary evaluation media");
}

fn offline_eval_providers(base: &str) -> Result<ProviderState, String> {
    let url = reqwest::Url::parse(base).map_err(|error| error.to_string())?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("offline service must be an HTTP 127.0.0.1 origin".into());
    }
    let http = reqwest::Client::builder()
        .no_proxy()
        // Force even ancillary artwork HTTP/CONNECT attempts through the
        // fixture server, which never forwards traffic to another host.
        .proxy(reqwest::Proxy::all(base).map_err(|error| error.to_string())?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    let base = base.trim_end_matches('/');
    Ok(ProviderState::at(
        http,
        &format!("{base}/musicbrainz/ws/2"),
        &format!("{base}/discogs"),
    ))
}

#[test]
fn offline_eval_rejects_external_endpoints_and_credentials() {
    for url in [
        "https://musicbrainz.org",
        "http://localhost:1234",
        "http://127.0.0.1/path",
        "http://key@127.0.0.1",
        "http://127.0.0.1?token=secret",
    ] {
        assert!(offline_eval_providers(url).is_err(), "accepted {url}");
    }
    assert!(offline_eval_providers("http://127.0.0.1:1234").is_ok());
}

#[tokio::test]
async fn offline_fixture_service_uses_both_production_provider_parsers() {
    use std::io::BufRead;
    use std::process::Stdio;
    struct ServerProcess(std::process::Child, PathBuf);
    impl Drop for ServerProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
            let _ = fs::remove_dir_all(&self.1);
        }
    }
    let root =
        std::env::temp_dir().join(format!("soundrobe-provider-mock-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let manifest = root.join("manifest.json");
    let script = fixture_path("../scripts/mock-provider-service.cjs");
    assert!(Command::new("node")
        .arg(&script)
        .arg("import-pools")
        .arg(fixture_path(CANDIDATE_POOLS_RELATIVE))
        .arg(&manifest)
        .status()
        .unwrap()
        .success());
    let mut server = ServerProcess(
        Command::new("node")
            .arg(&script)
            .arg("serve")
            .arg(&manifest)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
        root,
    );
    let mut line = String::new();
    std::io::BufReader::new(server.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let endpoints: Value = serde_json::from_str(&line).unwrap();
    let base = endpoints["discogs"]
        .as_str()
        .unwrap()
        .trim_end_matches("/discogs");
    let providers = offline_eval_providers(base).unwrap();
    let discogs = DiscogsClient::at(providers.http(), None, providers.discogs_base());
    let album = discogs.release_metadata_result("1459867").await.unwrap();
    assert_eq!(album.id, "1459867");
    assert!(!album.tracks.is_empty());
    let mb = crate::state::providers::MusicBrainzClient::at(
        providers.http(),
        providers.musicbrainz_base(),
    );
    let album = mb
        .release_by_id_result("627377a9-be56-4c45-a56d-9ae941546ef0")
        .await
        .unwrap();
    assert_eq!(album.tracks.len(), 15);
    assert!(discogs
        .release_metadata_result("not-captured")
        .await
        .is_err());
    let inventory: Value = providers
        .http()
        .get(format!("{base}/__fixtures"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(inventory["misses"].as_array().unwrap().len(), 1);
    assert_eq!(
        providers
            .http()
            .get("http://unreachable.invalid/artwork")
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        501
    );
    assert!(providers
        .http()
        .get("https://unreachable.invalid/artwork")
        .send()
        .await
        .is_err());
}
