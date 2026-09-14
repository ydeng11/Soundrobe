use super::*;

fn track(title: &str) -> TrackCandidate {
    TrackCandidate {
        title: Some(title.into()),
        length: Some(90.0),
        ..Default::default()
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
        ("Dr. West", "Dr. West (SKIT)"),
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
    assert_eq!(collision.title_rejections[0].kind, SkipKind::ComparisonAmbiguous);
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
