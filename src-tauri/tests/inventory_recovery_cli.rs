//! Recovery is an offline, read-only CLI export, not an approval or repair.
#![cfg(feature = "server")]

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::SystemTime,
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("inventory-recovery-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("artist/Album")).unwrap();
        fs::create_dir(root.join("home")).unwrap();
        let fixture = Self(root);
        // Two physical paths with identical valid PCM WAV bytes must stay two rows.
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(40u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(44100u32.to_le_bytes());
        bytes.extend(176400u32.to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(4u32.to_le_bytes());
        bytes.extend([31i16, -17i16].into_iter().flat_map(i16::to_le_bytes));
        let files: Vec<_> = ["Album/a.wav", "Album/b.wav"]
            .into_iter()
            .map(|path| {
                fs::write(fixture.0.join("artist").join(path), &bytes).unwrap();
                json!({"path":path,"sha256":format!("{:x}", Sha256::digest(&bytes))})
            })
            .collect();
        fs::write(
            fixture.0.join("manifest.json"),
            json!({"schemaVersion":1,"files":files,"observationFiles":[]}).to_string(),
        )
        .unwrap();
        fixture
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_soundrobe-inventory"));
        command
            .arg(self.0.join("artist"))
            .arg("--worker-data")
            .arg(self.0.join("worker"))
            .env("HOME", self.0.join("home"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("APPDATA", self.0.join("appdata"))
            .env("LOCALAPPDATA", self.0.join("localappdata"));
        command
    }

    fn recovery_command(&self) -> Command {
        let mut command = self.command();
        command
            .arg("--recovery-preview")
            .arg(self.0.join("manifest.json"));
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn successful(output: &Output) {
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// Include directories as well as bytes and mtimes: even an empty new cache or
// worker directory is an observable side effect of a supposedly read-only mode.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)> {
    fn collect(
        root: &Path,
        path: &Path,
        result: &mut BTreeMap<PathBuf, (Option<Vec<u8>>, SystemTime)>,
    ) {
        let metadata = fs::metadata(path).unwrap();
        result.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            (
                metadata.is_file().then(|| fs::read(path).unwrap()),
                metadata.modified().unwrap(),
            ),
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                collect(root, &entry.unwrap().path(), result);
            }
        }
    }
    let mut result = BTreeMap::new();
    collect(root, root, &mut result);
    result
}

#[test]
fn recovery_exports_each_physical_row_without_creating_cache_or_changing_sources() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.0);
    let baseline = fixture.command().output().unwrap();
    successful(&baseline);
    let output = fixture.recovery_command().output().unwrap();
    successful(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let files = report["recoveryPreview"]["files"].as_array().unwrap();
    let mut rows: Vec<_> = files
        .iter()
        .map(|file| {
            assert_eq!(file["status"], "needs_review");
            file["relativePath"].as_str().unwrap()
        })
        .collect();
    rows.sort_unstable();
    assert_eq!(rows, ["Album/a.wav", "Album/b.wav"]);
    for mode in ["audioEvidence", "reuseProposals", "recordingDiscovery"] {
        assert!(report.get(mode).is_none(), "unexpected mode: {mode}");
    }
    let baseline: Value = serde_json::from_slice(&baseline.stdout).unwrap();
    assert_eq!(report["summary"], baseline["summary"]);
    assert_eq!(snapshot(&fixture.0), before);
}

#[test]
fn recovery_markdown_appends_the_recovery_ledger() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.0);
    let output = fixture
        .recovery_command()
        .args(["--format", "markdown"])
        .output()
        .unwrap();
    successful(&output);
    let markdown = String::from_utf8(output.stdout).unwrap();
    let heading = markdown
        .lines()
        .find(|line| line.starts_with('#') && line.to_lowercase().contains("recovery"))
        .expect("a separate recovery ledger heading");
    let ledger = &markdown[markdown.find(heading).unwrap()..];
    for path in ["Album/a.wav", "Album/b.wav"] {
        assert!(ledger.contains(path), "missing recovery row: {path}");
    }
    assert!(ledger.contains("needs_review"));
    assert_eq!(snapshot(&fixture.0), before);
}

#[test]
fn recovery_accepts_a_manifest_in_the_current_directory() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .current_dir(&fixture.0)
        .args(["--recovery-preview", "manifest.json"])
        .output()
        .unwrap();
    successful(&output);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["recoveryPreview"]["files"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn recovery_rejects_other_evidence_modes_before_any_side_effects() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.0);
    for mode in [
        "--audio-pairs",
        "--reuse-proposals",
        "--recording-discovery",
    ] {
        for recovery_first in [true, false] {
            let mut command = fixture.command();
            let flags = if recovery_first {
                ["--recovery-preview", mode]
            } else {
                [mode, "--recovery-preview"]
            };
            for flag in flags {
                command.arg(flag).arg(fixture.0.join("manifest.json"));
            }
            let output = command.output().unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("not multiple modes"),
                "expected explicit mode rejection: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(snapshot(&fixture.0), before);
        }
    }
}

#[test]
fn recovery_rejects_network_and_discovery_cache_options() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.0);
    for flags in [
        vec!["--allow-network"],
        vec!["--discovery-cache", "isolated-cache"],
        vec!["--discovery-cache", "isolated-cache", "--allow-network"],
    ] {
        let output = fixture
            .recovery_command()
            .current_dir(&fixture.0)
            .args(flags)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("discovery-only"),
            "expected explicit offline-mode rejection: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(snapshot(&fixture.0), before);
    }
}
