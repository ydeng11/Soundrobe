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
    MusicBrainzClient, ProviderRetryContext, ProviderRetryMetrics, ProviderState,
};
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
    pub track_matches: Vec<AcoustidTrackEvidence>,
    pub candidate_count: usize,
    pub checked_candidates: usize,
    pub rejections: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcoustidTrackEvidence {
    pub file_name: String,
    pub recording_id: String,
    pub score: f64,
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
struct SelectedRecording {
    recording: AcoustidRecording,
    score: f64,
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

    let metrics = ProviderRetryMetrics::default();
    let musicbrainz =
        MusicBrainzClient::at(providers.http(), providers.musicbrainz_base()).with_retry_context(
            ProviderRetryContext::new(Arc::new(AtomicBool::new(false)), metrics.clone()),
        );
    let mut fingerprints = Vec::new();
    let mut selected = Vec::new();
    let mut evidence = AcoustidEvidence {
        release_id: String::new(),
        track_matches: Vec::new(),
        candidate_count: 0,
        checked_candidates: 0,
        rejections: Vec::new(),
    };
    let mut confirmed_empty = 0;
    let mut lookup_retryable = false;
    let mut lookup_retry_after = None;
    for (file, local) in files.iter().zip(&request.tracks) {
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
        };
        let mut fingerprint = None;
        let found = async {
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
            let results = client
                .lookup(api_key, &value, cancelled)
                .await
                .map_err(|error| {
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
            let recording = select_recording(&results, &value).map_err(str::to_string)?;
            fingerprint = Some(value);
            Ok(recording)
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
        evidence.track_matches.push(row);
        if let Err(error) =
            super::auto_tag::checkpoint_evidence(serde_json::json!({"acoustid":evidence}))
        {
            return AcoustidResolution::Unavailable(format!(
                "diagnostic checkpoint failed: {error}"
            ));
        }
    }
    if confirmed_empty == files.len() {
        return AcoustidResolution::NoMatch;
    }

    let groups = unique_recording_indices(&selected, &fingerprints);
    let mut accepted = Vec::new();
    let mut reason_code = "partial_collection";
    let mut fetch_failed = false;
    if let Ok(indices) = &groups {
        if selected.iter().all(Option::is_some) {
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
            if common.len() > MAX_RELEASE_CANDIDATES {
                reason_code = "discovery_budget_limited";
                evidence
                    .rejections
                    .push("too many editions to establish shared album fields safely".into());
            } else {
                let mut unique_request = request.clone();
                unique_request.tracks = indices
                    .iter()
                    .map(|index| {
                        let mut track = request.tracks[*index].clone();
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
                for id in common {
                    if !valid_musicbrainz_id(&id) {
                        evidence.rejections.push("invalid_release_identity".into());
                        fetch_failed = true;
                        continue;
                    }
                    let response = tokio::select! { result = musicbrainz.release_by_id_result(&id) => result, _ = wait_for_cancel(cancelled) => return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string()) };
                    evidence.checked_candidates += 1;
                    let release = match response {
                        Ok(release) if release.id == id => release,
                        _ => {
                            fetch_failed = true;
                            evidence
                                .rejections
                                .push(format!("release_detail_unavailable:{id}"));
                            continue;
                        }
                    };
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
                                local
                                    .title
                                    .iter()
                                    .chain(local.filename.iter())
                                    .any(|title| {
                                        !versions_compatible(
                                            title,
                                            remote.title.as_deref().unwrap_or_default(),
                                        )
                                    })
                                    || local.musicbrainz_track_id.as_deref().is_some_and(|id| {
                                        Some(id) != remote.musicbrainz_track_id.as_deref()
                                    })
                                    || local
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
                if accepted.len() == 1 && !fetch_failed {
                    let candidate = accepted.remove(0);
                    evidence.release_id =
                        candidate.musicbrainz_album_id.clone().unwrap_or_default();
                    return AcoustidResolution::Candidate {
                        candidate: Box::new(candidate),
                        evidence,
                    };
                }
                if accepted.len() > 1 {
                    reason_code = "ambiguous_edition";
                }
            }
        }
    } else {
        reason_code = "recording_conflict";
        evidence
            .rejections
            .push("same recording ID has incompatible acoustic evidence".into());
    }
    let shared_album = if !fetch_failed && accepted.len() > 1 {
        shared_album_fields(&accepted).map(Box::new)
    } else {
        None
    };
    // A release conflict does not erase strong recording evidence. Fetch the
    // authoritative recording text independently; omit all unresolved fields.
    let mut verified = HashMap::<String, crate::state::providers::ProviderRecording>::new();
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
        if groups.is_err()
            && selected
                .iter()
                .flatten()
                .filter(|other| other.recording.id == *id)
                .count()
                > 1
        {
            evidence.track_matches[index].reason = Some("recording_audio_conflict".into());
            continue;
        }
        if !valid_musicbrainz_id(id) {
            evidence.track_matches[index].reason = Some("invalid_recording_identity".into());
            continue;
        }
        let response = if let Some(cached) = verified.get(id) {
            Ok(cached.clone())
        } else {
            tokio::select! { result = musicbrainz.recording_by_id_result(id) => result, _ = wait_for_cancel(cancelled) => return AcoustidResolution::Unavailable(AcoustidError::Cancelled.to_string()) }
        };
        let remote = match response {
            Ok(remote) => {
                verified.insert(id.clone(), remote.clone());
                remote
            }
            Err(_) => {
                fetch_failed = true;
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
        let version_safe = local
            .title
            .iter()
            .chain(local.filename.iter())
            .all(|title| versions_compatible(title, &remote.title));
        if !credits_safe
            || !version_safe
            || local
                .musicbrainz_track_id
                .as_deref()
                .is_some_and(|existing| existing != id)
            || (local.length.unwrap_or(0.0) - remote.duration).abs()
                > 5.0_f64.max(remote.duration * 0.03)
        {
            evidence.track_matches[index].reason = Some("recording_identity_conflict".into());
            continue;
        }
        tracks[index] = TrackCandidate {
            source: local.source.clone(),
            title: Some(remote.title),
            artist: Some(remote.artist),
            artists: remote.artists,
            musicbrainz_track_id: Some(remote.id),
            filename: local.filename.clone(),
            length: local.length,
            ..TrackCandidate::default()
        };
    }
    if lookup_retryable || (fetch_failed && metrics.transient_failure()) {
        reason_code = "provider_retryable";
    }
    AcoustidResolution::Recordings {
        tracks,
        shared_album,
        evidence,
        reason_code: reason_code.into(),
        retryable: lookup_retryable || metrics.transient_failure(),
        retry_after_seconds: lookup_retry_after.max(metrics.max_retry_after_seconds()),
    }
}

fn unique_recording_indices(
    selected: &[Option<SelectedRecording>],
    fingerprints: &[Option<Fingerprint>],
) -> Result<Vec<usize>, ()> {
    let mut indices = Vec::new();
    let mut seen = HashMap::<&str, usize>::new();
    for (index, recording) in selected.iter().enumerate() {
        let Some(recording) = recording else {
            continue;
        };
        if let Some(previous) = seen.get(recording.recording.id.as_str()) {
            let a = fingerprints[*previous].as_ref().ok_or(())?;
            let b = fingerprints[index].as_ref().ok_or(())?;
            if a.value != b.value || a.duration_seconds.abs_diff(b.duration_seconds) > 2 {
                return Err(());
            }
        } else {
            seen.insert(&recording.recording.id, index);
            indices.push(index);
        }
    }
    Ok(indices)
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
    fn duplicate_groups_require_recording_fingerprint_and_duration_evidence() {
        let selected = SelectedRecording {
            recording: result(0.99, "same-recording", Some(125.0))
                .recordings
                .remove(0),
            score: 0.99,
        };
        let selected_rows = vec![Some(selected.clone()), Some(selected.clone())];
        let same = vec![Some(fingerprint(125)), Some(fingerprint(125))];
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
