use super::*;
use crate::infra::audio_evidence::ProvenAudioGroups;

fn local(title: &str, position: u32, seconds: f64) -> TrackCandidate {
    TrackCandidate {
        title: Some(title.into()),
        filename: Some(format!("{position:02} {title}")),
        artist: Some("Lead".into()),
        artists: vec!["Lead".into()],
        length: Some(seconds),
        track_number: Some(position),
        tagged_track_number: Some(position),
        disc_number: Some(1),
        ..TrackCandidate::default()
    }
}

fn provider(title: &str, position: u32, seconds: f64, id: &str) -> TrackCandidate {
    TrackCandidate {
        musicbrainz_track_id: Some(id.into()),
        length: Some(seconds * 1000.0),
        ..local(title, position, seconds)
    }
}

fn align(
    local: &[TrackCandidate],
    remote: &[TrackCandidate],
    groups: Vec<Vec<usize>>,
) -> Result<MatchedCandidate, String> {
    let proof = ProvenAudioGroups::for_test(
        (0..local.len())
            .map(|i| (format!("{i}.flac").into(), "a".repeat(64)))
            .collect(),
        groups,
    );
    match_proven_audio_groups(local, remote, "musicbrainz", &["Lead".into()], &proof)
}

#[test]
fn compatible_copies_share_one_occurrence_without_sharing_local_metadata() {
    let mut first = local("Song (Studio Version)", 1, 203.0);
    first.artists.push("Guest A".into());
    let mut copy = first.clone();
    copy.filename = Some("another copy".into());
    copy.artists = vec!["Lead".into(), "Guest B".into(), "Guest C".into()];
    let other = local("Other", 2, 187.0);
    let locals = vec![first.clone(), other, copy.clone()];
    let remote = vec![
        provider("Other", 2, 187.0, "other"),
        provider("Song", 1, 203.0, "song"),
    ];
    let matched = align(&locals, &remote, vec![vec![0, 2], vec![1]]).unwrap();
    assert_eq!(matched.remote_indices, vec![Some(1), Some(0), Some(1)]);
    assert_eq!(matched.stats.matched, 3);
    assert!(!matched.is_full_ordered_match);
    assert_eq!(matched.tracks[0].filename, first.filename);
    assert_eq!(matched.tracks[2].filename, copy.filename);
    assert_eq!(matched.tracks[0].title, first.title);
    assert_eq!(matched.tracks[2].title, copy.title);
    assert_eq!(matched.tracks[0].artists, first.artists);
    assert_eq!(matched.tracks[2].artists, copy.artists);
    assert_eq!(
        matched.tracks[2].musicbrainz_track_id.as_deref(),
        Some("song")
    );
    let reversed = vec![copy, locals[1].clone(), first];
    assert_eq!(
        align(&reversed, &remote, vec![vec![2, 0], vec![1]])
            .unwrap()
            .remote_indices,
        matched.remote_indices
    );
}

#[test]
fn equal_audio_does_not_resolve_conflicting_titles_ids_versions_or_positions() {
    let base = local("Schoolin' Life", 14, 206.0);
    let remote = vec![provider("Schoolin' Life", 14, 206.0, "school")];
    let mut conflicts = Vec::new();
    let mut title = base.clone();
    title.title = Some("Lay Up Under Me".into());
    conflicts.push(title);
    let mut version = base.clone();
    version.title = Some("Schoolin' Life (Live)".into());
    conflicts.push(version);
    let mut position = base.clone();
    position.tagged_track_number = Some(15);
    conflicts.push(position);
    let mut disc = base.clone();
    disc.disc_number = Some(2);
    conflicts.push(disc);
    let mut id = base.clone();
    id.musicbrainz_track_id = Some("not-school".into());
    conflicts.push(id);
    let mut duration = base.clone();
    duration.length = None;
    conflicts.push(duration);
    for conflicting in conflicts {
        assert!(
            align(
                &[base.clone(), conflicting.clone()],
                &remote,
                vec![vec![0, 1]]
            )
            .is_err(),
            "accepted {conflicting:?}"
        );
        assert!(align(&[conflicting, base.clone()], &remote, vec![vec![0, 1]]).is_err());
    }
}

#[test]
fn repeated_provider_recording_ids_remain_distinct_occurrences() {
    let mut one = local("Reprise", 1, 201.0);
    one.musicbrainz_track_id = Some("same-recording".into());
    let copy = one.clone();
    let mut second = local("Reprise", 3, 201.0);
    second.musicbrainz_track_id = Some("same-recording".into());
    let remote = vec![
        provider("Reprise", 1, 201.0, "same-recording"),
        provider("Reprise", 3, 201.0, "same-recording"),
    ];
    let matched = align(&[one, copy, second], &remote, vec![vec![0, 1], vec![2]]).unwrap();
    assert_eq!(matched.remote_indices, vec![Some(0), Some(0), Some(1)]);
    let mut ambiguous = local("Reprise", 1, 201.0);
    ambiguous.tagged_track_number = None;
    assert!(align(&[ambiguous.clone(), ambiguous], &remote, vec![vec![0, 1]]).is_err());
}

#[test]
fn singleton_rows_cannot_reuse_an_occurrence_without_audio_proof() {
    let locals = vec![local("Song", 1, 203.0), local("Song", 1, 203.0)];
    let remote = vec![provider("Song", 1, 203.0, "song")];
    assert!(align(&locals, &remote, vec![vec![0], vec![1]]).is_err());
}

#[test]
fn exact_id_targets_remain_reserved_before_weaker_group_titles() {
    let mut exact = local("Song", 1, 203.0);
    exact.musicbrainz_track_id = Some("song".into());
    let weak = local("Song", 1, 203.0);
    assert!(align(
        &[weak.clone(), weak, exact],
        &[provider("Song", 1, 203.0, "song")],
        vec![vec![0, 1], vec![2]]
    )
    .is_err());
}

#[test]
fn matching_filenames_cannot_silently_repair_meaningful_duplicate_titles() {
    let mut wrong = local("Wrong title", 1, 203.0);
    wrong.filename = Some("01 Song".into());
    assert!(align(
        &[wrong.clone(), wrong],
        &[provider("Song", 1, 203.0, "song")],
        vec![vec![0, 1]]
    )
    .is_err());
}
