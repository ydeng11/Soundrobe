//! AcoustID recording recognition and conservative release assignment.
//!
//! AcoustID contributes identities; MusicBrainz recording/release details
//! supply verified text. Partial recognition never invents an album edition.

use crate::commands::auto_tag::{
    accept_acoustid_release, musicbrainz_candidate, AlbumCandidate, LookupRequest, TrackCandidate,
};
use crate::infra::acoustid::{
    fingerprint_file_with, fpcalc_executable, AcoustidClient, AcoustidError, AcoustidRecording,
    Fingerprint,
};
use crate::state::providers::{
    MusicBrainzClient, ProviderRecording, ProviderRetryContext, ProviderRetryMetrics,
    ProviderState, RecordingLookupFailure,
};
use crate::state::sqlite::CacheState;
use crate::{commands::library::collect_audio_files, infra::acoustid};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MINIMUM_SCORE: f64 = 0.90;
const TIED_SCORE_DISTANCE: f64 = 0.02;
const MAX_TRACKS: usize = 200;
const MAX_RELEASE_CANDIDATES: usize = 8;
const RECORDING_FACTS: &str = "acoustid-recording-v3";
const RECORDING_ATTEMPTS: &str = "acoustid-recording-attempt-v3";
const RECORDING_PROVIDER_COOLDOWN: &str = "__provider_cooldown__";
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AcoustidResolution {
    NoMatch,
    Recordings {
        tracks: Vec<TrackCandidate>,
        shared_album: Option<Box<AlbumCandidate>>,
        evidence: AcoustidEvidence,
        reason_code: String,
        retryable: bool,
        retry_after_seconds: Option<u64>,
    },
    Candidate {
        candidate: Box<AlbumCandidate>,
        evidence: AcoustidEvidence,
    },
    Unavailable(String),
    Incomplete(String),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcoustidEvidence {
    pub release_id: String,
    pub decision_rule_version: u8,
    pub validation_complete: bool,
    pub track_matches: Vec<AcoustidTrackEvidence>,
    pub candidate_count: usize,
    pub checked_candidates: usize,
    pub rejections: Vec<String>,
    pub pending_candidates: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcoustidTrackEvidence {
    pub file_name: String,
    pub recording_id: String,
    pub score: f64,
    pub reason: Option<String>,
    pub candidate_set_complete: bool,
    pub candidates: Vec<serde_json::Value>,
    pub candidate_count: usize,
    pub omitted_candidates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookup_observation: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
struct SelectedRecording {
    recording: AcoustidRecording,
    score: f64,
}

pub(crate) async fn resolve_cached(
    request: &LookupRequest,
    api_key: Option<&str>,
    providers: &ProviderState,
    cancelled: &AtomicBool,
    cache: &CacheState,
) -> AcoustidResolution {
    let client = AcoustidClient::new(providers.http());
    resolve_with_cache(
        request,
        api_key,
        providers,
        cancelled,
        &client,
        &fpcalc_executable(),
        Some(cache),
    )
    .await
}

pub(crate) async fn resolve_with(
    request: &LookupRequest,
    api_key: Option<&str>,
    providers: &ProviderState,
    external_cancelled: &AtomicBool,
    client: &AcoustidClient,
    fpcalc: &Path,
) -> AcoustidResolution {
    resolve_with_cache(
        request,
        api_key,
        providers,
        external_cancelled,
        client,
        fpcalc,
        None,
    )
    .await
}

pub(crate) async fn resolve_with_cache(
    request: &LookupRequest,
    api_key: Option<&str>,
    providers: &ProviderState,
    external_cancelled: &AtomicBool,
    client: &AcoustidClient,
    fpcalc: &Path,
    cache: Option<&CacheState>,
) -> AcoustidResolution {
    resolve_with_deadline(
        request,
        api_key,
        providers,
        external_cancelled,
        client,
        fpcalc,
        cache,
        tokio::time::sleep(TOTAL_TIMEOUT),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_with_deadline(
    request: &LookupRequest,
    api_key: Option<&str>,
    providers: &ProviderState,
    external_cancelled: &AtomicBool,
    client: &AcoustidClient,
    fpcalc: &Path,
    cache: Option<&CacheState>,
    deadline: impl std::future::Future<Output = ()>,
) -> AcoustidResolution {
    tokio::pin!(deadline);
    let Some(api_key) = api_key.filter(|key| !key.trim().is_empty()) else {
        return AcoustidResolution::Unavailable(
            "AcoustID application key is not configured".into(),
        );
    };
    if external_cancelled.load(Ordering::Acquire) {
        return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());
    }

    let files = collect_audio_files(Path::new(&request.path));
    let seed = AcoustidEvidence {
        release_id: String::new(),
        decision_rule_version: 3,
        validation_complete: false,
        track_matches: files
            .iter()
            .map(|file| AcoustidTrackEvidence {
                file_name: Path::new(file)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                recording_id: String::new(),
                score: 0.0,
                reason: Some("pending".into()),
                candidate_set_complete: false,
                candidates: Vec::new(),
                candidate_count: 0,
                omitted_candidates: 0,
                lookup_observation: Some(serde_json::json!({"version":1,"provider":"acoustid","source":null,"status":"not_attempted","observedAt":null,"responseUrl":null,"fullAlternativeCount":null,"eligibleHypothesisCount":null,"fingerprintScope":"fpcalc_default_provider_lookup"})),
            })
            .collect(),
        candidate_count: 0,
        checked_candidates: 0,
        rejections: Vec::new(),
        pending_candidates: Vec::new(),
    };
    let progress = Mutex::new(seed.clone());
    if let Err(reason) = checkpoint_snapshot(&seed, &progress) {
        return AcoustidResolution::Unavailable(reason);
    }
    // The internal cancellation flag lets the total budget or caller stop an
    // active fingerprint child, drain/reap it, and interrupt HTTP bodies.
    let cancelled = Arc::new(AtomicBool::new(false));
    let external_cancellation = wait_for_external_cancel(external_cancelled);
    tokio::pin!(external_cancellation);
    let inner = resolve_inner(
        request, api_key, providers, client, fpcalc, &cancelled, cache, &progress,
    );
    tokio::pin!(inner);
    let result = tokio::select! {
        biased;
        _ = &mut external_cancellation => {
            cancelled.store(true, Ordering::Release);
            let _ = inner.await;
            AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string())
        }
        _ = &mut deadline => {
            cancelled.store(true, Ordering::Release);
            let _ = inner.await;
            if external_cancelled.load(Ordering::Acquire) {return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());}
            let mut evidence = match progress.lock() {Ok(snapshot)=>snapshot.clone(),Err(_)=>return AcoustidResolution::Unavailable("diagnostic checkpoint unavailable".into())};
            evidence.validation_complete = false;
            evidence.rejections.push("total time budget exhausted before all validation completed".into());
            for row in &mut evidence.track_matches {
                row.recording_id.clear();
                if row.reason.is_none() || row.reason.as_deref()==Some("pending") {row.reason=Some("budget_validation_pending".into());}
            }
            if let Err(reason) = checkpoint_snapshot(&evidence,&progress) {return AcoustidResolution::Unavailable(reason);}
            AcoustidResolution::Recordings { tracks: request.tracks.iter().map(|local| TrackCandidate {filename:local.filename.clone(),source:local.source.clone(),..TrackCandidate::default()}).collect(), shared_album:None, evidence, reason_code:"acoustid_budget_exhausted".into(), retryable:true, retry_after_seconds:None }
        }
        result = &mut inner => result,
    };
    if external_cancelled.load(Ordering::Acquire) {
        return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());
    }
    result
}

type RecordingDetails = HashMap<String, Result<ProviderRecording, RecordingLookupFailure>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecordingAttempt {
    failure: RecordingLookupFailure,
    retry_at: u64,
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn local_failure(kind: &str, reason: &str, retryable: bool) -> RecordingLookupFailure {
    RecordingLookupFailure {
        kind: kind.into(),
        reason: reason.into(),
        status: None,
        retryable,
        retry_after_seconds: None,
        attempts: 0,
        observation: None,
    }
}

fn cached_recording(cache: Option<&CacheState>, id: &str) -> Option<ProviderRecording> {
    cache
        .and_then(|cache| cache.release_detail(RECORDING_FACTS, id))
        .and_then(|value| serde_json::from_value::<ProviderRecording>(value).ok())
        .filter(|remote| {
            (remote.requested_id == id || remote.id == id)
                && valid_musicbrainz_id(&remote.id)
                && !remote.title.trim().is_empty()
                && !remote.artist.trim().is_empty()
                && !remote.artists.is_empty()
                && remote
                    .artists
                    .iter()
                    .all(|artist| !artist.trim().is_empty())
                && remote.duration.is_finite()
                && remote.duration > 0.0
        })
}

fn previous_attempt(cache: Option<&CacheState>, id: &str) -> Option<RecordingAttempt> {
    cache
        .and_then(|cache| cache.release_detail(RECORDING_ATTEMPTS, id))
        .and_then(|value| serde_json::from_value(value).ok())
}

/// Cache raw provider facts and failed attempts, never an accepted match.
async fn recording_detail(
    id: &str,
    provider: &MusicBrainzClient,
    cache: Option<&CacheState>,
    cancelled: &AtomicBool,
    details: &mut RecordingDetails,
    requests: &mut usize,
    file_requests: &mut usize,
) -> Result<ProviderRecording, RecordingLookupFailure> {
    if let Some(result) = details.get(id) {
        return result.clone();
    }
    let previous = previous_attempt(cache, id);
    let result = if let Some(remote) = cached_recording(cache, id) {
        Ok(remote)
    } else if let Some(attempt) = previous.filter(|attempt| attempt.retry_at > now_seconds()) {
        let mut failure = attempt.failure;
        failure.retry_after_seconds = Some(attempt.retry_at.saturating_sub(now_seconds()));
        Err(failure)
    } else if *requests >= 32 || *file_requests >= 4 {
        Err(local_failure(
            "budget",
            "recording_detail_budget_limited",
            true,
        ))
    } else {
        *requests += 1;
        *file_requests += 1;
        let mut result = tokio::select! {
            result = provider.recording_by_id_result(id) => result,
            _ = wait_for_cancel(cancelled) => return Err(local_failure("cancelled", "cancelled", false)),
        };
        if let Err(error) = &mut result {
            if error.retryable && error.retry_after_seconds.is_none() {
                error.retry_after_seconds = Some(if error.status == Some(429) { 60 } else { 30 });
            }
        }
        if let Some(cache) = cache {
            let (namespace, value) = match &result {
                Ok(remote) => (RECORDING_FACTS, serde_json::to_value(remote)),
                Err(failure) => {
                    let delay = failure.retry_after_seconds.unwrap_or(if failure.retryable {
                        30
                    } else {
                        3600
                    });
                    (
                        RECORDING_ATTEMPTS,
                        serde_json::to_value(RecordingAttempt {
                            failure: failure.clone(),
                            retry_at: now_seconds().saturating_add(delay),
                        }),
                    )
                }
            };
            let value = value
                .map_err(|_| local_failure("checkpoint", "recording checkpoint invalid", false))?;
            if result.as_ref().is_err_and(|failure| failure.retryable) {
                cache
                    .set_release_detail(RECORDING_ATTEMPTS, RECORDING_PROVIDER_COOLDOWN, &value)
                    .map_err(|_| {
                        local_failure(
                            "checkpoint",
                            "provider cooldown could not be persisted",
                            false,
                        )
                    })?;
            }
            cache
                .set_release_detail(namespace, id, &value)
                .map_err(|_| {
                    local_failure(
                        "checkpoint",
                        "recording checkpoint could not be persisted",
                        false,
                    )
                })?;
        }
        result
    };
    // A skipped lookup is local to this file's quota; another file may spend
    // its own budget on the same identity in this pass.
    if result
        .as_ref()
        .is_err_and(|failure| failure.kind == "budget")
    {
        return result;
    }
    details.insert(id.into(), result.clone());
    if let Ok(remote) = &result {
        details.insert(remote.id.clone(), result.clone());
    }
    result
}

fn recording_identity_compatible(
    existing: Option<&str>,
    canonical: &str,
    details: &RecordingDetails,
) -> bool {
    existing.is_none_or(|existing| {
        existing == canonical
            || details
                .get(existing)
                .and_then(|detail| detail.as_ref().ok())
                .is_some_and(|detail| detail.id == canonical)
    })
}

fn recording_evidence(
    score: f64,
    recording: &AcoustidRecording,
    results: &[acoustid::AcoustidResult],
) -> serde_json::Value {
    serde_json::json!({
        "recordingId":recording.id, "score":score,
        "acousticDuration":recording.duration, "acousticTitle":recording.title,
        "acousticArtists":recording.artists.iter().filter_map(|artist| artist.name.as_ref()).collect::<Vec<_>>(),
        "resultIds":results.iter().filter(|result| result.recordings.iter().any(|r| r.id == recording.id)).filter_map(|result| result.id.as_ref()).collect::<BTreeSet<_>>(),
        "releaseIds":recording.releases.iter().map(|release| &release.id).collect::<BTreeSet<_>>(),
        "releaseMembershipsComplete":recording.releases_complete, "disposition":"pending"
    })
}

fn checkpoint_snapshot(
    evidence: &AcoustidEvidence,
    progress: &Mutex<AcoustidEvidence>,
) -> Result<(), String> {
    *progress
        .lock()
        .map_err(|_| "diagnostic checkpoint unavailable")? = evidence.clone();
    super::auto_tag::checkpoint_evidence(
        serde_json::json!({"acoustid":evidence,"decisionRuleVersion":3}),
    )
    .map_err(|_| "candidate evidence could not be persisted".into())
}

fn checkpoint_candidates(
    evidence: &AcoustidEvidence,
    row: &AcoustidTrackEvidence,
    index: usize,
    progress: &Mutex<AcoustidEvidence>,
) -> Result<(), String> {
    let mut snapshot = evidence.clone();
    snapshot.track_matches[index] = row.clone();
    checkpoint_snapshot(&snapshot, progress)
}

async fn wait_for_external_cancel(cancelled: &AtomicBool) {
    while !cancelled.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_inner(
    request: &LookupRequest,
    api_key: &str,
    providers: &ProviderState,
    client: &AcoustidClient,
    fpcalc: &Path,
    cancelled: &AtomicBool,
    cache: Option<&CacheState>,
    progress: &Mutex<AcoustidEvidence>,
) -> AcoustidResolution {
    let files = collect_audio_files(Path::new(&request.path));
    if files.is_empty() || request.tracks.is_empty() || files.len() != request.tracks.len() {
        return AcoustidResolution::Incomplete(
            "local audio files and readable track records do not cover the same album".into(),
        );
    }
    if files.len() > MAX_TRACKS {
        return AcoustidResolution::Incomplete("album exceeds the AcoustID track limit".into());
    }

    let metrics = ProviderRetryMetrics::default();
    let musicbrainz =
        MusicBrainzClient::at(providers.http(), providers.musicbrainz_base()).with_retry_context(
            ProviderRetryContext::new(Arc::new(AtomicBool::new(false)), metrics.clone()),
        );
    let mut fingerprints = Vec::new();
    let mut selected = Vec::new();
    let mut evidence = match progress.lock() {
        Ok(snapshot) => snapshot.clone(),
        Err(_) => {
            return AcoustidResolution::Unavailable("diagnostic checkpoint unavailable".into())
        }
    };
    let mut confirmed_empty = 0;
    let mut lookup_retryable = false;
    let mut lookup_retry_after = None;
    let mut detail_retry_after = previous_attempt(cache, RECORDING_PROVIDER_COOLDOWN)
        .filter(|attempt| attempt.retry_at > now_seconds())
        .map(|attempt| attempt.retry_at.saturating_sub(now_seconds()));
    let mut recording_details = RecordingDetails::new();
    let mut alternatives = vec![Vec::new(); files.len()];
    let mut possible_ids = vec![HashSet::new(); files.len()];
    let mut disambiguation_requests = 0usize;
    for (index, (file, local)) in files.iter().zip(&request.tracks).enumerate() {
        if cancelled.load(Ordering::Acquire) {
            return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());
        }
        let mut row = AcoustidTrackEvidence {
            file_name: Path::new(file)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            recording_id: String::new(),
            score: 0.0,
            reason: None,
            candidate_set_complete: false,
            candidates: Vec::new(),
            candidate_count: 0,
            omitted_candidates: 0,
            lookup_observation: Some(
                serde_json::json!({"version":1,"provider":"acoustid","source":local.source,"status":"not_attempted","observedAt":null,"responseUrl":null,"fullAlternativeCount":null,"eligibleHypothesisCount":null,"fingerprintScope":"fpcalc_default_provider_lookup"}),
            ),
        };
        let mut fingerprint = None;
        let mut file_requests = 0;
        let found = async {
            checkpoint_candidates(&evidence, &row, index, progress)?;
            if lookup_retry_after.is_some_and(|delay| delay > 0) {
                return Err("provider_cooldown_pending".into());
            }
            if Path::new(file).file_stem().and_then(|name| name.to_str())
                != local.filename.as_deref()
            {
                return Err("local file ordering could not be mapped to track records".to_string());
            }
            let duration = local
                .length
                .filter(|duration| duration.is_finite() && *duration > 0.0)
                .ok_or("local duration unavailable")?;
            let value = fingerprint_file_with(
                fpcalc,
                Path::new(file),
                cancelled,
                Duration::from_secs(45),
                1024 * 1024,
                64 * 1024,
            )
            .await
            .map_err(|error| error.to_string())?;
            if (f64::from(value.duration_seconds) - duration).abs() > 5.0_f64.max(duration * 0.03) {
                return Err("fingerprint duration conflicts with local audio".into());
            }
            let lookup = row.lookup_observation.as_mut().unwrap();
            lookup["fingerprintDurationSeconds"] = value.duration_seconds.into();
            { use sha2::{Digest, Sha256}; lookup["fingerprintDigest"] = format!("{:x}", Sha256::digest(value.value.as_bytes())).into(); }
            lookup["status"] = "attempted".into();
            let response = client
                .lookup(api_key, &value, cancelled)
                .await
                .map_err(|error| {
                    let lookup = row.lookup_observation.as_mut().unwrap();
                    lookup["status"] = "unsuccessful".into();
                    lookup["observedAt"] = now_seconds().into();
                    lookup["failure"] = error.to_string().into();
                    lookup_retryable |= error.retryable();
                    if let AcoustidError::HttpStatus {
                        retry_after_seconds: Some(delay),
                        ..
                    } = error
                    {
                        lookup_retry_after = Some(lookup_retry_after.unwrap_or(0).max(delay));
                    }
                    error.to_string()
                })?;
            let lookup = row.lookup_observation.as_mut().unwrap();
            lookup["responseUrl"] = response.response_url.into();
            lookup["httpStatus"] = response.http_status.into();
            lookup["responseDigest"] = response.response_digest.into();
            lookup["responseDigestBasis"] = "http_decoded_body_sha256".into();
            lookup["observedAt"] = response.observed_at.into();
            let results = response.results;
            let hypotheses = recording_hypotheses(&results, &value);
            let lookup = row.lookup_observation.as_mut().unwrap();
            lookup["status"] = if results.is_empty() {"successful_empty"} else {"successful_nonempty"}.into();
            lookup["resultCount"] = results.len().into();
            lookup["responseRecordingAssociationCount"] = results.iter().map(|r| r.recordings.len()).sum::<usize>().into();
            lookup["eligibleHypothesisCount"] = serde_json::json!(hypotheses.as_ref().ok().map(Vec::len));
            lookup["countScope"] = "parsed_lookup_response_not_universal_catalog".into();
            if let Ok(candidates) = &hypotheses {
                row.candidate_set_complete = true;
                row.candidate_count = candidates.len();
                row.omitted_candidates = candidates.len().saturating_sub(16);
                row.candidates = candidates.iter().take(16).map(|candidate| recording_evidence(candidate.score,&candidate.recording,&results)).collect();
            }
            // Retain rejected alternatives too; they are needed to explain a failed run.
            let mut raw: Vec<_> = results.iter().flat_map(|result| result.recordings.iter().map(move |recording| (result.score,recording))).collect();
            raw.sort_by(|a,b| b.0.total_cmp(&a.0).then_with(||a.1.id.cmp(&b.1.id)));
            for (score,recording) in raw {
                if row.candidates.len() == 16 { break; }
                if row.candidates.iter().any(|candidate| candidate["recordingId"] == recording.id) { continue; }
                let mut detail = recording_evidence(score,recording,&results);
                if recording.duration.is_some_and(|duration| !duration.is_finite() || duration <= 0.0 || (duration-f64::from(value.duration_seconds)).abs() > 5.0_f64.max(f64::from(value.duration_seconds)*0.03)) { detail["disposition"] = "duration_conflict".into(); }
                else if score < MINIMUM_SCORE { detail["disposition"] = "low_confidence".into(); }
                row.candidates.push(detail);
            }
            for result in results.iter().filter(|result|result.recordings.is_empty()) {
                if row.candidates.len() == 16 { break; }
                row.candidates.push(serde_json::json!({"recordingId":null,"resultIds":result.id.iter().collect::<Vec<_>>(),"score":result.score,"disposition":"recording_id_unavailable"}));
            }
            fingerprint = Some(value);
            checkpoint_candidates(&evidence, &row, index, progress)?;
            let hypothesis_failure = hypotheses.as_ref().err().copied();
            // An unidentified competitor blocks this row, but cannot hide its
            // known viable identities from cross-file audio comparisons.
            let mut candidates = hypotheses.or_else(|_| {
                let known: Vec<_> = results.iter().filter(|result| !result.recordings.is_empty() && result.score.is_finite() && (0.0..=1.0).contains(&result.score)).cloned().collect();
                recording_hypotheses(&known, fingerprint.as_ref().unwrap())
            }).map_err(str::to_string)?;
            if candidates.is_empty() { if let Some(reason)=hypothesis_failure {return Err(reason.into());} }
            if candidates.is_empty() { return Ok(None); }
            // A display cap is not a decision cap. Cached facts are free; unseen IDs
            // precede deferred failures so a bad first candidate cannot starve later ones.
            candidates.sort_by_cached_key(|candidate| {
                let id = &candidate.recording.id;
                if recording_details.get(id).is_some_and(Result::is_ok) || cached_recording(cache, id).is_some() { (0, 0, id.clone()) }
                else if let Some(attempt) = previous_attempt(cache, id) { (2, attempt.retry_at, id.clone()) }
                else { (1, 0, id.clone()) }
            });
            for candidate in &candidates {
                if let Some(attempt)=previous_attempt(cache,&candidate.recording.id).filter(|attempt| attempt.failure.retryable && attempt.retry_at>now_seconds()) {
                    detail_retry_after=detail_retry_after.max(Some(attempt.retry_at.saturating_sub(now_seconds())));
                }
            }
            let mut survivors = Vec::new();
            let mut unknown = false;
            let mut budget_limited = false;
            for mut candidate in candidates {
                let candidate_index = row.candidates.iter().position(|entry| entry["recordingId"] == candidate.recording.id);
                let mut observation = recording_evidence(candidate.score, &candidate.recording, &results);
                if !valid_musicbrainz_id(&candidate.recording.id) { unknown = true; row.candidate_set_complete = false; if let Some(i)=candidate_index {row.candidates[i]["disposition"] = "invalid_identity".into();} continue; }
                let known = recording_details.contains_key(&candidate.recording.id)
                    || cached_recording(cache,&candidate.recording.id).is_some()
                    || previous_attempt(cache,&candidate.recording.id).is_some_and(|attempt|attempt.retry_at>now_seconds());
                let detail = if !known && detail_retry_after.is_some_and(|delay|delay>0) {
                    let mut failure=local_failure("cooldown","provider_cooldown_pending",true);
                    failure.retry_after_seconds=detail_retry_after;
                    Err(failure)
                } else {
                    recording_detail(&candidate.recording.id,&musicbrainz,cache,cancelled,&mut recording_details,&mut disambiguation_requests,&mut file_requests).await
                };
                match detail {
                    Ok(remote) => {
                        observation["providerObservation"] = serde_json::json!(remote.observation);
                        observation["authoritativeTitle"] = remote.title.clone().into();
                        observation["disambiguation"] = serde_json::json!(remote.disambiguation);
                        observation["authoritativeDuration"] = remote.duration.into();
                        observation["artistCredits"] = serde_json::json!(remote.artists);
                        observation["canonicalRecordingId"] = remote.id.clone().into();
                        if (remote.duration - local.length.unwrap_or(0.0)).abs() > 5.0_f64.max(remote.duration * 0.03) {
                            observation["disposition"] = "duration_conflict".into();
                        } else {
                            candidate.recording.id = remote.id;
                            possible_ids[index].insert(candidate.recording.id.clone());
                            observation["disposition"] = "supported".into();
                            survivors.push(candidate);
                        }
                    },
                    Err(error) => {
                        unknown = true;
                        row.candidate_set_complete = false;
                        possible_ids[index].insert(candidate.recording.id.clone());
                        budget_limited |= error.kind == "budget";
                        lookup_retryable |= error.retryable;
                        if error.retryable { detail_retry_after = detail_retry_after.max(error.retry_after_seconds); }
                        observation["detailFailure"] = serde_json::to_value(&error).unwrap_or_default();
                        observation["disposition"] = error.reason.into();
                    }
                }
                if let Some(i) = candidate_index { row.candidates[i] = observation; }
                checkpoint_candidates(&evidence, &row, index, progress)?;
            }
            if unknown { return Err(if budget_limited {"recording_candidate_budget_limited"} else {"recording_candidates_incomplete"}.into()); }
            survivors.sort_by(|a,b| a.recording.id.cmp(&b.recording.id));
            survivors.dedup_by(|incoming,retained| {
                if incoming.recording.id != retained.recording.id { return false; }
                merge_memberships(&mut retained.recording,&incoming.recording);
                retained.score=retained.score.max(incoming.score);
                true
            });
            if let Some(reason) = hypothesis_failure {return Err(reason.into());}
            alternatives[index] = survivors.clone();
            if survivors.len() == 1 {
                let selected=survivors.pop().unwrap();
                if selected.score < MINIMUM_SCORE { Err("AcoustID match confidence is below the safe threshold".into()) } else { Ok(Some(selected)) }
            } else { Err("AcoustID returned tied recording identities".into()) }
        }
        .await;
        match found {
            Ok(Some(recording)) => {
                row.recording_id = recording.recording.id.clone();
                row.score = recording.score;
                selected.push(Some(recording));
            }
            Ok(None) => {
                confirmed_empty += 1;
                selected.push(None);
                row.reason = Some("no_matching_recording".into());
            }
            Err(reason) => {
                selected.push(None);
                row.reason = Some(reason);
            }
        }
        fingerprints.push(fingerprint);
        evidence.track_matches[index] = row;
        if let Err(error) = checkpoint_snapshot(&evidence, progress) {
            return AcoustidResolution::Unavailable(error);
        }
    }
    if confirmed_empty == files.len() {
        evidence.validation_complete = true;
        if let Err(reason) = checkpoint_snapshot(&evidence, progress) {
            return AcoustidResolution::Unavailable(reason);
        }
        return AcoustidResolution::NoMatch;
    }

    // Later files may resolve aliases that earlier quota-limited rows retained
    // as requested IDs. Compare all physical files in the final canonical space.
    for ids in &mut possible_ids {
        *ids = ids
            .drain()
            .map(|id| {
                recording_details
                    .get(&id)
                    .and_then(|detail| detail.as_ref().ok())
                    .map_or(id.clone(), |remote| remote.id.clone())
            })
            .collect();
    }
    let mut raw_fingerprints = vec![None; files.len()];
    let mut recording_counts = HashMap::<&str, usize>::new();
    for ids in &possible_ids {
        for id in ids {
            *recording_counts.entry(id).or_default() += 1;
        }
    }
    for (index, ids) in possible_ids.iter().enumerate() {
        if ids.iter().any(|id| recording_counts[id.as_str()] > 1) {
            raw_fingerprints[index] = crate::infra::acoustid::raw_fingerprint_file_with(
                fpcalc,
                Path::new(&files[index]),
                cancelled,
            )
            .await
            .ok();
        }
    }
    let mut incomplete_comparisons = HashSet::new();
    for (index, ids) in possible_ids.iter().enumerate() {
        if raw_fingerprints[index].is_none()
            && fingerprints[index]
                .as_ref()
                .is_some_and(|fp| fp.duration_seconds >= 120)
        {
            incomplete_comparisons.extend(
                ids.iter()
                    .filter(|id| recording_counts[id.as_str()] > 1)
                    .cloned(),
            );
        }
    }
    let (group_indices, _) = recording_groups(&selected, &fingerprints, &raw_fingerprints);
    let conflicting_groups =
        possible_audio_conflicts(&possible_ids, &fingerprints, &raw_fingerprints);
    let groups = if conflicting_groups.is_empty() {
        Ok(group_indices)
    } else {
        Err(())
    };
    let mut accepted = Vec::new();
    let mut reason_code = "partial_collection";
    let mut fetch_failed = false;
    if let Ok(indices) = &groups {
        if selected.iter().enumerate().all(|(index, row)| {
            row.as_ref().is_some_and(|row| {
                row.recording.releases_complete
                    && recording_details
                        .get(&row.recording.id)
                        .and_then(|detail| detail.as_ref().ok())
                        .is_some_and(|remote| {
                            recording_versions_compatible(
                                &request.tracks[index],
                                &remote.title,
                                remote.disambiguation.as_deref(),
                            )
                        })
            })
        }) {
            let mut common: BTreeSet<String> = selected[0]
                .as_ref()
                .unwrap()
                .recording
                .releases
                .iter()
                .map(|release| release.id.clone())
                .collect();
            for recording in selected.iter().flatten().skip(1) {
                let releases: BTreeSet<_> = recording
                    .recording
                    .releases
                    .iter()
                    .map(|release| &release.id)
                    .collect();
                common.retain(|id| releases.contains(id));
            }
            evidence.candidate_count = common.len();
            {
                let mut unique_request = request.clone();
                unique_request.tracks = indices
                    .iter()
                    .map(|index| {
                        let mut track = request.tracks[*index].clone();
                        // Validate the special filename against its original
                        // artist before removing representative-only credits.
                        if let Some(remote) = selected[*index]
                            .as_ref()
                            .and_then(|selected| recording_details.get(&selected.recording.id))
                            .and_then(|detail| detail.as_ref().ok())
                        {
                            if track.filename.as_deref().is_some_and(|name| {
                                !versions_compatible_with_disambiguation(
                                    name,
                                    &remote.title,
                                    remote.disambiguation.as_deref(),
                                )
                            }) && recording_versions_compatible(
                                &track,
                                &remote.title,
                                remote.disambiguation.as_deref(),
                            ) {
                                track.filename = track.title.clone();
                            }
                        }
                        // Credits belong to each physical file, not to its
                        // representative in an acoustic duplicate group.
                        track.artist = None;
                        track.artists.clear();
                        track
                    })
                    .collect();
                let ids: Vec<_> = indices
                    .iter()
                    .map(|index| selected[*index].as_ref().unwrap().recording.id.clone())
                    .collect();
                for (local, canonical) in unique_request.tracks.iter_mut().zip(&ids) {
                    if local.musicbrainz_track_id.is_some()
                        && recording_identity_compatible(
                            local.musicbrainz_track_id.as_deref(),
                            canonical,
                            &recording_details,
                        )
                    {
                        local.musicbrainz_track_id = Some(canonical.clone());
                    }
                }
                let mut network_details = 0usize;
                let mut ranked: Vec<_> = common.into_iter().collect();
                ranked.sort_by_key(|id| {
                    (
                        request.musicbrainz_album_id.as_ref() != Some(id),
                        id.clone(),
                    )
                });
                for id in ranked {
                    if !valid_musicbrainz_id(&id) {
                        evidence.rejections.push("invalid_release_identity".into());
                        fetch_failed = true;
                        continue;
                    }
                    let cached = cache
                        .and_then(|cache| cache.release_detail("acoustid-release-v2", &id))
                        .and_then(|value| {
                            serde_json::from_value::<crate::state::providers::ProviderAlbum>(value)
                                .ok()
                        })
                        .filter(|release| release.id == id);
                    let response = if let Some(cached) = cached {
                        Ok(cached)
                    } else if network_details >= MAX_RELEASE_CANDIDATES {
                        evidence.pending_candidates.push(id);
                        continue;
                    } else {
                        network_details += 1;
                        let result = tokio::select! { result = musicbrainz.release_by_id_result(&id) => result, _ = wait_for_cancel(cancelled) => return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string()) };
                        if let (Some(cache), Ok(release)) = (cache, &result) {
                            if release.id == id {
                                let persisted =
                                    serde_json::to_value(release).ok().and_then(|value| {
                                        cache
                                            .set_release_detail("acoustid-release-v2", &id, &value)
                                            .ok()
                                    });
                                if persisted.is_none() {
                                    return AcoustidResolution::Incomplete(
                                        "edition checkpoint could not be persisted".into(),
                                    );
                                }
                            }
                        }
                        result
                    };
                    evidence.checked_candidates += 1;
                    let mut release = match response {
                        Ok(release) if release.id == id => release,
                        _ => {
                            fetch_failed = true;
                            evidence
                                .rejections
                                .push(format!("release_detail_unavailable:{id}"));
                            continue;
                        }
                    };
                    // Decorate only this evaluation copy. The durable cache keeps raw provider data.
                    for track in &mut release.tracks {
                        if let Some((local, detail, title)) =
                            track.recording_id.as_ref().and_then(|id| {
                                let local = unique_request
                                    .tracks
                                    .iter()
                                    .zip(&ids)
                                    .find(|(_, canonical)| *canonical == id)?
                                    .0;
                                let detail = recording_details.get(id)?.as_ref().ok()?;
                                let title = track.title.as_deref()?;
                                Some((local, detail, title))
                            })
                        {
                            if crate::infra::text::comparison_text(title).to_lowercase()
                                == crate::infra::text::comparison_text(&detail.title).to_lowercase()
                                && recording_versions_compatible(
                                    local,
                                    &detail.title,
                                    detail.disambiguation.as_deref(),
                                )
                            {
                                track.title = Some(recording_display_title(local, title));
                            }
                        }
                    }
                    match accept_acoustid_release(
                        &unique_request,
                        &musicbrainz_candidate(release),
                        &ids,
                    ) {
                        Ok(mut candidate) => {
                            // Expand the acoustic recording groups back to every physical file.
                            let mapped = candidate.accepted_match.as_ref().unwrap().tracks.clone();
                            let by_id: HashMap<_, _> = mapped
                                .into_iter()
                                .map(|track| (track.musicbrainz_track_id.clone().unwrap(), track))
                                .collect();
                            let mut expanded: Vec<_> = selected
                                .iter()
                                .zip(&request.tracks)
                                .map(|(recording, local)| {
                                    let mut track =
                                        by_id[&recording.as_ref().unwrap().recording.id].clone();
                                    track.filename = local.filename.clone();
                                    track
                                })
                                .collect();
                            if expanded.iter().zip(&request.tracks).any(|(remote, local)| {
                                !recording_versions_compatible(
                                    local,
                                    remote.title.as_deref().unwrap_or_default(),
                                    None,
                                ) || !recording_identity_compatible(
                                    local.musicbrainz_track_id.as_deref(),
                                    remote.musicbrainz_track_id.as_deref().unwrap_or_default(),
                                    &recording_details,
                                ) || local
                                    .tagged_track_number
                                    .is_some_and(|number| Some(number) != remote.track_number)
                                    || local
                                        .disc_number
                                        .is_some_and(|number| Some(number) != remote.disc_number)
                            }) {
                                evidence
                                    .rejections
                                    .push("duplicate_member_identity_conflict".into());
                                continue;
                            }
                            super::auto_tag::editions::preserve_collaborators(
                                &request.tracks,
                                &mut expanded,
                            );
                            candidate.accepted_match.as_mut().unwrap().tracks = expanded;
                            candidate.accepted_match.as_mut().unwrap().coverage = files.len();
                            accepted.push(candidate);
                        }
                        Err(reason) => evidence.rejections.push(format!("{id}:{reason}")),
                    }
                }
                if !evidence.pending_candidates.is_empty() {
                    reason_code = "discovery_budget_limited";
                    fetch_failed = true;
                    evidence.rejections.push(
                        "edition candidates remain unexamined; retry resumes cached details".into(),
                    );
                }
                if accepted.len() == 1 && !fetch_failed {
                    let candidate = accepted.remove(0);
                    evidence.release_id =
                        candidate.musicbrainz_album_id.clone().unwrap_or_default();
                    evidence.validation_complete = true;
                    if let Err(reason) = checkpoint_snapshot(&evidence, progress) {
                        return AcoustidResolution::Unavailable(reason);
                    }
                    return AcoustidResolution::Candidate {
                        candidate: Box::new(candidate),
                        evidence,
                    };
                }
                if accepted.len() > 1 && !fetch_failed {
                    reason_code = "ambiguous_edition";
                }
            }
        }
    } else {
        reason_code = "recording_conflict";
        evidence
            .rejections
            .push("shared recording identity has conflicting or incomplete audio evidence".into());
    }
    let shared_album = if !fetch_failed && accepted.len() > 1 {
        shared_album_fields(&accepted).map(Box::new)
    } else {
        None
    };
    // A release conflict does not erase strong recording evidence. Fetch the
    // authoritative recording text independently; omit all unresolved fields.
    let mut verified = recording_details;
    let mut tracks: Vec<_> = request
        .tracks
        .iter()
        .map(|local| TrackCandidate {
            source: local.source.clone(),
            filename: local.filename.clone(),
            ..TrackCandidate::default()
        })
        .collect();
    for (index, recording) in selected.iter().enumerate() {
        let Some(recording) = recording else {
            continue;
        };
        let id = &recording.recording.id;
        if conflicting_groups.contains(id) {
            evidence.track_matches[index].reason = Some(
                if incomplete_comparisons.contains(id) {
                    "duplicate_comparison_incomplete"
                } else {
                    "recording_audio_conflict"
                }
                .into(),
            );
            continue;
        }
        if !valid_musicbrainz_id(id) {
            evidence.track_matches[index].reason = Some("invalid_recording_identity".into());
            continue;
        }
        let response = recording_detail(
            id,
            &musicbrainz,
            cache,
            cancelled,
            &mut verified,
            &mut disambiguation_requests,
            &mut 0,
        )
        .await;
        let remote = match response {
            Ok(remote) => {
                verified.insert(id.clone(), Ok(remote.clone()));
                remote
            }
            Err(error) => {
                fetch_failed = true;
                evidence.track_matches[index].candidate_set_complete = false;
                for candidate in &mut evidence.track_matches[index].candidates {
                    candidate["disposition"] = error.reason.clone().into();
                }
                evidence.track_matches[index].reason = Some("recording_detail_unavailable".into());
                continue;
            }
        };
        let local = &request.tracks[index];
        let local_artists =
            super::auto_tag::split_collaborative_artists(&local.artist, &local.artists);
        let credits_safe = local_artists.len() <= 1
            || local_artists.iter().all(|artist| {
                remote.artists.iter().any(|credit| {
                    crate::infra::text::comparison_text(artist).to_lowercase()
                        == crate::infra::text::comparison_text(credit).to_lowercase()
                })
            });
        let version_safe =
            recording_versions_compatible(local, &remote.title, remote.disambiguation.as_deref());
        if !credits_safe
            || !version_safe
            || !recording_identity_compatible(
                local.musicbrainz_track_id.as_deref(),
                &remote.id,
                &verified,
            )
            || (local.length.unwrap_or(0.0) - remote.duration).abs()
                > 5.0_f64.max(remote.duration * 0.03)
        {
            evidence.track_matches[index].reason = Some(
                if !credits_safe {
                    "recording_credit_conflict"
                } else if !version_safe {
                    "recording_version_conflict"
                } else if !recording_identity_compatible(
                    local.musicbrainz_track_id.as_deref(),
                    &remote.id,
                    &verified,
                ) {
                    "existing_recording_id_conflict"
                } else {
                    "recording_duration_conflict"
                }
                .into(),
            );
            continue;
        }
        if !evidence.track_matches[index]
            .candidates
            .iter()
            .any(|candidate| {
                candidate["recordingId"] == *id || candidate["canonicalRecordingId"] == remote.id
            })
        {
            let row = &mut evidence.track_matches[index];
            if row.candidates.len() == 16 {
                row.candidates.pop();
            }
            row.candidates.push(serde_json::json!({"recordingId":id,"canonicalRecordingId":remote.id,"score":recording.score,"authoritativeTitle":remote.title,"authoritativeDuration":remote.duration,"artistCredits":remote.artists,"disambiguation":remote.disambiguation,"disposition":"verified"}));
        }
        for candidate in &mut evidence.track_matches[index].candidates {
            if candidate["recordingId"] == *id || candidate["canonicalRecordingId"] == remote.id {
                candidate["providerObservation"] = serde_json::json!(remote.observation);
                candidate["canonicalRecordingId"] = remote.id.clone().into();
                candidate["authoritativeTitle"] = remote.title.clone().into();
                candidate["disambiguation"] = serde_json::json!(remote.disambiguation);
                candidate["authoritativeDuration"] = remote.duration.into();
                candidate["artistCredits"] = serde_json::json!(remote.artists);
                candidate["disposition"] = "verified".into();
            }
        }
        evidence.track_matches[index].recording_id = remote.id.clone();
        tracks[index] = TrackCandidate {
            source: local.source.clone(),
            title: Some(recording_display_title(local, &remote.title)),
            artist: Some(remote.artist),
            artists: remote.artists,
            musicbrainz_track_id: Some(remote.id),
            filename: local.filename.clone(),
            length: local.length,
            ..TrackCandidate::default()
        };
    }
    for (index, candidates) in alternatives.iter().enumerate() {
        if possible_ids[index]
            .iter()
            .any(|id| conflicting_groups.contains(id))
        {
            evidence.track_matches[index].reason = Some(
                if possible_ids[index]
                    .iter()
                    .any(|id| incomplete_comparisons.contains(id))
                {
                    "duplicate_comparison_incomplete"
                } else {
                    "recording_audio_conflict"
                }
                .into(),
            );
            continue;
        }
        if selected[index].is_some() || !evidence.track_matches[index].candidate_set_complete {
            continue;
        }
        if let Some(remote) = common_recording_text(&request.tracks[index], candidates, &verified) {
            let local = &request.tracks[index];
            tracks[index] = TrackCandidate {
                source: local.source.clone(),
                filename: local.filename.clone(),
                length: local.length,
                title: Some(recording_display_title(local, &remote.title)),
                artist: Some(remote.artist),
                artists: remote.artists,
                ..TrackCandidate::default()
            };
            evidence.track_matches[index].reason = Some("recording_fields_consensus".into());
            evidence.track_matches[index].score = candidates
                .iter()
                .map(|candidate| candidate.score)
                .fold(1.0, f64::min);
        }
    }
    if lookup_retryable || (fetch_failed && metrics.transient_failure()) {
        reason_code = "provider_retryable";
    }
    evidence.validation_complete = true;
    if let Err(reason) = checkpoint_snapshot(&evidence, progress) {
        return AcoustidResolution::Unavailable(reason);
    }
    AcoustidResolution::Recordings {
        tracks,
        shared_album,
        evidence,
        reason_code: reason_code.into(),
        retryable: lookup_retryable || metrics.transient_failure(),
        retry_after_seconds: lookup_retry_after
            .max(detail_retry_after)
            .max(metrics.max_retry_after_seconds()),
    }
}

/// Calibrated on copied encoding pairs. Require both full-span and local-window
/// agreement so a shared intro or localized edit cannot pass on mean alone.
fn acoustic_frames_match(left: &[u32], right: &[u32]) -> bool {
    let overlap = left.len().min(right.len());
    if overlap < 120
        || overlap * 100 < left.len().max(right.len()) * 99
        || left.iter().copied().collect::<HashSet<_>>().len() < 8
        || right.iter().copied().collect::<HashSet<_>>().len() < 8
    {
        return false;
    }
    (-3_i32..=3).any(|offset| {
        let (a, b) = if offset < 0 {
            (&left[(-offset) as usize..], right)
        } else {
            (left, &right[offset as usize..])
        };
        let count = a.len().min(b.len());
        let errors: Vec<_> = a
            .iter()
            .zip(b)
            .map(|(a, b)| (a ^ b).count_ones() as usize)
            .collect();
        errors.iter().sum::<usize>() * 100 <= count * 32 * 2
            && errors
                .windows(64)
                .step_by(32)
                .all(|window| window.iter().sum::<usize>() * 100 <= 64 * 32 * 5)
    })
}

fn fingerprints_match(
    a: usize,
    b: usize,
    fingerprints: &[Option<Fingerprint>],
    raw: &[Option<Vec<u32>>],
) -> bool {
    fingerprints[a]
        .as_ref()
        .zip(fingerprints[b].as_ref())
        .is_some_and(|(left, right)| {
            left.duration_seconds.abs_diff(right.duration_seconds) <= 2
                && raw[a].as_ref().zip(raw[b].as_ref()).map_or(
                    left.duration_seconds < 120
                        && right.duration_seconds < 120
                        && left.value == right.value,
                    |(left, right)| acoustic_frames_match(left, right),
                )
        })
}

fn possible_audio_conflicts(
    ids: &[HashSet<String>],
    fingerprints: &[Option<Fingerprint>],
    raw: &[Option<Vec<u32>>],
) -> HashSet<String> {
    let mut seen = HashMap::<&str, Vec<usize>>::new();
    let mut conflicts = HashSet::new();
    for (index, ids) in ids.iter().enumerate() {
        for id in ids {
            let previous = seen.entry(id).or_default();
            if previous
                .iter()
                .any(|other| !fingerprints_match(*other, index, fingerprints, raw))
            {
                conflicts.insert(id.clone());
            }
            previous.push(index);
        }
    }
    conflicts
}

fn common_recording_text(
    local: &TrackCandidate,
    candidates: &[SelectedRecording],
    details: &RecordingDetails,
) -> Option<ProviderRecording> {
    if candidates.len() < 2
        || local.musicbrainz_track_id.is_some()
        || candidates
            .iter()
            .any(|candidate| candidate.score < MINIMUM_SCORE)
    {
        return None;
    }
    let duration = local
        .length
        .filter(|duration| duration.is_finite() && *duration > 0.0)?;
    let first = details.get(&candidates[0].recording.id)?.as_ref().ok()?;
    let credits = super::auto_tag::split_collaborative_artists(&local.artist, &local.artists);
    for candidate in candidates {
        let remote = details.get(&candidate.recording.id)?.as_ref().ok()?;
        if !valid_musicbrainz_id(&remote.id)
            || remote.title != first.title
            || remote.artist != first.artist
            || remote.artists != first.artists
            || remote.disambiguation != first.disambiguation
            || (duration - remote.duration).abs() > 5.0_f64.max(remote.duration * 0.03)
            || !recording_versions_compatible(
                local,
                &remote.title,
                remote.disambiguation.as_deref(),
            )
            || !consensus_qualifiers_compatible(local, remote)
            || (credits.len() > 1
                && !credits.iter().all(|artist| {
                    remote.artists.iter().any(|credit| {
                        crate::infra::text::comparison_text(artist).to_lowercase()
                            == crate::infra::text::comparison_text(credit).to_lowercase()
                    })
                }))
        {
            return None;
        }
    }
    let mut shared = first.clone();
    shared.id.clear();
    shared.requested_id.clear();
    Some(shared)
}

fn recording_groups(
    selected: &[Option<SelectedRecording>],
    fingerprints: &[Option<Fingerprint>],
    raw: &[Option<Vec<u32>>],
) -> (Vec<usize>, HashSet<String>) {
    let mut indices = Vec::new();
    let mut seen = HashMap::<&str, Vec<usize>>::new();
    let mut conflicts = HashSet::new();
    for (index, recording) in selected.iter().enumerate() {
        let Some(recording) = recording else {
            continue;
        };
        if let Some(previous) = seen.get_mut(recording.recording.id.as_str()) {
            let matches = previous
                .iter()
                .all(|previous| fingerprints_match(*previous, index, fingerprints, raw));
            previous.push(index);
            if !matches {
                conflicts.insert(recording.recording.id.clone());
            }
        } else {
            seen.insert(&recording.recording.id, vec![index]);
            indices.push(index);
        }
    }
    (indices, conflicts)
}

#[cfg(test)]
fn unique_recording_indices(
    selected: &[Option<SelectedRecording>],
    fingerprints: &[Option<Fingerprint>],
) -> Result<Vec<usize>, ()> {
    let (indices, conflicts) =
        recording_groups(selected, fingerprints, &vec![None; selected.len()]);
    if conflicts.is_empty() {
        Ok(indices)
    } else {
        Err(())
    }
}

fn shared_album_fields(candidates: &[AlbumCandidate]) -> Option<AlbumCandidate> {
    let first = candidates.first()?;
    if first.album.is_none()
        || first.album_artists.is_empty()
        || candidates.iter().any(|candidate| {
            candidate.album != first.album || candidate.album_artists != first.album_artists
        })
    {
        return None;
    }
    Some(AlbumCandidate {
        album: first.album.clone(),
        album_artist: first.album_artist.clone(),
        album_artists: first.album_artists.clone(),
        ..AlbumCandidate::default()
    })
}

async fn wait_for_cancel(cancelled: &AtomicBool) {
    while !cancelled.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn valid_musicbrainz_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn version_words(value: &str) -> String {
    crate::infra::text::comparison_text(value)
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn recording_versions_compatible(
    local: &TrackCandidate,
    remote: &str,
    disambiguation: Option<&str>,
) -> bool {
    if local
        .title
        .iter()
        .any(|title| !versions_compatible_with_disambiguation(title, remote, disambiguation))
    {
        return false;
    }
    let Some(filename) = local.filename.as_deref() else {
        return true;
    };
    if versions_compatible_with_disambiguation(filename, remote, disambiguation) {
        return true;
    }
    let Some(title) = local.title.as_deref() else {
        return false;
    };
    if crate::infra::text::comparison_text(title).to_lowercase()
        != crate::infra::text::comparison_text(remote).to_lowercase()
        || filename.matches(" _ ").count() != 1
    {
        return false;
    }
    let brackets = regex::Regex::new(r"[\[(]([^\])]+)[\])]").unwrap();
    let named: Vec<_> = brackets
        .captures_iter(remote)
        .filter(|capture| {
            capture[1].contains(" & ")
                && capture[1]
                    .split_whitespace()
                    .any(|word| word.eq_ignore_ascii_case("remix"))
        })
        .collect();
    if named.len() != 1 {
        return false;
    }
    let expected = &named[0][1];
    let changed = filename.replacen(" _ ", " & ", 1);
    if !brackets.captures_iter(&changed).any(|capture| {
        crate::infra::text::comparison_text(&capture[1]).to_lowercase()
            == crate::infra::text::comparison_text(expected).to_lowercase()
    }) {
        return false;
    }
    let full = version_words(&changed);
    let expected = version_words(title);
    (full == expected
        || full.strip_suffix(&expected).is_some_and(|prefix| {
            prefix.ends_with(' ')
                && local
                    .artist
                    .as_ref()
                    .is_some_and(|artist| version_words(artist) == prefix.trim())
        }))
        && versions_compatible_with_disambiguation(&changed, remote, disambiguation)
}

// Ambiguous identities must not discard local qualifiers that the historical
// unique-ID comparator does not recognize (for example a dub or radio edit).
fn consensus_qualifiers_compatible(local: &TrackCandidate, remote: &ProviderRecording) -> bool {
    let version_markers = [
        "dub",
        "edit",
        "mix",
        "remix",
        "acoustic",
        "extended",
        "version",
        "live",
        "demo",
        "remaster",
        "instrumental",
        "mono",
        "stereo",
    ];
    // Check the actual written title, not annotation words that could disappear
    // from the patch. Full phrases prevent Radio Edit matching Extended Edit.
    let artist_prefix_matches = |prefix: &str, leading_credit: bool| {
        prefix.ends_with(' ')
            && (local
                .artist
                .as_ref()
                .is_some_and(|artist| version_words(artist) == prefix.trim())
                || leading_credit
                    && local
                        .artists
                        .first()
                        .is_some_and(|artist| version_words(artist) == prefix.trim()))
    };
    let written = recording_display_title(local, &remote.title);
    let live_title = format!("{} live", version_words(&remote.title));
    // The caller requires unanimous authoritative annotations. Only this
    // affirmative live form may support a literal Live suffix kept on disk.
    let retained_live = remote.disambiguation.as_deref().is_some_and(|annotation| {
        let annotation = annotation.trim().to_lowercase();
        let annotation_words = version_words(&annotation)
            .replace("a cappella", "acapella")
            .replace("a capella", "acapella");
        annotation.starts_with("live, from ")
            && !annotation_words.split_whitespace().any(|word| {
                matches!(
                    word,
                    "no" | "not" | "non" | "studio" | "acapella" | "clean" | "explicit"
                ) || version_markers
                    .iter()
                    .any(|marker| *marker != "live" && *marker == word)
            })
    }) && local
        .title
        .as_deref()
        .is_some_and(|title| version_words(title) == live_title)
        && local.filename.as_deref().is_some_and(|filename| {
            let words = version_words(filename);
            words == live_title
                || words
                    .strip_suffix(&live_title)
                    .is_some_and(|prefix| artist_prefix_matches(prefix, true))
        })
        && written
            .strip_prefix(&remote.title)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(" (Live)"));
    let remote_words = version_words(if retained_live {
        &written
    } else {
        &remote.title
    });
    let context = format!(" {remote_words} ");
    let brackets = regex::Regex::new(r"[\[(]([^\])]+)[\])]").unwrap();
    for source in local.title.iter().chain(local.filename.iter()) {
        let words = version_words(source);
        let unbracketed = format!(" {} ", version_words(&brackets.replace_all(source, " ")));
        if version_markers
            .iter()
            .any(|marker| unbracketed.contains(&format!(" {marker} ")))
        {
            let full_agreement = words == remote_words
                || words
                    .strip_suffix(&remote_words)
                    .is_some_and(|prefix| artist_prefix_matches(prefix, retained_live));
            if !full_agreement {
                return false;
            }
        }
        if brackets
            .captures_iter(source)
            .any(|capture| !context.contains(&format!(" {} ", version_words(&capture[1]))))
        {
            return false;
        }
    }
    true
}

/// Called only after the provider has supported every local version qualifier.
fn recording_display_title(local: &TrackCandidate, remote: &str) -> String {
    if recording_versions_compatible(local, remote, None) {
        return remote.into();
    }
    let Some(source) = local
        .title
        .iter()
        .chain(local.filename.iter())
        .find(|title| !versions_compatible(title, remote))
    else {
        return remote.into();
    };
    let source = crate::infra::text::comparison_text(source);
    let qualifiers = regex::Regex::new(r"[\[(]([^\])]+)[\])]")
        .unwrap()
        .captures_iter(&source)
        .filter_map(|capture| {
            (!versions_compatible(&capture[1], "")).then(|| format!("({})", &capture[1]))
        })
        .collect::<Vec<_>>();
    if qualifiers.is_empty() {
        remote.into()
    } else {
        format!("{remote} {}", qualifiers.join(" "))
    }
}

fn versions_compatible_with_disambiguation(
    local: &str,
    remote: &str,
    disambiguation: Option<&str>,
) -> bool {
    if versions_compatible(local, remote) {
        return true;
    }
    let Some(disambiguation) = disambiguation else {
        return false;
    };
    let prepare = |value: &str| {
        crate::infra::text::comparison_text(value)
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let marker =
        regex::Regex::new(r"(?i)\b(live|remix|demo|instrumental|acapella|clean|explicit)\b")
            .unwrap();
    // An explicit contradictory title cannot be repaired by an unrelated annotation.
    if marker.is_match(&crate::infra::text::comparison_text(remote)) {
        return false;
    }
    let brackets = regex::Regex::new(r"[\[(]([^\])]+)[\])]").unwrap();
    let local = crate::infra::text::comparison_text(local);
    let context = format!(" {} ", prepare(disambiguation));
    let qualifiers: Vec<_> = brackets
        .captures_iter(&local)
        .filter_map(|capture| {
            let value = capture[1].to_owned();
            marker.is_match(&value).then_some(value)
        })
        .collect();
    !qualifiers.is_empty()
        && qualifiers
            .iter()
            .all(|qualifier| context.contains(&format!(" {} ", prepare(qualifier))))
        && versions_compatible(&local, &format!("{} ({})", remote, qualifiers.join(") (")))
}

pub(crate) fn versions_compatible(local: &str, remote: &str) -> bool {
    let local = crate::infra::text::comparison_text(local);
    let remote = crate::infra::text::comparison_text(remote);
    let (local, remote) = (local.as_str(), remote.as_str());
    let markers = |value: &str| {
        let value = value
            .to_lowercase()
            .replace("a cappella", "acapella")
            .replace("a capella", "acapella");
        [
            "remix",
            "live",
            "demo",
            "instrumental",
            "acapella",
            "clean",
            "explicit",
        ]
        .into_iter()
        .filter(|marker| {
            value
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| word == *marker)
        })
        .collect::<BTreeSet<_>>()
    };
    let local_markers = markers(local);
    if local_markers.is_empty() {
        return true;
    }
    if local_markers != markers(remote) {
        return false;
    }
    // A named remix/demo is part of recording identity, not an edition suffix.
    let qualifiers = |title: &str| {
        regex::Regex::new(r"[\[(]([^\])]+)[\])]")
            .unwrap()
            .captures_iter(title)
            .filter_map(|capture| {
                let qualifier = &capture[1];
                (!markers(qualifier).is_empty())
                    .then(|| crate::infra::text::comparison_text(qualifier).to_lowercase())
            })
            .collect::<BTreeSet<_>>()
    };
    let brackets = regex::Regex::new(r"[\[(]([^\])]+)[\])]").unwrap();
    let outside_local = brackets.replace_all(local, " ");
    let outside_remote = brackets.replace_all(remote, " ");
    if markers(&outside_local).is_empty() && markers(&outside_remote).is_empty() {
        let local_qualifiers = qualifiers(local);
        return local_qualifiers.is_empty() || local_qualifiers == qualifiers(remote);
    }
    // Mixed and unbracketed names carry evidence too. Compare their full
    // normalized spelling, tolerating only an additional filename prefix.
    let normalized = |value: &str| {
        crate::infra::text::comparison_text(value)
            .chars()
            .flat_map(char::to_lowercase)
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let local = normalized(local);
    let remote = normalized(remote);
    local == remote
        || local
            .strip_suffix(&remote)
            .is_some_and(|prefix| prefix.ends_with(' '))
        || remote
            .strip_suffix(&local)
            .is_some_and(|prefix| prefix.ends_with(' '))
}

fn merge_memberships(retained: &mut AcoustidRecording, incoming: &AcoustidRecording) {
    retained.releases.extend(incoming.releases.iter().cloned());
    retained.releases.sort_by(|a, b| a.id.cmp(&b.id));
    retained.releases.dedup_by(|a, b| a.id == b.id);
    retained.releases_complete &= incoming.releases_complete && retained.releases.len() <= 100;
    retained.releases.truncate(100);
}

fn recording_hypotheses(
    results: &[acoustid::AcoustidResult],
    fingerprint: &Fingerprint,
) -> Result<Vec<SelectedRecording>, &'static str> {
    if results.is_empty() {
        return Ok(Vec::new());
    }
    let mut best = HashMap::<String, (f64, AcoustidRecording)>::new();
    for result in results {
        if !result.score.is_finite() || !(0.0..=1.0).contains(&result.score) {
            return Err("AcoustID returned an invalid match score");
        }
        for recording in &result.recordings {
            if recording.duration.is_some_and(|duration| {
                !duration.is_finite()
                    || duration <= 0.0
                    || (duration - f64::from(fingerprint.duration_seconds)).abs()
                        > 5.0_f64.max(f64::from(fingerprint.duration_seconds) * 0.03)
            }) {
                continue;
            }
            best.entry(recording.id.clone())
                .and_modify(|entry| {
                    entry.0 = entry.0.max(result.score);
                    merge_memberships(&mut entry.1, recording);
                })
                .or_insert_with(|| (result.score, recording.clone()));
        }
    }
    if best.is_empty() {
        return Err(
            if results.iter().all(|result| result.recordings.is_empty()) {
                "AcoustID returned positive results without recording identities"
            } else {
                "AcoustID recording duration conflicts with local audio"
            },
        );
    }
    let mut ranked = best.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.0.total_cmp(&left.0));
    let best_score = ranked[0].0;
    if results.iter().any(|result| {
        result.recordings.is_empty() && result.score >= best_score - TIED_SCORE_DISTANCE
    }) {
        return Err("AcoustID returned unidentified competing recording candidates");
    }
    if best_score < MINIMUM_SCORE {
        return Err("AcoustID match confidence is below the safe threshold");
    }
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    Ok(ranked
        .into_iter()
        .filter(|(score, _)| best_score - score <= TIED_SCORE_DISTANCE)
        .map(|(score, recording)| SelectedRecording { recording, score })
        .collect())
}

#[cfg(test)]
fn select_recording(
    results: &[acoustid::AcoustidResult],
    fingerprint: &Fingerprint,
) -> Result<Option<SelectedRecording>, &'static str> {
    let mut candidates = recording_hypotheses(results, fingerprint)?;
    if candidates.len() > 1 {
        return Err("AcoustID returned tied recording identities");
    }
    Ok(candidates.pop())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::acoustid::{AcoustidRelease, AcoustidResult};

    fn fingerprint(duration_seconds: u32) -> Fingerprint {
        Fingerprint {
            duration_seconds,
            value: "test-fingerprint".into(),
        }
    }

    fn result(score: f64, recording_id: &str, duration: Option<f64>) -> AcoustidResult {
        AcoustidResult {
            id: None,
            score,
            recordings: vec![AcoustidRecording {
                releases_complete: true,
                id: recording_id.into(),
                title: Some("Track".into()),
                duration,
                artists: Vec::new(),
                releases: vec![AcoustidRelease {
                    id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
                    title: Some("Album".into()),
                }],
            }],
        }
    }

    #[test]
    fn coverage_v3_consensus_requires_every_identity_and_local_guard() {
        let a = "11111111-1111-4111-8111-111111111111";
        let b = "22222222-2222-4222-8222-222222222222";
        for scenario in [
            "valid",
            "title",
            "display_credit",
            "credit_order",
            "credit_vector",
            "annotation",
            "version",
            "existing_id",
            "unknown",
            "score",
            "duration",
            "invalid_duration",
            "invalid_id",
            "unreported_dub",
            "unreported_edit",
            "supported_dub",
            "mismatched_edit",
            "different_named_mix",
            "different_named_edit",
            "same_named_mix",
            "supported_live",
            "artist_prefixed_live",
            "leading_credit_live",
            "guest_prefixed_live",
            "live_missing_title",
            "live_missing_filename",
            "live_credit_order",
            "unbracketed_live",
            "negated_live",
            "unrelated_live",
            "studio_live",
            "extended_live",
            "dub_live",
            "acoustic_live",
            "mono_live",
            "clean_live",
            "a_cappella_live",
            "a_capella_live",
            "different_live_show",
            "different_live_base",
            "different_live_filename",
            "extra_live_qualifier",
        ] {
            let mut local = TrackCandidate {
                title: Some("Song".into()),
                filename: Some("Song".into()),
                length: Some(1.0),
                ..TrackCandidate::default()
            };
            let mut remote = ProviderRecording {
                requested_id: a.into(),
                observation: None,
                id: a.into(),
                title: "Song".into(),
                artist: "Singer & Guest".into(),
                artists: vec!["Singer".into(), "Guest".into()],
                duration: 1.0,
                disambiguation: None,
            };
            let mut second = remote.clone();
            second.id = b.into();
            second.requested_id = b.into();
            let mut candidates = vec![
                SelectedRecording {
                    recording: result(0.99, a, Some(1.0)).recordings.remove(0),
                    score: 0.99,
                },
                SelectedRecording {
                    recording: result(0.99, b, Some(1.0)).recordings.remove(0),
                    score: 0.99,
                },
            ];
            match scenario {
                "title" => second.title = "Other song".into(),
                "display_credit" => second.artist = "Singer featuring Guest".into(),
                "credit_order" => second.artists.reverse(),
                "credit_vector" => second.artists = vec!["Singer & Guest".into()],
                "annotation" => second.disambiguation = Some("live".into()),
                "supported_live"
                | "artist_prefixed_live"
                | "leading_credit_live"
                | "guest_prefixed_live"
                | "live_missing_title"
                | "live_missing_filename"
                | "live_credit_order"
                | "unbracketed_live"
                | "negated_live"
                | "unrelated_live"
                | "studio_live"
                | "extended_live"
                | "dub_live"
                | "acoustic_live"
                | "mono_live"
                | "clean_live"
                | "a_cappella_live"
                | "a_capella_live"
                | "different_live_show"
                | "different_live_base"
                | "different_live_filename"
                | "extra_live_qualifier" => {
                    local.title = Some("Song (Live)".into());
                    local.filename = Some("Song(Live)".into());
                    remote.disambiguation = Some(
                        match scenario {
                            "negated_live" => "live, from a not live recording",
                            "unrelated_live" => "album with live tracks",
                            "studio_live" => "live, from a studio version",
                            "extended_live" => "live, from World Tour (extended mix)",
                            "dub_live" => "live, from World Tour (dub mix)",
                            "acoustic_live" => "live, from World Tour (acoustic)",
                            "mono_live" => "live, from World Tour (mono)",
                            "clean_live" => "live, from World Tour (clean)",
                            "a_cappella_live" => "live, from World Tour (a cappella)",
                            "a_capella_live" => "live, from World Tour (a capella)",
                            _ => "live, from World Tour",
                        }
                        .into(),
                    );
                    second.disambiguation = remote.disambiguation.clone();
                    match scenario {
                        "artist_prefixed_live" => {
                            local.artist = Some("Singer".into());
                            local.filename = Some("Singer-Song(Live)".into());
                        }
                        "leading_credit_live" | "guest_prefixed_live" => {
                            local.artist = Some("Singer & Guest".into());
                            local.artists = vec!["Singer".into(), "Guest".into()];
                            local.filename = Some(
                                if scenario == "leading_credit_live" {
                                    "Singer-Song(Live)"
                                } else {
                                    "Guest-Song(Live)"
                                }
                                .into(),
                            );
                        }
                        "live_missing_title" => local.title = None,
                        "live_missing_filename" => local.filename = None,
                        "live_credit_order" => second.artists.reverse(),
                        "unbracketed_live" => {
                            local.title = Some("Song Live".into());
                            local.filename = Some("Song Live".into());
                        }
                        "different_live_show" => {
                            second.disambiguation = Some("live, from Another Tour".into())
                        }
                        "different_live_base" => local.title = Some("Other Song (Live)".into()),
                        "different_live_filename" => {
                            local.filename = Some("Other Song(Live)".into())
                        }
                        "extra_live_qualifier" => {
                            local.title = Some("Song (Live) (Radio Edit)".into())
                        }
                        _ => {}
                    }
                }
                "version" => local.title = Some("Song (Demo)".into()),
                "existing_id" => local.musicbrainz_track_id = Some(a.into()),
                "score" => candidates[1].score = 0.89,
                "duration" => second.duration = 90.0,
                "invalid_duration" => local.length = Some(f64::NAN),
                "invalid_id" => second.id = "invalid".into(),
                "unreported_dub" => local.title = Some("Song (Dub)".into()),
                "unreported_edit" => local.title = Some("Song Radio Edit".into()),
                "different_named_mix" | "same_named_mix" => {
                    local.title = Some("Song Club Mix".into());
                    remote.title = if scenario == "same_named_mix" {
                        "Song Club Mix"
                    } else {
                        "Song Radio Mix"
                    }
                    .into();
                    second.title = remote.title.clone();
                }
                "different_named_edit" => {
                    local.title = Some("Song Single Edit".into());
                    remote.title = "Song Extended Edit".into();
                    second.title = remote.title.clone();
                }
                "supported_dub" => {
                    local.title = Some("Song (Dub)".into());
                    remote.disambiguation = Some("Dub".into());
                    second.disambiguation = remote.disambiguation.clone();
                }
                "mismatched_edit" => {
                    local.title = Some("Song Radio Edit".into());
                    remote.disambiguation = Some("Extended Edit".into());
                    second.disambiguation = remote.disambiguation.clone();
                }
                _ => {}
            }
            let mut details =
                RecordingDetails::from([(a.into(), Ok(remote)), (b.into(), Ok(second))]);
            if scenario == "unknown" {
                details.remove(b);
            }
            assert_eq!(
                common_recording_text(&local, &candidates, &details).is_some(),
                matches!(
                    scenario,
                    "valid"
                        | "same_named_mix"
                        | "supported_live"
                        | "artist_prefixed_live"
                        | "leading_credit_live"
                ),
                "{scenario}: full agreement and every guard are required, not a majority"
            );
        }
    }

    #[test]
    fn coverage_v3_filename_connector_requires_embedded_and_full_named_remix_agreement() {
        let remote = "Song (Olli Collins & Fred Portelli Remix)";
        let mut local = TrackCandidate {
            title: Some(remote.into()),
            artist: Some("Singer".into()),
            filename: Some("Singer-Song (Olli Collins _ Fred Portelli Remix)".into()),
            ..TrackCandidate::default()
        };
        assert!(recording_versions_compatible(&local, remote, None));
        assert_eq!(
            recording_display_title(&local, remote),
            remote,
            "display must not append the filename's underscored qualifier"
        );
        for bad in [
            "Singer-Song (Fred Portelli _ Olli Collins Remix)",
            "Singer-Song (Olli Collins _ Fred Portelli & Other Remix)",
            "Singer-Other Song (Olli Collins _ Fred Portelli Remix)",
            "Singer-Song (Olli Collins _ Fred Portelli Remix Dub)",
            "Singer _ Song (Olli Collins _ Fred Portelli Remix)",
        ] {
            local.filename = Some(bad.into());
            assert!(
                !recording_versions_compatible(&local, remote, None),
                "{bad}"
            );
        }
        local.filename = Some("Singer-Song (Olli Collins _ Fred Portelli Remix)".into());
        local.title = None;
        assert!(!recording_versions_compatible(&local, remote, None));
        local.title = Some("Song (Other Remix)".into());
        assert!(!recording_versions_compatible(&local, remote, None));
    }

    #[test]
    fn acoustic_encoding_pairs_match_but_other_remixes_and_local_edits_do_not() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/autotag-collections/acoustic-pairs.json"
        ))
        .unwrap();
        let pairs = fixture["pairs"].as_array().unwrap();
        let frames = |value: &serde_json::Value| {
            value["frames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect::<Vec<_>>()
        };
        for pair in pairs {
            assert!(acoustic_frames_match(
                &frames(&pair["left"]),
                &frames(&pair["right"])
            ));
        }
        assert!(!acoustic_frames_match(
            &frames(&pairs[0]["left"]),
            &frames(&pairs[1]["left"])
        ));
        let original = frames(&pairs[0]["left"]);
        let other = frames(&pairs[1]["left"]);
        let count = original.len().min(other.len());
        assert!(
            !acoustic_frames_match(&original[..count], &other[..count]),
            "different remixes still reject with equal frame counts"
        );
        let mut edited = original.clone();
        for frame in &mut edited[200..260] {
            *frame = !*frame;
        }
        assert!(
            !acoustic_frames_match(&original, &edited),
            "localized edits must not hide in an overall mean"
        );
        assert!(
            !acoustic_frames_match(&vec![0; 300], &vec![1; 300]),
            "low-information fingerprints cannot establish an approximate duplicate"
        );
    }

    #[test]
    fn duplicate_comparison_requires_the_whole_audio_and_conflicts_stay_in_their_group() {
        let selected = |id: &str| {
            Some(SelectedRecording {
                recording: result(0.99, id, Some(240.0)).recordings.remove(0),
                score: 0.99,
            })
        };
        let rows = [selected("same"), selected("same"), selected("other")];
        let (indices, conflicts) = recording_groups(
            &rows,
            &[
                Some(fingerprint(240)),
                Some(fingerprint(240)),
                Some(fingerprint(240)),
            ],
            &[None, None, None],
        );
        assert_eq!(indices, [0, 2]);
        assert!(
            conflicts.contains("same"),
            "a matching first 120 seconds cannot establish a full-duration duplicate"
        );
        assert!(
            !conflicts.contains("other"),
            "another duplicate group's failure must not suppress an unrelated identity"
        );
    }

    #[test]
    fn fuzzy_duplicate_groups_require_pairwise_agreement_not_just_a_shared_anchor() {
        let a: Vec<_> = (0..300u32).map(|i| i.wrapping_mul(2654435761)).collect();
        let b: Vec<_> = a
            .iter()
            .enumerate()
            .map(|(i, v)| v ^ u32::from(i % 2 == 0))
            .collect();
        let c: Vec<_> = a
            .iter()
            .enumerate()
            .map(|(i, v)| v ^ (2 * u32::from(i % 2 == 0)))
            .collect();
        assert!(acoustic_frames_match(&a, &b) && acoustic_frames_match(&a, &c));
        assert!(!acoustic_frames_match(&b, &c));
        let selected = Some(SelectedRecording {
            recording: result(0.99, "same", Some(240.0)).recordings.remove(0),
            score: 0.99,
        });
        let (_, conflicts) = recording_groups(
            &[selected.clone(), selected.clone(), selected],
            &vec![Some(fingerprint(240)); 3],
            &[Some(a), Some(b), Some(c)],
        );
        assert!(
            conflicts.contains("same"),
            "near-equality is not transitive"
        );
    }

    #[test]
    fn repeated_identity_memberships_are_unioned_before_any_edition_decision() {
        let first = result(0.99, "same", Some(125.0));
        let mut second = first.clone();
        second.recordings[0].releases[0].id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into();
        for responses in [
            [first.clone(), second.clone()],
            [second.clone(), first.clone()],
        ] {
            let candidates = recording_hypotheses(&responses, &fingerprint(125)).unwrap();
            assert_eq!(candidates.len(), 1);
            assert_eq!(
                candidates[0]
                    .recording
                    .releases
                    .iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>(),
                [
                    "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                    "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
                ],
                "response order cannot erase a plausible edition"
            );
        }
    }

    #[test]
    fn a_near_threshold_competitor_is_not_erased_by_the_winners_confidence_gate() {
        assert!(select_recording(
            &[
                result(0.905, "winner", None),
                result(0.895, "competitor", None)
            ],
            &fingerprint(125)
        )
        .is_err());
        let mut unknown = result(0.895, "unknown", None);
        unknown.recordings.clear();
        assert!(
            recording_hypotheses(&[result(0.905, "winner", None), unknown], &fingerprint(125))
                .is_err()
        );
    }

    #[test]
    fn high_scoring_unidentified_clusters_remain_unknown_competitors() {
        let mut unidentified = result(0.99, "unidentified", None);
        unidentified.recordings.clear();
        assert!(
            recording_hypotheses(
                &[result(0.99, "known", None), unidentified],
                &fingerprint(125)
            )
            .is_err(),
            "a missing MusicBrainz link is not an eliminated competitor"
        );
    }

    #[test]
    fn tied_recordings_with_incompatible_durations_are_eliminated_before_ambiguity() {
        let responses = [
            result(0.99, "recording-a", Some(125.0)),
            result(0.99, "recording-b", Some(300.0)),
        ];
        assert_eq!(
            select_recording(&responses, &fingerprint(125))
                .unwrap()
                .unwrap()
                .recording
                .id,
            "recording-a"
        );
    }

    #[tokio::test]
    async fn cached_recording_details_do_not_consume_or_exceed_the_next_network_budget() {
        let root = std::env::temp_dir().join(format!(
            "soundrobe-recording-budget-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = CacheState::new_in(root.clone());
        assert!(cache.initialize(None));
        let id = "11111111-1111-4111-8111-111111111111";
        cache.set_release_detail(RECORDING_FACTS,id,&serde_json::json!({"requested_id":id,"id":id,"title":"Song","artist":"Singer","artists":["Singer"],"duration":125.0,"disambiguation":null})).unwrap();
        let provider = MusicBrainzClient::at(reqwest::Client::new(), "http://127.0.0.1:1");
        let mut requests = 32;
        let mut details = HashMap::new();
        assert_eq!(
            recording_detail(
                id,
                &provider,
                Some(&cache),
                &AtomicBool::new(false),
                &mut details,
                &mut requests,
                &mut 0,
            )
            .await
            .unwrap()
            .id,
            id
        );
        assert_eq!(requests, 32);
        assert_eq!(
            recording_detail(
                "22222222-2222-4222-8222-222222222222",
                &provider,
                Some(&cache),
                &AtomicBool::new(false),
                &mut details,
                &mut requests,
                &mut 0,
            )
            .await
            .unwrap_err()
            .reason,
            "recording_detail_budget_limited"
        );
        assert_eq!(requests, 32);
        drop(cache);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_titles_keep_verified_qualifiers_from_tags_or_filenames() {
        for title in [None, Some("Song"), Some("Wrong base (Live)")] {
            let local = TrackCandidate {
                title: title.map(str::to_owned),
                filename: Some("01 Song (Live)".into()),
                ..TrackCandidate::default()
            };
            assert_eq!(
                recording_display_title(&local, "Song"),
                "Song (Live)",
                "keep version wording while replacing an incorrect base title"
            );
        }
        let local = TrackCandidate {
            title: Some("Song (Live)".into()),
            ..TrackCandidate::default()
        };
        assert_eq!(
            recording_display_title(&local, "Song (Live)"),
            "Song (Live)"
        );
    }

    #[test]
    fn live_disambiguation_supports_version_without_replacing_display_text() {
        assert!(versions_compatible_with_disambiguation(
            "Ave Maria (Live)",
            "Ave Maria",
            Some("live, from I Am… World Tour")
        ));
        assert!(!versions_compatible_with_disambiguation(
            "Ave Maria (Live)",
            "Ave Maria",
            Some("studio recording")
        ));
        assert!(!versions_compatible_with_disambiguation(
            "Song (Cella Remix)",
            "Song",
            Some("Other Remix")
        ));
        assert!(versions_compatible_with_disambiguation(
            "Song (Cella Remix)",
            "Song",
            Some("Cella remix")
        ));
    }

    #[test]
    fn duplicate_groups_require_recording_fingerprint_and_duration_evidence() {
        let selected = SelectedRecording {
            recording: result(0.99, "same-recording", Some(125.0))
                .recordings
                .remove(0),
            score: 0.99,
        };
        let selected_rows = vec![Some(selected.clone()), Some(selected.clone())];
        let same = vec![Some(fingerprint(60)), Some(fingerprint(60))];
        assert_eq!(
            unique_recording_indices(&selected_rows, &same).unwrap(),
            vec![0]
        );
        let mut different = same.clone();
        different[1].as_mut().unwrap().value = "different-audio".into();
        assert!(
            unique_recording_indices(&selected_rows, &different).is_err(),
            "same recording ID alone cannot merge different audio"
        );
        let mut other = selected.clone();
        other.recording.id = "other-recording".into();
        assert_eq!(
            unique_recording_indices(&[Some(selected), Some(other)], &same).unwrap(),
            vec![0, 1]
        );
    }

    #[test]
    fn equivalent_editions_share_only_unanimous_album_fields() {
        let first = AlbumCandidate {
            album: Some("Album".into()),
            album_artist: Some("Singer".into()),
            album_artists: vec!["Singer".into()],
            year: Some("2008".into()),
            musicbrainz_album_id: Some("edition-a".into()),
            ..AlbumCandidate::default()
        };
        let mut second = first.clone();
        second.musicbrainz_album_id = Some("edition-b".into());
        second.year = Some("2012".into());
        let shared = shared_album_fields(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(shared.album, first.album);
        assert!(shared.year.is_none() && shared.musicbrainz_album_id.is_none());
        assert!(shared.tracks.is_empty());
        second.album = Some("Other Album".into());
        assert!(shared_album_fields(&[first, second]).is_none());
    }

    #[test]
    fn recording_identity_is_useful_without_a_complete_release_and_versions_stay_distinct() {
        let mut response = result(0.99, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", Some(125.0));
        response.recordings[0].releases.clear();
        assert!(
            select_recording(&[response], &fingerprint(125))
                .unwrap()
                .is_some(),
            "recording identity does not require an album edition"
        );
        assert!(!versions_compatible("Song (Sorry Demo)", "Song"));
        assert!(!versions_compatible("Song (Clean)", "Song (Explicit)"));
        assert!(!versions_compatible("Song (Live)", "Song (Studio Version)"));
        assert!(!versions_compatible("Song（Ｌｉｖｅ）", "Song"));
        assert!(!versions_compatible("Song (Liv&#101;)", "Song"));
        assert!(versions_compatible(
            "Song (Cella Remix)",
            "Song (Cella Remix)"
        ));
        assert!(!versions_compatible(
            "Song (Cella Remix)",
            "Song (Other Remix)"
        ));
        assert!(!versions_compatible(
            "Song - Cella Remix",
            "Song - Other Remix"
        ));
        assert!(!versions_compatible("Song Cella Remix", "Song Other Remix"));
        assert!(!versions_compatible(
            "Song (Live) - Cella Remix",
            "Song (Live) - Other Remix"
        ));
        assert!(versions_compatible(
            "01 Artist - Song - Cella Remix",
            "Song - Cella Remix"
        ));
        assert!(!versions_compatible(
            "Song - Early Demo",
            "Song - Final Demo"
        ));
    }

    #[test]
    fn recording_selection_requires_one_strong_duration_consistent_identity() {
        assert!(select_recording(&[], &fingerprint(125)).unwrap().is_none());
        assert!(select_recording(&[result(0.899, "weak", None)], &fingerprint(125)).is_err());
        assert!(select_recording(
            &[result(0.98, "first", None), result(0.97, "second", None),],
            &fingerprint(125),
        )
        .is_err());
        assert!(select_recording(
            &[result(0.99, "duration-conflict", Some(90.0))],
            &fingerprint(125),
        )
        .is_err());
        let selected = select_recording(
            &[
                result(0.94, "accepted", Some(124.0)),
                result(0.91, "runner-up", None),
            ],
            &fingerprint(125),
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.recording.id, "accepted");
        assert_eq!(selected.score, 0.94);
    }
}
