use super::*;
use crate::commands::auto_tag_acoustid::{resolve_with_cache, AcoustidResolution};

fn collection(names: &[&str]) -> (std::path::PathBuf, std::path::PathBuf, LookupRequest) {
    let root = super::super::tests::temp_root();
    let album = root.join("Lead/Collection");
    fs::create_dir_all(&album).unwrap();
    for name in names {
        fs::copy(super::super::tests::corpus_wav(), album.join(name)).unwrap();
    }
    let mut request = build_lookup_request(&album).unwrap();
    for track in &mut request.tracks {
        track.title = Some("Song".into());
        track.artist = None;
        track.artists.clear();
        track.musicbrainz_track_id = None;
    }
    (root, album, request)
}

fn recording_id(index: usize) -> String {
    format!("11111111-1111-4111-8111-{index:012x}")
}

#[tokio::test]
async fn coverage_v3_hydration_resumes_after_reopen_and_examines_hidden_competitors() {
    for count in [5, 17] {
        let (root, _, request) = collection(&["Song.wav"]);
        let ids: Vec<_> = (0..count).map(recording_id).collect();
        let winner = ids.last().unwrap().clone();
        let expected = winner.clone();
        let (base, stop, server) = review_fixture_server(move |request| {
            if request.starts_with("POST /lookup ") {
                return ("200 OK".into(), serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":ids.iter().map(|id|serde_json::json!({"id":id})).collect::<Vec<_>>()}]}).to_string());
            }
            let id = request
                .split("/recording/")
                .nth(1)
                .unwrap()
                .split('?')
                .next()
                .unwrap();
            ("200 OK".into(), serde_json::json!({"id":id,"title":"Song","length":if id==winner {1000}else{60000},"artist-credit":[{"name":"Singer"}]}).to_string())
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::super::tests::fake_fpcalc(&fpcalc);
        let mut results = Vec::new();
        for _ in 0..count.div_ceil(4) {
            let cache = CacheState::new_in(root.join("cache"));
            assert!(cache.initialize(None));
            results.push(
                resolve_with_cache(
                    &request,
                    Some("fixture"),
                    &providers,
                    &AtomicBool::new(false),
                    &client,
                    &fpcalc,
                    Some(&cache),
                )
                .await,
            );
        }
        stop.store(true, Ordering::Release);
        let sent = server.join().unwrap();
        assert_eq!(
            sent.iter().filter(|r| r.contains("/recording/")).count(),
            count,
            "reopen reuses facts and reaches every competitor without raising the per-pass budget"
        );
        let passes = results.len();
        for (index, result) in results.into_iter().enumerate() {
            let AcoustidResolution::Recordings {
                tracks, evidence, ..
            } = result
            else {
                panic!("recording resolution expected")
            };
            assert_eq!(
                tracks[0].musicbrainz_track_id.as_deref(),
                (index + 1 == passes).then_some(expected.as_str()),
                "unexamined fifth or seventeenth identity must block an early winner"
            );
            assert!(evidence.track_matches[0].candidates.len() <= 16);
            if index + 1 == passes {
                assert!(
                    evidence.track_matches[0]
                        .candidates
                        .iter()
                        .any(|c| c["canonicalRecordingId"] == expected),
                    "the bounded display must retain the chosen identity even beyond entry16"
                );
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn coverage_v3_complete_field_consensus_has_no_recording_or_edition_identity() {
    let (root, _, request) = collection(&["Song.wav"]);
    let (base, stop, server) = review_fixture_server(|request| {
        if request.starts_with("POST /lookup ") {
            return ("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording_id(0)},{"id":recording_id(1)}]}]}).to_string());
        }
        let id = request
            .split("/recording/")
            .nth(1)
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        ("200 OK".into(),serde_json::json!({"id":id,"title":"Song","length":1000,"artist-credit":[{"name":"Singer"}]}).to_string())
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    super::super::tests::fake_fpcalc(&fpcalc);
    let result = resolve_with_cache(
        &request,
        Some("fixture"),
        &providers,
        &AtomicBool::new(false),
        &client,
        &fpcalc,
        None,
    )
    .await;
    stop.store(true, Ordering::Release);
    server.join().unwrap();
    let AcoustidResolution::Recordings {
        tracks,
        shared_album,
        evidence,
        ..
    } = result
    else {
        panic!("recordings expected")
    };
    assert_eq!(
        tracks[0].title.as_deref(),
        Some("Song"),
        "complete identical fields may be shared without choosing an identity"
    );
    assert_eq!(tracks[0].artist.as_deref(), Some("Singer"));
    assert!(tracks[0].musicbrainz_track_id.is_none() && shared_album.is_none());
    assert!(evidence.track_matches[0].recording_id.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn coverage_v3_consensus_flows_through_normal_writer_without_an_id() {
    for (live, lead) in [(false, "Singer"), (true, "Singer"), (true, "Live")] {
        let name = if live {
            format!("{lead}-Song(Live).wav")
        } else {
            "Song.wav".to_string()
        };
        let title = if live { "Song (Live)" } else { "Song" };
        let (root, album, _) = collection(&[&name]);
        let file = album.join(&name);
        let queue = WriteQueue::default();
        write_track_queued(&queue,file.clone(),serde_json::from_value(serde_json::json!({"title":title,"artist":lead,"album":"Personal collection","trackNumber":7})).unwrap()).await.unwrap();
        let audio = super::super::tests::wav_audio_payload(&file);
        let (base, stop, server) = review_fixture_server(move |request| {
            if request.starts_with("POST /lookup ") {
                return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording_id(0)},{"id":recording_id(1)}]}]}).to_string());
            }
            if request.contains("/recording/") {
                let id = request
                    .split("/recording/")
                    .nth(1)
                    .unwrap()
                    .split('?')
                    .next()
                    .unwrap();
                return("200 OK".into(),serde_json::json!({"id":id,"title":"Song","length":1000,"disambiguation":if live {Some("live, from World Tour")}else{None},"artist-credit":if live {serde_json::json!([{"name":lead,"joinphrase":" feat. "},{"name":"Guest"}])}else{serde_json::json!([{"name":lead}])}}).to_string());
            }
            (
                "200 OK".into(),
                serde_json::json!({"artists":[],"releases":[],"artist-count":0,"release-count":0})
                    .to_string(),
            )
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let cache = CacheState::new_in(root.join("cache"));
        assert!(cache.initialize(None));
        let fpcalc = root.join("fpcalc");
        super::super::tests::fake_fpcalc(&fpcalc);
        for pass in 0..2 {
            let before = fs::read(&file).unwrap();
            let result = resolve_and_apply_album_with_acoustid_override(
                &album,
                &AutoTagConfig {
                    acoustid_enabled: Some(true),
                    acoustid_api_key: Some("fixture".into()),
                    remote_lookup_enabled: Some(true),
                    discogs_enabled: Some(false),
                    ..AutoTagConfig::default()
                },
                AutoTagServices {
                    providers: &providers,
                    cache: &cache,
                    queue: &queue,
                    alias_file: &root.join("aliases"),
                },
                &AtomicBool::new(false),
                None,
                |_| Ok(()),
                Some((&client, &fpcalc)),
                |_, _| {},
                |_, _, _| {},
            )
            .await;
            let result = result.unwrap();
            assert_eq!(
                result.written, 1,
                "ID-free verified fields must not be discarded by the early caller"
            );
            let after = read_track_metadata(&file).unwrap();
            assert_eq!(after.title.as_deref(), Some(title));
            assert_eq!(
                after.artist.as_deref(),
                Some(
                    if live {
                        format!("{lead} feat. Guest")
                    } else {
                        lead.to_string()
                    }
                    .as_str()
                )
            );
            assert_eq!(
                after.artists,
                if live {
                    vec![lead, "Guest"]
                } else {
                    vec![lead]
                }
            );
            assert_eq!(result.outcome, AutoTagOutcome::Applied);
            assert_eq!(result.completion_basis, Some("verified_track_core"));
            assert_eq!(after.album.as_deref(), Some("Personal collection"));
            assert_eq!(after.track_number, Some(7));
            assert!(after.musicbrainz_track_id.is_none() && after.musicbrainz_album_id.is_none());
            assert_eq!(super::super::tests::wav_audio_payload(&file), audio);
            if pass > 0 {
                assert_eq!(
                    fs::read(&file).unwrap(),
                    before,
                    "verified ID-free live credits must stay stable on retry"
                );
            }
        }
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn coverage_v3_failed_identity_does_not_starve_later_candidates_or_become_no_match() {
    let (root, _, request) = collection(&["Song.wav"]);
    let ids: Vec<_> = (0..5).map(recording_id).collect();
    let failed = ids[0].clone();
    let winner = ids[4].clone();
    let failure_id = failed.clone();
    let (base, stop, server) = review_fixture_server(move |request| {
        if request.starts_with("POST /lookup ") {
            return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":ids.iter().map(|id|serde_json::json!({"id":id})).collect::<Vec<_>>()}]}).to_string());
        }
        let id = request
            .split("/recording/")
            .nth(1)
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        if id == failed {
            return ("404 Not Found".into(), "{}".into());
        }
        ("200 OK".into(),serde_json::json!({"id":id,"title":"Song","length":if id==winner{1000}else{60000},"artist-credit":[{"name":"Singer"}]}).to_string())
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    super::super::tests::fake_fpcalc(&fpcalc);
    let mut result = None;
    for _ in 0..2 {
        let cache = CacheState::new_in(root.join("cache"));
        assert!(cache.initialize(None));
        result = Some(
            resolve_with_cache(
                &request,
                Some("fixture"),
                &providers,
                &AtomicBool::new(false),
                &client,
                &fpcalc,
                Some(&cache),
            )
            .await,
        );
    }
    stop.store(true, Ordering::Release);
    let sent = server.join().unwrap();
    assert_eq!(
        sent.iter().filter(|r| r.contains("/recording/")).count(),
        5,
        "failed first identity is retained while unseen later identities are examined"
    );
    assert_eq!(
        sent.iter()
            .filter(|r| r.contains(&format!("/recording/{failure_id}")))
            .count(),
        1
    );
    let AcoustidResolution::Recordings {
        tracks, evidence, ..
    } = result.unwrap()
    else {
        panic!("unknown identity cannot be confirmed no-match")
    };
    assert!(tracks[0].title.is_none());
    assert!(!evidence.track_matches[0].candidate_set_complete);
    let failure = evidence.track_matches[0]
        .candidates
        .iter()
        .find(|c| c["recordingId"] == failure_id)
        .unwrap();
    assert_eq!(failure["detailFailure"]["status"], 404);
    assert_eq!(failure["detailFailure"]["retryable"], false);
    let observed = serde_json::to_value(&evidence.track_matches[0]).unwrap();
    assert_eq!(
        observed["lookupObservation"]["status"], "successful_nonempty",
        "a missing provider detail is not an absent acoustic lookup or confirmed no-match"
    );
    assert_eq!(observed["lookupObservation"]["resultCount"], 1);
    assert!(observed["lookupObservation"]["observedAt"].is_u64());
    assert_eq!(
        observed["lookupObservation"]["fullAlternativeCount"],
        serde_json::Value::Null
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn coverage_v3_timeout_keeps_progress_but_never_patches_before_duplicate_validation() {
    for cancel_at_deadline in [false, true] {
        let (root, album, request) = collection(&["1.wav", "2.wav", "3.wav"]);
        let before: Vec<_> = collect_audio_files(&album)
            .iter()
            .map(fs::read)
            .collect::<Result<_, _>>()
            .unwrap();
        let (base, stop, server) = review_fixture_server(|request| {
            if request.starts_with("POST /lookup ") {
                return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording_id(0)}]}]}).to_string());
            }
            ("200 OK".into(),serde_json::json!({"id":recording_id(0),"title":"Song","length":1000,"artist-credit":[{"name":"Singer"}]}).to_string())
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::super::tests::fake_fpcalc(&fpcalc);
        let marker = root.join("blocked");
        fs::write(&fpcalc,format!("#!/bin/sh\nfor audio do :; done\ncase \"$audio\" in *2.wav) touch '{}'; exec sleep 20;; esac\nprintf '%s' '{{\"duration\":1,\"fingerprint\":\"fixture\"}}'\n",marker.display())).unwrap();
        let cancelled = AtomicBool::new(false);
        let deadline = async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            if cancel_at_deadline {
                cancelled.store(true, Ordering::Release);
            }
        };
        let result = crate::commands::auto_tag_acoustid::resolve_with_deadline(
            &request,
            Some("fixture"),
            &providers,
            &cancelled,
            &client,
            &fpcalc,
            None,
            deadline,
        )
        .await;
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        if cancel_at_deadline {
            assert!(
                matches!(result,AcoustidResolution::Unavailable(reason) if reason.contains("cancel")),
                "external cancellation must win over budget retry"
            );
        } else {
            let AcoustidResolution::Recordings {
                ref tracks,
                ref evidence,
                retryable,
                ..
            } = result
            else {
                panic!("timeout must retain per-file evidence")
            };
            assert!(
                retryable
                    && tracks
                        .iter()
                        .all(|t| t.title.is_none() && t.musicbrainz_track_id.is_none())
            );
            assert_eq!(evidence.track_matches.len(), 3);
            let unattempted = serde_json::to_value(&evidence.track_matches[2]).unwrap();
            assert_eq!(unattempted["lookupObservation"]["status"], "not_attempted");
            assert!(unattempted["lookupObservation"]["eligibleHypothesisCount"].is_null());
            assert_eq!(
                evidence.track_matches[0].candidates[0]["authoritativeTitle"],
                "Song"
            );
            let applied = apply_recording_resolution(
                &album,
                &request,
                result,
                &WriteQueue::default(),
                &mut |_| Ok(()),
            )
            .await
            .unwrap();
            assert_eq!(applied.written, 0);
        }
        let after: Vec<_> = collect_audio_files(&album)
            .iter()
            .map(fs::read)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(before, after);
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn coverage_v3_confirmed_no_match_has_complete_durable_diagnostics() {
    let (root, _, request) = collection(&["Song.wav"]);
    let (base, stop, server) = review_fixture_server(|_| {
        (
            "200 OK".into(),
            serde_json::json!({"status":"ok","results":[]}).to_string(),
        )
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    super::super::tests::fake_fpcalc(&fpcalc);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(serde_json::Value::Null));
    let captured = seen.clone();
    let checkpoint: EvidenceCheckpoint = std::sync::Arc::new(move |value| {
        *captured.lock().unwrap() = value;
        Ok(())
    });
    let result = EVIDENCE_CHECKPOINT
        .scope(
            checkpoint,
            resolve_with_cache(
                &request,
                Some("fixture"),
                &providers,
                &AtomicBool::new(false),
                &client,
                &fpcalc,
                None,
            ),
        )
        .await;
    stop.store(true, Ordering::Release);
    server.join().unwrap();
    assert_eq!(result, AcoustidResolution::NoMatch);
    assert_eq!(
        seen.lock().unwrap()["acoustid"]["validationComplete"],
        true,
        "confirmed empty differs from interrupted validation"
    );
    let retained = seen.lock().unwrap();
    let observation = &retained["acoustid"]["trackMatches"][0]["lookupObservation"];
    assert_eq!(observation["responseUrl"], format!("{base}/lookup"));
    assert_eq!(observation["httpStatus"], 200);
    use sha2::{Digest, Sha256};
    let body = serde_json::json!({"status":"ok","results":[]}).to_string();
    assert_eq!(
        observation["responseDigest"],
        format!("{:x}", Sha256::digest(body.as_bytes()))
    );
    assert_eq!(
        observation["responseDigestBasis"],
        "http_decoded_body_sha256"
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn coverage_v3_recording_retry_after_stops_other_uncached_identity_requests() {
    let (root, _, request) = collection(&["Song.wav"]);
    let mut lookup = 0;
    let (base, stop, server) = review_fixture_server(move |request| {
        if request.starts_with("POST /lookup ") {
            lookup += 1;
            let start = if lookup <= 2 { 0 } else { 5 };
            return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":(start..start+5).map(|i|serde_json::json!({"id":recording_id(i)})).collect::<Vec<_>>()}]}).to_string());
        }
        (
            "429 Too Many Requests\r\nRetry-After: 120".into(),
            "{}".into(),
        )
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    super::super::tests::fake_fpcalc(&fpcalc);
    let cache = CacheState::new_in(root.join("cache"));
    assert!(cache.initialize(None));
    let result = resolve_with_cache(
        &request,
        Some("fixture"),
        &providers,
        &AtomicBool::new(false),
        &client,
        &fpcalc,
        Some(&cache),
    )
    .await;
    drop(cache);
    let cache = CacheState::new_in(root.join("cache"));
    assert!(cache.initialize(None));
    let _ = resolve_with_cache(
        &request,
        Some("fixture"),
        &providers,
        &AtomicBool::new(false),
        &client,
        &fpcalc,
        Some(&cache),
    )
    .await;
    drop(cache);
    let cache = CacheState::new_in(root.join("cache"));
    assert!(cache.initialize(None));
    let _ = resolve_with_cache(
        &request,
        Some("fixture"),
        &providers,
        &AtomicBool::new(false),
        &client,
        &fpcalc,
        Some(&cache),
    )
    .await;
    stop.store(true, Ordering::Release);
    let sent = server.join().unwrap();
    assert_eq!(
        sent.iter().filter(|r| r.contains("/recording/")).count(),
        1,
        "Retry-After applies across identities, not just repeated requests for the same ID"
    );
    let AcoustidResolution::Recordings {
        tracks,
        retryable,
        retry_after_seconds,
        ..
    } = result
    else {
        panic!("retryable evidence expected")
    };
    assert!(retryable);
    assert_eq!(retry_after_seconds, Some(120));
    assert!(tracks[0].title.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn coverage_v3_tied_possible_id_audio_conflicts_block_both_sides_only() {
    for (tied_second, same_audio, missing_raw, unidentified_first) in [
        (false, false, false, false),
        (true, false, false, false),
        (false, true, false, false),
        (true, true, false, false),
        (true, true, true, false),
        (false, false, false, true),
    ] {
        let (root, _, mut request) = collection(&["1.wav", "2.wav", "3.wav"]);
        for track in &mut request.tracks {
            track.length = Some(240.0);
        }
        let mut index = 0;
        let (base, stop, server) = review_fixture_server(move |request| {
            if request.starts_with("POST /lookup ") {
                let ids = if index == 0 || (index == 1 && tied_second) {
                    vec![recording_id(0), recording_id(1)]
                } else if index == 1 {
                    vec![recording_id(0)]
                } else {
                    vec![recording_id(2)]
                };
                index += 1;
                let mut results = vec![
                    serde_json::json!({"score":0.99,"recordings":ids.into_iter().map(|id|serde_json::json!({"id":id})).collect::<Vec<_>>()}),
                ];
                if unidentified_first && index == 1 {
                    results.push(serde_json::json!({"score":0.99,"recordings":[]}));
                }
                return (
                    "200 OK".into(),
                    serde_json::json!({"status":"ok","results":results}).to_string(),
                );
            }
            let id = request
                .split("/recording/")
                .nth(1)
                .unwrap()
                .split('?')
                .next()
                .unwrap();
            ("200 OK".into(),serde_json::json!({"id":id,"title":"Song","length":240000,"artist-credit":[{"name":"Singer"}]}).to_string())
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::super::tests::fake_fpcalc(&fpcalc);
        let frames: Vec<_> = (0..300u32).map(|i| i.wrapping_mul(2654435761)).collect();
        let other: Vec<_> = frames.iter().map(|v| !v).collect();
        fs::write(&fpcalc,format!("#!/bin/sh\nif [ \"$2\" = '-raw' ]; then\nfor audio do :; done\ncase \"$audio\" in *2.wav) printf '%s' '{{\"fingerprint\":{}}}';; *) printf '%s' '{{\"fingerprint\":{}}}';; esac\nelse printf '%s' '{{\"duration\":240,\"fingerprint\":\"same-prefix\"}}'; fi\n",serde_json::to_string(if same_audio{&frames}else{&other}).unwrap(),serde_json::to_string(&frames).unwrap())).unwrap();
        if missing_raw {
            fs::write(
                &fpcalc,
                r#"#!/bin/sh
printf '%s' '{"duration":240,"fingerprint":"same-prefix"}'
"#,
            )
            .unwrap();
        }
        let result = resolve_with_cache(
            &request,
            Some("fixture"),
            &providers,
            &AtomicBool::new(false),
            &client,
            &fpcalc,
            None,
        )
        .await;
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        let AcoustidResolution::Recordings {
            tracks, evidence, ..
        } = result
        else {
            panic!("partial recordings expected")
        };
        assert_eq!(tracks[0].title.is_some(),same_audio && !missing_raw,"ties must compare full audio against both tied and uniquely selected files sharing any possible ID");
        assert_eq!(
            tracks[1].title.is_some(),
            same_audio && !missing_raw,
            "the uniquely selected side cannot bypass a contradictory tied counterpart"
        );
        assert_eq!(
            tracks[2].musicbrainz_track_id,
            Some(recording_id(2)),
            "unrelated recording remains independently writable"
        );
        assert!(tracks[0].musicbrainz_track_id.is_none());
        if missing_raw {
            assert_eq!(
                evidence.track_matches[0].reason.as_deref(),
                Some("duplicate_comparison_incomplete")
            );
        }
        if tied_second {
            assert!(tracks[1].musicbrainz_track_id.is_none());
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn coverage_v3_folder_budget_resumes_without_raising_per_file_limits() {
    let names: Vec<_> = (0..9).map(|i| format!("{i}.wav")).collect();
    let refs: Vec<_> = names.iter().map(String::as_str).collect();
    let (root, _, request) = collection(&refs);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let mut lookup = 0;
    let (base, stop, server) = review_fixture_server(move |request| {
        if request.starts_with("POST /lookup ") {
            let start = (lookup % 9) * 4;
            lookup += 1;
            return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":(start..start+4).map(|i|serde_json::json!({"id":recording_id(i)})).collect::<Vec<_>>()}]}).to_string());
        }
        counted.fetch_add(1, Ordering::AcqRel);
        let id = request
            .split("/recording/")
            .nth(1)
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        ("200 OK".into(),serde_json::json!({"id":id,"title":"Song","length":1000,"artist-credit":[{"name":"Singer"}]}).to_string())
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    super::super::tests::fake_fpcalc(&fpcalc);
    let mut outcomes = Vec::new();
    let mut counts = Vec::new();
    for _ in 0..2 {
        let cache = CacheState::new_in(root.join("cache"));
        assert!(cache.initialize(None));
        outcomes.push(
            crate::commands::auto_tag_acoustid::resolve_with_deadline(
                &request,
                Some("fixture"),
                &providers,
                &AtomicBool::new(false),
                &client,
                &fpcalc,
                Some(&cache),
                std::future::pending(),
            )
            .await,
        );
        counts.push(calls.load(Ordering::Acquire));
    }
    stop.store(true, Ordering::Release);
    server.join().unwrap();
    assert_eq!(counts,[32,36],"cache restart must advance only remaining facts, keeping32 uncached detail lookups per folder");
    let AcoustidResolution::Recordings { ref tracks, .. } = outcomes[0] else {
        panic!("partial expected")
    };
    assert!(tracks[8].title.is_none());
    let AcoustidResolution::Recordings { ref tracks, .. } = outcomes[1] else {
        panic!("recordings expected")
    };
    assert!(tracks
        .iter()
        .all(|track| track.title.is_some() && track.musicbrainz_track_id.is_none()));
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn coverage_v3_lossy_legacy_credit_facts_cannot_prove_consensus() {
    let (root, _, request) = collection(&["Song.wav"]);
    let cache = CacheState::new_in(root.join("cache"));
    assert!(cache.initialize(None));
    for i in 0..2 {
        let id = recording_id(i);
        cache.set_release_detail("acoustid-recording-v2",&id,&serde_json::json!({"requested_id":id,"id":id,"title":"Song","artist":"Singer","artists":["Singer"],"duration":1.0,"disambiguation":null})).unwrap();
    }
    let (base, stop, server) = review_fixture_server(|request| {
        if request.starts_with("POST /lookup ") {
            return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording_id(0)},{"id":recording_id(1)}]}]}).to_string());
        }
        let id = request
            .split("/recording/")
            .nth(1)
            .unwrap()
            .split('?')
            .next()
            .unwrap();
        let mut credits = vec![serde_json::json!({"name":"Singer"})];
        if id == recording_id(1) {
            credits.push(serde_json::json!({}));
        }
        (
            "200 OK".into(),
            serde_json::json!({"id":id,"title":"Song","length":1000,"artist-credit":credits})
                .to_string(),
        )
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    super::super::tests::fake_fpcalc(&fpcalc);
    let result = resolve_with_cache(
        &request,
        Some("fixture"),
        &providers,
        &AtomicBool::new(false),
        &client,
        &fpcalc,
        Some(&cache),
    )
    .await;
    stop.store(true, Ordering::Release);
    let sent = server.join().unwrap();
    assert_eq!(sent.iter().filter(|r| r.contains("/recording/")).count(), 2);
    let AcoustidResolution::Recordings {
        tracks, evidence, ..
    } = result
    else {
        panic!("incomplete metadata expected")
    };
    assert!(
        tracks[0].title.is_none(),
        "v2 facts may have silently omitted a guest and cannot establish new consensus"
    );
    assert!(!evidence.track_matches[0].candidate_set_complete);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn coverage_v3_recording_failures_name_missing_authoritative_fields() {
    for (field, kind, replacement) in [
        ("title", "recording_title_missing", serde_json::Value::Null),
        (
            "length",
            "recording_duration_missing",
            serde_json::Value::Null,
        ),
        (
            "artist-credit",
            "recording_credit_incomplete",
            serde_json::Value::Null,
        ),
        (
            "disambiguation",
            "recording_annotation_invalid",
            serde_json::json!({"unexpected":true}),
        ),
    ] {
        let mut value = serde_json::json!({"id":recording_id(0),"title":"Song","length":1000,"artist-credit":[{"name":"Singer"}]});
        value[field] = replacement;
        let (base, stop, server) =
            review_fixture_server(move |_| ("200 OK".into(), value.to_string()));
        let provider = crate::state::providers::MusicBrainzClient::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
        );
        let result = provider.recording_by_id_result(&recording_id(0)).await;
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        let error = result.unwrap_err();
        assert_eq!(
            error.kind, kind,
            "HTTP success must not conceal which identity evidence is missing"
        );
        assert_eq!(error.status, Some(200));
        assert!(!error.retryable);
    }
}

#[tokio::test]
async fn coverage_v3_file_budget_skip_does_not_poison_another_file() {
    for redirected_conflict in [false, true] {
        let (root, _, mut request) = collection(&["1.wav", "2.wav"]);
        if redirected_conflict {
            for track in &mut request.tracks {
                track.length = Some(240.0);
            }
        }
        let mut lookups = 0;
        let (base, stop, server) = review_fixture_server(move |request| {
            if request.starts_with("POST /lookup ") {
                lookups += 1;
                let ids = if lookups == 1 {
                    (0..5).map(recording_id).collect::<Vec<_>>()
                } else {
                    vec![recording_id(4)]
                };
                return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":ids.iter().map(|id|serde_json::json!({"id":id})).collect::<Vec<_>>()}]}).to_string());
            }
            let id = request
                .split("/recording/")
                .nth(1)
                .unwrap()
                .split(['?', ' '])
                .next()
                .unwrap();
            if redirected_conflict && id == recording_id(4) {
                return (
                    format!("302 Found\r\nLocation: /mb/recording/{}", recording_id(9)),
                    "{}".into(),
                );
            }
            ("200 OK".into(),serde_json::json!({"id":id,"title":"Song","length":if id==recording_id(9){240000}else if id==recording_id(4){1000}else{60000},"artist-credit":[{"name":"Singer"}]}).to_string())
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::super::tests::fake_fpcalc(&fpcalc);
        if redirected_conflict {
            let frames: Vec<_> = (0..300u32).map(|i| i.wrapping_mul(2654435761)).collect();
            let other: Vec<_> = frames.iter().map(|v| !v).collect();
            fs::write(&fpcalc,format!("#!/bin/sh\nif [ \"$2\" = '-raw' ]; then\nfor audio do :; done\ncase \"$audio\" in *2.wav) printf '%s' '{{\"fingerprint\":{}}}';; *) printf '%s' '{{\"fingerprint\":{}}}';; esac\nelse printf '%s' '{{\"duration\":240,\"fingerprint\":\"same-prefix\"}}'; fi\n",serde_json::to_string(&other).unwrap(),serde_json::to_string(&frames).unwrap())).unwrap();
        }
        let result = resolve_with_cache(
            &request,
            Some("fixture"),
            &providers,
            &AtomicBool::new(false),
            &client,
            &fpcalc,
            None,
        )
        .await;
        stop.store(true, Ordering::Release);
        let sent = server.join().unwrap();
        assert_eq!(
            sent.iter().filter(|r| r.contains("/recording/")).count(),
            if redirected_conflict { 6 } else { 5 },
            "another physical file can spend its own quota after the first file's skip"
        );
        let AcoustidResolution::Recordings {
            tracks, evidence, ..
        } = result
        else {
            panic!("recordings expected")
        };
        if redirected_conflict {
            assert_eq!(
                evidence.track_matches[1].reason.as_deref(),
                Some("recording_audio_conflict"),
                "{evidence:?}"
            );
        }
        assert!(tracks[0].title.is_none());
        assert_eq!(
            tracks[1].musicbrainz_track_id.as_deref(),
            (!redirected_conflict).then_some(recording_id(4)).as_deref()
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn coverage_v3_cancellation_after_last_detail_prevents_a_successful_resolution() {
    let (root, _, request) = collection(&["Song.wav"]);
    let cancelled = std::sync::Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    let (base, stop, server) = review_fixture_server(move |request| {
        if request.starts_with("POST /lookup ") {
            return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording_id(0)}]}]}).to_string());
        }
        signal.store(true, Ordering::Release);
        ("200 OK".into(),serde_json::json!({"id":recording_id(0),"title":"Song","length":1000,"artist-credit":[{"name":"Singer"}]}).to_string())
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    super::super::tests::fake_fpcalc(&fpcalc);
    let result = resolve_with_cache(
        &request,
        Some("fixture"),
        &providers,
        &cancelled,
        &client,
        &fpcalc,
        None,
    )
    .await;
    stop.store(true, Ordering::Release);
    server.join().unwrap();
    assert!(
        matches!(result,AcoustidResolution::Unavailable(ref reason) if reason.contains("cancel")),
        "late cancellation must win even between polling ticks: {result:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn coverage_v3_artist_prefixed_remix_filename_reaches_verified_release() {
    for authority_conflict in [false, true] {
        let title = "Song (Olli Collins & Fred Portelli Remix)";
        let (root, _, mut request) = collection(&[if authority_conflict {
            "Song (Olli Collins _ Fred Portelli Remix).wav"
        } else {
            "Singer-Song (Olli Collins _ Fred Portelli Remix).wav"
        }]);
        request.tracks[0].title = Some(title.into());
        request.tracks[0].artist = Some("Singer".into());
        request.album_hint = Some("Album".into());
        request.artist_hint = Some("Singer".into());
        let (base, stop, server) = review_fixture_server(move |request| {
            if request.starts_with("POST /lookup ") {
                return("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording_id(0),"releases":[{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}]}]}]}).to_string());
            }
            if request.contains("/recording/") {
                return("200 OK".into(),serde_json::json!({"id":recording_id(0),"title":if authority_conflict {"Song (Other Remix)"} else {title},"length":1000,"artist-credit":[{"name":"Singer"}]}).to_string());
            }
            let mut release: serde_json::Value =
                serde_json::from_str(&review_fixture_release(&recording_id(0))).unwrap();
            release["artist-credit"] = serde_json::json!([{"name":"Singer"}]);
            let track = &mut release["media"][0]["tracks"][0];
            track["title"] = title.into();
            track["recording"]["title"] = title.into();
            track["recording"]["length"] = 1000.into();
            track["recording"]["artist-credit"] = serde_json::json!([{"name":"Singer"}]);
            ("200 OK".into(), release.to_string())
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::super::tests::fake_fpcalc(&fpcalc);
        let result = resolve_with_cache(
            &request,
            Some("fixture"),
            &providers,
            &AtomicBool::new(false),
            &client,
            &fpcalc,
            None,
        )
        .await;
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        assert!(matches!(result,AcoustidResolution::Candidate{..}) == !authority_conflict,"clearing representative credits must not discard already-supported filename evidence: {result:?}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn coverage_v3_acoustic_conflicts_never_escape_through_album_fallback() {
    for unknown in [false, true] {
        let (root, album, _) = collection(&["1.wav", "2.wav"]);
        let file = album.join("1.wav");
        let queue = WriteQueue::default();
        write_track_queued(&queue,file.clone(),serde_json::from_value(serde_json::json!({"title":"Song","artist":"Singer","album":"Personal collection","trackNumber":7})).unwrap()).await.unwrap();
        fs::copy(&file, album.join("2.wav")).unwrap();
        let audio = super::super::tests::wav_audio_payload(&file);
        let (base, stop, server) = review_fixture_server(move |request| {
            if request.starts_with("POST /lookup ") {
                let mut body = serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording_id(0)}]}]});
                if unknown {
                    body["results"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({"score":0.99,"recordings":[]}));
                }
                return ("200 OK".into(), body.to_string());
            }
            if request.contains("/recording/") {
                let id = request
                    .split("/recording/")
                    .nth(1)
                    .unwrap()
                    .split('?')
                    .next()
                    .unwrap();
                return("200 OK".into(),serde_json::json!({"id":id,"title":"Song","length":1000,"artist-credit":[{"name":"Singer"}]}).to_string());
            }
            (
                "200 OK".into(),
                serde_json::json!({"artists":[],"releases":[],"artist-count":0,"release-count":0})
                    .to_string(),
            )
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let cache = CacheState::new_in(root.join("cache"));
        assert!(cache.initialize(None));
        let fpcalc = root.join("fpcalc");
        super::super::tests::fake_fpcalc(&fpcalc);
        let frames: Vec<_> = (0..300u32).map(|i| i.wrapping_mul(2654435761)).collect();
        let other: Vec<_> = frames.iter().map(|v| !v).collect();
        fs::write(&fpcalc,format!("#!/bin/sh\nif [ \"$2\" = '-raw' ]; then\nfor audio do :; done\ncase \"$audio\" in *2.wav) printf '%s' '{{\"fingerprint\":{}}}';; *) printf '%s' '{{\"fingerprint\":{}}}';; esac\nelse printf '%s' '{{\"duration\":1,\"fingerprint\":\"same-prefix\"}}'; fi\n",serde_json::to_string(&other).unwrap(),serde_json::to_string(&frames).unwrap())).unwrap();
        let result = resolve_and_apply_album_with_acoustid_override(
            &album,
            &AutoTagConfig {
                acoustid_enabled: Some(true),
                acoustid_api_key: Some("fixture".into()),
                remote_lookup_enabled: Some(true),
                discogs_enabled: Some(false),
                ..AutoTagConfig::default()
            },
            AutoTagServices {
                providers: &providers,
                cache: &cache,
                queue: &queue,
                alias_file: &root.join("aliases"),
            },
            &AtomicBool::new(false),
            None,
            |_| Ok(()),
            Some((&client, &fpcalc)),
            |_, _| {},
            |_, _, _| {},
        )
        .await;
        stop.store(true, Ordering::Release);
        let sent = server.join().unwrap();
        assert!(result.is_ok(), "conflict is a review outcome: {result:?}");
        assert!(
            sent.iter()
                .all(|r| r.starts_with("POST /lookup ") || r.contains("/recording/")),
            "album lookup must not escape an explicit acoustic rejection: {sent:?}"
        );
        assert_eq!(result.unwrap().written, 0);
        assert_eq!(super::super::tests::wav_audio_payload(&file), audio);
        fs::remove_dir_all(root).unwrap();
    }
}
