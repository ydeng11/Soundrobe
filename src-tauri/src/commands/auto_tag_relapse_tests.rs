use super::*;
use std::fs;

#[tokio::test]
#[ignore = "requires the original 22-track Relapse album and live providers"]
async fn live_relapse_deluxe_smoke() {
    let source = PathBuf::from(std::env::var("SOUNDROBE_RELAPSE_SOURCE").expect("source required"));
    let actual = build_lookup_request(&source).unwrap();
    let (expected, _) = fixture();
    for (local, expected) in actual.tracks.iter().zip(&expected.tracks) {
        assert_eq!(local.title, expected.title);
        assert_eq!(local.track_number, expected.track_number);
    }
    super::edition_tests::run_live_edition_smoke(source, "Eminem", 22).await;
}

fn fixture() -> (LookupRequest, AlbumCandidate) {
    (
        serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/relapse-deluxe/local.json"
        ))
        .unwrap(),
        serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/relapse-deluxe/candidate-36441795.json"
        ))
        .unwrap(),
    )
}

#[test]
fn relapse_selects_complete_content_not_just_equal_track_counts() {
    let (request, target) = fixture();
    let mut standard = target.clone();
    standard.discogs_release_id = Some("standard-fixture".into());
    standard.tracks.truncate(20);
    let wrong_bonus = discogs_candidate(
        serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/relapse-deluxe/provider-album-16649340.json"
        ))
        .unwrap(),
    );
    assert!(strict_provider_candidate_credibility(&request, &target).is_ok());
    assert!(provider_candidate_credibility(&request, &standard).is_err());
    assert!(provider_candidate_credibility(&request, &wrong_bonus).is_err());
    let selected =
        select_credible_provider_candidate(&request, vec![standard, wrong_bonus, target]).unwrap();
    assert_eq!(selected.discogs_release_id.as_deref(), Some("36441795"));
    assert!(
        selected.accepted_match.is_none(),
        "ordinary matcher must suffice"
    );
    let cached: AlbumCandidate =
        serde_json::from_value(serde_json::to_value(&selected).unwrap()).unwrap();
    let mut conflicting = request.clone();
    conflicting.tracks[0].length = Some(120.0);
    assert!(provider_candidate_credibility(&conflicting, &cached).is_err());
}

#[test]
fn relapse_rejection_diagnostics_explain_track_evidence() {
    let (request, target) = fixture();
    for (case, expected) in [
        ("duration", "duration_conflict"),
        ("missing", "duration_missing"),
        ("ambiguous", "ambiguous_comparison"),
        ("position", "positional_only"),
    ] {
        let mut changed = target.clone();
        match case {
            "duration" => changed.tracks[0].length = Some(120.0),
            "missing" => changed.tracks[0].length = None,
            "ambiguous" => changed.tracks.push(changed.tracks[0].clone()),
            "position" => changed.tracks[0].title = Some("Different recording".into()),
            _ => unreachable!(),
        }
        let reason = provider_candidate_credibility(&request, &changed).unwrap_err();
        assert!(reason.contains(expected), "{case}: {reason}");
        assert!(reason.contains("Dr. West"), "{reason}");
        let diagnostics = provider_selection_diagnostics(&request, &[changed], &[]);
        assert!(diagnostics[0]["rejections"].to_string().contains(expected));
    }
}

#[tokio::test]
async fn relapse_discovery_and_write_preserve_title_identity_with_reordered_bonus_tracks() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let size = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]);
            let path = request.split_whitespace().nth(1).unwrap();
            let body = if path.starts_with("/database/search?") {
                assert!(path.contains("type=master"));
                serde_json::json!({"pagination":{"items":1},"results":[{"id":100118,"type":"master","title":"Eminem - Relapse"}]})
            } else if path.starts_with("/masters/100118/versions?") {
                serde_json::json!({"pagination":{"pages":1},"versions":[{"id":36441795,"title":"Relapse (Deluxe)","country":"US","released":"2009"}]})
            } else {
                assert_eq!(path, "/releases/36441795");
                serde_json::from_str(include_str!("../../../test/fixtures/tauri/relapse-deluxe/release-36441795.json")).unwrap()
            }.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let root = std::env::temp_dir().join(format!("soundrobe-relapse-{}", uuid::Uuid::new_v4()));
    let album = root
        .as_path()
        .join("Eminem/2009 Eminem - Relapse (With Bonus)");
    fs::create_dir_all(&album).unwrap();
    let (mut request, _) = fixture();
    request.path = album.to_string_lossy().into_owned();
    let media = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../test/fixtures/tauri/media-corpus/minimal.flac");
    for track in &request.tracks {
        fs::copy(
            &media,
            album.join(format!("{}.flac", track.filename.as_ref().unwrap())),
        )
        .unwrap();
    }
    // A provider credit must not erase an existing collaborator on a guarded match.
    request.tracks[0].artist = Some("Eminem & Guest".into());
    request.tracks[0].artists = vec!["Eminem".into(), "Guest".into()];
    let cache = CacheState::new(root.as_path().to_path_buf());
    assert!(cache.initialize(Some(root.as_path().join("cache.db").to_str().unwrap())));
    let client = DiscogsClient::at(reqwest::Client::new(), None, &base);
    let cancelled = AtomicBool::new(false);
    for phase in ["cold", "warm"] {
        let mut discovery = editions::DiscogsDiscovery::new(&client, &cache, &cancelled);
        discovery.expand(&request).await.unwrap();
        let selected =
            select_credible_provider_candidate(&request, discovery.candidates()).unwrap();
        assert_eq!(
            selected.discogs_release_id.as_deref(),
            Some("36441795"),
            "{phase}"
        );
        let protected = protect_candidate_tracks(&request, &selected);
        assert_eq!(
            apply_candidate_tags(&album, &protected, &WriteQueue::default())
                .await
                .unwrap(),
            22
        );
        for (index, local) in request.tracks.iter().enumerate() {
            let path = album.join(format!("{}.flac", local.filename.as_ref().unwrap()));
            let read = crate::commands::tracks::read_track_metadata(&path).unwrap();
            assert_eq!(read.title, local.title);
            assert_eq!(
                read.track_number,
                Some(match index {
                    20 => 22,
                    21 => 21,
                    _ => index as u32 + 1,
                })
            );
            // The minimal corpus FLAC uses the bounded reader; inspect separate
            // Vorbis totals and multi-value credits directly on this fixture.
            use lofty::{config::ParseOptions, file::AudioFile, flac::FlacFile};
            let flac = FlacFile::read_from(
                &mut fs::File::open(&path).unwrap(),
                ParseOptions::new().read_properties(false),
            )
            .unwrap();
            let comments = flac.vorbis_comments().unwrap();
            assert!(comments
                .items()
                .any(|(key, value)| key == "TRACKTOTAL" && value == "22"));
            assert_eq!(read.discogs_release_id.as_deref(), Some("36441795"));
            if index == 0 {
                assert!(comments
                    .items()
                    .any(|(key, value)| key == "ARTISTS" && value == "Guest"));
            }
            if index == 17 {
                assert!(comments
                    .items()
                    .any(|(key, value)| key == "ARTISTS" && value == "50 Cent"));
            }
        }
    }
    server.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}
