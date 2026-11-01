//! The investigation snapshot describes collection shape, never catalog truth.
use super::*;
use crate::commands::tracks::read_track_metadata;
use std::{fs, process::Command};

#[cfg(unix)]
fn review_fixture_server(
    mut respond: impl FnMut(&str) -> (String, String) + Send + 'static,
) -> (
    String,
    std::sync::Arc<AtomicBool>,
    std::thread::JoinHandle<Vec<String>>,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        while !stopped.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let request = super::tests::read_http_request(&mut stream);
                    let (status, body) = respond(&request);
                    super::tests::write_http_response(&mut stream, &status, &body);
                    requests.push(request);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        }
        requests
    });
    (base, stop, server)
}

#[cfg(unix)]
fn review_fixture_release(recording: &str) -> String {
    serde_json::json!({"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","title":"Album","artist-credit":[{"name":"Lead","artist":{"id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"}}],"media":[{"position":1,"tracks":[{"position":"1","title":"Song","recording":{"id":recording,"title":"Song","length":2000,"artist-credit":[{"name":"Lead"}]}}]}]}).to_string()
}

#[cfg(unix)]
#[tokio::test]
async fn duplicate_encodings_preserve_each_files_own_collaborators() {
    for guests in [["Guest A", "Guest B"], ["Guest B", "Guest A"]] {
        let root = super::tests::temp_root();
        let album = root.join("Lead/Album");
        fs::create_dir_all(&album).unwrap();
        for name in ["a.wav", "b.wav"] {
            fs::copy(super::tests::corpus_wav(), album.join(name)).unwrap();
        }
        let mut request = build_lookup_request(&album).unwrap();
        for (track, guest) in request.tracks.iter_mut().zip(guests) {
            track.artist = Some(format!("Lead feat. {guest}"));
            track.artists = vec!["Lead".into(), guest.into()];
        }
        let recording = "11111111-1111-4111-8111-111111111111";
        let (base, stop, server) = review_fixture_server(move |request| {
            let body = if request.starts_with("POST /lookup ") {
                serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording,"releases":[{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}]}]}]}).to_string()
            } else if request.starts_with("GET /mb/recording/") {
                serde_json::json!({"id":recording,"title":"Song","length":2000,"artist-credit":[{"name":"Lead"}]}).to_string()
            } else {
                review_fixture_release(recording)
            };
            ("200 OK".into(), body)
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::tests::fake_fpcalc(&fpcalc);
        let result = super::super::auto_tag_acoustid::resolve_with(
            &request,
            Some("test-key"),
            &providers,
            &AtomicBool::new(false),
            &client,
            &fpcalc,
        )
        .await;
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        let super::super::auto_tag_acoustid::AcoustidResolution::Candidate { candidate, .. } =
            result
        else {
            panic!("complete duplicate group must resolve");
        };
        let tracks = candidate.accepted_match.unwrap().tracks;
        for (track, guest) in tracks.iter().zip(guests) {
            assert_eq!(
                track.artists,
                ["Lead", guest],
                "a duplicate's credit must not spread or disappear"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn acoustic_cooldown_stops_new_track_requests_and_retains_pending_evidence() {
    let root = super::tests::temp_root();
    let album = root.join("Unknown");
    fs::create_dir_all(&album).unwrap();
    for name in ["a.wav", "b.wav"] {
        fs::copy(super::tests::corpus_wav(), album.join(name)).unwrap();
    }
    let (base, stop, server) = review_fixture_server(|_| {
        (
            "429 Too Many Requests\r\nRetry-After: 300".into(),
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
    super::tests::fake_fpcalc(&fpcalc);
    let result = super::super::auto_tag_acoustid::resolve_with(
        &build_lookup_request(&album).unwrap(),
        Some("test-key"),
        &providers,
        &AtomicBool::new(false),
        &client,
        &fpcalc,
    )
    .await;
    stop.store(true, Ordering::Release);
    let requests = server.join().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "provider cooldown must apply to subsequent tracks too"
    );
    let super::super::auto_tag_acoustid::AcoustidResolution::Recordings {
        evidence,
        retryable,
        retry_after_seconds,
        ..
    } = result
    else {
        panic!("cooldown must remain retryable");
    };
    assert!(retryable);
    assert_eq!(retry_after_seconds, Some(300));
    assert_eq!(evidence.track_matches.len(), 2);
    assert_eq!(
        evidence.track_matches[1].reason.as_deref(),
        Some("provider_cooldown_pending")
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
async fn identified_single_track_case(
    key: Option<String>,
    fingerprint_match: bool,
    fpcalc_available: bool,
) {
    let root = super::tests::temp_root();
    let album = root.join("Lead/Album");
    fs::create_dir_all(&album).unwrap();
    let file = album.join("Song.wav");
    // Two seconds keeps this fixture outside the matcher's short-duration
    // unit heuristic, so the ordinary provider map is genuinely credible.
    let mut wav = fs::read(super::tests::corpus_wav()).unwrap();
    wav.resize(44 + 176400, 0);
    wav[4..8].copy_from_slice(&176436_u32.to_le_bytes());
    wav[40..44].copy_from_slice(&176400_u32.to_le_bytes());
    fs::write(&file, wav).unwrap();
    let queue = WriteQueue::default();
    write_track_queued(&queue, file.clone(), serde_json::from_value(serde_json::json!({"title":"Song","artist":"Lead","album":"Album","musicbrainzAlbumId":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","musicbrainzArtistId":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"})).unwrap()).await.unwrap();
    let art = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../test/fixtures/tauri/library-scan/Artist One/Album A/cover.jpg");
    fs::copy(&art, album.join("cover.jpg")).unwrap();
    fs::copy(&art, album.parent().unwrap().join("artist.jpg")).unwrap();
    let recording = "11111111-1111-4111-8111-111111111111";
    let mut release_requests = 0;
    let (base, stop, server) = review_fixture_server(move |request| {
        let body = if request.starts_with("POST /lookup ") {
            if fingerprint_match {
                serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording,"releases":[{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}]}]}]}).to_string()
            } else {
                r#"{"status":"ok","results":[]}"#.into()
            }
        } else if request.starts_with("GET /mb/recording/") {
            serde_json::json!({"id":recording,"title":"Song","length":2000,"artist-credit":[{"name":"Lead"}]}).to_string()
        } else if request.starts_with("GET /mb/release/") {
            release_requests += 1;
            review_fixture_release(if fingerprint_match && release_requests > 1 {
                "22222222-2222-4222-8222-222222222222"
            } else {
                recording
            })
        } else {
            r#"{"releases":[],"release-count":0,"release-offset":0}"#.into()
        };
        ("200 OK".into(), body)
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
    if fpcalc_available {
        super::tests::fake_fpcalc(&fpcalc);
    }
    let result = resolve_and_apply_album_with_acoustid_override(
        &album,
        &AutoTagConfig {
            acoustid_enabled: Some(true),
            acoustid_api_key: key,
            remote_lookup_enabled: Some(true),
            discogs_enabled: Some(false),
            lyrics_download_enabled: Some(false),
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
    .await
    .unwrap();
    stop.store(true, Ordering::Release);
    let requests = server.join().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("GET /mb/release/")),
        "optional unavailable/empty fingerprinting must not block a known provider ID"
    );
    if fingerprint_match {
        assert!(
            requests
                .iter()
                .filter(|request| request.starts_with("GET /mb/release/"))
                .count()
                >= 2,
            "exercise both acoustic and ordinary provider maps: {:?}",
            result.diagnostics
        );
    }
    assert_eq!(result.written, 1);
    assert_eq!(
        read_track_metadata(&file)
            .unwrap()
            .musicbrainz_track_id
            .as_deref(),
        Some(recording)
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn identified_single_track_uses_provider_ids_when_acoustic_evidence_is_absent() {
    for key in [None, Some("test-key".to_string())] {
        identified_single_track_case(key, false, true).await;
    }
    identified_single_track_case(Some("test-key".into()), false, false).await;
}

#[cfg(unix)]
#[tokio::test]
async fn verified_acoustic_identity_survives_a_conflicting_provider_recording_map() {
    identified_single_track_case(Some("test-key".into()), true, true).await;
}

fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/autotag-collections/manifest.json"
    ))
    .unwrap()
}

#[test]
fn investigation_corpus_covers_all_folders_and_preserves_healthy_subset_risk() {
    let manifest = manifest();
    let albums = manifest["albums"].as_array().unwrap();
    assert_eq!(albums.len(), 66);
    assert_eq!(
        albums
            .iter()
            .map(|album| album["tracks"].as_array().unwrap().len())
            .sum::<usize>(),
        453
    );
    assert_eq!(
        albums
            .iter()
            .filter(|album| album["tracks"].as_array().unwrap().len() == 1
                && album["observedStatus"] != "healthy")
            .count(),
        26
    );
    assert_eq!(
        albums
            .iter()
            .filter(|album| album["observedStatus"] == "healthy")
            .count(),
        6
    );
    assert_eq!(
        albums
            .iter()
            .filter(|album| album["observedStatus"] == "healthy"
                && album["tracks"].as_array().unwrap().len() == 1)
            .count(),
        4
    );
    assert!(!serde_json::to_string(&manifest)
        .unwrap()
        .contains("/Users/"));
}

fn file_hash(path: &Path) -> String {
    // Use the optimized system SHA256 implementation for the 8.4GB gate;
    // debug Rust hashing dominates this test without improving its assertion.
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "file hash failed");
    let output = String::from_utf8(output.stdout).unwrap();
    let hash = output.split_whitespace().next().unwrap();
    assert!(hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    hash.to_string()
}

fn audio_hash(path: &Path) -> String {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-c:a", "copy", "-f", "hash", "-hash", "sha256", "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "audio hash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn audio_hashes(files: &[String]) -> Vec<String> {
    // Bound decoder processes while keeping the large copied-media gate practical.
    std::thread::scope(|scope| {
        let workers: Vec<_> = files
            .chunks(files.len().div_ceil(4).max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|file| audio_hash(Path::new(file)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect()
    })
}

/// Explicitly selected, network-free safety gate. The supplied recording rows
/// echo local fixture text; they test omission and media safety, NOT recognition.
#[tokio::test]
#[ignore = "requires independent copies of all 66 folders in SOUNDROBE_COPIED_FIXTURE_ROOT; ffmpeg; no provider calls"]
async fn copied_66_folder_recording_patch_preserves_audio_unresolved_fields_and_idempotence() {
    let root = PathBuf::from(
        std::env::var("SOUNDROBE_COPIED_FIXTURE_ROOT").expect("copied fixture root required"),
    )
    .canonicalize()
    .unwrap();
    assert!(
        root.to_string_lossy()
            .starts_with("/private/tmp/soundrobe-autotag-copies-"),
        "refuse any original library path"
    );
    let queue = WriteQueue::default();
    let manifest = manifest();
    let mut total = 0;
    let mut verified_fields = 0;
    for album in manifest["albums"].as_array().unwrap() {
        let path = root.join(album["id"].as_str().unwrap());
        let files = collect_audio_files(&path);
        assert_eq!(files.len(), album["tracks"].as_array().unwrap().len());
        let mut before = Vec::new();
        let mut rows = Vec::new();
        let original_audio = audio_hashes(&files);
        for (file, audio) in files.iter().zip(original_audio) {
            let file = Path::new(file);
            assert!(!fs::symlink_metadata(file).unwrap().file_type().is_symlink());
            assert!(file.canonicalize().unwrap().starts_with(&root));
            let track = read_track_metadata(file).unwrap();
            rows.push(TrackCandidate {
                title: track.title.clone(),
                artist: track.artist.clone(),
                artists: track.artists.clone(),
                musicbrainz_track_id: Some(
                    track
                        .musicbrainz_track_id
                        .clone()
                        .unwrap_or_else(|| "00000000-0000-4000-8000-000000000001".into()),
                ),
                ..TrackCandidate::default()
            });
            before.push((track, audio));
        }
        verified_fields += apply_recording_tags(&path, &rows, None, &queue)
            .await
            .unwrap();
        let mut once = Vec::new();
        let after_audio = audio_hashes(&files);
        for ((file, (original, audio)), observed_audio) in
            files.iter().zip(&before).zip(after_audio)
        {
            let after = read_track_metadata(Path::new(file)).unwrap();
            assert_eq!(&observed_audio, audio);
            assert_eq!(after.album, original.album);
            assert_eq!(after.album_artist, original.album_artist);
            assert_eq!(after.album_artists, original.album_artists);
            assert_eq!(after.track_number, original.track_number);
            assert_eq!(after.track_total, original.track_total);
            assert_eq!(after.disc_number, original.disc_number);
            assert_eq!(after.disc_total, original.disc_total);
            assert_eq!(after.year, original.year);
            assert_eq!(after.musicbrainz_album_id, original.musicbrainz_album_id);
            assert_eq!(after.discogs_release_id, original.discogs_release_id);
            assert_eq!(after.title, original.title, "version text must survive");
            assert_eq!(
                after.artist, original.artist,
                "display credits must survive"
            );
            once.push(file_hash(Path::new(file)));
            total += 1;
        }
        apply_recording_tags(&path, &rows, None, &queue)
            .await
            .unwrap();
        for (file, expected) in files.iter().zip(once) {
            assert_eq!(file_hash(Path::new(file)), expected);
        }
    }
    assert_eq!(total, 453);
    println!("copied safety coverage: 66 folders / {total} files, {verified_fields} fixture recording patches; complete album assignments 0; live identity coverage unmeasured");
}

#[cfg(unix)]
#[tokio::test]
async fn partial_fingerprint_identity_writes_only_verified_recording_fields() {
    use super::tests::{
        corpus_wav, fake_fpcalc, read_http_request, temp_root, wav_audio_payload,
        write_http_response,
    };
    use std::{net::TcpListener, thread};
    let root = temp_root();
    let album = root.join("Personal Collection");
    fs::create_dir_all(&album).unwrap();
    let file = album.join("01 Song (Demo).wav");
    fs::copy(corpus_wav(), &file).unwrap();
    let queue = WriteQueue::default();
    let patch = serde_json::from_value(serde_json::json!({"title":"Song (Demo)","artist":"Lead feat. Guest","album":"My Collection","albumArtist":"Collector","trackNumber":7,"year":"1999","musicbrainzAlbumId":"existing-edition"})).unwrap();
    write_track_queued(&queue, file.clone(), patch)
        .await
        .unwrap();
    let audio = wav_audio_payload(&file);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let id = "11111111-1111-4111-8111-111111111111";
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        assert!(read_http_request(&mut stream).starts_with("POST /lookup "));
        write_http_response(
            &mut stream,
            "200 OK",
            &serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":id}]}]})
                .to_string(),
        );
        let (mut stream, _) = listener.accept().unwrap();
        assert!(read_http_request(&mut stream).starts_with("GET /mb/recording/"));
        write_http_response(&mut stream, "200 OK", &serde_json::json!({"id":id,"title":"Verified Song (Demo)","length":10,"artist-credit":[{"name":"Lead","joinphrase":" feat. "},{"name":"Guest"}]}).to_string());
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
    fake_fpcalc(&fpcalc);
    let config = AutoTagConfig {
        acoustid_enabled: Some(true),
        acoustid_api_key: Some("test-key".into()),
        clear_tags_enabled: Some(true),
        clear_tags: Some(vec!["album".into()]),
        ..AutoTagConfig::default()
    };
    let mut checkpoints = 0;
    let result = resolve_and_apply_album_with_acoustid_override(
        &album,
        &config,
        AutoTagServices {
            providers: &providers,
            cache: &cache,
            queue: &queue,
            alias_file: &root.join("aliases"),
        },
        &AtomicBool::new(false),
        None,
        |_| {
            checkpoints += 1;
            Ok(())
        },
        Some((&client, &fpcalc)),
        |_, _| {},
        |_, _, _| {},
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(result.outcome, AutoTagOutcome::NeedsReview);
    assert_eq!(result.written, 1);
    assert_eq!(checkpoints, 1, "review intent must precede partial writes");
    let after = read_track_metadata(&file).unwrap();
    assert_eq!(after.title.as_deref(), Some("Verified Song (Demo)"));
    assert_eq!(after.artist.as_deref(), Some("Lead feat. Guest"));
    assert_eq!(after.album.as_deref(), Some("My Collection"));
    assert_eq!(after.album_artist.as_deref(), Some("Collector"));
    assert_eq!(after.year.as_deref(), Some("1999"));
    assert_eq!(after.track_number, Some(7));
    assert_eq!(
        after.musicbrainz_album_id.as_deref(),
        Some("existing-edition")
    );
    assert_eq!(after.musicbrainz_track_id.as_deref(), Some(id));
    assert_eq!(wav_audio_payload(&file), audio);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn acoustic_long_retry_after_returns_a_durable_deferral_without_spending_total_budget() {
    use super::tests::{
        corpus_wav, fake_fpcalc, read_http_request, temp_root, write_http_response,
    };
    use std::{net::TcpListener, thread, time::Instant};
    let root = temp_root();
    let album = root.join("Unknown");
    fs::create_dir_all(&album).unwrap();
    fs::copy(corpus_wav(), album.join("01.wav")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (response_sent, response_time) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        assert!(read_http_request(&mut stream).starts_with("POST /lookup "));
        write_http_response(
            &mut stream,
            "200 OK",
            r#"{"status":"ok","results":[{"score":0.99,"recordings":[{"id":"11111111-1111-4111-8111-111111111111"}]}]}"#,
        );
        let (mut stream, _) = listener.accept().unwrap();
        assert!(read_http_request(&mut stream).starts_with("GET /mb/recording/"));
        write_http_response(
            &mut stream,
            "503 Service Unavailable\r\nRetry-After: 300",
            "{}",
        );
        response_sent.send(Instant::now()).unwrap();
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    fake_fpcalc(&fpcalc);
    let request = build_lookup_request(&album).unwrap();
    let result = super::super::auto_tag_acoustid::resolve_with(
        &request,
        Some("test-key"),
        &providers,
        &AtomicBool::new(false),
        &client,
        &fpcalc,
    )
    .await;
    server.join().unwrap();
    assert!(
        response_time.recv().unwrap().elapsed() < Duration::from_secs(3),
        "long provider deadline must defer immediately"
    );
    assert!(matches!(
        result,
        super::super::auto_tag_acoustid::AcoustidResolution::Recordings {
            retryable: true,
            retry_after_seconds: Some(300),
            ..
        }
    ));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn coverage_v2_candidate_details_resolve_duration_but_not_unknown_or_true_ties() {
    for mode in [
        "duration",
        "unavailable",
        "genuine_tie",
        "near_threshold",
        "near_threshold_loser",
    ] {
        let root = super::tests::temp_root();
        let album = root.join("Collection");
        fs::create_dir_all(&album).unwrap();
        fs::copy(super::tests::corpus_wav(), album.join("Song (Live).wav")).unwrap();
        let mut request = build_lookup_request(&album).unwrap();
        request.tracks[0].length = Some(125.0);
        request.tracks[0].title = None;
        request.tracks[0].artist = None;
        request.tracks[0].artists.clear();
        request.tracks[0].musicbrainz_track_id = None;
        let a = "11111111-1111-4111-8111-111111111111";
        let b = "22222222-2222-4222-8222-222222222222";
        let (base, stop, server) = review_fixture_server(move |request| {
            if request.starts_with("POST /lookup ") {
                let results = if mode.starts_with("near_threshold") {
                    serde_json::json!([{"id":"acoustic-cluster","score":0.905,"recordings":[{"id":a}]},{"id":"competitor-cluster","score":0.895,"recordings":[{"id":b}]}])
                } else {
                    serde_json::json!([{"id":"acoustic-cluster","score":0.99,"recordings":[{"id":b},{"id":a}]}])
                };
                return (
                    "200 OK".into(),
                    serde_json::json!({"status":"ok","results":results}).to_string(),
                );
            }
            let second = request.contains(b);
            if second && mode == "unavailable" {
                return ("503 Unavailable".into(), "{}".into());
            }
            ("200 OK".into(), serde_json::json!({"id":if second {b} else {a},"title":"Song","disambiguation":"live, from Tour","length":if (second && mode == "duration") || (!second && mode == "near_threshold_loser") {300000} else {125000},"artist-credit":[{"name":"Singer"}]}).to_string())
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::tests::fake_fpcalc(&fpcalc);
        fs::write(
            &fpcalc,
            "#!/bin/sh\nprintf '%s' '{\"duration\":125,\"fingerprint\":\"fixture-fingerprint\"}'\n",
        )
        .unwrap();
        let result = super::super::auto_tag_acoustid::resolve_with(
            &request,
            Some("fixture"),
            &providers,
            &AtomicBool::new(false),
            &client,
            &fpcalc,
        )
        .await;
        stop.store(true, Ordering::Release);
        let requests = server.join().unwrap();
        let super::super::auto_tag_acoustid::AcoustidResolution::Recordings {
            tracks,
            evidence,
            ..
        } = result
        else {
            panic!("recording-only outcome required");
        };
        if mode == "duration" {
            assert_eq!(tracks[0].musicbrainz_track_id.as_deref(), Some(a));
            assert_eq!(tracks[0].title.as_deref(), Some("Song (Live)"));
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.contains(&format!("/recording/{a}")))
                    .count(),
                1,
                "authoritative details reused for patch construction"
            );
        } else {
            assert!(
                tracks[0].musicbrainz_track_id.is_none(),
                "unknown competitor or two genuine identities cannot be guessed"
            );
        }
        assert_eq!(evidence.track_matches[0].candidates.len(), 2);
        assert_eq!(
            evidence.track_matches[0].candidates[0]["resultIds"][0],
            "acoustic-cluster"
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn coverage_v2_editions_resume_eight_uncached_details_per_pass() {
    let root = super::tests::temp_root();
    let album = root.join("Lead/Album");
    fs::create_dir_all(&album).unwrap();
    fs::copy(super::tests::corpus_wav(), album.join("Song.wav")).unwrap();
    let mut request = build_lookup_request(&album).unwrap();
    request.artist_hint = None;
    request.album_hint = Some("Album".into());
    let recording = "11111111-1111-4111-8111-111111111111";
    let release_ids: Vec<_> = (0..20)
        .map(|i| format!("aaaaaaaa-aaaa-4aaa-8aaa-{i:012x}"))
        .collect();
    let (base, stop, server) = review_fixture_server(move |request| {
        let body = if request.starts_with("POST /lookup ") {
            serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":recording,"releases":release_ids.iter().map(|id|serde_json::json!({"id":id})).collect::<Vec<_>>()}]}]}).to_string()
        } else if request.contains("/release/") {
            let id = request
                .split("/release/")
                .nth(1)
                .unwrap()
                .split('?')
                .next()
                .unwrap();
            let mut release: serde_json::Value =
                serde_json::from_str(&review_fixture_release(recording)).unwrap();
            release["id"] = id.into();
            release.to_string()
        } else {
            serde_json::json!({"id":recording,"title":"Song","length":2000,"artist-credit":[{"name":"Lead"}]}).to_string()
        };
        ("200 OK".into(), body)
    });
    let providers = ProviderState::at(
        ProviderState::new().http(),
        &format!("{base}/mb"),
        &format!("{base}/discogs"),
    );
    let client =
        crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
    let fpcalc = root.join("fpcalc");
    super::tests::fake_fpcalc(&fpcalc);
    let cache_dir = root.join("cache");
    let cache = CacheState::new_in(cache_dir.clone());
    assert!(cache.initialize(None));
    for (pass, remaining) in [12, 4, 0].into_iter().enumerate() {
        let reopened = CacheState::new_in(cache_dir.clone());
        assert!(reopened.initialize(None));
        let result = super::super::auto_tag_acoustid::resolve_with_cache(
            &request,
            Some("fixture"),
            &providers,
            &AtomicBool::new(false),
            &client,
            &fpcalc,
            Some(&reopened),
        )
        .await;
        let super::super::auto_tag_acoustid::AcoustidResolution::Recordings {
            evidence,
            reason_code,
            shared_album,
            ..
        } = result
        else {
            panic!("twenty indistinguishable editions cannot select one pressing");
        };
        assert_eq!(evidence.pending_candidates.len(), remaining);
        if pass < 2 {
            assert_eq!(reason_code, "discovery_budget_limited");
            assert!(
                shared_album.is_none(),
                "unexamined editions block consensus"
            );
        } else {
            assert!(shared_album.is_some());
        }
    }
    stop.store(true, Ordering::Release);
    let requests = server.join().unwrap();
    assert_eq!(
        requests.iter().filter(|r| r.contains("/release/")).count(),
        20,
        "cached details survive reopen; no repeated network work"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn coverage_v2_verified_redirect_aliases_resume_but_body_mismatches_never_do() {
    for redirected in [true, false] {
        let root = super::tests::temp_root();
        let album = root.join("Collection");
        fs::create_dir_all(&album).unwrap();
        fs::copy(super::tests::corpus_wav(), album.join("Song.wav")).unwrap();
        let mut request = build_lookup_request(&album).unwrap();
        let a = "11111111-1111-4111-8111-111111111111";
        let b = "22222222-2222-4222-8222-222222222222";
        let canonical = "33333333-3333-4333-8333-333333333333";
        request.tracks[0].musicbrainz_track_id = Some(a.into());
        request.tracks[0].artist = None;
        request.tracks[0].artists.clear();
        let (base, stop, server) = review_fixture_server(move |request| {
            if request.starts_with("POST /lookup ") {
                return ("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":a},{"id":b}]}]}).to_string());
            }
            if redirected && !request.contains(canonical) {
                return (
                    format!("302 Found\r\nLocation: /mb/recording/{canonical}"),
                    "".into(),
                );
            }
            ("200 OK".into(),serde_json::json!({"id":canonical,"title":"Song","length":1000,"artist-credit":[{"name":"Singer"}]}).to_string())
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::tests::fake_fpcalc(&fpcalc);
        let cache_dir = root.join("cache");
        let mut results = Vec::new();
        for _ in 0..3 {
            let cache = CacheState::new_in(cache_dir.clone());
            assert!(cache.initialize(None));
            results.push(
                super::super::auto_tag_acoustid::resolve_with_cache(
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
        request.tracks[0].title = Some("Song (Other Remix)".into());
        let cache = CacheState::new_in(cache_dir.clone());
        assert!(cache.initialize(None));
        let changed = super::super::auto_tag_acoustid::resolve_with_cache(
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
        let requests = server.join().unwrap();
        let super::super::auto_tag_acoustid::AcoustidResolution::Recordings {
            tracks: changed_tracks,
            ..
        } = changed
        else {
            panic!("changed local version requires review");
        };
        assert!(
            changed_tracks[0].musicbrainz_track_id.is_none(),
            "cached provider data must be reevaluated against changed local evidence"
        );
        for result in results {
            let super::super::auto_tag_acoustid::AcoustidResolution::Recordings { tracks, .. } =
                result
            else {
                panic!("recordings expected");
            };
            assert_eq!(
                tracks[0].musicbrainz_track_id.as_deref(),
                redirected.then_some(canonical),
                "only verified redirect aliases may supersede an existing ID"
            );
        }
        if redirected {
            assert_eq!(requests.iter().filter(|r|r.starts_with(&format!("GET /mb/recording/{a}")) ||r.starts_with(&format!("GET /mb/recording/{b}"))).count(),2,"validated public details survive cache reopen and do not consume the next pass's budget");
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn coverage_v2_aliases_cannot_bypass_full_audio_conflicts_across_files() {
    let root = super::tests::temp_root();
    let album = root.join("Collection");
    fs::create_dir_all(&album).unwrap();
    for name in ["1.wav", "2.wav", "3.wav", "4.wav"] {
        fs::copy(super::tests::corpus_wav(), album.join(name)).unwrap();
    }
    let mut request = build_lookup_request(&album).unwrap();
    for track in &mut request.tracks {
        track.length = Some(240.0);
        track.artist = None;
        track.artists.clear();
        track.musicbrainz_track_id = None;
    }
    let a = "11111111-1111-4111-8111-111111111111";
    let b = "22222222-2222-4222-8222-222222222222";
    let canonical = "33333333-3333-4333-8333-333333333333";
    let other = "44444444-4444-4444-8444-444444444444";
    let mut lookup = 0;
    let (base, stop, server) = review_fixture_server(move |request| {
        if request.starts_with("POST /lookup ") {
            let id = [a, b, other, other][lookup];
            lookup += 1;
            return ("200 OK".into(),serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":id}]}]}).to_string());
        }
        if request.starts_with(&format!("GET /mb/recording/{a}"))
            || request.starts_with(&format!("GET /mb/recording/{b}"))
        {
            return (
                format!("302 Found\r\nLocation: /mb/recording/{canonical}"),
                "".into(),
            );
        }
        let id = if request.starts_with(&format!("GET /mb/recording/{canonical}")) {
            canonical
        } else {
            other
        };
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
    super::tests::fake_fpcalc(&fpcalc);
    let a_frames: Vec<_> = (0..300u32).map(|i| i.wrapping_mul(2654435761)).collect();
    let b_frames: Vec<_> = a_frames.iter().map(|v| !v).collect();
    let other_frames: Vec<_> = a_frames.iter().map(|v| v.rotate_left(13)).collect();
    fs::write(&fpcalc,format!("#!/bin/sh\nif [ \"$2\" = '-raw' ]; then\nfor audio do :; done\ncase \"$audio\" in\n*1.wav) printf '%s' '{{\"fingerprint\":{}}}';;\n*2.wav) printf '%s' '{{\"fingerprint\":{}}}';;\n*) printf '%s' '{{\"fingerprint\":{}}}';;\nesac\nelse printf '%s' '{{\"duration\":240,\"fingerprint\":\"same-prefix\"}}'; fi\n",serde_json::to_string(&a_frames).unwrap(),serde_json::to_string(&b_frames).unwrap(),serde_json::to_string(&other_frames).unwrap())).unwrap();
    let result = super::super::auto_tag_acoustid::resolve_with(
        &request,
        Some("fixture"),
        &providers,
        &AtomicBool::new(false),
        &client,
        &fpcalc,
    )
    .await;
    stop.store(true, Ordering::Release);
    server.join().unwrap();
    let super::super::auto_tag_acoustid::AcoustidResolution::Recordings {
        tracks, evidence, ..
    } = result
    else {
        panic!("recordings expected");
    };
    assert!(
        tracks[..2]
            .iter()
            .all(|track| track.musicbrainz_track_id.is_none()),
        "canonical aliases with conflicting full audio cannot receive an identity"
    );
    assert!(
        tracks[2..]
            .iter()
            .all(|track| track.musicbrainz_track_id.as_deref() == Some(other)),
        "an unrelated compatible duplicate group still receives its verified identity"
    );
    assert_eq!(
        evidence.track_matches[0].reason.as_deref(),
        Some("recording_audio_conflict")
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn coverage_v2_consensus_album_writes_count_without_claiming_an_edition() {
    let root = super::tests::temp_root();
    let album = root.join("Collection");
    fs::create_dir_all(&album).unwrap();
    let file = album.join("Song.wav");
    fs::copy(super::tests::corpus_wav(), &file).unwrap();
    let request = build_lookup_request(&album).unwrap();
    let mut track = request.tracks[0].clone();
    track.source = Some(RecordingSource::capture(&file).unwrap());
    track.title = Some("Song".into());
    track.artist = Some("Singer".into());
    track.artists = vec!["Singer".into()];
    let shared = AlbumCandidate {
        album: Some("Consensus album".into()),
        album_artist: Some("Singer".into()),
        album_artists: vec!["Singer".into()],
        source: LookupSource::Musicbrainz,
        ..AlbumCandidate::default()
    };
    let evidence = super::super::auto_tag_acoustid::AcoustidEvidence {
        release_id: String::new(),
        decision_rule_version: 2,
        track_matches: vec![],
        candidate_count: 2,
        checked_candidates: 2,
        rejections: vec![],
        pending_candidates: vec![],
    };
    let resolution = super::super::auto_tag_acoustid::AcoustidResolution::Recordings {
        tracks: vec![track],
        shared_album: Some(Box::new(shared)),
        evidence,
        reason_code: "ambiguous_edition".into(),
        retryable: false,
        retry_after_seconds: None,
    };
    let result = apply_recording_resolution(
        &album,
        &request,
        resolution,
        &WriteQueue::default(),
        &mut |_| Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(
        read_track_metadata(&file).unwrap().album.as_deref(),
        Some("Consensus album")
    );
    let coverage = result
        .diagnostics
        .iter()
        .find(|row| row["stage"] == "coverage")
        .unwrap();
    assert_eq!(
        coverage["providerVerifiedAlbumTracks"], 1,
        "count the verified album values actually written"
    );
    assert_eq!(
        coverage["providerVerifiedEditionTracks"], 0,
        "consensus never invents a pressing identity"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn coverage_v2_live_disambiguation_supports_a_complete_album_but_studio_does_not() {
    for (live, alias) in [(true, false), (true, true), (false, false)] {
        let root = super::tests::temp_root();
        let album = root.join("Lead/Album");
        fs::create_dir_all(&album).unwrap();
        fs::copy(super::tests::corpus_wav(), album.join("Song (Live).wav")).unwrap();
        let mut request = build_lookup_request(&album).unwrap();
        request.artist_hint = Some("Lead".into());
        request.album_hint = Some("Album".into());
        let id = "11111111-1111-4111-8111-111111111111";
        let canonical = if alias {
            "22222222-2222-4222-8222-222222222222"
        } else {
            id
        };
        if alias {
            request.tracks[0].musicbrainz_track_id = Some(id.into());
        }
        let (base, stop, server) = review_fixture_server(move |request| {
            if alias && request.starts_with(&format!("GET /mb/recording/{id}")) {
                return (
                    format!("302 Found\r\nLocation: /mb/recording/{canonical}"),
                    "".into(),
                );
            }
            let body = if request.starts_with("POST /lookup ") {
                serde_json::json!({"status":"ok","results":[{"score":0.99,"recordings":[{"id":id,"releases":[{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}]}]}]}).to_string()
            } else if request.starts_with("GET /mb/recording/") {
                serde_json::json!({"id":canonical,"title":"Song","disambiguation":if live {"live, from Tour"}else{"studio recording"},"length":2000,"artist-credit":[{"name":"Lead"}]}).to_string()
            } else {
                review_fixture_release(canonical)
            };
            ("200 OK".into(), body)
        });
        let providers = ProviderState::at(
            ProviderState::new().http(),
            &format!("{base}/mb"),
            &format!("{base}/discogs"),
        );
        let client =
            crate::infra::acoustid::AcoustidClient::at(providers.http(), format!("{base}/lookup"));
        let fpcalc = root.join("fpcalc");
        super::tests::fake_fpcalc(&fpcalc);
        let result = super::super::auto_tag_acoustid::resolve_with(
            &request,
            Some("fixture"),
            &providers,
            &AtomicBool::new(false),
            &client,
            &fpcalc,
        )
        .await;
        stop.store(true, Ordering::Release);
        server.join().unwrap();
        if live {
            let super::super::auto_tag_acoustid::AcoustidResolution::Candidate {
                candidate, ..
            } = result
            else {
                panic!("verified live context supports the complete release mapping");
            };
            assert_eq!(
                candidate.accepted_match.unwrap().tracks[0].title.as_deref(),
                Some("Song (Live)")
            );
        } else {
            let super::super::auto_tag_acoustid::AcoustidResolution::Recordings { tracks, .. } =
                result
            else {
                panic!("studio conflicts require review");
            };
            assert!(tracks[0].musicbrainz_track_id.is_none());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
