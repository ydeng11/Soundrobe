//! Real offline CLI contracts; explicitly run with installed FFmpeg/ffprobe.
#![cfg(all(feature = "server", unix))]

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("inventory-audio-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wav(&self, name: &str, frames: &[[i16; 2]]) -> Value {
        let size = (frames.len() * 4) as u32;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((size + 36).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(44100u32.to_le_bytes());
        bytes.extend(176400u32.to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        for frame in frames {
            for sample in frame {
                bytes.extend(sample.to_le_bytes());
            }
        }
        fs::write(self.0.join(name), &bytes).unwrap();
        json!({"path":name,"sha256":format!("{:x}", Sha256::digest(&bytes))})
    }
    fn run(&self, pairs: Option<Value>) -> Value {
        let mut command = Command::new(env!("CARGO_BIN_EXE_soundrobe-inventory"));
        command.arg(&self.0);
        if let Some(pairs) = pairs {
            let manifest = self.0.join("pairs.json");
            fs::write(&manifest, json!({"pairs":pairs}).to_string()).unwrap();
            command.arg("--audio-pairs").arg(manifest);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn strip_times(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("observedAt");
            for child in map.values_mut() {
                strip_times(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_times(child);
            }
        }
        _ => (),
    }
}

impl Fixture {
    fn source(&self, name: &str) -> Value {
        json!({"path":name,"sha256":format!("{:x}", Sha256::digest(fs::read(self.0.join(name)).unwrap()))})
    }

    fn flac(&self, name: &str, frames: &[[i16; 2]], comment: &str) -> Value {
        self.wav("encoding-input.wav", frames);
        assert!(Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(self.0.join("encoding-input.wav"))
            .args(["-metadata", &format!("comment={comment}")])
            .arg(self.0.join(name))
            .status()
            .unwrap()
            .success());
        self.source(name)
    }

    // Synthetic independent provider observation, not a real lookup or identity claim.
    // Publish through the existing terminal store; never add a reuse receipt.
    fn donor_receipt(&self, name: &str, consensus: bool) -> Value {
        use std::os::unix::fs::MetadataExt;
        let path = self.0.join(name).canonicalize().unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let source = json!({"path":path,"fingerprint":self.source(name)["sha256"],
            "physicalIdentity":format!("{}:{}",metadata.dev(),metadata.ino())});
        let ids = [
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
        ];
        let count = if consensus { 2 } else { 1 };
        let candidates: Vec<_> = ids[..count].iter().map(|id| json!({
            "recordingId":id,"canonicalRecordingId":id,"score":0.99,
            "authoritativeTitle":"Program","authoritativeDuration":0.034829931972789115,
            "artistCredits":["Lead","Guest"],"disambiguation":null,
            "disposition":if consensus {"supported"} else {"verified"},"resultIds":["synthetic-result"],
            "providerObservation":{"version":1,"provider":"musicbrainz","sourceLevel":"recording",
                "canonicalId":id,"responseId":id,"title":"Program","recordingDurationMs":34.829_931_972_789_11,
                "rawOrderedBilling":[{"name":"Lead","joinphrase":" feat. "},{"name":"Guest","joinphrase":""}],
                "fetchedAt":null,"responseDigest":null,"responseUrl":null}})).collect();
        let core = json!({"title":"Program","artist":"Lead feat. Guest","artists":["Lead","Guest"],
            "musicbrainzTrackId":if consensus {Value::Null} else {json!(ids[0])}});
        let evidence = json!({"source":source,"queryScope":"existing_acoustid_recording_resolution",
            "trackMatch":{"fileName":name,"recordingId":if consensus {""} else {ids[0]},"score":0.99,
                "candidateSetComplete":true,"candidateCount":count,"omittedCandidates":0,
                "reason":if consensus {Some("recording_fields_consensus")} else {None},"candidates":candidates,
                "lookupObservation":{"version":1,"provider":"acoustid","source":source,
                    "status":"successful_nonempty","eligibleHypothesisCount":count,"observedAt":null}}});
        json!({"version":1,"decisionRuleVersion":3,"evidenceVersion":1,"runId":null,
            "attemptId":"synthetic-independent-attempt","preWrite":source,"postWrite":source,
            "sourceStable":true,"payloadPreservation":"validated_by_atomic_writer",
            "payloadDigest":"a".repeat(64),"writeOutcome":"unchanged","accepted":core,"readback":core,
            "decisionBasis":if consensus {"recording_fields_consensus"} else {"selected_recording"},
            "evidenceDigest":format!("{:x}",Sha256::digest(evidence.to_string().as_bytes())),
            "evidence":evidence,"observedAt":1,"publishedAt":null,"parentProof":null})
    }

    fn publish_donors(&self, receipts: Vec<Value>) {
        let worker = soundrobe_lib::state::worker::WorkerStore::new_in(self.0.join("state"));
        assert!(worker.initialize());
        worker
            .enqueue(std::slice::from_ref(&self.0), "manual")
            .unwrap();
        worker.mark_running(&self.0, "independent-run").unwrap();
        worker
            .finish_with_evidence(
                &self.0,
                "independent-run",
                "needs_review",
                None,
                None,
                &json!({"proofVersion":1,"coreReceipts":receipts}),
            )
            .unwrap();
    }

    fn reuse(&self, pairs: Value, markdown: bool) -> std::process::Output {
        let manifest = self.0.join("reuse.json");
        fs::write(&manifest, json!({"pairs":pairs}).to_string()).unwrap();
        Command::new(env!("CARGO_BIN_EXE_soundrobe-inventory"))
            .arg(&self.0)
            .arg("--worker-data")
            .arg(self.0.join("state"))
            .arg("--reuse-proposals")
            .arg(manifest)
            .args(["--format", if markdown { "markdown" } else { "json" }])
            .output()
            .unwrap()
    }
}

#[test]
#[ignore = "native offline proposal contract: requires FFmpeg and ffprobe"]
fn reuse_proposes_only_core_from_independently_rooted_complete_audio_without_writes() {
    let fixture = Fixture::new();
    let frames: Vec<_> = (0..1536)
        .map(|i| {
            [
                ((i * 73) % 1301 - 650) as i16,
                ((i * 19) % 787 - 393) as i16,
            ]
        })
        .collect();
    let donor = fixture.flac("donor.flac", &frames, "independent donor");
    let recipient = fixture.flac("recipient.flac", &frames, "recipient edition must survive");
    assert_ne!(
        donor["sha256"], recipient["sha256"],
        "different containers/tags, identical complete PCM"
    );
    fixture.publish_donors(vec![fixture.donor_receipt("donor.flac", false)]);
    let default = Command::new(env!("CARGO_BIN_EXE_soundrobe-inventory"))
        .arg(&fixture.0)
        .arg("--worker-data")
        .arg(fixture.0.join("state"))
        .output()
        .unwrap();
    assert!(default.status.success());
    let mut default: Value = serde_json::from_slice(&default.stdout).unwrap();
    let before: Vec<_> = ["donor.flac", "recipient.flac", "state/worker.db"]
        .iter()
        .map(|name| fs::read(fixture.0.join(name)).unwrap())
        .collect();
    let pairs = json!([{"left":donor,"right":recipient},{"left":donor,"right":recipient}]);
    let output = fixture.reuse(pairs.clone(), false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let proposals = &report["reuseProposals"];
    assert_eq!(
        proposals["proposalCount"], 1,
        "deduplicate recipients and duplicate pair selections"
    );
    let proposal = &proposals["proposals"][0];
    assert_eq!(proposal["state"], "proposal_only");
    assert_eq!(
        proposal["patch"],
        json!({"title":"Program","artist":"Lead feat. Guest","artists":["Lead","Guest"],"musicbrainzTrackId":"11111111-1111-4111-8111-111111111111"})
    );
    assert_eq!(proposal["recipient"]["fingerprint"], recipient["sha256"]);
    assert_eq!(
        proposal["donors"][0]["source"]["fingerprint"],
        donor["sha256"]
    );
    assert_eq!(proposal["donors"][0]["root"]["runId"], "independent-run");
    assert_eq!(proposal["donors"].as_array().unwrap().len(), 1);
    assert_eq!(proposal["independentCorroborationCount"], Value::Null);
    assert_eq!(
        report["summary"]["verifiedCoreFiles"], 1,
        "the recipient stays unresolved"
    );
    assert_eq!(
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["relativePath"] == "recipient.flac")
            .unwrap()["coreState"],
        "unresolved"
    );
    let markdown = fixture.reuse(pairs, true);
    assert!(markdown.status.success());
    assert!(String::from_utf8_lossy(&markdown.stdout)
        .contains("Verified-audio reuse proposals (no writes)"));
    let mut unchanged_inventory = report.clone();
    unchanged_inventory
        .as_object_mut()
        .unwrap()
        .remove("reuseProposals");
    strip_times(&mut unchanged_inventory);
    strip_times(&mut default);
    assert_eq!(
        default, unchanged_inventory,
        "all default proof/ranking fields stay unchanged"
    );
    for (name, original) in ["donor.flac", "recipient.flac", "state/worker.db"]
        .iter()
        .zip(before)
    {
        assert_eq!(
            fs::read(fixture.0.join(name)).unwrap(),
            original,
            "read-only export changed {name}"
        );
    }
}

#[test]
#[ignore = "native offline proposal contract: requires FFmpeg and ffprobe"]
fn reuse_requires_current_independent_root_and_keeps_id_free_consensus_distinct() {
    for fault in [
        "none",
        "consensus",
        "missing",
        "pending",
        "pending_run",
        "old_run",
        "rule",
        "lineage",
        "sparse",
        "wrong_scope",
        "cross_file",
        "reversed",
        "dropped_guest",
        "wrong_id",
        "changed",
        "replaced",
        "all_zero",
    ] {
        let fixture = Fixture::new();
        let frames = if fault == "all_zero" {
            vec![[0, 0]; 32]
        } else {
            [[37, -19], [21, 0], [-8, 93], [1, -2]].repeat(8)
        };
        let donor = fixture.flac("donor.flac", &frames, "donor");
        let recipient = fixture.flac("recipient.flac", &frames, "recipient");
        fixture.publish_donors(vec![
            fixture.donor_receipt("donor.flac", fault == "consensus")
        ]);
        let db = rusqlite::Connection::open(fixture.0.join("state/worker.db")).unwrap();
        let stored: String = db
            .query_row("SELECT evidence FROM worker_evidence", [], |row| row.get(0))
            .unwrap();
        let mut evidence: Value = serde_json::from_str(&stored).unwrap();
        let receipt = &mut evidence["coreReceipts"][0];
        match fault {
            "missing" => evidence["coreReceipts"] = json!([]),
            "pending" => {
                evidence["pendingCoreReceipts"] = evidence["coreReceipts"].take();
            }
            "old_run" => {
                db.execute("UPDATE worker_items SET run_id='later-run'", [])
                    .unwrap();
            }
            "pending_run" => {
                db.execute("UPDATE worker_items SET status='pending'", [])
                    .unwrap();
            }
            "rule" => receipt["decisionRuleVersion"] = json!(2),
            "lineage" => receipt["parentProof"] = json!("unproven-reuse-parent"),
            "sparse" => receipt["evidence"]["trackMatch"]["candidates"] = json!([]),
            "wrong_scope" => receipt["evidence"]["queryScope"] = json!("copied_tags"),
            "cross_file" => {
                receipt["evidence"]["trackMatch"]["lookupObservation"]["source"]["path"] =
                    json!(fixture.0.join("recipient.flac"))
            }
            "reversed" => {
                receipt["evidence"]["trackMatch"]["candidates"][0]["artistCredits"] =
                    json!(["Guest", "Lead"])
            }
            "dropped_guest" => {
                receipt["evidence"]["trackMatch"]["candidates"][0]["providerObservation"]
                    ["rawOrderedBilling"] = json!([{"name":"Lead"}])
            }
            "wrong_id" => {
                receipt["evidence"]["trackMatch"]["candidates"][0]["canonicalRecordingId"] =
                    json!("33333333-3333-4333-8333-333333333333")
            }
            "changed" => {
                let mut b = fs::read(fixture.0.join("donor.flac")).unwrap();
                let last = b.len() - 1;
                b[last] ^= 1;
                fs::write(fixture.0.join("donor.flac"), b).unwrap();
            }
            "replaced" => {
                fs::copy(fixture.0.join("donor.flac"), fixture.0.join("replacement")).unwrap();
                fs::rename(fixture.0.join("replacement"), fixture.0.join("donor.flac")).unwrap();
            }
            _ => (),
        }
        if let Some(receipt) = evidence["coreReceipts"]
            .as_array_mut()
            .and_then(|r| r.first_mut())
        {
            receipt["evidenceDigest"] = json!(format!(
                "{:x}",
                Sha256::digest(receipt["evidence"].to_string().as_bytes())
            ));
        }
        db.execute(
            "UPDATE worker_evidence SET evidence=?1",
            [evidence.to_string()],
        )
        .unwrap();
        drop(db);
        let output = fixture.reuse(json!([{"left":donor,"right":recipient}]), false);
        assert!(
            output.status.success(),
            "{fault}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["reuseProposals"]["proposalCount"],
            if matches!(fault, "none" | "consensus") {
                1
            } else {
                0
            },
            "{fault}: {}",
            report["reuseProposals"]
        );
        if fault == "consensus" {
            let proposal = &report["reuseProposals"]["proposals"][0];
            assert_eq!(proposal["patch"]["musicbrainzTrackId"], Value::Null);
            assert!(
                proposal["patch"].get("musicbrainzTrackId").is_none(),
                "an unknown ID is not an instruction to erase the recipient ID"
            );
            assert_eq!(proposal["decisionBasis"], "recording_fields_consensus");
            assert_eq!(proposal["donors"][0]["root"]["parentProof"], Value::Null);
        }
    }
}

#[test]
#[ignore = "native offline proposal contract: requires FFmpeg and ffprobe"]
fn reuse_rechecks_live_sources_and_worker_ownership_after_decoding() {
    let ffmpeg = Command::new("which").arg("ffmpeg").output().unwrap();
    let ffmpeg = String::from_utf8(ffmpeg.stdout).unwrap().trim().to_owned();
    for fault in [
        "donor",
        "recipient",
        "ownership",
        "wrong_folder",
        "earlier_pair",
        "cancelled",
    ] {
        let fixture = Fixture::new();
        let frames = [[37, -19], [21, 0], [-8, 93], [1, -2]].repeat(8);
        let donor = fixture.flac("donor.flac", &frames, "donor");
        let recipient = fixture.flac("recipient.flac", &frames, "recipient");
        fixture.publish_donors(vec![fixture.donor_receipt("donor.flac", false)]);
        let marker = fixture.0.join("decode-ready");
        let release = fixture.0.join("decode-release");
        let counter = fixture.0.join("decode-count");
        let decoder=fixture.script("decode-held",&format!(
            "if [ \"$1\" != -version ]; then\n n=$(cat '{counter}' 2>/dev/null || echo 0); n=$((n+1)); echo $n > '{counter}'\n if [ $n = {at} ]; then touch '{marker}'; i=0; while [ ! -e '{release}' ] && [ $i -lt 1000 ]; do sleep 0.01; i=$((i+1)); done; fi\nfi\nexec '{ffmpeg}' \"$@\"\n",
            counter=counter.display(),at=if fault=="earlier_pair" {3} else {1},marker=marker.display(),release=release.display()));
        let manifest = fixture.0.join("reuse.json");
        let pair = json!({"left":donor,"right":recipient});
        fs::write(
            &manifest,
            json!({"pairs":if fault=="earlier_pair" {json!([pair,pair])} else {json!([pair])}})
                .to_string(),
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_soundrobe-inventory"))
            .arg(&fixture.0)
            .arg("--worker-data")
            .arg(fixture.0.join("state"))
            .arg("--reuse-proposals")
            .arg(manifest)
            .env("AUTO_TAG_FFMPEG_PATH", decoder)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let start = std::time::Instant::now();
        while !marker.exists()
            && child.try_wait().unwrap().is_none()
            && start.elapsed() < std::time::Duration::from_secs(10)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "{fault}: decoder barrier not reached");
        if fault == "cancelled" {
            assert_eq!(
                unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
                0
            );
        } else if fault == "ownership" {
            rusqlite::Connection::open(fixture.0.join("state/worker.db"))
                .unwrap()
                .execute(
                    "UPDATE worker_items SET run_id='new-owner',status='running'",
                    [],
                )
                .unwrap();
        } else if fault == "wrong_folder" {
            let other = fixture.0.join("other-folder");
            fs::create_dir(&other).unwrap();
            let db = rusqlite::Connection::open(fixture.0.join("state/worker.db")).unwrap();
            for table in ["worker_items", "worker_evidence"] {
                db.execute(
                    &format!("UPDATE {table} SET path=?1"),
                    [other.to_string_lossy().as_ref()],
                )
                .unwrap();
            }
        } else {
            let path = fixture.0.join(if fault == "donor" {
                "donor.flac"
            } else {
                "recipient.flac"
            });
            fs::copy(&path, fixture.0.join("replacement")).unwrap();
            fs::rename(fixture.0.join("replacement"), path).unwrap();
        }
        fs::write(release, b"continue").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{fault}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["reuseProposals"]["proposalCount"], 0,
            "{fault}: {}",
            report["reuseProposals"]
        );
    }
}

#[test]
#[ignore = "native offline proposal contract: requires FFmpeg and ffprobe"]
fn reuse_requires_compatible_native_rate_channels_precision_and_layout() {
    let fixture = Fixture::new();
    let frames: Vec<_> = (0..96)
        .map(|i| [(i * 13 - 600) as i16, (i * 31 - 1500) as i16])
        .collect();
    let donor = fixture.flac("donor.flac", &frames, "donor");
    fixture.publish_donors(vec![fixture.donor_receipt("donor.flac", false)]);
    let mut pairs = Vec::new();
    for (name, rate, channels, bits) in [
        ("rate", 48000u32, 2u16, 16u16),
        ("channels", 44100, 1, 16),
        ("precision", 44100, 2, 24),
        ("layout", 44100, 4, 16),
    ] {
        fixture.wav("reinterpret.wav", &frames);
        let mut bytes = fs::read(fixture.0.join("reinterpret.wav")).unwrap();
        bytes[22..24].copy_from_slice(&channels.to_le_bytes());
        bytes[24..28].copy_from_slice(&rate.to_le_bytes());
        bytes[28..32].copy_from_slice(&(rate * u32::from(channels * bits / 8)).to_le_bytes());
        bytes[32..34].copy_from_slice(&(channels * bits / 8).to_le_bytes());
        bytes[34..36].copy_from_slice(&bits.to_le_bytes());
        fs::write(fixture.0.join("reinterpret.wav"), bytes).unwrap();
        let mut encoder = Command::new("ffmpeg");
        encoder
            .args(["-v", "error", "-y", "-i"])
            .arg(fixture.0.join("reinterpret.wav"));
        if bits == 24 {
            encoder.args(["-sample_fmt", "s32", "-bits_per_raw_sample", "24"]);
        }
        assert!(encoder
            .arg(fixture.0.join(format!("{name}.flac")))
            .status()
            .unwrap()
            .success());
        pairs.push(json!({"left":donor,"right":fixture.source(&format!("{name}.flac"))}));
    }
    let output = fixture.reuse(json!(pairs), false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["reuseProposals"]["proposalCount"], 0);
    for pair in &report["reuseProposals"]["pairs"].as_array().unwrap()[..3] {
        assert_eq!(
            pair["left"]["pcm"]["sha256"], pair["right"]["pcm"]["sha256"],
            "same bytes alone cannot override interpretation"
        );
        assert_eq!(pair["comparison"]["status"], "unavailable");
    }
    assert_eq!(
        report["reuseProposals"]["pairs"][3]["right"]["status"],
        "incomplete"
    );
}

#[test]
#[ignore = "native offline proposal contract: requires FFmpeg and ffprobe"]
fn reuse_rejects_edges_partial_audio_unproven_completeness_and_conflicting_roots() {
    let fixture = Fixture::new();
    let program = [[37, -19], [21, 0], [-8, 93], [1, -2]].repeat(8);
    let donor = fixture.flac("donor.flac", &program, "donor");
    let equal = fixture.flac("equal.flac", &program, "equal");
    let padded_frames = [vec![[0, 0]], program.clone(), vec![[0, 0]]].concat();
    let padded = fixture.flac("padded.flac", &padded_frames, "padded");
    let mut changed_tail = program.clone();
    changed_tail.last_mut().unwrap()[1] = -1;
    let tail = fixture.flac("tail.flac", &changed_tail, "tail");
    let partial = fixture.flac("partial.flac", &program[..31], "partial");
    let wav = fixture.wav("same.wav", &program);
    let mut bytes = fs::read(fixture.0.join("equal.flac")).unwrap();
    bytes[26] ^= 1;
    fs::write(fixture.0.join("bad.flac"), bytes).unwrap();
    let bad = fixture.source("bad.flac");
    fixture.publish_donors(vec![fixture.donor_receipt("donor.flac", false)]);
    let pairs = json!([{"left":donor,"right":padded},{"left":donor,"right":tail},
        {"left":donor,"right":partial},{"left":donor,"right":wav},
        {"left":donor,"right":bad},{"left":donor,"right":donor}]);
    let output = fixture.reuse(pairs, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["reuseProposals"]["proposalCount"], 0);
    assert_eq!(
        report["reuseProposals"]["pairs"][0]["comparison"]["zeroEdgeExcludedEqual"],
        true
    );
    assert_eq!(
        report["reuseProposals"]["pairs"][3]["comparison"]["wholePcmEqual"],
        true
    );
    assert_eq!(
        report["reuseProposals"]["pairs"][3]["rejection"],
        "checksum_completeness_unavailable"
    );
    assert_eq!(
        report["reuseProposals"]["pairs"][5]["rejection"],
        "self_or_same_physical_source"
    );
    // Two valid, independently rooted but contradictory labels must not vote.
    fs::copy(
        fixture.0.join("donor.flac"),
        fixture.0.join("conflict.flac"),
    )
    .unwrap();
    let mut conflicting = fixture.donor_receipt("conflict.flac", false);
    for core in ["accepted", "readback"] {
        conflicting[core]["title"] = json!("Other Program");
    }
    conflicting["evidence"]["trackMatch"]["candidates"][0]["authoritativeTitle"] =
        json!("Other Program");
    conflicting["evidence"]["trackMatch"]["candidates"][0]["providerObservation"]["title"] =
        json!("Other Program");
    conflicting["evidenceDigest"] = json!(format!(
        "{:x}",
        Sha256::digest(conflicting["evidence"].to_string().as_bytes())
    ));
    // A new isolated store avoids reclaiming an existing run in fixture setup.
    fs::remove_dir_all(fixture.0.join("state")).unwrap();
    fixture.publish_donors(vec![
        fixture.donor_receipt("donor.flac", false),
        conflicting,
    ]);
    let output = fixture.reuse(
        json!([{"left":donor,"right":equal},
        {"left":fixture.source("conflict.flac"),"right":equal}]),
        false,
    );
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["reuseProposals"]["proposalCount"], 0);
    assert_eq!(
        report["reuseProposals"]["conflicts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
#[ignore = "native offline proposal contract: requires FFmpeg and ffprobe"]
fn reuse_conflicts_include_current_independent_receipts_that_cannot_donate() {
    for scenario in [
        "eligible",
        "qualified",
        "basis",
        "pending",
        "stale",
        "unbound",
        "lineage",
        "invalid_version",
    ] {
        let fixture = Fixture::new();
        let program = [[37, -19], [21, 0], [-8, 93], [1, -2]].repeat(8);
        let donor = fixture.flac("donor.flac", &program, "Program");
        let recipient = fixture.flac("recipient.flac", &program, "Program (Live)");
        let mut conflicting = fixture.donor_receipt("recipient.flac", scenario == "basis");
        if scenario != "basis" {
            let id = "22222222-2222-4222-8222-222222222222";
            for core in ["accepted", "readback"] {
                conflicting[core]["title"] = json!("Program (Live)");
                conflicting[core]["musicbrainzTrackId"] = json!(id);
            }
            let row = &mut conflicting["evidence"]["trackMatch"];
            row["recordingId"] = json!(id);
            let candidate = &mut row["candidates"][0];
            candidate["recordingId"] = json!(id);
            candidate["canonicalRecordingId"] = json!(id);
            candidate["authoritativeTitle"] = json!(if scenario == "eligible" {
                "Program (Live)"
            } else {
                "Program"
            });
            candidate["disambiguation"] = json!("live, from World Tour");
            candidate["providerObservation"]["canonicalId"] = json!(id);
            candidate["providerObservation"]["responseId"] = json!(id);
            candidate["providerObservation"]["title"] = candidate["authoritativeTitle"].clone();
        }
        conflicting["evidenceDigest"] = json!(format!(
            "{:x}",
            Sha256::digest(conflicting["evidence"].to_string().as_bytes())
        ));
        fixture.publish_donors(vec![
            fixture.donor_receipt("donor.flac", false),
            conflicting,
        ]);
        let db = rusqlite::Connection::open(fixture.0.join("state/worker.db")).unwrap();
        let stored: String = db
            .query_row("SELECT evidence FROM worker_evidence", [], |r| r.get(0))
            .unwrap();
        let mut evidence: Value = serde_json::from_str(&stored).unwrap();
        let receipt = &mut evidence["coreReceipts"][1];
        match scenario {
            "stale" => receipt["postWrite"]["fingerprint"] = json!("f".repeat(64)),
            "unbound" => {
                receipt["evidence"]["trackMatch"]["lookupObservation"]["source"]["path"] =
                    json!(fixture.0.join("donor.flac"));
            }
            "lineage" => receipt["parentProof"] = json!("unproven-parent"),
            "invalid_version" => receipt["decisionRuleVersion"] = json!(2),
            _ => (),
        }
        receipt["evidenceDigest"] = json!(format!(
            "{:x}",
            Sha256::digest(receipt["evidence"].to_string().as_bytes())
        ));
        if scenario == "pending" {
            let receipt = evidence["coreReceipts"].as_array_mut().unwrap().remove(1);
            evidence["pendingCoreReceipts"] = json!([receipt]);
        }
        db.execute(
            "UPDATE worker_evidence SET evidence=?1",
            [evidence.to_string()],
        )
        .unwrap();
        drop(db);
        let output = fixture.reuse(json!([{"left":donor,"right":recipient}]), false);
        assert!(output.status.success());
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let conflict = matches!(scenario, "eligible" | "qualified" | "basis");
        assert_eq!(
            report["reuseProposals"]["proposalCount"],
            if conflict { 0 } else { 1 },
            "{scenario}: current independent contradictions must survive narrower donation rejection"
        );
        assert_eq!(
            report["reuseProposals"]["conflicts"]
                .as_array()
                .unwrap()
                .len(),
            usize::from(conflict),
            "{scenario}: invalid or non-independent receipts cannot supply conflict labels"
        );
        if scenario == "qualified" {
            assert_eq!(report["summary"]["verifiedCoreFiles"], 2);
            assert_eq!(
                report["reuseProposals"]["pairs"][0]["recipientRootError"],
                "root_recording_core_conflict_or_missing_facts",
                "a conflict label does not make this receipt eligible to donate"
            );
        }
        assert_eq!(fixture.source("donor.flac"), donor);
        assert_eq!(fixture.source("recipient.flac"), recipient);
    }
}

#[test]
#[ignore = "native offline contract: requires FFmpeg and ffprobe"]
fn selected_pair_export_preserves_nonzero_tail_and_default_inventory() {
    let fixture = Fixture::new();
    // One silent channel and opposite polarities must NOT become digital silence.
    let program = [[0, 19], [321, -321], [7, 33], [-89, 2]];
    let mut padded = vec![[0, 0]; 3];
    padded.extend(program);
    padded.push([1, 0]); // One native LSB is nonzero, however quiet.
    let left = fixture.wav("padded.wav", &padded);
    let right = fixture.wav("program.wav", &program);
    let before = fs::read(fixture.0.join("padded.wav")).unwrap();
    let mut default = fixture.run(None);
    let mut export = fixture.run(Some(json!([{"left":left,"right":right,"offsetFrames":3}])));
    let evidence = export["audioEvidence"].take();
    let pair = &evidence["pairs"][0];
    assert_eq!(pair["left"]["status"], "complete");
    assert_eq!(pair["left"]["pcm"]["frames"], 8);
    assert_eq!(pair["left"]["pcm"]["leadingZeroFrames"], 3);
    assert_eq!(pair["left"]["pcm"]["trailingZeroFrames"], 0);
    assert_eq!(pair["right"]["pcm"]["leadingZeroFrames"], 0);
    assert_eq!(pair["comparison"]["wholePcmEqual"], false);
    assert_eq!(pair["comparison"]["zeroEdgeExcludedEqual"], false);
    assert_eq!(pair["comparison"]["exactAlignedOverlapEqual"], true);
    assert_eq!(pair["comparison"]["coverage"]["left"]["matchedFrames"], 4);
    assert_eq!(pair["comparison"]["coverage"]["right"]["matchedFrames"], 4);
    assert_eq!(
        pair["comparison"]["coverage"]["left"]["wholeTimelineFraction"],
        0.5
    );
    assert_eq!(
        pair["comparison"]["coverage"]["right"]["wholeTimelineFraction"],
        1.0
    );
    assert_eq!(
        pair["comparison"]["coverage"]["left"]["zeroEdgeExcludedFraction"],
        0.8
    );
    assert_eq!(
        pair["comparison"]["coverage"]["right"]["zeroEdgeExcludedFraction"],
        1.0
    );
    assert_eq!(
        pair["comparison"]["coverage"]["left"]["unmatched"][1],
        json!({"startFrame":7,"endFrame":8,"nonzeroFrames":1})
    );
    export.as_object_mut().unwrap().remove("audioEvidence");
    strip_times(&mut default);
    strip_times(&mut export);
    assert_eq!(
        default, export,
        "opt-in diagnostics must not alter ranking or decisions"
    );
    assert_eq!(fs::read(fixture.0.join("padded.wav")).unwrap(), before);
}

#[test]
#[ignore = "native offline contract: requires FFmpeg and ffprobe"]
fn zero_edge_equality_is_diagnostic_and_stale_input_is_incomplete() {
    let fixture = Fixture::new();
    let program = [[31, -8], [0, 9], [-2, 71]];
    let mut padded = vec![[0, 0]; 2];
    padded.extend(program);
    padded.extend([[0, 0]; 4]);
    let left = fixture.wav("left.wav", &padded);
    let right = fixture.wav("right.wav", &program);
    let pair = fixture.run(Some(json!([{"left":left,"right":right}])));
    let comparison = &pair["audioEvidence"]["pairs"][0]["comparison"];
    assert_eq!(comparison["wholePcmEqual"], false);
    assert_eq!(comparison["zeroEdgeExcludedEqual"], true);
    assert_eq!(comparison["offsetFrames"], 2);
    assert_eq!(pair["summary"]["verifiedCoreFiles"], 0);
    // Same-length replacement after the selected source hash was captured.
    fixture.wav("left.wav", &[[1, 2]; 9]);
    let stale = fixture.run(Some(json!([{"left":left,"right":right}])));
    assert_eq!(
        stale["audioEvidence"]["pairs"][0]["left"]["status"],
        "incomplete"
    );
    assert!(stale["audioEvidence"]["pairs"][0]["left"]["error"]
        .as_str()
        .unwrap()
        .contains("source"));
    assert!(stale["audioEvidence"]["pairs"][0]["comparison"]["wholePcmEqual"].is_null());
}

#[test]
#[ignore = "native offline contract: requires FFmpeg and ffprobe"]
fn flac_checksum_and_sample_count_are_required_not_decoder_exit() {
    let fixture = Fixture::new();
    fixture.wav("input.wav", &[[17, -42]; 1000]);
    let flac = fixture.0.join("input.flac");
    assert!(Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(fixture.0.join("input.wav"))
        .arg(&flac)
        .status()
        .unwrap()
        .success());
    let source = |path: &Path| {
        json!({"path":path.file_name().unwrap().to_str().unwrap(),
        "sha256":format!("{:x}", Sha256::digest(fs::read(path).unwrap()))})
    };
    let good = source(&flac);
    let result = fixture.run(Some(json!([{"left":good,"right":good}])));
    assert_eq!(
        result["audioEvidence"]["pairs"][0]["left"]["integrity"]["basis"],
        "flac_streaminfo_md5_and_frames"
    );
    assert_eq!(
        result["audioEvidence"]["pairs"][0]["comparison"]["wholePcmEqual"],
        true
    );
    let mut bytes = fs::read(&flac).unwrap();
    bytes[26] ^= 1;
    fs::write(&flac, bytes).unwrap();
    let bad = source(&flac);
    let result = fixture.run(Some(json!([{"left":bad,"right":bad}])));
    assert_eq!(
        result["audioEvidence"]["pairs"][0]["left"]["status"],
        "incomplete"
    );
    assert!(result["audioEvidence"]["pairs"][0]["comparison"]["wholePcmEqual"].is_null());
    // A matching checksum alone is insufficient if the declared length lies.
    let mut bytes = fs::read(&flac).unwrap();
    bytes[26] ^= 1;
    bytes[25] ^= 1;
    fs::write(&flac, bytes).unwrap();
    let bad = source(&flac);
    let result = fixture.run(Some(json!([{"left":bad,"right":bad}])));
    assert_eq!(
        result["audioEvidence"]["pairs"][0]["left"]["status"],
        "incomplete"
    );
}

#[test]
#[ignore = "native offline contract: requires FFmpeg and ffprobe"]
fn native_float_and_24_bit_samples_are_not_quantized_to_silence() {
    let fixture = Fixture::new();
    for (name, code, bits, bytes) in [
        (
            "float.wav",
            3u16,
            32u16,
            [0.0f32, -0.0, 0.0, 1e-30, 0.0, 0.0]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        ),
        (
            "integer.wav",
            1,
            24,
            vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        ),
    ] {
        fixture.wav(name, &[[0, 0]; 1]);
        let mut header = fs::read(fixture.0.join(name)).unwrap();
        header.truncate(44);
        header[4..8].copy_from_slice(&(36 + bytes.len() as u32).to_le_bytes());
        header[20..22].copy_from_slice(&code.to_le_bytes());
        header[28..32].copy_from_slice(&(44100u32 * u32::from(bits) / 4).to_le_bytes());
        header[32..34].copy_from_slice(&(bits / 4).to_le_bytes());
        header[34..36].copy_from_slice(&bits.to_le_bytes());
        header[40..44].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        header.extend(bytes);
        fs::write(fixture.0.join(name), &header).unwrap();
        let input = json!({"path":name,"sha256":format!("{:x}",Sha256::digest(header))});
        let report = fixture.run(Some(json!([{"left":input,"right":input}])));
        let pcm = &report["audioEvidence"]["pairs"][0]["left"]["pcm"];
        assert_eq!(pcm["frames"], 3);
        assert_eq!(pcm["leadingZeroFrames"], 1);
        assert_eq!(pcm["trailingZeroFrames"], 1);
    }
}

#[test]
fn exact_coverage_rejects_a_different_ending_and_nonoverlapping_offset() {
    let fixture = Fixture::new();
    let left = fixture.wav("left.wav", &[[0, 7], [9, -9], [3, 1]]);
    let right = fixture.wav("right.wav", &[[0, 7], [9, -9], [4, 1]]);
    let mut command = fixture.controlled_command(
        json!({"left":left,"right":right,"offsetFrames":0}),
        "tail -c +45\n",
    );
    let output = command.output().unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let comparison = &report["audioEvidence"]["pairs"][0]["comparison"];
    assert_eq!(comparison["wholePcmEqual"], false);
    assert_eq!(comparison["exactAlignedOverlapEqual"], false);
    assert!(comparison["coverage"].is_null());
    // Also exercise extreme negative offsets without arithmetic overflow.
    let output = fixture
        .controlled_command(
            json!({"left":left,"right":left,"offsetFrames":i64::MIN}),
            "tail -c +45\n",
        )
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report["audioEvidence"]["pairs"][0]["comparison"]["coverage"].is_null());

    let silent = fixture.wav("silent.wav", &[[0, 0]; 5]);
    let output = fixture
        .controlled_command(json!({"left":silent,"right":silent}), "tail -c +45\n")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let pair = &report["audioEvidence"]["pairs"][0];
    assert_eq!(pair["left"]["pcm"]["leadingZeroFrames"], 5);
    assert_eq!(pair["left"]["pcm"]["trailingZeroFrames"], 5);
    assert!(pair["comparison"]["coverage"].is_null());
}

// These process-boundary tests need no installed media tools. The controlled
// decoder emits known native PCM; the real-tool contracts above test decoding.
impl Fixture {
    fn script(&self, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn controlled_command(&self, pair: Value, decoder: &str) -> Command {
        let probe = self.script("probe", "printf '%s' '{\"streams\":[{\"codec_name\":\"pcm_s16le\",\"sample_fmt\":\"s16\",\"sample_rate\":\"44100\",\"channels\":2}]}'\n");
        let decoder = self.script(
            "decode",
            &format!(
                "if [ \"$1\" = -version ]; then echo controlled-native-pcm; exit 0; fi\n{decoder}"
            ),
        );
        let manifest = self.0.join("pairs.json");
        fs::write(&manifest, json!({"pairs":[pair]}).to_string()).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_soundrobe-inventory"));
        command
            .arg(&self.0)
            .arg("--audio-pairs")
            .arg(manifest)
            .env("AUTO_TAG_FFPROBE_PATH", probe)
            .env("AUTO_TAG_FFMPEG_PATH", decoder);
        command
    }
}

#[test]
fn offline_export_rejects_replacement_during_decode_and_retains_warnings() {
    let fixture = Fixture::new();
    let source = fixture.wav("source.wav", &[[7, -3]; 4]);
    let body = format!(
        "tail -c +45\nprintf '%s' 'decoder warning retained' >&2\nprintf changed > '{}'\n",
        fixture.0.join("source.wav").display()
    );
    let output = fixture
        .controlled_command(json!({"left":source,"right":source}), &body)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let left = &report["audioEvidence"]["pairs"][0]["left"];
    assert_eq!(left["status"], "incomplete");
    assert_eq!(left["source"]["stable"], false);
    assert_eq!(left["decoder"]["warnings"], "decoder warning retained");
    assert!(report["audioEvidence"]["pairs"][0]["comparison"]["wholePcmEqual"].is_null());

    // Identical bytes at a replacement inode are not the held physical input.
    let source = fixture.wav("source.wav", &[[7, -3]; 4]);
    let path = fixture.0.join("source.wav");
    let body = format!(
        "tail -c +45\ncp -p '{0}' '{0}.replacement'\nmv '{0}.replacement' '{0}'\n",
        path.display()
    );
    let output = fixture
        .controlled_command(json!({"left":source,"right":source}), &body)
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let left = &report["audioEvidence"]["pairs"][0]["left"];
    assert_eq!(
        left["source"]["beforeSha256"],
        left["source"]["afterSha256"]
    );
    assert_eq!(left["status"], "incomplete");
    assert_eq!(left["source"]["stable"], false);
    assert!(report["audioEvidence"]["pairs"][0]["comparison"]["wholePcmEqual"].is_null());
}

#[test]
fn offline_export_keeps_warning_eof_distinct_from_checksum_proof() {
    let fixture = Fixture::new();
    let source = fixture.wav("source.wav", &[[0, 5], [2, -2], [0, 0]]);
    let output = fixture
        .controlled_command(
            json!({"left":source,"right":source}),
            "tail -c +45\nprintf warning >&2\n",
        )
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let pair = &report["audioEvidence"]["pairs"][0];
    assert_eq!(pair["left"]["status"], "complete");
    assert_eq!(pair["left"]["decoder"]["warnings"], "warning");
    assert_eq!(
        pair["left"]["integrity"]["basis"],
        "wav_riff_chunks_and_frames_not_source_master_completeness"
    );
    assert_eq!(pair["left"]["pcm"]["leadingZeroFrames"], 0);
    assert_eq!(pair["left"]["pcm"]["trailingZeroFrames"], 1);
    assert_eq!(pair["comparison"]["wholePcmEqual"], true);
    assert_eq!(pair["comparison"]["identityVerified"], false);
}

#[test]
fn offline_export_cancels_and_reaps_active_decoder() {
    let fixture = Fixture::new();
    let source = fixture.wav("source.wav", &[[7, -3]; 4]);
    let pidfile = fixture.0.join("decoder-pid");
    let body = format!(
        "printf warning-before-cancel >&2\necho $$ > '{}'\nwhile :; do :; done\n",
        pidfile.display()
    );
    let mut child = fixture
        .controlled_command(json!({"left":source,"right":source}), &body)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while !pidfile.exists() {
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let pid: i32 = fs::read_to_string(pidfile).unwrap().trim().parse().unwrap();
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    // Bound the test too: cancellation may not strand the CLI or its child.
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > std::time::Duration::from_secs(10) {
            child.kill().unwrap();
            panic!("cancelled CLI did not terminate");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["audioEvidence"]["cancelled"], true);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "decoder must be reaped");
    let left = &report["audioEvidence"]["pairs"][0]["left"];
    assert!(left["decoder"]["warnings"]
        .as_str()
        .unwrap()
        .contains("warning-before-cancel"));
    assert!(
        left["source"]["afterSha256"].is_string()
            || left["source"]["finalizationError"].is_string()
    );
}

#[test]
fn offline_export_cancellation_terminates_decoder_group_workers() {
    let fixture = Fixture::new();
    let source = fixture.wav("source.wav", &[[31, -17]; 4]);
    let pidfile = fixture.0.join("worker-pid");
    let body = format!("sleep 60 &\necho $! > '{}'\nwait\n", pidfile.display());
    let mut child = fixture
        .controlled_command(json!({"left":source,"right":source}), &body)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while !pidfile.exists() {
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let pid: i32 = fs::read_to_string(pidfile).unwrap().trim().parse().unwrap();
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    while child.try_wait().unwrap().is_none() {
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["audioEvidence"]["cancelled"], true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while unsafe { libc::kill(pid, 0) } == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    if alive {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(!alive, "decoder worker must not survive cancellation");
}

#[test]
#[ignore = "native offline contract: requires FFmpeg and ffprobe"]
fn damaged_mp3_decode_errors_are_not_complete_pcm() {
    let fixture = Fixture::new();
    let path = fixture.0.join("source.mp3");
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=431:sample_rate=44100:duration=2",
            "-ac",
            "2",
            "-codec:a",
            "libmp3lame",
            "-b:a",
            "128k"
        ])
        .arg(&path)
        .status()
        .unwrap()
        .success());
    let source =
        |bytes: &[u8]| json!({"path":"source.mp3","sha256":format!("{:x}",Sha256::digest(bytes))});
    let mut bytes = fs::read(&path).unwrap();
    let good = source(&bytes);
    let control = fixture.run(Some(json!([{"left":good,"right":good}])));
    assert_eq!(
        control["audioEvidence"]["pairs"][0]["left"]["status"],
        "complete"
    );
    let middle = bytes.len() / 2;
    bytes[middle..middle + 1000].fill(0);
    fs::write(&path, &bytes).unwrap();
    let bad = source(&bytes);
    let report = fixture.run(Some(json!([{"left":bad,"right":bad}])));
    let pair = &report["audioEvidence"]["pairs"][0];
    assert_eq!(pair["left"]["status"], "incomplete");
    assert!(!pair["left"]["decoder"]["warnings"]
        .as_str()
        .unwrap()
        .is_empty());
    assert_eq!(pair["left"]["source"]["afterSha256"], bad["sha256"]);
    assert!(pair["comparison"]["wholePcmEqual"].is_null());
}

#[test]
#[ignore = "native offline contract: requires FFmpeg and ffprobe"]
fn review_regression_wav_truncation_is_not_complete_pcm() {
    let fixture = Fixture::new();
    let good = fixture.wav("source.wav", &[[31, -17]; 100]);
    let control = fixture.run(Some(json!([{"left":good,"right":good}])));
    assert_eq!(
        control["audioEvidence"]["pairs"][0]["left"]["pcm"]["frames"],
        100
    );
    let path = fixture.0.join("source.wav");
    let mut bytes = fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 200); // RIFF/data headers still declare 100 stereo frames.
    fs::write(&path, &bytes).unwrap();
    let bad = json!({"path":"source.wav","sha256":format!("{:x}",Sha256::digest(&bytes))});
    let report = fixture.run(Some(json!([{"left":bad,"right":bad}])));
    let pair = &report["audioEvidence"]["pairs"][0];
    assert_eq!(pair["left"]["status"], "incomplete");
    assert!(pair["comparison"]["wholePcmEqual"].is_null());
    assert_eq!(pair["left"]["source"]["afterSha256"], bad["sha256"]);
}

#[test]
fn review_regression_ancestor_replacement_cannot_escape_the_inventory() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("inside")).unwrap();
    let left = fixture.wav("left.wav", &[[31, -17]; 4]);
    let right = fixture.wav("inside/right.wav", &[[31, -17]; 4]);
    let external = Fixture::new();
    external.wav("right.wav", &[[31, -17]; 4]);
    let body = format!("if [ ! -L '{0}/inside' ]; then mv '{0}/inside' '{0}/retired'; ln -s '{1}' '{0}/inside'; fi\ntail -c +45\n",fixture.0.display(),external.0.display());
    let output = fixture
        .controlled_command(json!({"left":left,"right":right}), &body)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let pair = &report["audioEvidence"]["pairs"][0];
    assert_eq!(pair["right"]["status"], "incomplete");
    assert_eq!(pair["right"]["source"]["stable"], false);
    assert!(pair["comparison"]["wholePcmEqual"].is_null());
}

#[test]
fn review_regression_fifo_manifest_is_rejected_without_a_writer() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    fixture.wav("source.wav", &[[31, -17]; 4]);
    let fifo = fixture.0.join("pairs.fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_soundrobe-inventory"))
        .arg(&fixture.0)
        .arg("--audio-pairs")
        .arg(fifo)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > std::time::Duration::from_secs(3) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("FIFO manifest blocked instead of being rejected");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("regular file"));
}

#[test]
fn review_regression_failed_process_keeps_diagnostics_and_source_finalization() {
    let fixture = Fixture::new();
    let source = fixture.wav("source.wav", &[[31, -17]; 4]);
    for body in [
        "printf warning-before-failure >&2\nexit 7\n",
        "printf warning-before-failure >&2\nexec head -c 70000 /dev/zero >&2\n",
    ] {
        let output = fixture
            .controlled_command(json!({"left":source,"right":source}), body)
            .output()
            .unwrap();
        assert!(output.status.success());
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let pair = &report["audioEvidence"]["pairs"][0];
        assert_eq!(pair["left"]["status"], "incomplete");
        assert!(pair["left"]["decoder"]["warnings"]
            .as_str()
            .is_some_and(|s| s.starts_with("warning-before-failure")));
        assert_eq!(pair["left"]["source"]["afterSha256"], source["sha256"]);
        assert!(pair["comparison"]["wholePcmEqual"].is_null());
    }
}
