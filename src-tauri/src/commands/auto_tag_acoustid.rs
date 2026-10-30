//! AcoustID identity lookup used only as a complete-release fallback.
//!
//! AcoustID contributes recording IDs. Album, artist and track metadata always
//! comes from the uniquely validated MusicBrainz release.

use crate::commands::auto_tag::{
    accept_acoustid_release, musicbrainz_candidate, AlbumCandidate, LookupRequest,
};
use crate::infra::acoustid::{
    fingerprint_file_with, fpcalc_executable, AcoustidClient, AcoustidError, AcoustidFailureKind,
    AcoustidRecording, Fingerprint,
};
use crate::state::providers::{MusicBrainzClient, ProviderState};
use crate::{commands::library::collect_audio_files, infra::acoustid};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MINIMUM_SCORE: f64 = 0.90;
const TIED_SCORE_DISTANCE: f64 = 0.02;
const MAX_TRACKS: usize = 200;
const MAX_RELEASE_CANDIDATES: usize = 8;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AcoustidResolution {
    NoMatch,
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
    pub track_matches: Vec<AcoustidTrackEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcoustidTrackEvidence {
    pub file_name: String,
    pub recording_id: String,
    pub score: f64,
}

#[derive(Clone, Debug)]
struct SelectedRecording {
    recording: AcoustidRecording,
    score: f64,
}

impl AcoustidResolution {
    fn from_error(error: AcoustidError) -> Self {
        match error.kind() {
            AcoustidFailureKind::Unavailable => Self::Unavailable(error.to_string()),
            AcoustidFailureKind::Incomplete => Self::Incomplete(error.to_string()),
            AcoustidFailureKind::Cancelled => Self::Unavailable(error.to_string()),
        }
    }
}

pub(crate) async fn resolve(
    request: &LookupRequest,
    api_key: Option<&str>,
    providers: &ProviderState,
    cancelled: &AtomicBool,
) -> AcoustidResolution {
    let client = AcoustidClient::new(providers.http());
    resolve_with(
        request,
        api_key,
        providers,
        cancelled,
        &client,
        &fpcalc_executable(),
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
    let Some(api_key) = api_key.filter(|key| !key.trim().is_empty()) else {
        return AcoustidResolution::Unavailable(
            "AcoustID application key is not configured".into(),
        );
    };
    if external_cancelled.load(Ordering::Acquire) {
        return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());
    }

    // The internal cancellation flag lets the total budget or caller stop an
    // active fingerprint child, drain/reap it, and interrupt HTTP bodies.
    let cancelled = Arc::new(AtomicBool::new(false));
    let external_cancellation = wait_for_external_cancel(external_cancelled);
    tokio::pin!(external_cancellation);
    let inner = resolve_inner(request, api_key, providers, client, fpcalc, &cancelled);
    tokio::pin!(inner);
    let result = tokio::select! {
        result = &mut inner => result,
        _ = &mut external_cancellation => {
            cancelled.store(true, Ordering::Release);
            let _ = inner.await;
            AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string())
        }
        _ = tokio::time::sleep(TOTAL_TIMEOUT) => {
            cancelled.store(true, Ordering::Release);
            let _ = inner.await;
            AcoustidResolution::Incomplete("AcoustID lookup exceeded the total time budget".into())
        }
    };
    result
}

async fn wait_for_external_cancel(cancelled: &AtomicBool) {
    while !cancelled.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn resolve_inner(
    request: &LookupRequest,
    api_key: &str,
    providers: &ProviderState,
    client: &AcoustidClient,
    fpcalc: &Path,
    cancelled: &AtomicBool,
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

    let mut fingerprints = Vec::with_capacity(files.len());
    for (file, local) in files.iter().zip(&request.tracks) {
        if cancelled.load(Ordering::Acquire) {
            return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());
        }
        let file_stem = Path::new(file).file_stem().and_then(|name| name.to_str());
        if file_stem != local.filename.as_deref() {
            return AcoustidResolution::Incomplete(
                "local audio file ordering could not be mapped to track records".into(),
            );
        }
        let Some(local_duration) = local
            .length
            .filter(|duration| duration.is_finite() && *duration > 0.0)
        else {
            return AcoustidResolution::Incomplete(
                "local audio duration is unavailable for an album track".into(),
            );
        };
        let fingerprint = match fingerprint_file_with(
            fpcalc,
            Path::new(file),
            cancelled,
            Duration::from_secs(45),
            1024 * 1024,
            64 * 1024,
        )
        .await
        {
            Ok(fingerprint) => fingerprint,
            Err(error) => return AcoustidResolution::from_error(error),
        };
        if (f64::from(fingerprint.duration_seconds) - local_duration).abs()
            > 5.0_f64.max(local_duration * 0.03)
        {
            return AcoustidResolution::Incomplete(
                "fpcalc duration conflicts with local audio metadata".into(),
            );
        }
        fingerprints.push(fingerprint);
    }

    let mut selected_recordings = Vec::<SelectedRecording>::with_capacity(files.len());
    let mut empty_results = 0usize;
    for fingerprint in &fingerprints {
        if cancelled.load(Ordering::Acquire) {
            return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());
        }
        let results = match client.lookup(api_key, fingerprint, cancelled).await {
            Ok(results) => results,
            Err(error) => return AcoustidResolution::from_error(error),
        };
        if results.is_empty() {
            empty_results += 1;
            continue;
        }
        match select_recording(&results, fingerprint) {
            Ok(Some(recording)) => selected_recordings.push(recording),
            Ok(None) => empty_results += 1,
            Err(reason) => return AcoustidResolution::Incomplete(reason.into()),
        }
    }
    if empty_results == files.len() {
        return AcoustidResolution::NoMatch;
    }
    if empty_results > 0 || selected_recordings.len() != files.len() {
        return AcoustidResolution::Incomplete(
            "AcoustID evidence covers only part of the album".into(),
        );
    }

    let recording_ids = selected_recordings
        .iter()
        .map(|selected| selected.recording.id.clone())
        .collect::<Vec<_>>();
    if recording_ids.iter().collect::<BTreeSet<_>>().len() != recording_ids.len()
        || recording_ids.iter().any(|id| !valid_musicbrainz_id(id))
    {
        return AcoustidResolution::Incomplete(
            "AcoustID matched a repeated recording and cannot establish a unique track map".into(),
        );
    }
    let mut common_releases = selected_recordings
        .first()
        .map(|selected| {
            selected
                .recording
                .releases
                .iter()
                .map(|release| release.id.clone())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    for selected in selected_recordings.iter().skip(1) {
        let releases = selected
            .recording
            .releases
            .iter()
            .map(|release| release.id.as_str())
            .collect::<BTreeSet<_>>();
        common_releases.retain(|release| releases.contains(release.as_str()));
    }
    if common_releases.is_empty() {
        return AcoustidResolution::Incomplete(
            "AcoustID recordings do not share one complete MusicBrainz release".into(),
        );
    }
    if common_releases
        .iter()
        .any(|release_id| !valid_musicbrainz_id(release_id))
    {
        return AcoustidResolution::Incomplete(
            "AcoustID returned an invalid MusicBrainz release identity".into(),
        );
    }
    if common_releases.len() > MAX_RELEASE_CANDIDATES {
        return AcoustidResolution::Incomplete(
            "AcoustID returned too many possible MusicBrainz editions to compare safely".into(),
        );
    }

    let musicbrainz = MusicBrainzClient::at(providers.http(), providers.musicbrainz_base());
    let mut accepted = Vec::new();
    let mut fetch_failed = false;
    for release_id in common_releases {
        if cancelled.load(Ordering::Acquire) {
            return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());
        }
        let fetched = tokio::select! {
            result = musicbrainz.release_by_id_result(&release_id) => result,
            _ = wait_for_cancel(cancelled) => {
                return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string());
            }
        };
        let release = match fetched {
            Ok(release) if release.id == release_id => release,
            Ok(_) | Err(_) => {
                fetch_failed = true;
                continue;
            }
        };
        let candidate = musicbrainz_candidate(release);
        match accept_acoustid_release(request, &candidate, &recording_ids) {
            Ok(candidate) => accepted.push(candidate),
            Err(_) => continue,
        }
    }
    match accepted.len() {
        1 if !fetch_failed => {
            let candidate = accepted.remove(0);
            let release_id = candidate.musicbrainz_album_id.clone().unwrap_or_default();
            let track_matches = files
                .iter()
                .zip(&selected_recordings)
                .map(|(file, selected)| AcoustidTrackEvidence {
                    file_name: Path::new(file)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    recording_id: selected.recording.id.clone(),
                    score: selected.score,
                })
                .collect();
            AcoustidResolution::Candidate {
                candidate: Box::new(candidate),
                evidence: AcoustidEvidence {
                    release_id,
                    track_matches,
                },
            }
        }
        1 => AcoustidResolution::Incomplete(
            "a possible MusicBrainz edition could not be checked".into(),
        ),
        count if count > 1 => AcoustidResolution::Incomplete(
            "AcoustID evidence matches more than one MusicBrainz edition".into(),
        ),
        _ => AcoustidResolution::Incomplete(
            "AcoustID evidence does not validate a complete MusicBrainz release".into(),
        ),
    }
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

fn select_recording(
    results: &[acoustid::AcoustidResult],
    fingerprint: &Fingerprint,
) -> Result<Option<SelectedRecording>, &'static str> {
    if results.is_empty() {
        return Ok(None);
    }
    let mut best = HashMap::<String, (f64, AcoustidRecording)>::new();
    for result in results {
        if !result.score.is_finite() || !(0.0..=1.0).contains(&result.score) {
            return Err("AcoustID returned an invalid match score");
        }
        for recording in &result.recordings {
            best.entry(recording.id.clone())
                .and_modify(|entry| {
                    if result.score > entry.0 {
                        *entry = (result.score, recording.clone());
                    }
                })
                .or_insert_with(|| (result.score, recording.clone()));
        }
    }
    if best.is_empty() {
        return Err("AcoustID returned positive results without recording identities");
    }
    let mut ranked = best.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.0.total_cmp(&left.0));
    let (score, recording) = ranked.remove(0);
    if score < MINIMUM_SCORE {
        return Err("AcoustID match confidence is below the safe threshold");
    }
    if ranked
        .first()
        .is_some_and(|(next_score, _)| score - next_score <= TIED_SCORE_DISTANCE)
    {
        return Err("AcoustID returned tied recording identities");
    }
    if recording.releases.is_empty() {
        return Err("AcoustID recording has no MusicBrainz release evidence");
    }
    if let Some(duration) = recording.duration {
        if !duration.is_finite()
            || duration <= 0.0
            || (duration - f64::from(fingerprint.duration_seconds)).abs()
                > 5.0_f64.max(f64::from(fingerprint.duration_seconds) * 0.03)
        {
            return Err("AcoustID recording duration conflicts with local audio");
        }
    }
    Ok(Some(SelectedRecording { recording, score }))
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
            score,
            recordings: vec![AcoustidRecording {
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
