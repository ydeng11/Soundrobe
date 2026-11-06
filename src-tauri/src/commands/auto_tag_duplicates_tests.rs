use super::*;
use crate::infra::audio_evidence::ProvenAudioGroups;

fn duplicate_request() -> (LookupRequest, AlbumCandidate) {
    let mut tracks = (1..=15)
        .map(|position| TrackCandidate {
            title: Some(format!("Song {position}")),
            filename: Some(format!("{position:02} Song {position}")),
            artist: Some("Lead".into()),
            artists: vec!["Lead".into()],
            track_number: Some(position),
            tagged_track_number: Some(position),
            disc_number: Some(1),
            length: Some(180.0 + f64::from(position)),
            source: Some(RecordingSource {
                path: format!("{position}.flac").into(),
                fingerprint: "a".repeat(64),
                physical_identity: None,
            }),
            ..TrackCandidate::default()
        })
        .collect::<Vec<_>>();
    let remote = tracks
        .iter()
        .enumerate()
        .map(|(i, track)| TrackCandidate {
            musicbrainz_track_id: Some(format!("recording-{i}")),
            length: track.length.map(|n| n * 1000.0),
            track_total: Some(15),
            ..track.clone()
        })
        .collect();
    tracks.push(tracks[3].clone());
    tracks[15].filename = Some("copy of song four".into());
    tracks[15].source.as_mut().unwrap().path = "copy.flac".into();
    tracks[15].artists.push("Guest".into());
    let groups = (0..15)
        .map(|i| if i == 3 { vec![3, 15] } else { vec![i] })
        .collect();
    let request = LookupRequest {
        artist_hint: Some("Lead".into()),
        album_hint: Some("Album".into()),
        audio_groups: Some(ProvenAudioGroups::for_test(
            tracks
                .iter()
                .map(|t| {
                    let s = t.source.as_ref().unwrap();
                    (s.path.clone(), s.fingerprint.clone())
                })
                .collect(),
            groups,
        )),
        tracks,
        ..LookupRequest::default()
    };
    let candidate = AlbumCandidate {
        artist: Some("Lead".into()),
        album: Some("Album".into()),
        musicbrainz_album_id: Some("release".into()),
        tracks: remote,
        source: LookupSource::Musicbrainz,
        ..AlbumCandidate::default()
    };
    (request, candidate)
}

#[test]
fn duplicate_mapping_survives_selection_protection_and_occurrence_coverage() {
    let (request, candidate) = duplicate_request();
    let selected = select_credible_provider_candidate(&request, vec![candidate.clone()]).unwrap();
    let accepted = selected.accepted_match.as_ref().unwrap();
    assert_eq!(accepted.coverage, 15);
    assert_eq!(accepted.remote_indices.len(), 16);
    assert_eq!(accepted.remote_indices[3], accepted.remote_indices[15]);
    let protected = protect_candidate_tracks(&request, &selected);
    assert_eq!(protected.tracks.len(), 16);
    assert_eq!(protected.tracks[15].artists, vec!["Lead", "Guest"]);
    assert_eq!(protected.tracks[15].filename, request.tracks[15].filename);
    assert_eq!(
        selected_track_match_diagnostic(&request, &selected)["remoteIndices"][15],
        3
    );
    let coverage = album_coverage(&request, Some(&protected), 16);
    assert_eq!(coverage["completeAlbum"], true);
    assert_eq!(coverage["coveredProviderOccurrences"], 15);
    assert_eq!(coverage["provenAudioGroups"], 15);
    assert_eq!(
        album_coverage(&request, Some(&protected), 15)["completeAlbum"],
        false
    );
    let mut unproven = request.clone();
    unproven.audio_groups = None;
    assert!(select_credible_provider_candidate(&unproven, vec![candidate]).is_none());
}

#[test]
fn grouped_conflicts_cannot_fall_back_to_an_ordinary_or_cached_mapping() {
    let (mut request, mut candidate) = duplicate_request();
    let selected = select_credible_provider_candidate(&request, vec![candidate.clone()]).unwrap();
    candidate.accepted_match = selected.accepted_match;
    request.tracks[15].title = Some("Conflicting title".into());
    assert!(select_credible_provider_candidate(&request, vec![candidate.clone()]).is_none());
    request.tracks[15].title = request.tracks[3].title.clone();
    request.tracks[15].tagged_track_number = Some(5);
    assert!(provider_candidate_credibility(&request, &candidate).is_err());
    request.tracks[15].tagged_track_number = Some(4);
    request.musicbrainz_album_id = Some("different-release".into());
    assert!(provider_candidate_credibility(&request, &candidate).is_err());
    request.musicbrainz_album_id = None;
    request.tracks[15].source.as_mut().unwrap().fingerprint = "b".repeat(64);
    assert!(provider_candidate_credibility(&request, &candidate).is_err());
    let decoded: AlbumCandidate =
        serde_json::from_value(serde_json::to_value(&candidate).unwrap()).unwrap();
    assert!(decoded.accepted_match.is_none());
}

#[test]
fn identical_recording_ids_do_not_erase_distinct_release_occurrences() {
    let (request, mut candidate) = duplicate_request();
    candidate.tracks[1].musicbrainz_track_id = candidate.tracks[0].musicbrainz_track_id.clone();
    let selected = select_credible_provider_candidate(&request, vec![candidate]).unwrap();
    let protected = protect_candidate_tracks(&request, &selected);
    let coverage = album_coverage(&request, Some(&protected), 16);
    assert_eq!(coverage["uniqueVerifiedRecordings"], 14);
    assert_eq!(coverage["coveredProviderOccurrences"], 15);
    assert_eq!(coverage["completeAlbum"], true);
}

#[test]
fn grouped_media_scopes_require_a_unique_feasible_assignment() {
    let (request, mut candidate) = duplicate_request();
    for track in &mut candidate.tracks {
        track.media_type = Some("CD".into());
    }
    let mut dvd = candidate.tracks.clone();
    for track in &mut dvd {
        track.media_type = Some("DVD".into());
    }
    candidate.tracks.extend(dvd);
    assert!(select_credible_provider_candidate(&request, vec![candidate.clone()]).is_none());
    for track in candidate.tracks.iter_mut().skip(15) {
        track.length = Some(900_000.0);
    }
    let selected = select_credible_provider_candidate(&request, vec![candidate]).unwrap();
    assert_eq!(selected.accepted_match.unwrap().coverage, 15);
}

#[test]
fn selected_disc_mapping_retains_original_provider_row_identity() {
    let (mut request, mut candidate) = duplicate_request();
    let mut first_disc = candidate.tracks.clone();
    for track in &mut candidate.tracks {
        track.disc_number = Some(2);
    }
    first_disc.extend(candidate.tracks);
    candidate.tracks = first_disc;
    request.selected_disc_number = Some(2);
    for track in &mut request.tracks {
        track.disc_number = Some(2);
    }
    let selected = select_credible_provider_candidate(&request, vec![candidate]).unwrap();
    let accepted = selected.accepted_match.unwrap();
    assert_eq!(accepted.remote_indices[0], Some(15));
    assert_eq!(accepted.remote_indices[15], Some(18));
    assert_eq!(accepted.coverage, 15);
}
