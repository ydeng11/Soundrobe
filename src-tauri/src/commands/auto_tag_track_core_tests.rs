use super::*;
use crate::commands::auto_tag_acoustid::{
    AcoustidEvidence, AcoustidResolution, AcoustidTrackEvidence,
};

const RECORDING: &str = "11111111-1111-4111-8111-111111111111";

#[tokio::test]
async fn receipt_payload_uses_established_writer_preparation() {
    for kind in ["flac", "wav"] {
        let (root, album, queue) = fixture(0).await;
        let path = album.join(format!("0.{kind}"));
        if kind == "flac" {
            fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../test/fixtures/tauri/writer-corpus/flac-trailing-ape.flac"),
                &path,
            )
            .unwrap();
        } else {
            let mut bytes = fs::read(super::super::tests::corpus_wav()).unwrap();
            bytes.extend_from_slice(&[255, 255, 255, 255, 255, 255, 0, 0]);
            let length = (bytes.len() as u32 - 8).to_le_bytes();
            bytes[4..8].copy_from_slice(&length);
            fs::write(&path, bytes).unwrap();
        }
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
            result.core_receipts.len(),
            1,
            "{kind}: {:?}",
            result.reason_code
        );
        assert_eq!(result.completion_basis, Some("verified_track_core"));
        result.core_receipts[0].validate_current(&path).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn queued_recording_write_rejects_post_guard_source_replacement() {
    for changed_audio in [true, false] {
        if !changed_audio && !cfg!(unix) {
            continue; // Identical-byte physical replacement detection is Unix-only.
        }
        let (root, album, queue) = fixture(1).await;
        let path = album.join("0.wav");
        let expected = RecordingSource::capture(&path).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        if changed_audio {
            let offset = bytes.windows(4).position(|b| b == b"data").unwrap() + 8;
            bytes[offset] ^= 65;
        }
        let replacement = root.join("replacement.wav");
        fs::write(&replacement, &bytes).unwrap();
        let target = path.clone();
        let original = fs::read(&path).unwrap();
        let patch = serde_json::from_value(serde_json::json!({"title":"Song"})).unwrap();
        let result = write_track_queued_guarded(
            &queue,
            path.clone(),
            patch,
            true,
            Some(expected),
            move |input| {
                assert_eq!(fs::read(input).unwrap(), original);
                fs::rename(replacement, target).unwrap();
                track_payload_digest(input)
            },
            |_, _, _| Ok(()),
        )
        .await;
        assert!(
            result.is_err(),
            "replacement after admission must not inherit lookup evidence"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            bytes,
            "do not tag the external replacement"
        );
        assert_eq!(
            fs::read_dir(&album).unwrap().count(),
            1,
            "staged inputs must be cleaned"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn desktop_completion_publishes_source_bound_receipts() {
    use crate::commands::auto_tag_review::{ReviewStore, ACTIVE_REVIEW};
    use crate::state::worker::WorkerStore;
    for fault in ["none", "cancelled", "stale", "database"] {
        let (root, album, queue) = fixture(1).await;
        let request = bound_request(&album);
        let reviews = ReviewStore::default();
        let tasks = TaskRegistry::default();
        let run = tasks.create("desktop-run", 9, "Starting");
        let worker = WorkerStore::new_in(root.join("state"));
        assert!(worker.initialize());
        worker
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        worker.mark_running(&album, &run).unwrap();
        let journal = queue
            .run_exclusive(async { reviews.begin(&run, &album) })
            .await
            .unwrap();
        let result = ACTIVE_REVIEW
            .scope(
                journal,
                apply_recording_resolution(
                    &album,
                    &request,
                    recognized(&request, false),
                    &queue,
                    &mut |_| Ok(()),
                ),
            )
            .await
            .unwrap();
        let data = serde_json::to_value(&result).unwrap();
        queue
            .run_exclusive(async { reviews.finish(&run, "applied", data.clone()) })
            .await
            .unwrap();
        match fault {
            "cancelled" => {
                tasks.cancel(&run);
            }
            "stale" => {
                worker
                    .requeue(std::slice::from_ref(&album), "explicit-retry")
                    .unwrap();
                worker.mark_running(&album, "new-owner").unwrap();
            }
            "database" => {
                rusqlite::Connection::open(root.join("state/worker.db")).unwrap().execute_batch("CREATE TRIGGER reject_proof BEFORE INSERT ON worker_evidence BEGIN SELECT RAISE(FAIL, 'proof unavailable'); END;").unwrap();
            }
            _ => {}
        }
        let completion =
            complete_auto_tag_run(&worker, &tasks, &queue, &album, &run, &result, data).await;
        if fault == "none" {
            let (status, _, published) = completion.unwrap();
            assert_eq!(status, TaskStatus::Completed);
            assert_eq!(published["coreReceipts"][0]["runId"], run);
            assert_eq!(tasks.get(&run).unwrap().result, published);
        } else {
            assert!(completion.is_err(), "{fault}");
            assert_ne!(tasks.get(&run).unwrap().status, TaskStatus::Completed);
        }
        drop(worker);
        let reopened = WorkerStore::new_in(root.join("state"));
        assert!(reopened.initialize());
        let retained = reopened.evidence(&album, &run).unwrap();
        assert_eq!(retained.is_some(), fault == "none");
        if let Some(proof) = retained {
            assert_eq!(proof["coreReceipts"][0]["runId"], run);
        }
        drop(reopened);
        reviews.clear().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn terminal_publication_waits_for_queued_mutation_and_revalidates() {
    use crate::state::worker::WorkerStore;
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    let (root, album, queue) = fixture(1).await;
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
    let data = serde_json::to_value(&result).unwrap();
    let tasks = TaskRegistry::default();
    let run = tasks.create("terminal", 9, "Running");
    let worker = WorkerStore::new_in(root.join("state"));
    assert!(worker.initialize());
    worker
        .enqueue(std::slice::from_ref(&album), "manual")
        .unwrap();
    worker.mark_running(&album, &run).unwrap();
    let (entered, held) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let writing_queue = queue.clone();
    let path = album.join("0.wav");
    let writer = tokio::spawn(async move {
        writing_queue
            .run_exclusive(async move {
                entered.send(()).unwrap();
                released.await.unwrap();
                let patch =
                    serde_json::from_value(serde_json::json!({"title":"External correction"}))
                        .unwrap();
                crate::commands::mutations::write_wav_atomic(&path, &patch).unwrap();
            })
            .await
    });
    held.await.unwrap();
    let mut completion = Box::pin(complete_auto_tag_run(
        &worker, &tasks, &queue, &album, &run, &result, data,
    ));
    let waiting = poll_fn(|cx| Poll::Ready(completion.as_mut().poll(cx).is_pending())).await;
    assert!(
        waiting,
        "terminal validation/commit must wait for the existing mutation owner"
    );
    release.send(()).unwrap();
    writer.await.unwrap();
    assert!(completion.await.is_err());
    assert_eq!(tasks.get(&run).unwrap().status, TaskStatus::Running);
    assert!(worker.evidence(&album, &run).unwrap().is_none());
    drop(worker);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "server")]
#[tokio::test]
async fn durable_receipts_cannot_certify_mismatched_evidence_or_nonterminal_runs() {
    use crate::state::worker::WorkerStore;
    for fault in [
        "ordered_credit",
        "dropped_guest",
        "id",
        "rule",
        "evidence_id",
        "checkpoint",
        "database",
        "cancelled",
        "cancelled_empty",
        "stale",
        "replaced",
    ] {
        if fault == "replaced" && !cfg!(unix) {
            continue; // Non-Unix receipts bind content, not native file identity.
        }
        let (root, album, queue) = fixture(1).await;
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
        let mut data = serde_json::to_value(result).unwrap();
        let store = WorkerStore::new_in(root.join("state"));
        assert!(store.initialize());
        store
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        store.mark_running(&album, "run").unwrap();
        store
            .record_review_flag(&album, "recording_identity", "run")
            .unwrap();
        match fault {
            "ordered_credit" => {
                data["coreReceipts"][0]["readback"]["artists"] =
                    serde_json::json!(["Guest", "Lead"])
            }
            "dropped_guest" => {
                data["coreReceipts"][0]["readback"]["artists"] = serde_json::json!(["Lead"])
            }
            "id" => data["coreReceipts"][0]["readback"]["musicbrainzTrackId"] = "other".into(),
            "rule" => data["coreReceipts"][0]["decisionRuleVersion"] = 2.into(),
            "evidence_id" => {
                data["coreReceipts"][0]["evidence"]["trackMatch"]["recordingId"] =
                    "22222222-2222-4222-8222-222222222222".into();
                data["coreReceipts"][0]["evidenceDigest"] = format!(
                    "{:x}",
                    Sha256::digest(data["coreReceipts"][0]["evidence"].to_string().as_bytes())
                )
                .into();
            }
            "checkpoint" => {
                assert!(store.checkpoint_evidence(&album, "run", &data).is_err());
                store
                    .checkpoint_evidence(
                        &album,
                        "run",
                        &serde_json::json!({"pendingCoreReceipts":data["coreReceipts"]}),
                    )
                    .unwrap();
                fs::copy(album.join("0.wav"), album.join("unobserved.wav")).unwrap();
                let export = crate::commands::artist_inventory::inventory(
                    &album,
                    Some(&root.join("state")),
                    None,
                )
                .unwrap();
                assert_eq!(export["files"][0]["coreProof"]["state"], "pending");
                assert_eq!(
                    export["files"][1]["coreProof"]["state"], "absent",
                    "one pending write cannot imply that another physical file was observed"
                );
                assert_eq!(export["summary"]["verifiedCoreFiles"], 0);
                fs::remove_dir_all(root).unwrap();
                continue;
            }
            "database" => {
                rusqlite::Connection::open(root.join("state/worker.db")).unwrap().execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON worker_evidence BEGIN SELECT RAISE(ABORT, 'injected receipt persistence failure'); END;").unwrap();
            }
            "cancelled" | "cancelled_empty" => {
                store
                    .cancel_paths(std::slice::from_ref(&album), "cancel")
                    .unwrap();
                if fault == "cancelled_empty" {
                    data["completionBasis"] = serde_json::Value::Null;
                    data["coreReceipts"] = serde_json::json!([]);
                }
            }
            "stale" => {
                store
                    .requeue(std::slice::from_ref(&album), "explicit-retry")
                    .unwrap();
                store.mark_running(&album, "new-run").unwrap();
            }
            "replaced" => {
                let path = album.join("0.wav");
                let replacement = root.join("same.wav");
                fs::copy(&path, &replacement).unwrap();
                fs::rename(replacement, path).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            store
                .finish_with_evidence(
                    &album,
                    "run",
                    if fault == "cancelled_empty" {
                        "needs_review"
                    } else {
                        "completed"
                    },
                    None,
                    None,
                    &data
                )
                .is_err(),
            "{fault} must not certify core"
        );
        assert!(store.evidence(&album, "run").unwrap().is_none(), "{fault}");
        assert_eq!(store.unreviewed_review_paths().unwrap(), vec![album]);
        fs::remove_dir_all(root).unwrap();
    }
}

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
            lookup_observation: None,
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

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires native ffmpeg; optional SOUNDROBE_PROOF_MEDIA is copied before mutation"]
async fn native_core_receipt_preserves_complete_pcm_and_is_idempotent() {
    use std::process::Command;
    let root = super::super::tests::temp_root();
    let album = root.join("Generated");
    fs::create_dir_all(&album).unwrap();
    let flac = album.join("0.flac");
    let generated = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=0.1*sin(523*2*PI*t)|0.2*sin(311*2*PI*t):s=44100:d=1",
            "-c:a",
            "flac",
            "-sample_fmt",
            "s32",
        ])
        .arg(&flac)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    if let Some(source) = std::env::var_os("SOUNDROBE_PROOF_MEDIA") {
        let source = Path::new(&source);
        let extension = source.extension().unwrap();
        fs::copy(
            source,
            album.join(PathBuf::from("1").with_extension(extension)),
        )
        .unwrap();
    }
    let decode = |path: &str| {
        let output = Command::new("ffmpeg")
            .args([
                "-v",
                "warning",
                "-xerror",
                "-i",
                path,
                "-map",
                "0:a:0",
                "-f",
                "f32le",
                "-c:a",
                "pcm_f32le",
                "-",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "native validation warnings must be inspected: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
        output.stdout
    };
    let files = collect_audio_files(&album);
    let pcm: Vec<_> = files.iter().map(|p| decode(p)).collect();
    assert_eq!(
        pcm[0].len(),
        44100 * 2 * 4,
        "independently constructed complete stereo timeline"
    );
    let queue = WriteQueue::default();
    for pass in 0..2 {
        let before: Vec<_> = files.iter().map(|p| fs::read(p).unwrap()).collect();
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
        assert_eq!(result.core_receipts.len(), files.len());
        for (index, file) in files.iter().enumerate() {
            result.core_receipts[index]
                .validate_current(Path::new(file))
                .unwrap();
            assert_eq!(
                decode(file),
                pcm[index],
                "complete decoded payload must be unchanged"
            );
            if pass == 1 {
                assert_eq!(fs::read(file).unwrap(), before[index]);
            }
        }
    }
    fs::remove_dir_all(root).unwrap();
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
