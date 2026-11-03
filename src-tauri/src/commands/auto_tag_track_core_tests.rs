use super::*;
use crate::commands::auto_tag_acoustid::{
    AcoustidEvidence, AcoustidResolution, AcoustidTrackEvidence,
};

const RECORDING: &str = "11111111-1111-4111-8111-111111111111";

fn bound_request(album: &Path) -> LookupRequest {
    let mut request = build_lookup_request(album).unwrap();
    for (file, track) in collect_audio_files(album).iter().zip(&mut request.tracks) {
        track.source = Some(RecordingSource::capture(Path::new(file)).unwrap());
    }
    request
}

fn recognized(request: &LookupRequest, consensus: bool) -> AcoustidResolution {
    let tracks = request
        .tracks
        .iter()
        .map(|local| TrackCandidate {
            source: local.source.clone(),
            filename: local.filename.clone(),
            title: Some("Song".into()),
            artist: Some("Lead feat. Guest".into()),
            artists: vec!["Lead".into(), "Guest".into()],
            musicbrainz_track_id: (!consensus).then(|| RECORDING.into()),
            ..TrackCandidate::default()
        })
        .collect();
    let track_matches = request
        .tracks
        .iter()
        .map(|local| AcoustidTrackEvidence {
            file_name: local
                .source
                .as_ref()
                .unwrap()
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into(),
            recording_id: if consensus {
                String::new()
            } else {
                RECORDING.into()
            },
            score: 0.99,
            reason: consensus.then(|| "recording_fields_consensus".into()),
            candidate_set_complete: true,
            candidates: vec![], // Display evidence is not the proof contract.
            candidate_count: 1,
            omitted_candidates: 0,
        })
        .collect();
    AcoustidResolution::Recordings {
        tracks,
        shared_album: Some(Box::new(AlbumCandidate {
            album: Some("Catalog Album".into()),
            album_artists: vec!["Lead".into()],
            ..AlbumCandidate::default()
        })),
        evidence: AcoustidEvidence {
            release_id: String::new(),
            decision_rule_version: 3,
            validation_complete: true,
            track_matches,
            candidate_count: 0,
            checked_candidates: 0,
            rejections: vec![],
            pending_candidates: vec![],
        },
        reason_code: "provider_retryable".into(),
        retryable: true,
        retry_after_seconds: Some(60),
    }
}

async fn fixture(count: usize) -> (PathBuf, PathBuf, WriteQueue) {
    let root = super::super::tests::temp_root();
    let album = root.join("Personal Collection");
    fs::create_dir_all(&album).unwrap();
    let queue = WriteQueue::default();
    for i in 0..count {
        let file = album.join(format!("{i}.wav"));
        fs::copy(super::super::tests::corpus_wav(), &file).unwrap();
        let patch = serde_json::from_value(serde_json::json!({
            "album":"Personal Collection", "albumArtist":"Collector", "year":"1999",
            "trackNumber":7, "trackTotal":99, "discNumber":2, "discTotal":3,
            "musicbrainzAlbumId":"kept-edition"
        }))
        .unwrap();
        write_track_queued(&queue, file, patch).await.unwrap();
    }
    (root, album, queue)
}

#[tokio::test]
async fn complete_recording_core_finishes_without_a_catalog_edition() {
    for consensus in [false, true] {
        let (root, album, queue) = fixture(2).await;
        let request = bound_request(&album);
        let result = apply_recording_resolution(
            &album,
            &request,
            recognized(&request, consensus),
            &queue,
            &mut |_| Ok(()),
        )
        .await
        .unwrap();
        assert_eq!(result.outcome, AutoTagOutcome::Applied,
            "all physical tracks have accepted core proof; an edition-only outage is not unfinished track tagging");
        assert!(
            result.provider_attempts.iter().any(|a| a.retryable),
            "retain the edition diagnostic without making it a core retry"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn recording_core_keeps_the_existing_grouping_and_positions() {
    let (root, album, queue) = fixture(1).await;
    let before = read_album(&album).unwrap().tracks.remove(0);
    let request = bound_request(&album);
    apply_recording_resolution(
        &album,
        &request,
        recognized(&request, false),
        &queue,
        &mut |_| Ok(()),
    )
    .await
    .unwrap();
    let after = read_album(&album).unwrap().tracks.remove(0);
    assert_eq!(after.album, before.album);
    assert_eq!(after.album_artist, before.album_artist);
    assert_eq!(after.track_number, before.track_number);
    assert_eq!(after.track_total, before.track_total);
    assert_eq!(after.disc_number, before.disc_number);
    assert_eq!(after.musicbrainz_album_id, before.musicbrainz_album_id);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unchanged_recording_retry_remains_complete_without_rewriting_media() {
    let (root, album, queue) = fixture(1).await;
    let request = bound_request(&album);
    apply_recording_resolution(
        &album,
        &request,
        recognized(&request, false),
        &queue,
        &mut |_| Ok(()),
    )
    .await
    .unwrap();
    let file = album.join("0.wav");
    let once = fs::read(&file).unwrap();
    let modified = fs::metadata(&file).unwrap().modified().unwrap();
    let request = bound_request(&album);
    let result = apply_recording_resolution(
        &album,
        &request,
        recognized(&request, false),
        &queue,
        &mut |_| Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(result.outcome, AutoTagOutcome::Applied);
    assert_eq!(result.completion_basis, Some("verified_track_core"));
    assert_eq!(fs::read(&file).unwrap(), once);
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), modified);
    assert_eq!(result.diagnostics.last().unwrap()["completeAlbum"], false);
    assert_eq!(
        result.diagnostics.last().unwrap()["completeTrackCore"],
        true
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn core_completion_requires_every_physical_files_source_and_accepted_evidence() {
    let (root, album, _) = fixture(2).await;
    let request = bound_request(&album);
    for variant in 0..14 {
        let AcoustidResolution::Recordings {
            mut tracks,
            mut evidence,
            ..
        } = recognized(&request, false)
        else {
            unreachable!()
        };
        match variant {
            0 => evidence.validation_complete = false,
            1 => evidence.track_matches[1].candidate_set_complete = false,
            2 => evidence.track_matches[1].reason = Some("recording_candidates_incomplete".into()),
            3 => evidence.track_matches[1].reason = Some("recording_audio_conflict".into()),
            4 => evidence.track_matches[1].reason = Some("recording_version_conflict".into()),
            5 => tracks[1].artists.clear(),
            6 => tracks[1].artists.push(" ".into()),
            7 => tracks[1].artist = Some(" ".into()),
            8 => tracks[1].title = None,
            9 => evidence.track_matches[1].recording_id = "another-recording".into(),
            10 => evidence.track_matches[1].file_name = "another.wav".into(),
            11 => tracks[1].source = None,
            12 => tracks[1].source = tracks[0].source.clone(),
            13 => {
                evidence.track_matches.pop();
            }
            _ => unreachable!(),
        }
        assert!(
            !complete_recording_core(&request, &tracks, &evidence),
            "variant {variant}"
        );
    }
    let AcoustidResolution::Recordings {
        tracks, evidence, ..
    } = recognized(&request, false)
    else {
        unreachable!()
    };
    assert!(!complete_recording_core(
        &LookupRequest::default(),
        &[],
        &evidence
    ));
    assert!(complete_recording_core(&request, &tracks, &evidence));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_edition_display_agreement_does_not_prove_ordered_artist_credits() {
    let first = AlbumCandidate {
        tracks: vec![TrackCandidate {
            title: Some("Song".into()),
            artist: Some("Lead & Guest".into()),
            artists: vec!["Lead".into(), "Guest".into()],
            ..TrackCandidate::default()
        }],
        ..AlbumCandidate::default()
    };
    let mut second = first.clone();
    second.tracks[0].artists.reverse();
    let shared = common_recording_fields(&[first, second]).unwrap();
    assert!(
        shared.tracks[0].artist.is_none(),
        "a shared display string must not replace disputed credits"
    );
    assert!(shared.tracks[0].artists.is_empty());
}

#[tokio::test]
async fn final_coverage_uses_the_verified_physical_manifest_instead_of_an_earlier_scan() {
    let (root, album, queue) = fixture(2).await;
    let request = bound_request(&album);
    let mut earlier = request.clone();
    earlier.tracks.truncate(1);
    let mut result = apply_recording_resolution(
        &album,
        &request,
        recognized(&request, false),
        &queue,
        &mut |_| Ok(()),
    )
    .await
    .unwrap();
    finalize_album_coverage(&earlier, &mut result, false);
    let rows: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|row| row["stage"] == "coverage")
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["localFiles"], 2,
        "the completion claim must use the same manifest as its proof"
    );
    assert_eq!(rows[0]["completeTrackCore"], true);
    finalize_album_coverage(&earlier, &mut result, true);
    assert_eq!(result.outcome, AutoTagOutcome::NeedsReview);
    assert!(result.completion_basis.is_none());
    assert_eq!(
        result.diagnostics.last().unwrap()["completeTrackCore"],
        false
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn strict_core_requires_ordered_credit_readback_in_the_actual_format() {
    let (root, album, queue) = fixture(0).await;
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("test/fixtures/tauri/media-corpus/ape-id3v1-fallback.ape");
    fs::copy(corpus, album.join("0.ape")).unwrap();
    let request = bound_request(&album);
    let result = apply_recording_resolution(
        &album,
        &request,
        recognized(&request, false),
        &queue,
        &mut |_| Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(
        result.outcome,
        AutoTagOutcome::NeedsReview,
        "queue success cannot prove a format represented the ordered credits correctly"
    );
    assert!(result.completion_basis.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn replacement_while_waiting_for_queue_admission_receives_no_verified_tags() {
    let (root, album, queue) = fixture(1).await;
    let request = bound_request(&album);
    let resolution = recognized(&request, false);
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let holder_queue = queue.clone();
    let (held, unlock) = (entered.clone(), release.clone());
    let holder = tokio::spawn(async move {
        holder_queue
            .run(async move {
                held.notify_one();
                unlock.notified().await;
            })
            .await;
    });
    entered.notified().await;
    let mut intent = |_: &AlbumCandidate| -> Result<(), ApiError> { Ok(()) };
    let mut writer = Box::pin(apply_recording_resolution(
        &album,
        &request,
        resolution,
        &queue,
        &mut intent,
    ));
    // The only suspension before dispatch is queue admission: poll through
    // the batch preflight while the queue gate is held.
    std::future::poll_fn(|context| {
        use std::future::Future;
        assert!(writer.as_mut().poll(context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let file = album.join("0.wav");
    let mut replacement = fs::read(&file).unwrap();
    let audio = replacement
        .windows(4)
        .position(|bytes| bytes == b"data")
        .unwrap()
        + 8;
    replacement[audio] ^= 1;
    fs::write(&file, &replacement).unwrap();
    release.notify_one();
    holder.await.unwrap();
    assert!(
        writer.await.is_err(),
        "lookup proof belongs to the pre-queue audio, not its replacement"
    );
    assert_eq!(fs::read(&file).unwrap(), replacement);
    fs::remove_dir_all(root).unwrap();
}
