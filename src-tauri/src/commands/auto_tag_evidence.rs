//! Offline evaluator observations only; never consumed by the resolver.
use super::*;

#[derive(serde::Serialize)]
pub(super) struct OriginalPrimaryFields {
    path: String,
    sha256: String,
    fields: Option<Vec<(String, String)>>,
}

pub(super) fn capture_primary_fields(original: &[TrackData]) -> Vec<OriginalPrimaryFields> {
    original
        .iter()
        .map(|track| {
            let path = Path::new(&track.path);
            let before = hash_file(path);
            let fields = crate::commands::mutations::diagnostic_flac_primary_comments(path);
            let unchanged = hash_file(path) == before;
            OriginalPrimaryFields {
                path: track.path.clone(),
                sha256: before,
                fields: unchanged.then_some(fields).flatten(),
            }
        })
        .collect()
}

fn candidate_identity_observation(
    request: &LookupRequest,
    original: &[TrackData],
    primary_fields: &[OriginalPrimaryFields],
    candidate: &AlbumCandidate,
    explicit_rejection: Option<bool>,
) -> Value {
    // Deliberately NOT exact_artist_identity: no punctuation, suffix,
    // transliteration, Chinese simplification or Unicode case folding.
    let name = |value: &str| {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    let result = |outcome: &str, correspondence: &str, reason: &str| {
        json!({
            "outcome":outcome, "correspondence":correspondence, "reason":reason,
            "scope":"candidate_single_primary_name_only_not_field_eligibility",
            "explicitDecisionVisibility":if explicit_rejection.is_some() {"provided_for_this_probe"} else {"unavailable"},
            "originalPrimaryClaims":original.iter().map(|track| json!({
                "physicalPath":track.path,"albumArtist":track.album_artist,"albumArtists":track.album_artists,
                "unscopedDiscogsArtistId":track.discogs_artist_id,"separateMusicbrainzArtistId":track.musicbrainz_artist_id
            })).collect::<Vec<_>>(),
            "secondaryFolderArtist":request.folder_artist_hint,
            "originalPrimaryFields":primary_fields,
            "requestArtistHintNotPrimaryProof":request.artist_hint,
            "origin":candidate.discogs_release_receipt.as_ref().map(|receipt| json!({
                "requestPath":receipt.request_path,"resourceId":receipt.resource_id,
                "responseSha256":receipt.response_sha256,"selector":"/artists/0"
            }))
        })
    };
    if explicit_rejection == Some(true) {
        return result("conflict", "insufficient", "explicit_rejection");
    }
    let Some(receipt) = &candidate.discogs_release_receipt else {
        return result(
            "insufficient",
            "insufficient",
            "originating_receipt_unavailable",
        );
    };
    let route = format!("/releases/{}", receipt.resource_id);
    let urls_bound = [&receipt.request_url, &receipt.response_url]
        .iter()
        .all(|url| {
            reqwest::Url::parse(url).is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.path().ends_with(&route)
            })
        })
        && receipt.request_url == receipt.response_url;
    let raw: Option<Value> = serde_json::from_str(&receipt.response_body).ok();
    if candidate.source != LookupSource::Discogs
        || receipt.resource_kind != "release"
        || receipt.request_method != "GET"
        || receipt.request_path != route
        || !urls_bound
        || receipt.response_status != 200
        || receipt
            .resource_id
            .parse::<u64>()
            .ok()
            .is_none_or(|id| id == 0)
        || format!("{:x}", Sha256::digest(receipt.response_body.as_bytes()))
            != receipt.response_sha256
        || raw
            .as_ref()
            .and_then(|body| body["id"].as_u64())
            .map(|id| id.to_string())
            .as_deref()
            != Some(receipt.resource_id.as_str())
        || !(candidate.discogs_release_id.as_deref() == Some(receipt.resource_id.as_str())
            || (candidate.verification.as_deref() == Some("album_content")
                && candidate.discogs_release_id.is_none()))
    {
        return result(
            "conflict",
            "insufficient",
            "receipt_or_candidate_binding_mismatch",
        );
    }
    let raw = raw.expect("validated JSON");
    let Some(primary) = raw["artists"]
        .as_array()
        .filter(|artists| artists.len() == 1)
    else {
        return result(
            "insufficient",
            "insufficient",
            "joint_or_missing_primary_scope",
        );
    };
    let credit = &primary[0];
    if ["join", "role", "tracks"].iter().any(|key| {
        credit[*key]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        return result("insufficient", "insufficient", "ambiguous_primary_role");
    }
    let Some(pair) = candidate
        .discogs_artist_name_variants
        .first()
        .filter(|_| candidate.discogs_artist_name_variants.len() == 1)
    else {
        return result("insufficient", "insufficient", "primary_pair_unavailable");
    };
    if pair.artist_index != 0
        || pair.release_id != receipt.resource_id
        || credit["id"]
            .as_u64()
            .filter(|id| *id > 0)
            .map(|id| id.to_string())
            .as_deref()
            != Some(pair.artist_id.as_str())
        || credit["name"].as_str() != Some(pair.canonical_name.as_str())
        || credit["anv"].as_str() != Some(pair.credited_name.as_str())
        || pair.canonical_name.trim().is_empty()
        || pair.credited_name.trim().is_empty()
    {
        return result("conflict", "insufficient", "primary_pair_binding_mismatch");
    }
    let members = [name(&pair.canonical_name), name(&pair.credited_name)];
    if candidate
        .artist
        .as_deref()
        .is_none_or(|artist| !members.contains(&name(artist)))
        || candidate.artists.len() > 1
        || candidate
            .artists
            .iter()
            .any(|artist| !members.contains(&name(artist)))
        || candidate
            .album_artist
            .as_deref()
            .is_some_and(|artist| !members.contains(&name(artist)))
        || candidate.album_artists.len() > 1
        || candidate
            .album_artists
            .iter()
            .any(|artist| !members.contains(&name(artist)))
        || candidate
            .discogs_artist_id
            .as_deref()
            .is_some_and(|id| id != pair.artist_id)
    {
        return result(
            "insufficient",
            "insufficient",
            "candidate_primary_identity_modified_or_ambiguous",
        );
    }
    let mut paths = BTreeSet::new();
    if original.is_empty()
        || original.len() != request.tracks.len()
        || original.iter().zip(&request.tracks).any(|(local, lookup)| {
            let path = Path::new(&local.path);
            !paths.insert(local.path.as_str())
                || path.parent() != Some(Path::new(&request.path))
                || path.file_stem().and_then(|v| v.to_str()) != lookup.filename.as_deref()
        })
    {
        return result(
            "insufficient",
            "insufficient",
            "original_physical_scope_unavailable",
        );
    }
    if original.iter().any(|local| {
        local
            .discogs_artist_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
    }) {
        // TrackData does not distinguish track-artist from album-primary IDs.
        // Neither a matching nor differing unscoped ID can be promoted.
        return result(
            "insufficient",
            "insufficient",
            "unscoped_local_discogs_artist_id",
        );
    }
    let local_names: Option<Vec<_>> = original
        .iter()
        .map(|local| {
            let primary = local
                .album_artist
                .as_deref()
                .filter(|v| !v.trim().is_empty())?;
            (local.album_artists.len() <= 1
                && local.album_artists.iter().all(|v| name(v) == name(primary)))
            .then(|| name(primary))
        })
        .collect();
    let Some(local_names) = local_names else {
        return result(
            "insufficient",
            "insufficient",
            "original_primary_scope_unavailable",
        );
    };
    if primary_fields.len() != original.len()
        || primary_fields.iter().zip(original).any(|(raw, local)| {
            let Some(fields) = &raw.fields else {
                return true;
            };
            let is_plural = |key: &str| {
                key.eq_ignore_ascii_case("ALBUMARTISTS")
                    || key.eq_ignore_ascii_case("ALBUM ARTISTS")
            };
            let singular: Vec<_> = fields.iter().filter(|(key, _)| !is_plural(key)).collect();
            let plural: Vec<_> = fields.iter().filter(|(key, _)| is_plural(key)).collect();
            raw.path != local.path
                || singular.len() != 1
                || plural.len() > 1
                || fields.iter().any(|(_, value)| {
                    local
                        .album_artist
                        .as_deref()
                        .is_none_or(|primary| name(primary) != name(value))
                })
        })
    {
        return result(
            "insufficient",
            "insufficient",
            "original_primary_completeness_unavailable_or_multiple",
        );
    }
    if local_names
        .iter()
        .any(|local| local != &local_names[0] || !members.contains(local))
    {
        return result(
            "insufficient",
            "insufficient",
            "original_primary_claims_unmatched_or_inconsistent",
        );
    }
    if explicit_rejection.is_none() {
        return result(
            "insufficient",
            "supported",
            "explicit_decisions_unavailable",
        );
    }
    result(
        "supported",
        "supported",
        "bound_single_primary_name_correspondence_only",
    )
}

pub(super) fn identity_observations(
    request: &LookupRequest,
    original: &[TrackData],
    primary_fields: &[OriginalPrimaryFields],
    traces: &[Value],
) -> Value {
    let mut observations = Vec::new();
    let mut supported_entities = BTreeSet::new();
    for (trace_index, trace) in traces.iter().enumerate() {
        for (candidate_index, row) in trace["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if row["candidate"]["source"] != "discogs" {
                continue;
            }
            let identity = match serde_json::from_value::<AlbumCandidate>(row["candidate"].clone())
            {
                Ok(candidate) => {
                    let observation = candidate_identity_observation(
                        request,
                        original,
                        primary_fields,
                        &candidate,
                        None,
                    );
                    if observation["correspondence"] == "supported" {
                        supported_entities
                            .insert(candidate.discogs_artist_name_variants[0].artist_id.clone());
                    }
                    observation
                }
                Err(_) => {
                    json!({"outcome":"insufficient", "correspondence":"insufficient", "reason":"candidate_decode_failed"})
                }
            };
            observations.push(json!({
                "traceIndex":trace_index,"candidateIndex":candidate_index,"field":trace["field"],
                "identity":identity,"nativeStrictError":row["strictError"],
                "nativeMappedVersions":row["versions"],
                "scope":"reported native route only; artist rejection does not execute or pass later predicates; no identity bypass"
            }));
        }
    }
    if supported_entities.len() > 1 {
        for row in &mut observations {
            if row["identity"]["correspondence"] == "supported" {
                row["identity"]["outcome"] = json!("insufficient");
                row["identity"]["correspondence"] = json!("insufficient");
                row["identity"]["reason"] = json!("competing_primary_entities");
                row["identity"]["competingDiscogsArtistIds"] = json!(supported_entities);
            }
        }
    }
    json!(observations)
}

#[tokio::test]
async fn runtime_identity_diagnostic_uses_original_primary_scope_not_guest_or_hints() {
    // Authentic response; synthetic two-file local setup, never catalog truth.
    let raw = include_str!("../../../test/fixtures/tauri/auto-tag-eval/provider-snapshots/reviewed-22-completion-20261108/discogs-036e0c8fc2fc0165.json");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0; 4096];
        let count = stream.read(&mut bytes).unwrap();
        assert!(String::from_utf8_lossy(&bytes[..count]).starts_with("GET /releases/37231524 "));
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{raw}", raw.len()).unwrap();
    });
    let album = DiscogsClient::at(ProviderState::new().http(), None, &base)
        .release_metadata_result("37231524")
        .await
        .unwrap();
    server.join().unwrap();
    let candidate = discogs_candidate(album);
    let root = super::super::tests::temp_root();
    let folder = root.join("卢广仲/勵志論");
    fs::create_dir_all(&folder).unwrap();
    let queue = WriteQueue::default();
    for filename in ["01.flac", "02.flac"] {
        let file = folder.join(filename);
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.flac"),
            &file,
        )
        .unwrap();
        write_track_queued(
            &queue,
            file,
            TrackPatch {
                artist: Patch::Value("Synthetic first-track guest".into()),
                album_artist: Patch::Value("盧廣仲".into()),
                album: Patch::Value("勵志論".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let request = build_lookup_request(&folder).unwrap();
    let original = crate::commands::tracks::read_album(&folder).unwrap().tracks;
    let primary_fields = capture_primary_fields(&original);
    let candidate_identity_observation =
        |request: &LookupRequest, original: &[TrackData], candidate: &AlbumCandidate, rejection| {
            self::candidate_identity_observation(
                request,
                original,
                &primary_fields,
                candidate,
                rejection,
            )
        };
    assert_eq!(
        request.tagged_artist_hint.as_deref(),
        Some("Synthetic first-track guest")
    );
    let known = candidate_identity_observation(&request, &original, &candidate, Some(false));
    assert_eq!(known["outcome"], "supported", "an actual original albumArtist claim, not the collapsed guest hint, matches the fetched ANV");
    for field in ["album_artist", "album_artists"] {
        let mut changed = serde_json::to_value(&candidate).unwrap();
        changed[field] = if field == "album_artist" {
            json!("Other primary")
        } else {
            json!(["Crowd Lu", "Other primary"])
        };
        let changed = serde_json::from_value(changed).unwrap();
        assert_eq!(
            candidate_identity_observation(&request, &original, &changed, Some(false))["outcome"],
            "insufficient",
            "candidate album-primary presentation cannot escape ownership checks: {field}"
        );
    }
    let unknown = candidate_identity_observation(&request, &original, &candidate, None);
    assert_eq!(unknown["outcome"], "insufficient");
    assert_eq!(unknown["correspondence"], "supported");
    assert_eq!(unknown["reason"], "explicit_decisions_unavailable");
    assert_eq!(
        candidate_identity_observation(&request, &original, &candidate, Some(true))["outcome"],
        "conflict"
    );
    let mut stripped = original.clone();
    for local in &mut stripped {
        local.album_artist = None;
        local.album_artists.clear();
    }
    assert_eq!(
        candidate_identity_observation(&request, &stripped, &candidate, Some(false))["reason"],
        "original_primary_scope_unavailable"
    );
    let mut unscoped = original.clone();
    unscoped[0].discogs_artist_id = Some("2461941".into());
    assert_eq!(
        candidate_identity_observation(&request, &unscoped, &candidate, Some(false))["reason"],
        "unscoped_local_discogs_artist_id"
    );
    for local_name in ["卢广仲", "Crowd-Lu", "Crowd Lu (2)", "CROWD LÜ"] {
        let mut changed = original.clone();
        for local in &mut changed {
            local.album_artist = Some(local_name.into());
            local.album_artists = vec![local_name.into()];
        }
        assert_eq!(
            candidate_identity_observation(&request, &changed, &candidate, Some(false))["outcome"],
            "insufficient",
            "no transliteration, punctuation stripping or entity-suffix folding: {local_name}"
        );
    }
    let mut legacy = candidate.clone();
    legacy.discogs_release_receipt = None;
    assert_eq!(
        candidate_identity_observation(&request, &original, &legacy, Some(false))["reason"],
        "originating_receipt_unavailable"
    );
    for field in [
        "responseSha256",
        "responseBody",
        "resourceId",
        "requestPath",
        "resourceKind",
        "responseUrl",
    ] {
        let mut changed = serde_json::to_value(&candidate).unwrap();
        changed["discogsReleaseReceipt"][field] = json!("synthetic mismatch");
        let changed = serde_json::from_value(changed).unwrap();
        assert_eq!(
            candidate_identity_observation(&request, &original, &changed, Some(false))["outcome"],
            "conflict",
            "{field}"
        );
    }
    for field in ["releaseId", "artistId", "canonicalName", "creditedName"] {
        let mut changed = serde_json::to_value(&candidate).unwrap();
        changed["discogsArtistNameVariants"][0][field] = json!("synthetic other entity");
        let changed = serde_json::from_value(changed).unwrap();
        assert_eq!(
            candidate_identity_observation(&request, &original, &changed, Some(false))["outcome"],
            "conflict",
            "{field}"
        );
    }
    assert!(
        provider_candidate_credibility(&request, &candidate).is_err(),
        "diagnostic support cannot authorize a guest-hint mismatch"
    );
    let mut bad_index = candidate.clone();
    bad_index.discogs_artist_name_variants[0].artist_index = 1;
    assert_eq!(
        candidate_identity_observation(&request, &original, &bad_index, Some(false))["outcome"],
        "conflict"
    );
    for name in ["  cRoWd   Lu  ", "盧廣仲"] {
        let mut locals = original.clone();
        for local in &mut locals {
            local.album_artist = Some(name.into());
            local.album_artists = vec![name.into()];
        }
        let fields: Vec<_> = locals
            .iter()
            .map(|local| OriginalPrimaryFields {
                path: local.path.clone(),
                sha256: "synthetic-complete-field-snapshot".into(),
                fields: Some(vec![("ALBUMARTIST".into(), name.into())]),
            })
            .collect();
        assert_eq!(
            self::candidate_identity_observation(
                &request,
                &locals,
                &fields,
                &candidate,
                Some(false)
            )["outcome"],
            "supported"
        );
    }
    let mut different_request = request.clone();
    different_request.path = "/another/profile".into();
    assert_eq!(
        candidate_identity_observation(&different_request, &original, &candidate, Some(false))
            ["reason"],
        "original_physical_scope_unavailable"
    );
    let mut resolved = request.clone();
    resolved.discogs_artist_id = Some("999".into());
    resolved.musicbrainz_artist_id = Some("different-provider-not-equated".into());
    assert_eq!(
        candidate_identity_observation(&resolved, &original, &candidate, Some(false))["outcome"],
        "supported",
        "collapsed or resolver-filled request IDs do not become original primary claims"
    );
    let mut local_joint = original.clone();
    local_joint[1]
        .album_artists
        .push("Unmatched collaborator".into());
    assert_eq!(
        candidate_identity_observation(&request, &local_joint, &candidate, Some(false))["outcome"],
        "insufficient"
    );
    // Synthetic altered snapshots demonstrate unsupported scopes, not real
    // catalog claims. Rehashing cannot turn joint/guest scope into support.
    for kind in ["joint", "guest", "blank", "role"] {
        let mut changed = candidate.clone();
        let receipt = changed.discogs_release_receipt.as_mut().unwrap();
        let mut body: Value = serde_json::from_str(&receipt.response_body).unwrap();
        match kind {
            "joint" => {
                let other = json!({"id":999,"name":"Other primary","anv":""});
                body["artists"].as_array_mut().unwrap().push(other);
            }
            "guest" => {
                body["extraartists"] = body["artists"].take();
                body["artists"] = json!([]);
            }
            "blank" => {
                body["artists"][0]["anv"] = json!("");
                changed.discogs_artist_name_variants.clear();
            }
            "role" => {
                body["artists"][0]["role"] = json!("Engineer");
            }
            _ => unreachable!(),
        }
        receipt.response_body = body.to_string();
        receipt.response_sha256 = format!("{:x}", Sha256::digest(receipt.response_body.as_bytes()));
        assert_eq!(
            candidate_identity_observation(&request, &original, &changed, Some(false))["outcome"],
            "insufficient",
            "{kind}"
        );
    }
    let cache_path = root.join("diagnostic-cache.db");
    let cache = CacheState::new(root.clone());
    assert!(cache.initialize(Some(cache_path.to_str().unwrap())));
    cache
        .set_lookup(
            "valid",
            &json!({}),
            &serde_json::to_value(&candidate).unwrap(),
            "discogs",
        )
        .unwrap();
    let mut tampered = serde_json::to_value(&candidate).unwrap();
    tampered["discogsReleaseReceipt"]["responseBody"] = json!("{}");
    cache
        .set_lookup("tampered", &json!({}), &tampered, "discogs")
        .unwrap();
    drop(cache);
    let cache = CacheState::new(root.clone());
    assert!(cache.initialize(Some(cache_path.to_str().unwrap())));
    for (key, expected) in [("valid", "supported"), ("tampered", "conflict")] {
        let restored = serde_json::from_value(cache.lookup(key).unwrap()).unwrap();
        assert_eq!(
            candidate_identity_observation(&request, &original, &restored, Some(false))["outcome"],
            expected
        );
    }
    let traced = vec![
        json!({"field":"genre", "candidates":[{"candidate":candidate,"strictError":"artist conflict","versions":[]}]}),
    ];
    let exported = identity_observations(&request, &original, &primary_fields, &traced);
    assert_eq!(exported[0]["identity"]["outcome"], "insufficient");
    assert_eq!(exported[0]["identity"]["correspondence"], "supported");
    assert_eq!(exported[0]["nativeStrictError"], "artist conflict");
    assert_eq!(exported[0]["nativeMappedVersions"], json!([]));
    // Synthetic competing valid envelope, not a captured catalog assertion.
    let mut competing = candidate.clone();
    let receipt = competing.discogs_release_receipt.as_mut().unwrap();
    let mut body: Value = serde_json::from_str(&receipt.response_body).unwrap();
    body["id"] = json!(42);
    body["artists"][0]["id"] = json!(999);
    receipt.resource_id = "42".into();
    receipt.request_path = "/releases/42".into();
    receipt.request_url = receipt.request_url.replace("37231524", "42");
    receipt.response_url = receipt.request_url.clone();
    receipt.response_body = body.to_string();
    receipt.response_sha256 = format!("{:x}", Sha256::digest(receipt.response_body.as_bytes()));
    competing.discogs_release_id = Some("42".into());
    competing.discogs_artist_id = Some("999".into());
    competing.discogs_artist_name_variants[0].artist_id = "999".into();
    competing.discogs_artist_name_variants[0].release_id = "42".into();
    let ambiguous = vec![
        json!({"field":"genre","candidates":[{"candidate":candidate},{"candidate":competing}]}),
    ];
    let observed = identity_observations(&request, &original, &primary_fields, &ambiguous);
    for row in observed.as_array().unwrap() {
        assert_eq!(
            row["identity"]["correspondence"], "insufficient",
            "a shared ANV cannot select between two available primary entities"
        );
        assert_eq!(row["identity"]["reason"], "competing_primary_entities");
    }
    // Synthetic conflicting spellings recognized by the actual FLAC reader.
    use lofty::tag::TagExt;
    let mut comments = lofty::ogg::VorbisComments::new();
    comments.push("ALBUMARTIST".into(), "盧廣仲".into());
    comments.push("ALBUMARTISTS".into(), "盧廣仲".into());
    comments.push("ALBUM ARTISTS".into(), "Another primary".into());
    comments
        .save_to_path(&original[0].path, lofty::config::WriteOptions::new())
        .unwrap();
    let spaced = crate::commands::tracks::read_album(&folder).unwrap().tracks;
    assert_eq!(spaced[0].album_artist.as_deref(), Some("盧廣仲"));
    assert_eq!(spaced[0].album_artists, vec!["盧廣仲"]);
    assert_eq!(
        self::candidate_identity_observation(
            &request,
            &spaced,
            &capture_primary_fields(&spaced),
            &candidate,
            Some(false)
        )["outcome"],
        "insufficient",
        "competing spaced plural alias cannot disappear behind the preferred DTO spelling"
    );
    let mut comments = lofty::ogg::VorbisComments::new();
    comments.push("ALBUMARTIST".into(), "盧廣仲".into());
    comments
        .save_to_path(&original[0].path, lofty::config::WriteOptions::new())
        .unwrap();
    let plain = fs::read(&original[0].path).unwrap();
    let mut frames = Vec::new();
    // Lofty exposes the last duplicate TPE2 in this construction.
    for primary in ["Another primary", "盧廣仲"] {
        frames.extend_from_slice(b"TPE2");
        frames.extend_from_slice(&[0, 0, 0, (primary.len() + 1) as u8, 0, 0, 3]);
        frames.extend_from_slice(primary.as_bytes());
    }
    assert!(frames.len() < 128);
    let mut prefixed = b"ID3\x04\0\0\0\0\0".to_vec();
    prefixed.push(frames.len() as u8);
    prefixed.extend(frames);
    prefixed.extend_from_slice(&plain);
    fs::write(&original[0].path, prefixed).unwrap();
    let id3 = crate::commands::tracks::read_album(&folder).unwrap().tracks;
    assert_eq!(id3[0].album_artist.as_deref(), Some("盧廣仲"));
    assert_eq!(
        self::candidate_identity_observation(
            &request,
            &id3,
            &capture_primary_fields(&id3),
            &candidate,
            Some(false)
        )["outcome"],
        "insufficient",
        "Vorbis-only completeness cannot certify an ID3-prefixed FLAC"
    );
    fs::write(&original[0].path, plain).unwrap();
    // A real reader-to-diagnostic control for repeated standard primary tags.
    // The existing DTO retains only the first; do not mistake it for completeness.
    let mut comments = lofty::ogg::VorbisComments::new();
    comments.push("ALBUMARTIST".into(), "盧廣仲".into());
    comments.push("ALBUMARTIST".into(), "Another primary".into());
    comments
        .save_to_path(&original[0].path, lofty::config::WriteOptions::new())
        .unwrap();
    let repeated = crate::commands::tracks::read_album(&folder).unwrap().tracks;
    assert_eq!(repeated[0].album_artist.as_deref(), Some("盧廣仲"));
    assert_eq!(
        self::candidate_identity_observation(
            &request,
            &repeated,
            &capture_primary_fields(&repeated),
            &candidate,
            Some(false)
        )["outcome"],
        "insufficient",
        "lossy DTO cannot certify original single-primary completeness"
    );
    drop(cache);
    fs::remove_dir_all(root).unwrap();
}

pub(super) fn guard_observations(request: &LookupRequest, inventory: &Value) -> Value {
    let mut sources = Vec::new();
    for receipt in inventory["records"].as_array().into_iter().flatten() {
        let query = receipt["query"]
            .as_str()
            .and_then(|query| serde_json::from_str::<Value>(query).ok());
        let received = inventory["requests"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|attempt| {
                attempt["provider"] == receipt["provider"]
                    && attempt["path"] == receipt["path"]
                    && Some(&attempt["query"]) == query.as_ref()
                    && attempt["status"] == 200
            });
        if !received || receipt["status"] != 200 {
            continue;
        }
        let body = &receipt["body"];
        let mut occurrences = Vec::new();
        if receipt["provider"] == "musicbrainz" {
            for (medium, value) in body["media"].as_array().into_iter().flatten().enumerate() {
                for (index, track) in value["tracks"].as_array().into_iter().flatten().enumerate() {
                    occurrences.push((format!("/media/{medium}/tracks/{index}"), track));
                }
            }
        } else if receipt["provider"] == "discogs" {
            for (index, track) in body["tracklist"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                occurrences.push((format!("/tracklist/{index}"), track));
            }
        }
        if occurrences.is_empty() {
            continue;
        }
        let mut pairs = Vec::new();
        for (local_index, local) in request.tracks.iter().enumerate() {
            for (selector, raw) in &occurrences {
                let recording_title = raw["recording"]["title"].as_str();
                let remote = TrackCandidate {
                    title: raw["title"]
                        .as_str()
                        .or(recording_title)
                        .map(str::to_string),
                    match_titles: recording_title
                        .filter(|title| Some(*title) != raw["title"].as_str())
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                    release_disambiguation: body["disambiguation"].as_str().map(str::to_string),
                    ..Default::default()
                };
                let filename = local.filename.as_deref().unwrap_or_default();
                let disambiguation = raw["recording"]["disambiguation"].as_str();
                let local_title = local
                    .title
                    .as_deref()
                    .or(local.filename.as_deref())
                    .unwrap_or_default();
                let calls = json!([
                    {"context":"named_local_to_raw_source_diagnostic", "function":"track_versions_compatible",
                     "args":[local,filename,remote],
                     "result":super::super::super::track_matcher::track_versions_compatible(local,filename,&remote)},
                    {"context":"named_local_to_raw_source_diagnostic", "function":"recording_versions_compatible",
                     "args":[local,recording_title,disambiguation],
                     "result":recording_title.map(|title| super::super::super::auto_tag_acoustid::recording_versions_compatible(local,title,disambiguation))},
                    {"context":"genre_route_diagnostic_not_execution", "function":"track_versions_compatible",
                     "args":[remote,"",local],
                     "result":super::super::super::track_matcher::track_versions_compatible(&remote,"",local)},
                    {"context":"genre_route_diagnostic_not_execution", "function":"recording_versions_compatible",
                     "args":[remote,local_title,Value::Null],
                     "result":super::super::super::auto_tag_acoustid::recording_versions_compatible(&remote,local_title,None)}
                ]);
                pairs.push(json!({"localIndex":local_index,"sourceSelector":selector,
                    "rawRecordingDisambiguation":raw["recording"]["disambiguation"],"calls":calls}));
            }
        }
        sources.push(
            json!({"provider":receipt["provider"],"path":receipt["path"],
            "query":query,"sha256":receipt["sha256"],"pairs":pairs}),
        );
    }
    json!(sources)
}

#[test]
fn evidence_export_retains_disambiguation_even_when_unchanged_guards_pass() {
    let local = TrackCandidate {
        title: Some("Old Time's Sake".into()),
        filename: Some("13 Old Time's Sake".into()),
        ..Default::default()
    };
    let request = LookupRequest {
        tracks: vec![local],
        ..Default::default()
    };
    let raw: Value = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/details-2026-11-07/musicbrainz-22be98f32a7f4174.json"
    )).unwrap();
    let receipt = json!({"provider":"musicbrainz", "path":"/release/10e769d6-c110-4e1f-934e-2d12677d4fbb", "query":"[]", "sha256":"fixture", "status":200, "body":raw});
    let inventory = json!({"records":[receipt], "requests":[{"provider":"musicbrainz", "path":"/release/10e769d6-c110-4e1f-934e-2d12677d4fbb", "query":[], "status":200}]});
    let exported = guard_observations(&request, &inventory);
    let row = &exported[0]["pairs"][12];
    assert_eq!(row["rawRecordingDisambiguation"], "explicit");
    assert_eq!(row["localIndex"], 0);
    assert_eq!(row["sourceSelector"], "/media/0/tracks/12");
    assert_eq!(row["calls"][1]["function"], "recording_versions_compatible");
    assert_eq!(row["calls"][1]["args"][2], "explicit");
    assert_eq!(row["calls"][1]["result"], true);
    assert_eq!(row["calls"][3]["args"][2], Value::Null);
    assert_eq!(
        row["calls"][3]["context"],
        "genre_route_diagnostic_not_execution"
    );
    let unreceived = json!({"records":inventory["records"], "requests":[]});
    assert_eq!(guard_observations(&request, &unreceived), json!([]));
}
