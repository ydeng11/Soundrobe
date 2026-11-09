use super::*;

fn track(title: &str) -> TrackCandidate {
    TrackCandidate {
        title: Some(title.into()),
        length: Some(90.0),
        ..Default::default()
    }
}

#[test]
fn aligned_titles_remove_only_known_artist_prefixes_confirmed_by_provider() {
    for (local, provider, expected) in [
        ("草東沒有派對 - Intro", "Intro", "Intro"),
        (
            "草東沒有派對 - Intro (instrumental)",
            "Intro (instrumental)",
            "Intro (instrumental)",
        ),
        ("未知樂團 - Intro", "Intro", "未知樂團 - Intro"),
        (
            "草東沒有派對 - Intro",
            "草東沒有派對 - Intro",
            "草東沒有派對 - Intro",
        ),
    ] {
        let result = match_remote_candidate_tracks(
            &[track(local)],
            &[],
            &[track(provider)],
            "discogs",
            &["草东没有派对".into()],
            &[],
        );
        assert_eq!(result.stats.matched, 1, "{local}");
        assert_eq!(result.tracks[0].title.as_deref(), Some(expected), "{local}");
    }
}

#[test]
fn relapse_requires_title_evidence_for_every_file_including_skits() {
    let local: serde_json::Value = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/relapse-deluxe/local.json"
    ))
    .unwrap();
    let remote: serde_json::Value = serde_json::from_str(include_str!(
        "../../../test/fixtures/tauri/relapse-deluxe/candidate-36441795.json"
    ))
    .unwrap();
    let local: Vec<TrackCandidate> = serde_json::from_value(local["tracks"].clone()).unwrap();
    let remote: Vec<TrackCandidate> = serde_json::from_value(remote["tracks"].clone()).unwrap();
    let filenames = local
        .iter()
        .map(|t| t.filename.clone().unwrap())
        .collect::<Vec<_>>();
    for filename_only in [false, true] {
        let mut local = local.clone();
        if filename_only {
            for track in &mut local {
                track.title = None;
            }
        }
        let result = match_remote_candidate_tracks(
            &local,
            &filenames,
            &remote,
            "discogs",
            &["Eminem".into()],
            &[],
        );
        assert_eq!(result.stats.matched, 22);
        for index in [0, 6, 10, 14, 15, 18] {
            assert_eq!(result.evidence[index], Some(MatchEvidence::GuardedTitle));
        }
        assert!(
            result
                .evidence
                .iter()
                .all(|e| e.is_some() && *e != Some(MatchEvidence::Position)),
            "{:?}",
            result.evidence
        );
        assert_eq!(&result.remote_indices[20..], &[Some(21), Some(20)]);
        assert_eq!(result.tracks[20].track_number, Some(22));
        assert_eq!(result.tracks[21].track_number, Some(21));
    }
}

#[test]
fn guarded_titles_need_duration_and_preserve_local_spelling() {
    for (local, remote) in [
        ("Deja Vu", "Déjà Vu"),
        ("Smør", "Smǿr"),
        ("Tonya [Skit]", "Tonya"),
    ] {
        let result = match_remote_candidate_tracks(
            &[track(local)],
            &[],
            &[track(remote)],
            "discogs",
            &[],
            &[],
        );
        assert_eq!(result.stats.matched, 1, "{local}");
        assert_eq!(result.tracks[0].title.as_deref(), Some(local));
        for duration in [
            None,
            Some(0.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(120.0),
        ] {
            for missing_local in [false, true] {
                let mut left = track(local);
                let mut right = track(remote);
                if missing_local {
                    left.length = duration;
                } else {
                    right.length = duration;
                }
                assert_eq!(
                    match_remote_candidate_tracks(&[left], &[], &[right], "discogs", &[], &[])
                        .stats
                        .matched,
                    0
                );
            }
        }
    }
}

#[test]
fn aligned_provider_titles_retain_descriptors_missing_from_local_titles() {
    for (local, remote) in [
        ("Dr. West", "Dr. West (skit)"),
        ("Doolin-Dalton", "Doolin‐Dalton (instrumental)"),
    ] {
        for filename_only in [false, true] {
            let mut input = track(local);
            input.filename = Some(format!("01 Lead - {local}.flac"));
            if filename_only {
                input.title = None;
            }
            let result = match_remote_candidate_tracks(
                &[input],
                &[format!("01 Lead - {local}.flac")],
                &[track(remote)],
                "musicbrainz",
                &["Lead".into()],
                &[],
            );
            assert_eq!(result.stats.matched, 1);
            assert_eq!(result.tracks[0].title.as_deref(), Some(remote));
        }
    }
}

#[test]
fn guarded_titles_preserve_non_latin_marks_and_accept_decomposed_latin() {
    assert_ne!(guarded_title("α"), guarded_title("ά"));
    assert_eq!(
        guarded_title("Deja Vu"),
        guarded_title("De\u{301}ja\u{300} Vu")
    );
    assert_eq!(guarded_title("Ebudae"), guarded_title("Ebudæ"));
    assert_eq!(guarded_title("AEon"), guarded_title("Æon"));
}

#[test]
fn guarded_titles_accept_provider_punctuation_and_latin_mark_variants() {
    let result = match_remote_candidate_tracks(
        &[track("Enya - Triad- St. Patrick , Cu Chulainn , Oisin")],
        &[],
        &[track("Triad: St. Patrick / Cú Chulainn / Oisin")],
        "musicbrainz",
        &["Enya".into()],
        &[],
    );

    assert_eq!(result.stats.matched, 1);
    assert_eq!(result.evidence, vec![Some(MatchEvidence::GuardedTitle)]);
    assert_eq!(
        result.tracks[0].title.as_deref(),
        Some("Triad- St. Patrick , Cu Chulainn , Oisin")
    );
}

#[test]
fn guarded_ligature_equivalence_requires_unique_duration_backing() {
    let result = match_remote_candidate_tracks(
        &[track("Ebudae")],
        &[],
        &[track("Ebudæ")],
        "discogs",
        &[],
        &[],
    );
    assert_eq!(result.stats.matched, 1);
    assert_eq!(result.evidence, vec![Some(MatchEvidence::GuardedTitle)]);

    let mut different_duration = track("Ebudæ");
    different_duration.length = Some(120.0);
    let result = match_remote_candidate_tracks(
        &[track("Ebudae")],
        &[],
        &[different_duration],
        "discogs",
        &[],
        &[],
    );
    assert_eq!(result.stats.matched, 0);
    assert_eq!(result.title_rejections[0].kind, SkipKind::DurationMismatch);

    let mut missing_duration = track("Ebudæ");
    missing_duration.length = None;
    let result = match_remote_candidate_tracks(
        &[track("Ebudae")],
        &[],
        &[missing_duration],
        "discogs",
        &[],
        &[],
    );
    assert_eq!(result.stats.matched, 0);
    assert_eq!(result.title_rejections[0].kind, SkipKind::DurationMissing);

    let collision = match_remote_candidate_tracks(
        &[track("Ebudae")],
        &[],
        &[track("Ebudæ"), track("EbudÆ")],
        "discogs",
        &[],
        &[],
    );
    assert_eq!(collision.stats.matched, 0);
    assert_eq!(
        collision.title_rejections[0].kind,
        SkipKind::ComparisonAmbiguous
    );
}

#[test]
fn guarded_titles_do_not_erase_performance_qualifiers() {
    for qualifier in [
        "Live",
        "Remix",
        "Instrumental",
        "Radio Version",
        "Clean",
        "Explicit",
    ] {
        let result = match_remote_candidate_tracks(
            &[track("Deja Vu")],
            &[],
            &[track(&format!("Déjà Vu ({qualifier})"))],
            "discogs",
            &[],
            &[],
        );
        assert_eq!(result.stats.matched, 0, "{qualifier}");
    }
}

#[test]
fn guarded_title_collisions_remain_ambiguous_after_exact_assignments() {
    for reverse in [false, true] {
        let mut local = vec![track("Deja Vu"), track("Déjà Vu")];
        if reverse {
            local.reverse();
        }
        let result =
            match_remote_candidate_tracks(&local, &[], &[track("Déjà Vu")], "discogs", &[], &[]);
        let fallback_index = local
            .iter()
            .position(|t| t.title.as_deref() == Some("Deja Vu"))
            .unwrap();
        assert_eq!(result.evidence[fallback_index], None);
        assert_eq!(result.stats.matched, 1);
    }
    let remote = vec![track("Déjà Vu"), track("Dèja Vu")];
    assert_eq!(
        match_remote_candidate_tracks(&[track("Deja Vu")], &[], &remote, "discogs", &[], &[])
            .stats
            .matched,
        0
    );
    // Equal positions on different discs must not make duplicate titles unique.
    let remote = (1..=2)
        .map(|disc| TrackCandidate {
            disc_number: Some(disc),
            track_number: Some(1),
            ..track("Tonya (Skit)")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        match_remote_candidate_tracks(&[track("Tonya")], &[], &remote, "discogs", &[], &[])
            .stats
            .matched,
        0
    );
}

#[test]
fn guarded_word_boundaries_match_frozen_watermark_titles_without_changing_display() {
    for filename_only in [false, true] {
        let mut local = track("The Long Ships");
        local.length = Some(219.2);
        if filename_only {
            local.title = None;
        }
        let mut remote = track("The Longships");
        remote.length = Some(218.84);
        let result = match_remote_candidate_tracks(
            &[local],
            &["10 - The Long Ships.flac".into()],
            &[remote],
            "musicbrainz",
            &["Enya".into()],
            &[],
        );
        assert_eq!(result.stats.matched, 1, "filename_only={filename_only}");
        assert_eq!(result.evidence, vec![Some(MatchEvidence::GuardedTitle)]);
        assert_eq!(result.tracks[0].title.as_deref(), Some("The Long Ships"));
    }
}

#[test]
fn guarded_word_boundaries_require_duration_and_retain_version_conflicts() {
    for (local_title, remote_title) in [
        ("The Long Ships (Live)", "The Longships"),
        ("The Long Ships", "The Longships (Live)"),
        ("The Long Ships (2014 Remastered)", "The Longships"),
        ("The Long Ships", "The Longships (2014 Remastered)"),
    ] {
        let result = match_remote_candidate_tracks(
            &[track(local_title)],
            &[],
            &[track(remote_title)],
            "discogs",
            &[],
            &[],
        );
        assert_eq!(result.stats.matched, 0, "{local_title} / {remote_title}");
    }
    for duration in [
        None,
        Some(0.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
        Some(260.0),
    ] {
        for missing_local in [false, true] {
            let mut local = track("The Long Ships");
            local.length = Some(219.2);
            let mut remote = track("The Longships");
            remote.length = Some(218.84);
            if missing_local {
                local.length = duration;
            } else {
                remote.length = duration;
            }
            let result =
                match_remote_candidate_tracks(&[local], &[], &[remote], "discogs", &[], &[]);
            assert_eq!(
                result.stats.matched, 0,
                "{duration:?}, local={missing_local}"
            );
        }
    }
}

#[test]
fn guarded_word_boundary_collisions_include_consumed_exact_titles() {
    for reverse in [false, true] {
        let mut local = vec![track("The Longships"), track("The Long Ships")];
        if reverse {
            local.reverse();
        }
        let result = match_remote_candidate_tracks(
            &local,
            &[],
            &[track("The Longships")],
            "discogs",
            &[],
            &[],
        );
        let compact_fallback = local
            .iter()
            .position(|track| track.title.as_deref() == Some("The Long Ships"))
            .unwrap();
        assert_eq!(result.stats.matched, 1);
        assert_eq!(result.evidence[compact_fallback], None);
        assert_eq!(
            result.title_rejections[0].kind,
            SkipKind::ComparisonAmbiguous
        );

        let mut remote = vec![track("The Longships"), track("TheLong Ships")];
        if reverse {
            remote.reverse();
        }
        let result = match_remote_candidate_tracks(
            &[track("The Longships"), track("The Long Ships")],
            &[],
            &remote,
            "discogs",
            &[],
            &[],
        );
        assert_eq!(result.stats.matched, 1);
        assert_eq!(result.evidence[1], None);
        assert_eq!(
            result.title_rejections[0].kind,
            SkipKind::ComparisonAmbiguous
        );
    }
}

#[test]
fn frozen_utada_romanized_subtitle_and_remaster_are_not_assumed_equivalent() {
    for remote_title in [
        "Hymne à l'amour 〜愛のアンセム〜",
        "Unrelated main title 〜愛のアンセム〜",
    ] {
        let mut local = track("Hikaru Utada - Hymne A L'Amour -Ai No Anthem- (2014 Remastered)");
        local.length = Some(389.947);
        let mut remote = track(remote_title);
        remote.length = Some(389.0);
        let result = match_remote_candidate_tracks(
            &[local],
            &["17 Hikaru Utada - Hymne A L'Amour -Ai No Anthem- (2014 Remastered).flac".into()],
            &[remote],
            "musicbrainz",
            &["Hikaru Utada".into()],
            &[],
        );
        assert_eq!(result.stats.matched, 0, "{remote_title}");
    }
}

#[test]
fn supported_tag_version_does_not_inherit_filename_remaster_wording() {
    let mut remote = track("Passion");
    remote.length = Some(285.0);
    let mut local = track("Passion ～single version～");
    local.length = Some(285.1);
    let remote: TrackCandidate = serde_json::from_value(serde_json::json!({
        "title":remote.title, "length":remote.length,
        "match_titles":["Passion (Single Version)"],
        "release_disambiguation":"2014 Remastered, 96kHz/24bit"
    }))
    .unwrap();
    let result = match_remote_candidate_tracks(
        &[local],
        &["10 Hikaru Utada - Passion (Single Version _ 2014 Remastered).flac".into()],
        &[remote],
        "musicbrainz",
        &["Hikaru Utada".into()],
        &[],
    );
    assert_eq!(result.stats.matched, 1);
    assert_eq!(result.evidence[0], Some(MatchEvidence::TagTitle));
    assert_eq!(
        result.tracks[0].title.as_deref(),
        Some("Passion ～single version～")
    );
}

#[test]
fn version_conflicts_cannot_use_ids_filenames_or_containment() {
    for (local_title, remote_title) in [
        ("Passion (After The Battle)", "Passion (Single Version)"),
        ("Passion (Album Version)", "Passion (Single Version)"),
        ("Grown Woman (Studio Version)", "Grown Woman (Live)"),
        ("Grown Woman (Live)", "Grown Woman (Studio Version)"),
    ] {
        for id_match in [false, true] {
            let mut local = track(local_title);
            let mut remote = track(remote_title);
            if id_match {
                local.musicbrainz_track_id = Some("same-recording".into());
                remote.musicbrainz_track_id = local.musicbrainz_track_id.clone();
            }
            let result = match_remote_candidate_tracks(
                &[local],
                &[format!("10 - {local_title}.flac")],
                &[remote],
                "musicbrainz",
                &[],
                &[],
            );
            assert_eq!(result.stats.matched, 0, "{local_title}, id={id_match}");
        }
    }
}

#[test]
fn local_remaster_claim_requires_the_same_provider_release_context() {
    for (context, provider_title, local_title, accepted) in [
        (
            None,
            "Passion (Single Version)",
            "Passion (Single Version _ 2014 Remastered)",
            false,
        ),
        (
            Some("2020 Remastered"),
            "Passion (Single Version)",
            "Passion (Single Version _ 2014 Remastered)",
            false,
        ),
        (
            Some("2014 Remastered, 96kHz/24bit"),
            "Passion (Single Version)",
            "Passion (Single Version _ 2014 Remastered)",
            true,
        ),
        (
            Some("2014 Remastered"),
            "Passion (Single Version _ 2020 Remastered)",
            "Passion (Single Version _ 2014 Remastered)",
            false,
        ),
        (
            None,
            "Passion (Single Version)",
            "Passion (Single Version _ ２０１４ Ｒｅｍａｓｔｅｒｅｄ)",
            false,
        ),
    ] {
        let remote: TrackCandidate = serde_json::from_value(serde_json::json!({
            "title":provider_title, "length":90.0,
            "release_disambiguation":context
        }))
        .unwrap();
        let result = match_remote_candidate_tracks(
            &[track(local_title)],
            &[],
            &[remote],
            "musicbrainz",
            &[],
            &[],
        );
        assert_eq!(result.stats.matched == 1, accepted, "{context:?}");
    }
}
