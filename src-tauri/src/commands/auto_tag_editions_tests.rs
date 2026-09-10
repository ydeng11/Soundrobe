use super::*;

fn album_fixture() -> (LookupRequest, AlbumCandidate) {
    let request: LookupRequest = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/discogs-editions/local.json"
    ))
    .unwrap();
    let release: serde_json::Value = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/discogs-editions/release-16211782.json"
    ))
    .unwrap();
    let mut tracks = request.tracks.clone();
    tracks.sort_by_key(|track| track.track_number);
    for (track, remote) in tracks.iter_mut().zip(
        release["tracklist"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["type_"] == "track"),
    ) {
        track.title = remote["title"].as_str().map(str::to_string);
        track.filename = None;
        let (minutes, seconds) = remote["duration"]
            .as_str()
            .unwrap()
            .split_once(':')
            .unwrap();
        track.length =
            Some(minutes.parse::<f64>().unwrap() * 60.0 + seconds.parse::<f64>().unwrap());
        track.track_total = Some(18);
    }
    let candidate = AlbumCandidate {
        source: LookupSource::Discogs,
        artist: Some("Ariana Grande".into()),
        album_artist: Some("Ariana Grande".into()),
        album: Some("My Everything".into()),
        year: Some("2020".into()),
        discogs_release_id: Some("16211782".into()),
        tracks,
        ..Default::default()
    };
    (request, candidate)
}

#[test]
fn discogs_edition_accepts_complete_reissue_with_one_small_timing_error() {
    let (request, candidate) = album_fixture();
    assert!(provider_candidate_credibility(&request, &candidate).is_ok());
    let selected = select_credible_provider_candidate(&request, vec![candidate]).unwrap();
    let protected = protect_candidate_tracks(&request, &selected);
    for (local, written) in request.tracks.iter().zip(&protected.tracks) {
        assert_eq!(written.track_number, local.track_number);
        assert_eq!(written.track_total, Some(18));
    }
}

#[test]
fn discogs_edition_rejects_weak_or_conflicting_album_evidence() {
    let (request, candidate) = album_fixture();
    for case in [
        "two_durations",
        "large_duration",
        "missing_duration",
        "order",
        "duplicate",
        "missing_track",
        "remix",
        "version",
        "live",
        "remaster",
    ] {
        let mut changed = candidate.clone();
        match case {
            "two_durations" => changed.tracks[0].length = Some(89.0),
            "large_duration" => changed.tracks[16].length = Some(230.0),
            "missing_duration" => changed.tracks[0].length = None,
            "order" => changed.tracks[0].track_number = Some(2),
            "duplicate" => changed.tracks[1].title = changed.tracks[0].title.clone(),
            "missing_track" => {
                changed.tracks.pop();
            }
            "remix" => changed.tracks[17].title = Some("Baby I (Remix)".into()),
            "version" => changed.tracks[17].title = Some("Baby I (Radio Version)".into()),
            "live" => changed.tracks[17].title = Some("Baby I (Live)".into()),
            "remaster" => changed.tracks[17].title = Some("Baby I (2020 Remaster)".into()),
            _ => unreachable!(),
        }
        assert!(
            provider_candidate_credibility(&request, &changed).is_err(),
            "{case}"
        );
    }
}

#[test]
fn discogs_edition_prefers_matching_year() {
    let (request, reissue) = album_fixture();
    let mut original = reissue.clone();
    original.year = Some("2014".into());
    original.discogs_release_id = Some("7138564".into());
    let selected = select_credible_provider_candidate(&request, vec![reissue, original]).unwrap();
    assert_eq!(selected.year.as_deref(), Some("2014"));
}

#[test]
fn discogs_edition_keeps_identity_constraints_and_never_trusts_cached_mapping() {
    let (request, candidate) = album_fixture();
    for field in ["artist", "album", "country", "disc"] {
        let mut changed = candidate.clone();
        let mut request = request.clone();
        match field {
            "artist" => changed.album_artist = Some("Wrong Artist".into()),
            "album" => changed.album = Some("My Everything / Another Album".into()),
            "country" => {
                request.country_hint = Some("US".into());
                changed.country = Some("Japan".into());
            }
            "disc" => request.selected_disc_number = Some(2),
            _ => unreachable!(),
        }
        if field == "disc" {
            for track in &mut changed.tracks {
                track.disc_number = Some(1);
            }
        }
        assert!(
            provider_candidate_credibility(&request, &changed).is_err(),
            "{field}"
        );
    }
    let selected = select_credible_provider_candidate(&request, vec![candidate]).unwrap();
    assert!(selected.accepted_match.is_some());
    let cached: AlbumCandidate =
        serde_json::from_value(serde_json::to_value(selected).unwrap()).unwrap();
    assert!(cached.accepted_match.is_none());
}

#[test]
fn discogs_edition_preserves_collaborators_missing_from_provider() {
    let (request, mut candidate) = album_fixture();
    for track in &mut candidate.tracks {
        track.artist = Some("Ariana Grande".into());
        track.artists = vec!["Ariana Grande".into()];
    }
    let selected = select_credible_provider_candidate(&request, vec![candidate]).unwrap();
    let mapped = protect_candidate_tracks(&request, &selected);
    for (local, mapped) in request.tracks.iter().zip(mapped.tracks) {
        assert_eq!(
            split_collaborative_artists(&local.artist, &local.artists),
            mapped.artists
        );
    }
}

#[tokio::test]
async fn discogs_edition_discovery_expands_versions_and_reuses_cache() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut paths = Vec::new();
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let size = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]);
            let path = request.split_whitespace().nth(1).unwrap().to_string();
            let body = if path.starts_with("/database/search?") {
                assert!(path.contains("type=master"));
                assert!(!path.contains("year="));
                serde_json::json!({"pagination":{"items":1},"results":[{"id":723794,"type":"master","title":"Ariana Grande (2) - My Everything"}]})
            } else if path.contains("/versions?") {
                let second = path.contains("page=2");
                serde_json::json!({"pagination":{"pages":2},"versions":[{"id":if second {16211782} else {1},"title":"My Everything","country":"Japan","released":if second {"2020"} else {"2014"}}]})
            } else {
                assert_eq!(path, "/releases/16211782");
                serde_json::from_str(include_str!(
                    "../../../test/fixtures/tauri/discogs-editions/release-16211782.json"
                ))
                .unwrap()
            };
            let body = body.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            paths.push(path);
        }
        paths
    });
    let root = std::env::temp_dir().join(format!(
        "soundrobe-edition-discovery-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let cache = CacheState::new(root.clone());
    assert!(cache.initialize(Some(root.join("cache.db").to_str().unwrap())));
    let (request, mut unsuitable) = album_fixture();
    unsuitable.discogs_release_id = Some("1".into());
    unsuitable.tracks.pop();
    let client = DiscogsClient::at(reqwest::Client::new(), None, &base);
    let cancelled = AtomicBool::new(false);
    for _ in 0..2 {
        let mut discovery = editions::DiscogsDiscovery::new(&client, &cache, &cancelled);
        discovery.remember(&unsuitable);
        let found = discovery.expand(&request).await.unwrap();
        assert_eq!(
            found.last().unwrap().discogs_release_id.as_deref(),
            Some("16211782")
        );
        assert!(provider_candidate_credibility(&request, found.last().unwrap()).is_ok());
    }
    assert_eq!(server.join().unwrap().len(), 4);
    std::fs::remove_dir_all(root).unwrap();
}

/// Explicit native integration gate: all mutations are confined to fresh copies.
#[tokio::test]
#[ignore = "requires the original 18-track album and live providers"]
async fn live_discogs_edition_smoke() {
    use crate::state::config::{load_from, ProcessEnv};
    use std::fs;
    let source = PathBuf::from(std::env::var("SOUNDROBE_EDITION_SOURCE").expect("source required"));
    let root = PathBuf::from("/private/tmp")
        .join(format!("soundrobe-edition-smoke-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let hash_files = |folder: &Path, audio_only: bool| {
        collect_audio_files(folder)
            .into_iter()
            .map(|path| {
                let bytes = fs::read(&path).unwrap();
                let mut offset = 0;
                if audio_only {
                    assert_eq!(&bytes[..4], b"fLaC");
                    offset = 4;
                    loop {
                        let last = bytes[offset] & 0x80 != 0;
                        let size = ((bytes[offset + 1] as usize) << 16)
                            | ((bytes[offset + 2] as usize) << 8)
                            | bytes[offset + 3] as usize;
                        offset += 4 + size;
                        if last {
                            break;
                        }
                    }
                }
                (
                    Path::new(&path)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    format!("{:x}", Sha256::digest(&bytes[offset..])),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let original_hashes = hash_files(&source, false);
    let audio_hashes = hash_files(&source, true);
    assert_eq!(original_hashes.len(), 18);
    fs::write(
        root.join("source-hashes.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "files": original_hashes, "audioPayloads": audio_hashes,
        }))
        .unwrap(),
    )
    .unwrap();
    let config_text =
        fs::read_to_string(dirs::home_dir().unwrap().join(".soundrobe/config.yaml")).unwrap();
    let mut config = load_from(&config_text, &ProcessEnv);
    // This gate must demonstrate provider authority, without AI assistance.
    config.llm_api_key = None;
    config.remote_lookup_enabled = Some(true);
    config.discogs_enabled = Some(true);
    config.lyrics_download_enabled = Some(false);
    let cache = CacheState::new(root.clone());
    assert!(cache.initialize(Some(root.join("cache.db").to_str().unwrap())));
    let baseline = std::env::var_os("SOUNDROBE_EDITION_BASELINE").is_some();
    let mut selected = None;
    for phase in if baseline {
        vec!["baseline"]
    } else {
        vec!["cold", "warm"]
    } {
        let album = root
            .join(phase)
            .join("Ariana Grande")
            .join(source.file_name().unwrap());
        fs::create_dir_all(&album).unwrap();
        for path in collect_audio_files(&source) {
            fs::copy(&path, album.join(Path::new(&path).file_name().unwrap())).unwrap();
        }
        assert_eq!(hash_files(&album, false), original_hashes);
        let before = build_lookup_request(&album).unwrap();
        assert!(before.discogs_release_id.is_none());
        let cancelled = Arc::new(AtomicBool::new(false));
        let result = resolve_and_apply_album_with_retry_context(
            &album,
            &config,
            AutoTagServices {
                providers: &ProviderState::new(),
                cache: &cache,
                queue: &WriteQueue::default(),
                alias_file: &root.join("aliases.json"),
            },
            &cancelled,
            Some(AutoTagRetryContexts::new(Arc::clone(&cancelled))),
            |_, message| println!("{phase}: {message}"),
            |kind, message, data| println!("{kind}: {message} {}", data.unwrap_or_default()),
        )
        .await
        .unwrap();
        fs::write(
            root.join(format!("{phase}.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        println!("SMOKE_RESULT {}", serde_json::to_string(&result).unwrap());
        assert_eq!(
            hash_files(&source, false),
            original_hashes,
            "originals changed"
        );
        assert_eq!(
            hash_files(&album, true),
            audio_hashes,
            "audio payload changed"
        );
        if baseline {
            assert_eq!(result.outcome, AutoTagOutcome::NeedsReview);
        } else {
            assert_eq!(result.outcome, AutoTagOutcome::Applied);
            assert_eq!(result.written, 18);
            let candidate = result.candidate.unwrap();
            assert_ne!(candidate.source, LookupSource::Llm);
            let after = build_lookup_request(&album).unwrap();
            assert!(after.discogs_release_id.is_some() || after.musicbrainz_album_id.is_some());
            let identity = (
                candidate.discogs_release_id.clone(),
                candidate.musicbrainz_album_id.clone(),
                candidate.year.clone(),
            );
            if let Some(expected) = &selected {
                assert_eq!(&identity, expected);
            }
            selected = Some(identity);
            for (old, new) in before.tracks.iter().zip(&after.tracks) {
                assert_eq!(old.track_number, new.track_number);
                assert_eq!(new.track_total, Some(18));
                assert!(new.title.as_ref().is_some_and(|title| !title.is_empty()));
                let old_artists = split_collaborative_artists(&old.artist, &old.artists);
                let new_artists = split_collaborative_artists(&new.artist, &new.artists);
                assert!(
                    new_artists.len() >= old_artists.len(),
                    "lost collaborators: {:?}",
                    old.title
                );
            }
            for file in collect_audio_files(&album) {
                let read = crate::commands::tracks::read_track_metadata(Path::new(&file)).unwrap();
                assert_eq!(read.year, candidate.year);
            }
        }
    }
    println!("Smoke artifacts: {}", root.display());
}
