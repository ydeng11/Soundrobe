//! Offline evidence ledger. Imported occurrence claims never become accepted mappings.

use super::{is_audio_file, RecordingSource};
use crate::commands::tracks::read_track_metadata_without_lyrics;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const SCOPE: &str = "Preview hypotheses only. Every file remains in review. No recognition, approval, accepted mapping, write permission, or folder completion. Imported observations and historical comparisons are unverified. Source stability is observed, not an atomic filesystem snapshot.";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    files: Vec<FileRequest>,
    #[serde(default)]
    observation_files: Vec<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRequest {
    path: PathBuf,
    sha256: String,
    #[serde(default)]
    leads: Vec<Lead>,
    #[serde(default)]
    claims: Vec<Claim>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Lead {
    title: String,
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Claim {
    observation_id: String,
    medium_index: u32,
    row_index: u32,
    reason: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Observation {
    schema_version: u32,
    id: String,
    provider: Provider,
    release_id: String,
    #[serde(default)]
    observed_at: Option<i64>,
    #[serde(default)]
    endpoint: Option<String>,
    completeness: Completeness,
    #[serde(default)]
    omitted_occurrences: Option<u32>,
    occurrences: Vec<Occurrence>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Provider {
    Musicbrainz,
    Discogs,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Completeness {
    Complete,
    Partial,
    Unknown,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Occurrence {
    medium_index: u32,
    row_index: u32,
    title: String,
    #[serde(default)]
    artists: Vec<String>,
    #[serde(default)]
    disc_number: Option<u32>,
    #[serde(default)]
    track_number: Option<u32>,
    #[serde(default)]
    recording_id: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

fn read_document<T: DeserializeOwned>(path: &Path, limit: u64) -> Result<(T, String), String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let input = options.open(path).map_err(|e| e.to_string())?;
    let metadata = input.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!(
            "document must be a regular file at most {limit} bytes"
        ));
    }
    let mut bytes = Vec::new();
    input
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("document exceeds size limit".into());
    }
    let parsed = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok((parsed, format!("{:x}", Sha256::digest(&bytes))))
}

fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|p| matches!(p, Component::Normal(_)))
}

fn safe_named_path(root: &Path, relative: &Path) -> bool {
    let path = root.join(relative);
    path.canonicalize().is_ok_and(|resolved| resolved == path)
        && fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn membership(root: &Path, folders: &BTreeSet<PathBuf>) -> (BTreeSet<PathBuf>, Vec<String>) {
    let mut paths = BTreeSet::new();
    let mut errors = Vec::new();
    for folder in folders {
        let absolute = root.join(folder);
        if absolute.canonicalize().ok().as_deref() != Some(absolute.as_path()) {
            errors.push(format!("membership unavailable: {}", folder.display()));
            continue;
        }
        match fs::read_dir(&absolute) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) if is_audio_file(&entry.path()) => {
                            paths.insert(folder.join(entry.file_name()));
                        }
                        Ok(_) => (),
                        Err(error) => errors.push(error.to_string()),
                    }
                }
            }
            Err(error) => errors.push(error.to_string()),
        }
    }
    (paths, errors)
}

fn observe(root: &Path, path: &Path, expected: Option<&str>, inventory_source: &Value) -> Value {
    let absolute = root.join(path);
    let mut row = json!({"relativePath":path,"status":"needs_review","observedAt":time::OffsetDateTime::now_utc().unix_timestamp(),
        "sourceState":"missing","source":null,"embedded":null,"hypotheses":[],"leads":[],"issues":["identity_unverified"]});
    if fs::symlink_metadata(&absolute).is_err() {
        return row;
    }
    if !safe_named_path(root, path) {
        row["sourceState"] = json!("unsafe");
        return row;
    }
    let before = match RecordingSource::capture(&absolute) {
        Ok(source) => source,
        Err(error) => {
            row["sourceState"] = json!("unreadable");
            row["sourceError"] = json!(error.to_string());
            return row;
        }
    };
    let source = serde_json::to_value(&before).expect("source serialization");
    row["source"] = source.clone();
    if expected.is_some_and(|hash| source["fingerprint"].as_str() != Some(hash))
        || (!inventory_source.is_null() && *inventory_source != source)
    {
        row["sourceState"] = json!("changed");
        return row;
    }
    match read_track_metadata_without_lyrics(&absolute) {
        Ok(track) if track.codec != "unknown" => {
            row["embedded"] = json!({"title":track.title,"artist":track.artist,"artists":track.artists,
                "album":track.album,"trackNumber":track.track_number,"trackTotal":track.track_total,
                "discNumber":track.disc_number,"discTotal":track.disc_total,"duration":track.duration,"codec":track.codec,
                "musicbrainzTrackId":track.musicbrainz_track_id,"musicbrainzAlbumId":track.musicbrainz_album_id,
                "musicbrainzArtistId":track.musicbrainz_artist_id,"discogsArtistId":track.discogs_artist_id,
                "discogsReleaseId":track.discogs_release_id});
            row["sourceState"] = json!("stable");
        }
        Ok(_) => {
            row["sourceState"] = json!("unreadable");
            row["sourceError"] = json!("native reader returned an unreadable-track fallback");
        }
        Err(error) => {
            row["sourceState"] = json!("unreadable");
            row["sourceError"] = json!(error.to_string());
        }
    }
    if !safe_named_path(root, path)
        || RecordingSource::capture(&absolute).ok().as_ref() != Some(&before)
    {
        row["sourceState"] = json!("changed");
        row["embedded"] = Value::Null;
    }
    row
}

/// Assemble imported claims without invoking recognition, selection, or a writer.
pub fn export(inventory: &Value, manifest_path: &Path) -> Result<Value, String> {
    let (manifest, manifest_digest): (Manifest, _) = read_document(manifest_path, 64 * 1024)?;
    if manifest.schema_version != 1
        || manifest.files.is_empty()
        || manifest.observation_files.len() > 8
    {
        return Err("recovery requires schemaVersion 1, physical files, and at most eight observation files".into());
    }
    let root = Path::new(inventory["root"].as_str().ok_or("inventory root missing")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let base = manifest_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut observations = BTreeMap::new();
    let mut serialized_observations = Vec::new();
    for name in manifest.observation_files {
        if !relative(&name) || !safe_named_path(&base, &name) {
            return Err("unsafe observation path".into());
        }
        let (observation, digest): (Observation, _) =
            read_document(&base.join(&name), 4 * 1024 * 1024)?;
        if observation.schema_version != 1
            || !nonempty(&observation.id)
            || !nonempty(&observation.release_id)
            || observation.occurrences.len() > 256
        {
            return Err("invalid or oversized release observation".into());
        }
        let mut keys = BTreeSet::new();
        for occurrence in &observation.occurrences {
            if !keys.insert((occurrence.medium_index, occurrence.row_index)) {
                return Err("duplicate occurrence in observation".into());
            }
            if !nonempty(&occurrence.title)
                || occurrence
                    .duration
                    .is_some_and(|d| !d.is_finite() || d <= 0.)
            {
                return Err("invalid occurrence facts".into());
            }
        }
        let mut serialized = serde_json::to_value(&observation).map_err(|e| e.to_string())?;
        serialized["payloadDigest"] = json!(digest);
        serialized["provenance"] = json!("imported_unverified");
        serialized_observations.push(serialized);
        if observations
            .insert(observation.id.clone(), (observation, digest))
            .is_some()
        {
            return Err("duplicate observation ID".into());
        }
    }
    let mut requests = BTreeMap::new();
    let mut folders = BTreeSet::new();
    for mut file in manifest.files {
        if !relative(&file.path)
            || !is_audio_file(&file.path)
            || file.sha256.len() != 64
            || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid physical source path or SHA-256".into());
        }
        file.sha256.make_ascii_lowercase();
        for lead in &file.leads {
            if !nonempty(&lead.title) || !nonempty(&lead.reason) {
                return Err("lead requires title and reason".into());
            }
        }
        let mut seen = BTreeSet::new();
        for claim in &file.claims {
            let observation = &observations
                .get(&claim.observation_id)
                .ok_or("unknown observation reference")?
                .0;
            if !nonempty(&claim.reason)
                || !observation
                    .occurrences
                    .iter()
                    .any(|o| (o.medium_index, o.row_index) == (claim.medium_index, claim.row_index))
            {
                return Err("invalid occurrence reference or missing reason".into());
            }
            if !seen.insert((&claim.observation_id, claim.medium_index, claim.row_index)) {
                return Err("duplicate occurrence claim".into());
            }
        }
        folders.insert(file.path.parent().unwrap_or(Path::new("")).to_owned());
        if requests.insert(file.path.clone(), file).is_some() {
            return Err("duplicate physical source".into());
        }
    }
    let (initial_members, mut membership_errors) = membership(&root, &folders);
    let mut paths: BTreeSet<_> = requests
        .keys()
        .cloned()
        .chain(initial_members.iter().cloned())
        .collect();
    let prior: BTreeMap<PathBuf, &Value> = inventory["files"]
        .as_array()
        .ok_or("inventory files missing")?
        .iter()
        .filter_map(|f| f["relativePath"].as_str().map(|p| (PathBuf::from(p), f)))
        .collect();
    paths.extend(
        prior
            .keys()
            .filter(|p| folders.contains(p.parent().unwrap_or(Path::new(""))))
            .cloned(),
    );
    let limited = paths.len() > 64;
    let mut rows = Vec::new();
    let mut claims: BTreeMap<(String, u32, u32), Vec<String>> = BTreeMap::new();
    for path in paths {
        let request = requests.get(&path);
        let mut row = if limited {
            json!({"relativePath":path,"status":"needs_review","sourceState":"unexamined","embedded":null,
                "source":null,"hypotheses":[],"leads":[],"issues":["identity_unverified","preview_limit"]})
        } else {
            observe(
                &root,
                &path,
                request.map(|r| r.sha256.as_str()),
                prior
                    .get(&path)
                    .map_or(&Value::Null, |v| &v["sourceIdentity"]),
            )
        };
        if let Some(request) = request {
            row["expectedSha256"] = json!(request.sha256);
            row["leads"] = serde_json::to_value(&request.leads).map_err(|e| e.to_string())?;
            for claim in &request.claims {
                let (observation, digest) = &observations[&claim.observation_id];
                let occurrence = observation
                    .occurrences
                    .iter()
                    .find(|o| {
                        (o.medium_index, o.row_index) == (claim.medium_index, claim.row_index)
                    })
                    .expect("validated occurrence");
                let embedded = &row["embedded"];
                let mut issues = Vec::new();
                if let Some(title) = embedded["title"].as_str().filter(|s| !s.trim().is_empty()) {
                    if title.trim().to_lowercase() != occurrence.title.trim().to_lowercase() {
                        issues.push("title_difference");
                    }
                }
                for (field, value) in [
                    ("trackNumber", occurrence.track_number),
                    ("discNumber", occurrence.disc_number),
                ] {
                    if value.is_some()
                        && embedded[field]
                            .as_u64()
                            .is_some_and(|n| Some(n) != value.map(u64::from))
                    {
                        issues.push("embedded_position_conflict");
                    }
                }
                if occurrence.recording_id.as_deref().is_some_and(|id| {
                    embedded["musicbrainzTrackId"]
                        .as_str()
                        .is_some_and(|local| local != id)
                }) {
                    issues.push("recording_id_conflict");
                }
                let release_field = match observation.provider {
                    Provider::Musicbrainz => "musicbrainzAlbumId",
                    Provider::Discogs => "discogsReleaseId",
                };
                if embedded[release_field]
                    .as_str()
                    .is_some_and(|id| id != observation.release_id)
                {
                    issues.push("release_id_conflict");
                }
                let duration_delta = embedded["duration"]
                    .as_f64()
                    .zip(occurrence.duration)
                    .map(|(a, b)| a - b);
                let hypothesis = json!({"observationId":claim.observation_id,"payloadDigest":digest,"provider":observation.provider,
                    "releaseId":observation.release_id,"occurrence":occurrence,"reason":claim.reason,
                    "assessmentState":"historical_unverified","durationDeltaSeconds":duration_delta,"issues":issues});
                row["hypotheses"].as_array_mut().unwrap().push(hypothesis);
                for issue in issues {
                    row["issues"].as_array_mut().unwrap().push(json!(issue));
                }
                claims
                    .entry((
                        claim.observation_id.clone(),
                        claim.medium_index,
                        claim.row_index,
                    ))
                    .or_default()
                    .push(path.to_string_lossy().into_owned());
            }
        } else {
            row["issues"]
                .as_array_mut()
                .unwrap()
                .push(json!("unexpected_member"));
        }
        rows.push(row);
    }
    // A complete batch observation must not conceal a replacement during another row's read.
    for row in &mut rows {
        if row["sourceState"] != "stable" {
            continue;
        }
        let path = Path::new(row["relativePath"].as_str().unwrap());
        let current = if safe_named_path(&root, path) {
            RecordingSource::capture(&root.join(path))
                .ok()
                .and_then(|s| serde_json::to_value(s).ok())
        } else {
            None
        };
        if current.as_ref() != Some(&row["source"]) {
            row["sourceState"] = json!("changed");
            row["embedded"] = Value::Null;
            row["issues"]
                .as_array_mut()
                .unwrap()
                .push(json!("source_changed_after_observation"));
        }
    }
    let (final_members, final_errors) = membership(&root, &folders);
    membership_errors.extend(final_errors);
    if final_members != initial_members {
        membership_errors.push("membership_changed_during_preview".into());
    }
    for path in final_members.difference(&initial_members) {
        if !rows.iter().any(|row| row["relativePath"] == json!(path)) {
            rows.push(json!({"relativePath":path,"status":"needs_review","sourceState":"unexamined","embedded":null,
                "hypotheses":[],"leads":[],"issues":["unexpected_member","membership_changed_during_preview"]}));
        }
    }
    let collisions: Vec<_> = claims.into_iter().filter(|(_,files)| files.len()>1).map(|((id,medium,row),files)| json!({
        "observationId":id,"mediumIndex":medium,"rowIndex":row,"files":files,"assessmentState":"historical_unverified"})).collect();
    for row in &mut rows {
        if collisions.iter().any(|c| {
            c["files"]
                .as_array()
                .unwrap()
                .contains(&row["relativePath"])
        }) {
            row["issues"]
                .as_array_mut()
                .unwrap()
                .push(json!("occurrence_collision"));
        }
    }
    Ok(
        json!({"schemaVersion":1,"scope":SCOPE,"manifestDigest":manifest_digest,"files":rows,
        "observations":serialized_observations,"collisions":collisions,"membershipIssues":membership_errors,
        "summary":{"physicalFiles":rows.len(),"reviewFiles":rows.len(),"previewLimited":limited,
            "collisionSets":collisions.len(),"discoveryCompleteness":"unknown","collisionAnalysis":"retained_claims_only"}}),
    )
}

pub fn markdown(preview: &Value) -> String {
    let mut out = format!("\n## Offline recovery preview\n\n{SCOPE}\n\n");
    for file in preview["files"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "### {}\n\n```json\n{}\n```\n\n",
            file["relativePath"],
            serde_json::to_string_pretty(file).unwrap()
        ));
    }
    out.push_str(&format!(
        "### Observation provenance and limits\n\n```json\n{}\n```\n",
        json!({
        "summary":preview["summary"],"membershipIssues":preview["membershipIssues"],
        "observations":preview["observations"],"collisions":preview["collisions"]})
    ));
    out
}
