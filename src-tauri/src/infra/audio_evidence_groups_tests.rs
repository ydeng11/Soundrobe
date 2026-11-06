use super::*;

struct Fixtures(PathBuf);
impl Fixtures {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("audio-groups-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn audio(&self, name: &str, signal: &str) -> PathBuf {
        let path = self.0.join(name);
        let output = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                signal,
                "-t",
                "0.08",
                "-metadata",
            ])
            .arg(format!("title={name}"))
            .args(["-y"])
            .arg(&path)
            .output()
            .expect("installed ffmpeg is required");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    }

    fn sources(&self, names: &[&str]) -> Vec<(PathBuf, String)> {
        names
            .iter()
            .map(|name| {
                let path = self.0.join(name);
                let hash = hash_source(&mut File::open(&path).unwrap()).unwrap();
                (path, hash)
            })
            .collect()
    }
}
impl Drop for Fixtures {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[tokio::test]
async fn exact_flac_groups_ignore_metadata_not_audio_and_bind_all_sources() {
    let f = Fixtures::new();
    for name in ["a.flac", "b.flac", "unselected.flac"] {
        f.audio(name, "sine=frequency=440:sample_rate=44100");
    }
    f.audio("different.flac", "sine=frequency=880:sample_rate=44100");
    let sources = f.sources(&["a.flac", "b.flac", "different.flac", "unselected.flac"]);
    assert_ne!(sources[0].1, sources[1].1);
    let proof =
        prove_duplicate_groups(&f.0, &sources, &[vec![0, 1, 2]], &AtomicBool::new(false)).await;
    assert_eq!(proof.groups(), &[vec![0, 1], vec![2], vec![3]]);
    assert!(proof.matches_sources(&sources));
    let mut changed = sources.clone();
    changed[3].1 = "0".repeat(64);
    assert!(!proof.matches_sources(&changed));
    changed = sources.clone();
    changed.swap(0, 1);
    assert!(!proof.matches_sources(&changed));
    assert!(proof.diagnostics().is_object());
    let deterministic =
        ProvenAudioGroups::for_test(sources.clone(), vec![vec![0, 1], vec![2], vec![3]]);
    assert!(deterministic.matches_sources(&sources));
}

#[tokio::test]
async fn exact_flac_groups_reject_corruption_silence_other_codecs_and_stale_hashes() {
    let f = Fixtures::new();
    for name in ["a.flac", "bad.flac", "stale.flac"] {
        f.audio(name, "sine=frequency=440:sample_rate=44100");
    }
    for name in ["zero1.flac", "zero2.flac"] {
        f.audio(name, "anullsrc=r=44100:cl=mono");
    }
    f.audio("other.wav", "sine=frequency=440:sample_rate=44100");
    let bad = f.0.join("bad.flac");
    let mut bytes = std::fs::read(&bad).unwrap();
    bytes[26] ^= 1;
    std::fs::write(&bad, bytes).unwrap();
    let mut sources = f.sources(&[
        "a.flac",
        "bad.flac",
        "stale.flac",
        "zero1.flac",
        "zero2.flac",
        "other.wav",
    ]);
    sources[2].1 = "0".repeat(64);
    let proof = prove_duplicate_groups(
        &f.0,
        &sources,
        &[vec![0, 1, 2, 3, 4, 5]],
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(proof.groups(), &(0..6).map(|i| vec![i]).collect::<Vec<_>>());
    assert!(
        proof.diagnostics()["observations"]
            .as_array()
            .unwrap()
            .len()
            >= 5
    );
}

#[tokio::test]
async fn exact_flac_groups_fail_closed_on_cancellation_bounds_and_aliases() {
    let f = Fixtures::new();
    let a = f.audio("a.flac", "sine=frequency=440:sample_rate=44100");
    std::fs::hard_link(&a, f.0.join("alias.flac")).unwrap();
    f.audio("copy.flac", "sine=frequency=440:sample_rate=44100");
    let sources = f.sources(&["a.flac", "alias.flac", "copy.flac"]);
    let singles = vec![vec![0], vec![1], vec![2]];
    let proof =
        prove_duplicate_groups(&f.0, &sources, &[vec![0, 1, 2]], &AtomicBool::new(false)).await;
    assert_eq!(proof.groups(), &singles);
    let proof = prove_duplicate_groups(&f.0, &sources, &[vec![0, 2]], &AtomicBool::new(true)).await;
    assert_eq!(proof.groups(), &singles);
    assert_eq!(proof.diagnostics()["cancelled"], true);
    for nominations in [vec![vec![0, 3]], vec![vec![0, 2]; 33]] {
        let proof =
            prove_duplicate_groups(&f.0, &sources, &nominations, &AtomicBool::new(false)).await;
        assert_eq!(proof.groups(), &singles);
        assert!(proof.diagnostics()["error"].is_string());
    }
}

#[tokio::test]
async fn exact_flac_groups_revalidate_earlier_file_and_directory_bindings() {
    for replace_directory in [false, true] {
        let f = Fixtures::new();
        for name in ["a.flac", "b.flac"] {
            f.audio(name, "sine=frequency=440:sample_rate=44100");
        }
        let sources = f.sources(&["a.flac", "b.flac"]);
        let stop = AtomicBool::new(false);
        let mut held = BTreeMap::new();
        for (i, (path, hash)) in sources.iter().enumerate() {
            let mut o = observe(&f.0, path, open_source(&f.0, path), hash, &stop).await;
            assert!(reusable_audio(&o.report));
            o.pcm = None;
            held.insert(i, o);
        }
        if replace_directory {
            let old = f.0.join("held-old");
            let replacement = f.0.with_extension("replacement");
            std::fs::create_dir(&replacement).unwrap();
            for name in ["a.flac", "b.flac"] {
                std::fs::copy(f.0.join(name), replacement.join(name)).unwrap();
            }
            let relocated = f.0.with_extension("relocated");
            std::fs::rename(&f.0, &relocated).unwrap();
            std::fs::rename(replacement, &f.0).unwrap();
            std::fs::rename(relocated, old).unwrap();
        } else {
            let bytes = std::fs::read(&sources[0].0).unwrap();
            std::fs::remove_file(&sources[0].0).unwrap();
            std::fs::write(&sources[0].0, bytes).unwrap();
        }
        let mut proof = ProvenAudioGroups::for_test(sources, vec![vec![0], vec![1]]);
        proof.diagnostics = json!({"error":null});
        finish_duplicate_groups(
            &f.0,
            &[vec![0, 1]],
            &mut held,
            &stop,
            tokio::time::Instant::now() + GROUP_TIMEOUT,
            &mut proof,
        );
        assert_eq!(proof.groups(), &[vec![0], vec![1]]);
        assert_eq!(held[&0].report["source"]["stable"], false);
    }
}

#[tokio::test]
async fn exact_flac_groups_cancel_during_observation_and_enforce_batch_deadline() {
    let f = Fixtures::new();
    for name in ["a.flac", "b.flac"] {
        f.audio(name, "sine=frequency=440:sample_rate=44100");
    }
    let sources = f.sources(&["a.flac", "b.flac"]);
    let stop = AtomicBool::new(false);
    let proof = prove_duplicate_groups_with_timeout(
        &f.0,
        &sources,
        &[vec![0, 1]],
        &stop,
        Duration::from_millis(1),
    )
    .await;
    assert_eq!(proof.groups(), &[vec![0], vec![1]]);
    assert!(proof.diagnostics()["error"]
        .as_str()
        .unwrap()
        .contains("deadline"));
    let mut held = BTreeMap::new();
    for (i, (path, hash)) in sources.iter().enumerate() {
        let mut o = observe(&f.0, path, open_source(&f.0, path), hash, &stop).await;
        o.pcm = None;
        held.insert(i, o);
    }
    let mut expired = ProvenAudioGroups::for_test(sources.clone(), vec![vec![0], vec![1]]);
    expired.diagnostics = json!({"error":null});
    finish_duplicate_groups(
        &f.0,
        &[vec![0, 1]],
        &mut held,
        &stop,
        tokio::time::Instant::now() - Duration::from_secs(1),
        &mut expired,
    );
    assert_eq!(expired.groups(), &[vec![0], vec![1]]);
    assert!(held.values().all(|o| o.report["source"]["stable"] == false));
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(10));
        signal.store(true, Ordering::Release);
    });
    let proof = prove_duplicate_groups(&f.0, &sources, &[vec![0, 1]], &stop).await;
    thread.join().unwrap();
    assert_eq!(proof.groups(), &[vec![0], vec![1]]);
    assert_eq!(proof.diagnostics()["cancelled"], true);
}

#[tokio::test]
async fn exact_flac_groups_never_expand_nominations_or_merge_transformed_audio() {
    let f = Fixtures::new();
    for name in ["a.flac", "b.flac", "c.flac", "d.flac"] {
        f.audio(name, "sine=frequency=440:sample_rate=44100");
    }
    f.audio("rate.flac", "sine=frequency=440:sample_rate=48000");
    let sources = f.sources(&["a.flac", "b.flac", "c.flac", "d.flac", "rate.flac"]);
    let proof = prove_duplicate_groups(
        &f.0,
        &sources,
        &[vec![2, 1, 1], vec![0, 1], vec![3], vec![0, 4]],
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(proof.groups(), &[vec![0, 1, 2], vec![3], vec![4]]);
    let mut stale = sources.clone();
    f.audio("b.flac", "sine=frequency=880:sample_rate=44100");
    let proof = prove_duplicate_groups(&f.0, &stale, &[vec![0, 1]], &AtomicBool::new(false)).await;
    assert_eq!(
        proof.groups(),
        &[vec![0], vec![1], vec![2], vec![3], vec![4]]
    );
    stale[0].0 = f.0.join("other-path.flac");
    assert!(!proof.matches_sources(&stale));
}

#[tokio::test]
async fn exact_flac_groups_preserve_native_precision_channels_and_full_frame_count() {
    let f = Fixtures::new();
    for name in ["a24.flac", "b24.flac"] {
        f.audio(
            name,
            "sine=frequency=440:sample_rate=44100,aformat=sample_fmts=s32",
        );
    }
    f.audio("a16.flac", "sine=frequency=440:sample_rate=44100");
    f.audio(
        "stereo.flac",
        "sine=frequency=440:sample_rate=44100,pan=stereo|c0=c0|c1=c0",
    );
    f.audio(
        "short.flac",
        "sine=frequency=440:sample_rate=44100,atrim=end=0.04",
    );
    let sources = f.sources(&[
        "a24.flac",
        "b24.flac",
        "a16.flac",
        "stereo.flac",
        "short.flac",
    ]);
    let proof = prove_duplicate_groups(
        &f.0,
        &sources,
        &[vec![0, 1, 2, 3, 4]],
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(proof.groups(), &[vec![0, 1], vec![2], vec![3], vec![4]]);
    assert_eq!(
        proof.diagnostics()["observations"][0]["observation"]["interpretation"]["pcmFormat"],
        "s24le"
    );
    assert!(proof.diagnostics()["observations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|o| o["observation"]["status"] == "complete"));
}

#[tokio::test]
async fn exact_flac_groups_bound_unique_decodes_and_reject_unsafe_paths() {
    let f = Fixtures::new();
    let sources = (0..33)
        .map(|i| (f.0.join(format!("{i}.flac")), "0".repeat(64)))
        .collect::<Vec<_>>();
    let proof = prove_duplicate_groups(
        &f.0,
        &sources,
        &[(0..17).collect(), (17..33).collect()],
        &AtomicBool::new(false),
    )
    .await;
    assert_eq!(proof.groups().len(), 33);
    assert_eq!(proof.diagnostics()["observations"], json!([]));
    assert!(proof.diagnostics()["error"].is_string());
    f.audio("a.flac", "sine=frequency=440:sample_rate=44100");
    #[cfg(unix)]
    std::os::unix::fs::symlink(f.0.join("a.flac"), f.0.join("symlink.flac")).unwrap();
    let mut sources = f.sources(&["a.flac", "a.flac", "a.flac"]);
    sources[1].0 = f.0.join("symlink.flac");
    sources[2].1 = "invalid".into();
    let proof =
        prove_duplicate_groups(&f.0, &sources, &[vec![0, 1, 2]], &AtomicBool::new(false)).await;
    assert_eq!(proof.groups(), &[vec![0], vec![1], vec![2]]);
}
