use super::*;
use sha2::{Digest, Sha256};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("recovery-preview-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("Album")).unwrap();
        Self(root)
    }
    fn write(&self, name: &str, value: &Value) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        path
    }
    fn media(&self, name: &str) -> Value {
        let path = self.0.join(name);
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.flac"),
            &path,
        )
        .unwrap();
        json!({"path":name,"sha256":format!("{:x}",Sha256::digest(fs::read(path).unwrap()))})
    }
    fn manifest(&self, files: Value) -> PathBuf {
        self.write(
            "manifest.json",
            &json!({"schemaVersion":1,"files":files,"observationFiles":["observation.json"]}),
        )
    }
    fn observation(&self) {
        self.write("observation.json", &json!({"schemaVersion":1,"id":"retained-release","provider":"musicbrainz",
            "releaseId":"release-a","completeness":"partial","occurrences":[
                {"mediumIndex":0,"rowIndex":0,"title":"Different title (Single Version)","artists":["Guest","Lead"],"trackNumber":7,"discNumber":2,"recordingId":"recording-a","duration":193.213},
                {"mediumIndex":1,"rowIndex":0,"title":"Repeated recording","artists":["Lead"],"trackNumber":1,"discNumber":3,"recordingId":"recording-a"}]}));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn recovery_preview_retains_files_alternatives_collisions_and_review_without_writes() {
    let f = Fixture::new();
    let mut a = f.media("Album/first.flac");
    let mut b = f.media("Album/second.flac");
    let c = f.media("Album/third.flac");
    f.observation();
    for file in [&mut a, &mut b] {
        file["claims"] = json!([{ "observationId":"retained-release","mediumIndex":0,"rowIndex":0,"reason":"retained single-file comparison"}]);
        file["leads"] = json!([{ "title":"Filename lead (Single Version)","reason":"explicit lead, not identity proof"}]);
    }
    let before = fs::read(f.0.join("Album/first.flac")).unwrap();
    let original = inventory(&f.0, None, None).unwrap();
    let manifest = f.manifest(json!([a, b, c]));
    let preview = recovery_preview(&original, &manifest).unwrap();
    assert_eq!(preview["files"].as_array().unwrap().len(), 3);
    assert_eq!(preview["collisions"].as_array().unwrap().len(), 1);
    assert_eq!(
        preview["collisions"][0]["files"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        preview["collisions"][0]["assessmentState"],
        "historical_unverified"
    );
    assert_eq!(preview["observations"][0]["completeness"], "partial");
    assert!(preview["observations"][0]["observedAt"].is_null());
    assert_eq!(
        preview["observations"][0]["provenance"],
        "imported_unverified"
    );
    for row in preview["files"].as_array().unwrap() {
        assert_eq!(row["status"], "needs_review");
        assert_eq!(row["sourceState"], "stable");
        assert!(!row["embedded"].is_null());
    }
    let row = &preview["files"][0];
    assert!(row["issues"]
        .as_array()
        .unwrap()
        .contains(&json!("title_difference")));
    assert_eq!(
        row["hypotheses"][0]["occurrence"]["title"],
        "Different title (Single Version)"
    );
    assert_eq!(row["leads"][0]["title"], "Filename lead (Single Version)");
    assert_eq!(fs::read(f.0.join("Album/first.flac")).unwrap(), before);
    assert!(preview.get("albumCandidate").is_none());
    assert!(
        serde_json::from_value::<crate::commands::album_search::ApplyCandidateRequest>(
            preview.clone()
        )
        .is_err()
    );
    assert_eq!(original["summary"]["verifiedCoreFiles"], 0);
    let markdown = recovery_preview_markdown(&preview);
    assert!(markdown.contains("Different title (Single Version)"));
    assert!(markdown.contains("needs_review"));
}

#[test]
fn recovery_preview_marks_changed_missing_unreadable_and_unexpected_sources() {
    let f = Fixture::new();
    let a = f.media("Album/changed.flac");
    let b = f.media("Album/missing.flac");
    f.media("Album/unexpected.flac");
    fs::write(f.0.join("Album/broken.flac"), b"unreadable audio").unwrap();
    let broken = json!({"path":"Album/broken.flac","sha256":format!("{:x}",Sha256::digest(b"unreadable audio"))});
    f.observation();
    let manifest = f.manifest(json!([a, b, broken]));
    let captured = inventory(&f.0, None, None).unwrap();
    fs::write(f.0.join("Album/changed.flac"), b"replacement").unwrap();
    fs::remove_file(f.0.join("Album/missing.flac")).unwrap();
    let preview = recovery_preview(&captured, &manifest).unwrap();
    let rows = preview["files"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let find = |path: &str| rows.iter().find(|r| r["relativePath"] == path).unwrap();
    assert_eq!(find("Album/changed.flac")["sourceState"], "changed");
    assert_eq!(find("Album/missing.flac")["sourceState"], "missing");
    assert_eq!(find("Album/broken.flac")["sourceState"], "unreadable");
    assert!(find("Album/unexpected.flac")["issues"]
        .as_array()
        .unwrap()
        .contains(&json!("unexpected_member")));
    assert!(rows.iter().all(|r| r["status"] == "needs_review"));
}

#[test]
fn recovery_preview_rejects_unsafe_and_invalid_references_without_inventing_catalog_facts() {
    let f = Fixture::new();
    let mut file = f.media("Album/a.flac");
    f.observation();
    file["claims"] = json!([{ "observationId":"retained-release","mediumIndex":4,"rowIndex":0,"reason":"invalid reference"}]);
    assert!(recovery_preview(
        &inventory(&f.0, None, None).unwrap(),
        &f.manifest(json!([file]))
    )
    .unwrap_err()
    .contains("occurrence"));
    let bad = json!({"path":"../escape.flac","sha256":"a".repeat(64)});
    assert!(recovery_preview(
        &inventory(&f.0, None, None).unwrap(),
        &f.manifest(json!([bad]))
    )
    .is_err());
    let mut observation: Value =
        serde_json::from_slice(&fs::read(f.0.join("observation.json")).unwrap()).unwrap();
    observation["occurrences"][1]["mediumIndex"] = json!(0);
    f.write("observation.json", &observation);
    let file = f.media("Album/b.flac");
    assert!(recovery_preview(
        &inventory(&f.0, None, None).unwrap(),
        &f.manifest(json!([file]))
    )
    .unwrap_err()
    .contains("duplicate occurrence"));
}

#[test]
fn recovery_preview_preserves_embedded_positions_versions_ids_and_ordered_credits() {
    use lofty::{
        config::WriteOptions,
        file::TaggedFileExt,
        prelude::Accessor,
        tag::{ItemKey, TagExt},
    };
    let f = Fixture::new();
    f.media("Album/Beyoncé-Blow.flac");
    let path = f.0.join("Album/Beyoncé-Blow.flac");
    let mut media = lofty::read_from_path(&path).unwrap();
    let tag = media.primary_tag_mut().unwrap();
    tag.set_title("Pretty Hurts (Studio Version)".into());
    tag.set_track(1);
    tag.set_disk(1);
    tag.insert_text(ItemKey::TrackArtist, "Lead;Guest".into());
    tag.insert_text(ItemKey::MusicBrainzRecordingId, "local-recording".into());
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    let source = json!({"path":"Album/Beyoncé-Blow.flac","sha256":format!("{:x}",Sha256::digest(fs::read(&path).unwrap())),
        "claims":[{"observationId":"retained-release","mediumIndex":0,"rowIndex":0,"reason":"retained alternative"}]});
    f.observation();
    let before = crate::commands::tracks::read_track_metadata_without_lyrics(&path).unwrap();
    let manifest = f.manifest(json!([source]));
    let preview = recovery_preview(&inventory(&f.0, None, None).unwrap(), &manifest).unwrap();
    let row = &preview["files"][0];
    assert_eq!(row["embedded"]["title"], "Pretty Hurts (Studio Version)");
    assert_eq!(row["embedded"]["trackNumber"], 1);
    assert_eq!(row["embedded"]["discNumber"], 1);
    assert_eq!(row["embedded"]["artists"], json!(before.artists));
    assert_eq!(row["embedded"]["musicbrainzTrackId"], "local-recording");
    for issue in [
        "title_difference",
        "embedded_position_conflict",
        "recording_id_conflict",
    ] {
        assert!(
            row["issues"].as_array().unwrap().contains(&json!(issue)),
            "missing {issue}"
        );
    }
    assert_eq!(row["status"], "needs_review");
    assert_eq!(
        crate::commands::tracks::read_track_metadata_without_lyrics(&path).unwrap(),
        before
    );
}

#[test]
fn recovery_preview_limits_comparisons_without_hiding_files_or_repeated_occurrences() {
    let f = Fixture::new();
    let mut a = f.media("Album/a.flac");
    let mut b = f.media("Album/b.flac");
    f.observation();
    a["claims"] = json!([{ "observationId":"retained-release","mediumIndex":0,"rowIndex":0,"reason":"first occurrence"}]);
    b["claims"] = json!([{ "observationId":"retained-release","mediumIndex":1,"rowIndex":0,"reason":"same recording at another occurrence"}]);
    let manifest = f.manifest(json!([a, b]));
    let preview = recovery_preview(&inventory(&f.0, None, None).unwrap(), &manifest).unwrap();
    assert!(preview["collisions"].as_array().unwrap().is_empty());
    for n in 0..63 {
        f.media(&format!("Album/extra-{n}.flac"));
    }
    let preview = recovery_preview(&inventory(&f.0, None, None).unwrap(), &manifest).unwrap();
    assert_eq!(preview["files"].as_array().unwrap().len(), 65);
    assert_eq!(preview["summary"]["previewLimited"], true);
    assert!(preview["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["sourceState"] == "unexamined" && r["status"] == "needs_review"));
}

#[cfg(unix)]
#[test]
fn recovery_preview_refuses_symlink_sources_and_nonregular_manifests() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut file = f.media("Album/a.flac");
    symlink(f.0.join("Album/a.flac"), f.0.join("Album/alias.flac")).unwrap();
    file["path"] = json!("Album/alias.flac");
    f.observation();
    let manifest = f.manifest(json!([file]));
    let inventory = inventory(&f.0, None, None).unwrap();
    let preview = recovery_preview(&inventory, &manifest).unwrap();
    let alias = preview["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["relativePath"] == "Album/alias.flac")
        .unwrap();
    assert_eq!(alias["sourceState"], "unsafe");
    assert!(alias["embedded"].is_null());
    let link = f.0.join("manifest-link.json");
    symlink(&manifest, &link).unwrap();
    assert!(recovery_preview(&inventory, &link).is_err());
    assert!(recovery_preview(&inventory, &f.0).is_err());
}
