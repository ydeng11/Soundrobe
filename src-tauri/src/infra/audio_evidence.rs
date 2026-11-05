//! Offline, exact PCM observations. Never supplies a tagging decision.
use md5::{Digest, Md5};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, Metadata},
    io::{Read, Seek},
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

const MAX_PCM: usize = 256 * 1024 * 1024;
const MAX_SOURCE: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST: u64 = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(45);
const SHADOW_WINDOW_FRAMES: usize = 4096;
pub const SCOPE: &str = "Read-only audio diagnostics, not identity proof or tagging approval. Shadow measurements use the fixed supplied offset (or zero), never inferred alignment; diagnostic only, with no acceptance tolerance or calibration. No recognition, near-silence classification, lag search, time warp, downmix identity or donor propagation. Zero-edge-excluded equality is diagnostic only. Reuse eligibility and resolver outputs never consume shadow measurements.";
pub const REUSE_SCOPE: &str = "Proposal only: no writes, new receipts, recipient verification or folder completion. Left is donor; right is recipient. Only independently rooted current core receipts and complete checksum-verified native FLAC PCM qualify. Conflicts cover selected sources only; no catalog or all-pairs search. Copies do not add independent corroboration. Source stability is observed, not a filesystem-wide lock.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    pairs: Vec<Pair>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pair {
    left: Source,
    right: Source,
    offset_frames: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    path: PathBuf,
    sha256: String,
}

/// The caller supplies the already inventoried physical-file set. Selection
/// cannot expand the scan or fetch a URL. Only explicit pairs incur decoding.
pub async fn export(
    inventory: &Value,
    manifest: &Path,
    cancelled: &AtomicBool,
) -> Result<Value, String> {
    export_selected(inventory, manifest, cancelled, false, None).await
}

pub async fn reuse_proposals(
    inventory: &Value,
    manifest: &Path,
    cancelled: &AtomicBool,
    worker_data: Option<&Path>,
) -> Result<Value, String> {
    export_selected(inventory, manifest, cancelled, true, worker_data).await
}

async fn export_selected(
    inventory: &Value,
    manifest: &Path,
    cancelled: &AtomicBool,
    reuse: bool,
    worker_data: Option<&Path>,
) -> Result<Value, String> {
    if !cfg!(unix) {
        return Err("held-source decoding is supported on Unix only".into());
    }
    let mut input = open_regular(manifest)?;
    let before = input.metadata().map_err(|e| e.to_string())?;
    if before.len() > MAX_MANIFEST {
        return Err("audio pair manifest exceeds 64 KiB".into());
    }
    let mut bytes = Vec::new();
    (&mut input)
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("audio pair manifest exceeds 64 KiB".into());
    }
    if !same_file(&before, &input.metadata().map_err(|e| e.to_string())?) {
        return Err("manifest changed during read".into());
    }
    let root = Path::new(inventory["root"].as_str().ok_or("missing inventory root")?);
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if manifest.pairs.is_empty() || manifest.pairs.len() > 16 {
        return Err("select between 1 and 16 audio pairs".into());
    }
    let mut pairs = Vec::new();
    let mut held_pairs = Vec::new();
    for pair in manifest.pairs {
        if reuse && pair.offset_frames.is_some() {
            return Err("reuse proposals do not accept offsets or partial alignment".into());
        }
        let resolve = |source: &Source| -> Result<PathBuf, String> {
            if source.sha256.len() != 64 || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("selected source needs a full SHA-256".into());
            }
            inventory["files"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|file| {
                    file["relativePath"]
                        .as_str()
                        .is_some_and(|p| Path::new(p) == source.path)
                })
                .and_then(|file| file["path"].as_str())
                .map(PathBuf::from)
                .ok_or_else(|| {
                    "selected source must be an inventoried relative path (no URLs or symlinks)"
                        .into()
                })
        };
        let left_path = resolve(&pair.left)?;
        let right_path = resolve(&pair.right)?;
        // Bind both physical selections before either decoder can run.
        let left_input = open_source(root, &left_path);
        let right_input = open_source(root, &right_path);
        let mut left = observe(root, &left_path, left_input, &pair.left.sha256, cancelled).await;
        let mut right = observe(
            root,
            &right_path,
            right_input,
            &pair.right.sha256,
            cancelled,
        )
        .await;
        // A left input can change while the right decoder runs.
        left.finalize(root, cancelled);
        right.finalize(root, cancelled);
        let comparison = compare(&left, &right, pair.offset_frames);
        if reuse {
            // Keep physical/directory bindings across the entire selection, but
            // release decoded buffers: peak PCM remains bounded to one pair.
            left.pcm = None;
            right.pcm = None;
            held_pairs.push((left, right, comparison));
        } else {
            let shadow = shadow_measurements(&left, &right, pair.offset_frames.unwrap_or(0));
            pairs.push(
                json!({"left":left.report,"right":right.report,"comparison":comparison,
                "shadowMeasurements":shadow}),
            );
        }
        if cancelled.load(Ordering::Acquire) {
            break;
        }
    }
    if reuse {
        let current_history = worker_data
            .map(crate::state::worker::WorkerStore::read_only_history)
            .transpose()?
            .unwrap_or_default();
        return Ok(propose(
            inventory,
            root,
            held_pairs,
            cancelled,
            &current_history,
        ));
    }
    Ok(json!({"schemaVersion":1,"scope":SCOPE,"pairs":pairs,
        "cancelled":cancelled.load(Ordering::Acquire),"limits":{"pairs":16,"pcmBytesPerFile":MAX_PCM,
        "sourceBytesPerFile":MAX_SOURCE,"secondsPerProcess":TIMEOUT.as_secs()}}))
}

fn observed_source(observation: &Observation) -> Value {
    #[cfg(unix)]
    if let Some(metadata) = &observation.snapshot {
        use std::os::unix::fs::MetadataExt;
        return json!({"path":observation.path,"fingerprint":observation.report["source"]["beforeSha256"],
            "physicalIdentity":format!("{}:{}",metadata.dev(),metadata.ino())});
    }
    #[cfg(not(unix))]
    let _ = observation;
    Value::Null
}

fn reusable_audio(observation: &Value) -> bool {
    observation["status"] == "complete"
        && observation["source"]["stable"] == true
        && observation["integrity"]["basis"] == "flac_streaminfo_md5_and_frames"
        && observation["integrity"]["matched"] == true
        && observation["pcm"]["frames"].as_u64().is_some_and(|n| n > 0)
        && observation["pcm"]["leadingZeroFrames"] != observation["pcm"]["frames"]
}

fn audio_key(observation: &Value) -> String {
    let interpretation = &observation["interpretation"];
    json!([
        interpretation["sampleRate"],
        interpretation["channels"],
        interpretation["channelLayout"],
        interpretation["pcmFormat"],
        observation["pcm"]["sha256"],
        observation["pcm"]["frames"]
    ])
    .to_string()
}

fn propose(
    inventory: &Value,
    root: &Path,
    mut held: Vec<(Observation, Observation, Value)>,
    cancelled: &AtomicBool,
    current_history: &[Value],
) -> Value {
    let find = |observation: &Observation| {
        inventory["files"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| f["path"] == json!(observation.path))
            .filter(|f| f["sourceIdentity"] == observed_source(observation))
            .ok_or_else(|| "inventory_decoded_source_binding_changed".to_owned())
    };
    // Validate roots first, then recheck ALL held sources, including earlier
    // selections. No later pair may silently change an earlier proposal input.
    let roots: Vec<_> = held
        .iter()
        .map(|(a, b, _)| {
            (
                find(a).and_then(|f| {
                    crate::commands::artist_inventory::reuse_root(f, current_history)
                }),
                find(b).and_then(|f| {
                    crate::commands::artist_inventory::reuse_root(f, current_history)
                }),
            )
        })
        .collect();
    for (a, b, _) in &mut held {
        a.finalize(root, cancelled);
        b.finalize(root, cancelled);
    }
    let mut labels = BTreeMap::<String, BTreeMap<String, Value>>::new();
    for ((a, b, _), (ar, br)) in held.iter().zip(&roots) {
        for (observation, proof) in [(a, ar), (b, br)] {
            if let Ok(proof) = proof {
                if reusable_audio(&observation.report)
                    && proof["source"] == observed_source(observation)
                {
                    labels
                        .entry(audio_key(&observation.report))
                        .or_default()
                        .insert(
                            json!([proof["accepted"], proof["decisionBasis"]]).to_string(),
                            proof.clone(),
                        );
                }
            }
        }
    }
    let conflicts: Vec<_> = labels.iter().filter(|(_,v)|v.len()>1)
        .map(|(key,values)|json!({"audioKey":key,"reason":"conflicting_valid_donor_core_labels","roots":values.values().collect::<Vec<_>>()})).collect();
    let mut proposals = BTreeMap::<String, Value>::new();
    let mut pairs = Vec::new();
    for ((a, b, comparison), (ar, br)) in held.iter().zip(&roots) {
        let a_source = observed_source(a);
        let b_source = observed_source(b);
        let key = audio_key(&a.report);
        let donor_eligibility = ar
            .as_ref()
            .map_err(Clone::clone)
            .and_then(crate::commands::artist_inventory::validate_reuse_donation);
        let recipient_eligibility = br
            .as_ref()
            .map_err(Clone::clone)
            .and_then(crate::commands::artist_inventory::validate_reuse_donation);
        let rejection = if a
            .snapshot
            .as_ref()
            .zip(b.snapshot.as_ref())
            .is_some_and(|(a, b)| same_identity(a, b))
        {
            Some("self_or_same_physical_source")
        } else if cancelled.load(Ordering::Acquire)
            || a.report["source"]["stable"] != true
            || b.report["source"]["stable"] != true
        {
            Some("cancelled_or_source_changed")
        } else if find(a).is_err() || find(b).is_err() {
            Some("inventory_decoded_source_binding_changed")
        } else if donor_eligibility.is_err() {
            Some("independent_donor_root_unavailable")
        } else if !reusable_audio(&a.report) || !reusable_audio(&b.report) {
            Some("checksum_completeness_unavailable")
        } else if a.report["interpretation"]["channelLayout"]
            != b.report["interpretation"]["channelLayout"]
            || comparison["wholePcmEqual"] != true
        {
            Some("complete_interpreted_pcm_not_equal")
        } else if labels.get(&key).is_some_and(|v| v.len() > 1) {
            Some("conflicting_valid_donor_core_labels")
        } else {
            None
        };
        pairs.push(json!({"left":a.report,"right":b.report,"comparison":comparison,
            "rejection":rejection,"donorRootError":donor_eligibility.err(),"recipientRootError":recipient_eligibility.err()}));
        if rejection.is_none() {
            let proof = ar.as_ref().unwrap();
            let proposal = proposals.entry(b.path.to_string_lossy().into_owned()).or_insert_with(|| {
                let mut patch = proof["accepted"].clone();
                if patch["musicbrainzTrackId"].is_null() {
                    patch.as_object_mut().unwrap().remove("musicbrainzTrackId");
                }
                json!({"state":"proposal_only","recipient":b_source,"patch":patch,
                    "decisionBasis":proof["decisionBasis"],"donors":[],"independentCorroborationCount":null,
                    "comparison":{"version":1,"method":"complete_native_pcm_bytes_equal","audioEvidenceVersion":1,
                        "donorDecoderVersion":a.report["decoder"]["version"],"recipientDecoderVersion":b.report["decoder"]["version"],
                        "interpretation":a.report["interpretation"],"recipientInterpretation":b.report["interpretation"],
                        "pcm":a.report["pcm"],"donorIntegrity":a.report["integrity"],"recipientIntegrity":b.report["integrity"]},
                    "recipientVerified":false,"folderCompletionChanged":false,
                    "unknowns":["recording edition, album grouping, positions, dates, enrichment and approval not inferred",
                        "independence between multiple donor copies is not established","conflicts outside selected sources not searched"],
                    "nextAction":"Review this core-only proposal; this export provides no apply operation or new proof."})
            });
            let donor =
                json!({"source":a_source,"root":proof["root"],"provenance":proof["provenance"]});
            let donors = proposal["donors"].as_array_mut().unwrap();
            if !donors.contains(&donor) {
                donors.push(donor);
            }
        }
    }
    let recipient_paths: BTreeSet<_> = held.iter().map(|(_, b, _)| &b.path).collect();
    json!({"schemaVersion":1,"scope":REUSE_SCOPE,"proposalCount":proposals.len(),"proposals":proposals.values().collect::<Vec<_>>(),
        "pairs":pairs,"selectedPairCount":held.len(),"selectedRecipientCount":recipient_paths.len(),"conflicts":conflicts,
        "cancelled":cancelled.load(Ordering::Acquire),"observedAt":time::OffsetDateTime::now_utc().unix_timestamp(),
        "limits":{"pairs":16,"pcmBytesPerFile":MAX_PCM,"sourceBytesPerFile":MAX_SOURCE,"secondsPerProcess":TIMEOUT.as_secs()}})
}

struct Observation {
    path: PathBuf,
    held: Option<HeldSource>,
    snapshot: Option<Metadata>,
    report: Value,
    pcm: Option<Pcm>,
}
impl Observation {
    fn incomplete(&mut self, reason: &str) {
        self.report["status"] = json!("incomplete");
        self.report["error"] = json!(reason);
        self.pcm = None;
    }
    fn finalize(&mut self, root: &Path, cancelled: &AtomicBool) {
        let result = (|| -> Result<(), String> {
            if cancelled.load(Ordering::Acquire) {
                return Err("cancelled; after-source hashing not attempted".into());
            }
            let held = self.held.as_mut().ok_or("source could not be opened")?;
            if held.file.metadata().map_err(|e| e.to_string())?.len() > MAX_SOURCE {
                return Err("after-source hashing exceeds source byte limit".into());
            }
            let hash = hash_source(&mut held.file)?;
            self.report["source"]["afterSha256"] = json!(hash);
            let before = self
                .snapshot
                .as_ref()
                .ok_or("initial source snapshot unavailable")?;
            let current = open_source(root, &self.path)?;
            if self.report["source"]["beforeSha256"].as_str() != Some(hash.as_str())
                || !same_file(before, &held.file.metadata().map_err(|e| e.to_string())?)
                || !same_file(before, &current.file.metadata().map_err(|e| e.to_string())?)
                || !held.same_directories(&current)
            {
                return Err("source or directory binding changed during observation".into());
            }
            Ok(())
        })();
        self.report["source"]["stable"] = json!(result.is_ok());
        self.report["source"]["finalizationError"] = json!(result.as_ref().err());
        if let Err(reason) = result {
            if self.pcm.is_some() {
                self.incomplete(&reason);
            }
        }
    }
}

fn same_identity(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.is_file()
            && b.is_file()
            && same_identity(a, b)
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
            && a.len() == b.len()
            && a.modified().ok() == b.modified().ok()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}

fn open_regular(path: &Path) -> Result<File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A replacement FIFO must not block a source check; never follow a
        // replacement symlink. The caller also validates the held metadata.
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("input must be a regular file".into());
    }
    Ok(file)
}

struct HeldSource {
    file: File,
    directories: Vec<File>,
}
impl HeldSource {
    fn same_directories(&self, other: &Self) -> bool {
        self.directories.len() == other.directories.len()
            && self
                .directories
                .iter()
                .zip(&other.directories)
                .all(|(a, b)| {
                    a.metadata()
                        .ok()
                        .zip(b.metadata().ok())
                        .is_some_and(|(a, b)| same_identity(&a, &b))
                })
    }
}

fn open_source(root: &Path, path: &Path) -> Result<HeldSource, String> {
    if !root.is_absolute() || path.strip_prefix(root).is_err() {
        return Err("selected path is outside the inventory root".into());
    }
    #[cfg(unix)]
    {
        use std::{
            os::{
                fd::{AsRawFd, FromRawFd},
                unix::ffi::OsStrExt,
            },
            path::Component,
        };
        let mut directories = vec![File::open("/").map_err(|e| e.to_string())?];
        let components = path.components().skip(1).collect::<Vec<_>>();
        if components.is_empty() || components.len() > 256 {
            return Err("unsupported source path depth".into());
        }
        for (i, component) in components.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err("noncanonical selected path".into());
            };
            let name = std::ffi::CString::new(name.as_bytes()).map_err(|e| e.to_string())?;
            let directory = i + 1 < components.len();
            let flags = libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK
                | libc::O_NOFOLLOW
                | if directory { libc::O_DIRECTORY } else { 0 };
            // Every component resolves against a held directory, including the
            // inventory root; the kernel may not follow ancestor symlinks.
            let fd = unsafe {
                libc::openat(
                    directories.last().unwrap().as_raw_fd(),
                    name.as_ptr(),
                    flags,
                )
            };
            if fd < 0 {
                return Err(format!(
                    "source directory/path binding: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let file = unsafe { File::from_raw_fd(fd) };
            if !directory {
                if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                    return Err("source must be a regular file".into());
                }
                return Ok(HeldSource { file, directories });
            }
            directories.push(file);
        }
        Err("missing selected file".into())
    }
    #[cfg(not(unix))]
    {
        Err("held-source decoding is supported on Unix only".into())
    }
}

fn hash_source(file: &mut File) -> Result<String, String> {
    file.rewind().map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > MAX_SOURCE {
            return Err("source exceeds bounded read limit".into());
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

async fn observe(
    root: &Path,
    path: &Path,
    input: Result<HeldSource, String>,
    expected: &str,
    cancelled: &AtomicBool,
) -> Observation {
    let mut observation = Observation {
        path: path.into(),
        held: None,
        snapshot: None,
        pcm: None,
        report: json!({"status":"incomplete","source":{"path":path,"expectedSha256":expected,
        "beforeSha256":null,"afterSha256":null,"stable":false},"interpretation":null,
        "integrity":null,"pcm":null,"error":null}),
    };
    let result = match input {
        Ok(mut held) => {
            let result = decode(&mut held.file, path, expected, cancelled, &mut observation).await;
            observation.held = Some(held);
            result
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(pcm) => {
            observation.report["status"] = json!("complete");
            observation.pcm = Some(pcm);
        }
        Err(reason) => observation.incomplete(&reason),
    }
    observation.finalize(root, cancelled);
    observation
}

/// Inputs remain held regular files. Unix /dev/fd/0 preserves seekability
/// (notably MP3 gapless interpretation), without reopening the selected path.
async fn decode(
    file: &mut File,
    path: &Path,
    expected: &str,
    cancelled: &AtomicBool,
    observation: &mut Observation,
) -> Result<Pcm, String> {
    if !cfg!(unix) {
        return Err("held-source decoding is supported on Unix only".into());
    }
    let demuxer = match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("flac") => "flac",
        Some("mp3") => "mp3",
        Some("wav") => "wav",
        _ => return Err("unsupported format: select FLAC, MP3 or WAV".into()),
    };
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > MAX_SOURCE {
        return Err("source is not a bounded regular file".into());
    }
    let hash = hash_source(file)?;
    observation.snapshot = Some(before.clone());
    observation.report["source"]["beforeSha256"] = json!(hash);
    observation.report["source"]["bytes"] = json!(before.len());
    observation.report["source"]["observedAt"] =
        json!(time::OffsetDateTime::now_utc().unix_timestamp());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        observation.report["source"]["heldFileIdentity"] = json!({"device":before.dev(),"inode":before.ino(),
            "ctimeSeconds":before.ctime(),"ctimeNanoseconds":before.ctime_nsec(),
            "mtimeSeconds":before.mtime(),"mtimeNanoseconds":before.mtime_nsec()});
    }
    if !hash.eq_ignore_ascii_case(expected) {
        return Err("source hash differs from selected input".into());
    }
    let probe_args = [
        "-v",
        "warning",
        "-protocol_whitelist",
        "file,pipe",
        "-f",
        demuxer,
        "-i",
        "/dev/fd/0",
        "-select_streams",
        "a",
        "-show_streams",
        "-show_program_version",
        "-of",
        "json",
    ];
    let probe = run_tool(
        "ffprobe",
        &probe_args,
        file,
        1024 * 1024,
        cancelled,
        &mut observation.report["probe"],
    )
    .await?;
    let value: Value = serde_json::from_slice(&probe).map_err(|e| e.to_string())?;
    let streams = value["streams"]
        .as_array()
        .ok_or("probe returned no audio streams")?;
    if streams.len() != 1 {
        return Err("select a source with exactly one audio stream".into());
    }
    let stream = &streams[0];
    let channels = stream["channels"].as_u64().ok_or("missing channels")? as usize;
    let rate = stream["sample_rate"]
        .as_str()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|r| *r > 0)
        .ok_or("missing sample rate")?;
    let layout = match channels {
        1 => "mono",
        2 => "stereo",
        _ => return Err("unsupported channel layout (mono/stereo only)".into()),
    };
    if stream["channel_layout"]
        .as_str()
        .is_some_and(|s| s != layout)
    {
        return Err("unsupported channel interpretation".into());
    }
    let codec = stream["codec_name"].as_str().ok_or("missing codec")?;
    let format = match (demuxer, codec, stream["sample_fmt"].as_str()) {
        ("flac", "flac", Some("s16")) => "s16le",
        ("flac", "flac", Some("s32")) if stream["bits_per_raw_sample"] == "24" => "s24le",
        ("wav", "pcm_s16le", _) => "s16le",
        ("wav", "pcm_s24le", _) => "s24le",
        ("wav", "pcm_f32le", _) | ("mp3", "mp3", Some("fltp" | "flt")) => "f32le",
        _ => return Err("unsupported native precision/codec; no quantization fallback".into()),
    };
    let descriptor = json!({"sampleRate":rate,"channels":channels,"channelLayout":layout,"pcmFormat":format,
        "streamIndex":stream["index"],"codec":codec,"nativeSampleFormat":stream["sample_fmt"],
        "transforms":"native channels in original order; interleaving only; no resampling, gain, downmix or trimming",
        "gapless":"decoder default; exact zeros are decoded observations, not certified encoder padding",
        "probeStream":stream,"probeVersion":value["program_version"]});
    observation.report["interpretation"] = descriptor;
    let flac = if demuxer == "flac" {
        file.rewind().map_err(|e| e.to_string())?;
        let mut header = [0u8; 42];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        if &header[..4] != b"fLaC" || header[4] & 0x7f != 0 || header[5..8] != [0, 0, 34] {
            return Err("unsupported FLAC STREAMINFO layout".into());
        }
        let packed = u64::from_be_bytes(header[18..26].try_into().unwrap());
        let frames = packed & 0xf_ffff_ffff;
        let bits = ((packed >> 36) & 31) + 1;
        if packed >> 44 != u64::from(rate)
            || ((packed >> 41) & 7) + 1 != channels as u64
            || !matches!((bits, format), (16, "s16le") | (24, "s24le"))
        {
            return Err("FLAC header/probe interpretation conflict".into());
        }
        let checksum = header[26..42].to_vec();
        if frames == 0 || checksum.iter().all(|b| *b == 0) {
            return Err(
                "FLAC declared frames/nonzero checksum unavailable; cannot certify complete PCM"
                    .into(),
            );
        }
        Some((frames, checksum))
    } else {
        None
    };
    let wav_frames = if demuxer == "wav" {
        Some(wav_frames(file, format, channels, rate)?)
    } else {
        None
    };
    let version = run_tool(
        "ffmpeg",
        &["-version"],
        file,
        65536,
        cancelled,
        &mut observation.report["decoderVersionProbe"],
    )
    .await?;
    let decoder_args = [
        "-nostdin",
        "-v",
        "warning",
        "-threads",
        "1",
        "-protocol_whitelist",
        "file,pipe",
        "-f",
        demuxer,
        "-i",
        "/dev/fd/0",
        "-map",
        "0:a:0",
        "-vn",
        "-sn",
        "-dn",
        "-c:a",
    ];
    let codec_out = format!("pcm_{format}");
    let mut args = decoder_args.to_vec();
    // Without a lossless checksum, recoverable decode errors cannot certify
    // complete PCM. FLAC recovery remains governed by native checksum/count.
    if demuxer != "flac" {
        args.insert(0, "-xerror");
    }
    args.extend([codec_out.as_str(), "-f", format, "pipe:1"]);
    let decoded = run_tool(
        "ffmpeg",
        &args,
        file,
        MAX_PCM,
        cancelled,
        &mut observation.report["decoder"],
    )
    .await;
    observation.report["decoder"]["version"] =
        json!(String::from_utf8_lossy(&version).lines().next());
    let pcm = Pcm::new(decoded?, format, channels, rate)?;
    observation.report["pcm"] = pcm.summary();
    if let Some(expected) = wav_frames {
        let matched = pcm.frames() as u64 == expected;
        observation.report["wavValidation"] =
            json!({"expectedFrames":expected,"decodedFrames":pcm.frames(),"matched":matched});
        if !matched {
            return Err("WAV declared/decoded sample-count mismatch".into());
        }
    }
    let basis = if let Some((frames, checksum)) = flac {
        let actual = Md5::digest(&pcm.bytes);
        let matched = pcm.frames() as u64 == frames && actual.as_slice() == checksum;
        observation.report["flacValidation"] = json!({"expectedFrames":frames,"decodedFrames":pcm.frames(),
            "expectedMd5":checksum.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "decodedMd5":format!("{actual:x}"),"matched":matched});
        if !matched {
            observation.report["integrity"] =
                json!({"basis":"flac_streaminfo_md5_and_frames","matched":false});
            return Err("FLAC checksum/sample-count mismatch".into());
        }
        "flac_streaminfo_md5_and_frames"
    } else if wav_frames.is_some() {
        "wav_riff_chunks_and_frames_not_source_master_completeness"
    } else {
        "decoder_eof_only_not_source_master_completeness"
    };
    observation.report["integrity"] = json!({"basis":basis,"matched":true,
        "limitation":if demuxer == "flac" {"Checksum certifies recovered local PCM, not recording identity"} else {"No lossless-source checksum; EOF does not prove an untruncated source master"}});
    Ok(pcm)
}

/// Only supported little-endian RIFF PCM/float layouts are certified. Chunk
/// extents include odd-byte padding; decoder EOF cannot repair a truncated RIFF.
fn wav_frames(file: &mut File, format: &str, channels: usize, rate: u32) -> Result<u64, String> {
    use std::io::SeekFrom;
    file.rewind().map_err(|e| e.to_string())?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err("unsupported WAV container (RIFF required)".into());
    }
    let end = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap())) + 8;
    if end < 12 || end > file.metadata().map_err(|e| e.to_string())?.len() {
        return Err("truncated WAV RIFF extent".into());
    }
    let sample_bytes = match format {
        "s16le" => 2,
        "s24le" => 3,
        "f32le" => 4,
        _ => return Err("unsupported WAV precision".into()),
    };
    let align = channels * sample_bytes;
    let mut position = 12;
    let mut fmt = false;
    let mut frames = None;
    for _ in 0..4096 {
        if position == end {
            return frames
                .filter(|_| fmt)
                .ok_or_else(|| "WAV requires one fmt and one data chunk".into());
        }
        if end - position < 8 {
            return Err("truncated WAV chunk header".into());
        }
        file.seek(SeekFrom::Start(position))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
        let size = u64::from(u32::from_le_bytes(chunk[4..].try_into().unwrap()));
        let next = position + 8 + size + (size % 2);
        if next > end {
            return Err("truncated WAV chunk extent/padding".into());
        }
        match &chunk[..4] {
            b"fmt " => {
                if fmt || size < 16 {
                    return Err("unsupported WAV fmt chunk".into());
                }
                let mut data = vec![0u8; size.min(40) as usize];
                file.read_exact(&mut data).map_err(|e| e.to_string())?;
                let mut tag = u16::from_le_bytes(data[..2].try_into().unwrap());
                let bits = u16::from_le_bytes(data[14..16].try_into().unwrap());
                if tag == 0xfffe {
                    if size < 40
                        || u16::from_le_bytes(data[16..18].try_into().unwrap()) != 22
                        || data[18..20] != data[14..16]
                        || data[26..40] != [0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]
                    {
                        return Err("unsupported WAV extensible precision/subformat".into());
                    }
                    tag = u16::from_le_bytes(data[24..26].try_into().unwrap());
                }
                if tag != if format == "f32le" { 3 } else { 1 }
                    || usize::from(u16::from_le_bytes(data[2..4].try_into().unwrap())) != channels
                    || u32::from_le_bytes(data[4..8].try_into().unwrap()) != rate
                    || u64::from(u32::from_le_bytes(data[8..12].try_into().unwrap()))
                        != u64::from(rate) * align as u64
                    || usize::from(u16::from_le_bytes(data[12..14].try_into().unwrap())) != align
                    || usize::from(bits) != sample_bytes * 8
                {
                    return Err("WAV native header/probe interpretation conflict".into());
                }
                fmt = true;
            }
            b"data" => {
                if frames.is_some() || size == 0 || size % align as u64 != 0 {
                    return Err("unsupported/incomplete WAV data frames".into());
                }
                frames = Some(size / align as u64);
            }
            _ => (),
        }
        position = next;
    }
    Err("WAV exceeds 4096 chunk observation limit".into())
}

async fn run_tool(
    tool: &str,
    args: &[&str],
    file: &mut File,
    limit: usize,
    cancelled: &AtomicBool,
    report: &mut Value,
) -> Result<Vec<u8>, String> {
    let executable = std::env::var_os(format!("AUTO_TAG_{}_PATH", tool.to_uppercase()))
        .unwrap_or_else(|| tool.into());
    *report = json!({"arguments":args,"executable":executable,"warnings":"","exitCode":null,"error":null});
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut code = None;
    let result=async {
        if cancelled.load(Ordering::Acquire) {return Err("cancelled".into())}
        file.rewind().map_err(|e| e.to_string())?;
        let mut command = Command::new(executable);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
        .args(args)
        .stdin(Stdio::from(file.try_clone().map_err(|e| e.to_string())?))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        let process_group = child.id();
    let mut stdout = child.stdout.take().ok_or("missing child stdout")?;
    let mut stderr = child.stderr.take().ok_or("missing child stderr")?;
        let result = tokio::select! {
        result = async {
            tokio::try_join!(bounded_read(&mut stdout,limit,&mut stdout_bytes),bounded_read(&mut stderr,65536,&mut stderr_bytes),async {
                code=child.wait().await.map_err(|e| e.to_string())?.code(); Ok(())
            })?;
            if code!=Some(0) {return Err("process failed; partial output is not comparison evidence".into())}
            Ok(())
        } => result,
        _ = tokio::time::sleep(TIMEOUT) => Err("decoder/probe deadline exceeded".into()),
        _ = async { loop { if cancelled.load(Ordering::Acquire) { break; } tokio::time::sleep(Duration::from_millis(25)).await; } } => Err("cancelled".into()),
    };
    if result.is_err() {
        #[cfg(unix)]
        if let Some(group) = process_group {
            // Include wrapper workers that inherited the invocation's group.
            unsafe { libc::kill(-(group as i32), libc::SIGKILL) };
        }
        let _ = child.kill().await;
        if let Ok(status)=child.wait().await {code=status.code();}
        // Preserve already emitted diagnostics even when another stream hit
        // its limit first. Descendants may retain pipes, so cleanup is bounded.
        let _=tokio::time::timeout(Duration::from_millis(100), async {
            tokio::join!(bounded_read(&mut stdout,limit,&mut stdout_bytes),bounded_read(&mut stderr,65536,&mut stderr_bytes))
        }).await;
    }
        result
    }.await;
    report["warnings"] = json!(String::from_utf8_lossy(&stderr_bytes));
    report["stdoutBytesRetained"] = json!(stdout_bytes.len());
    report["exitCode"] = json!(code);
    report["error"] = json!(result.as_ref().err());
    result.map(|()| stdout_bytes)
}

async fn bounded_read(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
    bytes: &mut Vec<u8>,
) -> Result<(), String> {
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(());
        }
        let keep = count.min(limit - bytes.len());
        bytes.extend_from_slice(&buffer[..keep]);
        if keep < count {
            return Err("process output exceeded byte limit".into());
        }
    }
}

struct Pcm {
    bytes: Vec<u8>,
    format: String,
    channels: usize,
    rate: u32,
    frame_bytes: usize,
    leading: usize,
    end: usize,
}
impl Pcm {
    fn new(bytes: Vec<u8>, format: &str, channels: usize, rate: u32) -> Result<Self, String> {
        let sample_bytes = match format {
            "s16le" => 2,
            "s24le" => 3,
            "f32le" => 4,
            _ => return Err("unsupported PCM format".into()),
        };
        let frame_bytes = sample_bytes * channels;
        if bytes.is_empty() || bytes.len() % frame_bytes != 0 {
            return Err("incomplete decoded PCM frames".into());
        }
        if format == "f32le"
            && bytes
                .chunks_exact(4)
                .any(|b| !f32::from_le_bytes(b.try_into().unwrap()).is_finite())
        {
            return Err("nonfinite native PCM".into());
        }
        let mut pcm = Self {
            bytes,
            format: format.into(),
            channels,
            rate,
            frame_bytes,
            leading: 0,
            end: 0,
        };
        pcm.leading = (0..pcm.frames()).take_while(|i| pcm.zero(*i)).count();
        pcm.end = (pcm.leading..pcm.frames())
            .rfind(|i| !pcm.zero(*i))
            .map_or(pcm.leading, |i| i + 1);
        Ok(pcm)
    }
    fn frames(&self) -> usize {
        self.bytes.len() / self.frame_bytes
    }
    fn zero(&self, frame: usize) -> bool {
        let bytes = &self.bytes[frame * self.frame_bytes..(frame + 1) * self.frame_bytes];
        if self.format == "f32le" {
            bytes
                .chunks_exact(4)
                .all(|b| f32::from_le_bytes(b.try_into().unwrap()) == 0.0)
        } else {
            bytes.iter().all(|b| *b == 0)
        }
    }
    fn core(&self) -> &[u8] {
        &self.bytes[self.leading * self.frame_bytes..self.end * self.frame_bytes]
    }
    fn summary(&self) -> Value {
        // Maximal zero edge runs overlap for an all-silent stream; its core
        // stays empty and comparison must still refuse to infer alignment.
        let trailing = if self.leading == self.frames() {
            self.frames()
        } else {
            self.frames() - self.end
        };
        json!({"frames":self.frames(),"bytes":self.bytes.len(),"seconds":self.frames() as f64/f64::from(self.rate),
            "leadingZeroFrames":self.leading,"trailingZeroFrames":trailing,
            "leadingZeroSeconds":self.leading as f64/f64::from(self.rate),
            "trailingZeroSeconds":trailing as f64/f64::from(self.rate),
            "zeroEdgeExcludedInterval":{"startFrame":self.leading,"endFrame":self.end},
            "allZero":self.leading==self.frames(),"sha256":format!("{:x}",Sha256::digest(&self.bytes)),
            "zeroEdgeExcludedSha256":if self.core().is_empty() {None} else {Some(format!("{:x}",Sha256::digest(self.core())))}})
    }
}

fn compare(left: &Observation, right: &Observation, requested: Option<i64>) -> Value {
    let unavailable = |reason: &str| {
        json!({"status":"unavailable","reason":reason,"wholePcmEqual":null,
        "zeroEdgeExcludedEqual":null,"exactAlignedOverlapEqual":null,"coverage":null})
    };
    let (Some(a), Some(b)) = (&left.pcm, &right.pcm) else {
        return unavailable("incomplete source observation");
    };
    if (a.rate, a.channels, &a.format) != (b.rate, b.channels, &b.format) {
        return unavailable("incompatible PCM interpretation; no conversion attempted");
    }
    let whole = a.bytes == b.bytes;
    let edges = !a.core().is_empty() && a.core() == b.core();
    let mut report = json!({"status":"observed","wholePcmEqual":whole,"zeroEdgeExcludedEqual":edges,
        "exactAlignedOverlapEqual":null,"coverage":null,"offsetFrames":null,
        "offsetConvention":"left frame = right frame + offsetFrames","identityVerified":false});
    if a.core().is_empty() || b.core().is_empty() {
        report["reason"] = json!("all-zero audio is uninformative; no alignment");
        return report;
    }
    let offset = requested.or_else(|| {
        if whole {
            Some(0)
        } else if edges {
            Some(a.leading as i64 - b.leading as i64)
        } else {
            None
        }
    });
    let Some(offset) = offset else {
        report["reason"] = json!("no exact alignment established; supply a proposed offset to test, not an approximate search");
        return report;
    };
    report["offsetFrames"] = json!(offset);
    // Check bounds before arithmetic, including i64::MIN supplied by a manifest.
    if offset <= -(b.frames() as i64) || offset >= a.frames() as i64 {
        report["reason"] = json!("proposed offset has no overlap");
        return report;
    }
    let start_a = offset.max(0) as usize;
    let start_b = (-offset).max(0) as usize;
    let count = (a.frames() - start_a).min(b.frames() - start_b);
    let equal = a.bytes[start_a * a.frame_bytes..(start_a + count) * a.frame_bytes]
        == b.bytes[start_b * b.frame_bytes..(start_b + count) * b.frame_bytes];
    report["exactAlignedOverlapEqual"] = json!(equal);
    if !equal {
        report["reason"] = json!(
            "nonidentical frames at proposed offset; no exact correspondence or coverage claimed"
        );
        return report;
    }
    let coverage = |pcm: &Pcm, start: usize| {
        let mut unmatched = Vec::new();
        for (from, to) in [(0, start), (start + count, pcm.frames())] {
            if from < to {
                unmatched.push(json!({"startFrame":from,"endFrame":to,
                "nonzeroFrames":(from..to).filter(|i| !pcm.zero(*i)).count()}));
            }
        }
        let core_count = (start + count)
            .min(pcm.end)
            .saturating_sub(start.max(pcm.leading));
        json!({"matchedInterval":{"startFrame":start,"endFrame":start+count},"matchedFrames":count,
            "wholeTimelineFraction":count as f64/pcm.frames() as f64,
            "zeroEdgeExcludedFraction":core_count as f64/(pcm.end-pcm.leading) as f64,"unmatched":unmatched})
    };
    report["coverage"] = json!({"left":coverage(a,start_a),"right":coverage(b,start_b)});
    report
}

impl Pcm {
    /// Full-scale units; no quantization, resampling or channel mixing.
    fn sample(&self, frame: usize, channel: usize) -> f64 {
        let width = self.frame_bytes / self.channels;
        let start = frame * self.frame_bytes + channel * width;
        let bytes = &self.bytes[start..start + width];
        match self.format.as_str() {
            "s16le" => f64::from(i16::from_le_bytes(bytes.try_into().unwrap())) / 32768.0,
            "s24le" => {
                let signed = i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) >> 8;
                f64::from(signed) / 8388608.0
            }
            "f32le" => f64::from(f32::from_le_bytes(bytes.try_into().unwrap())),
            _ => unreachable!("Pcm::new validates formats"),
        }
    }

    fn levels(&self, start: usize, count: usize, channel: usize) -> Value {
        let mut peak = 0.0f64;
        let mut squares = 0.0;
        for frame in start..start + count {
            let sample = self.sample(frame, channel);
            peak = peak.max(sample.abs());
            squares += sample * sample;
        }
        json!({"peak":peak,"rms":(squares / count as f64).sqrt()})
    }
}

fn shadow_channel(a: &Pcm, b: &Pcm, sa: usize, sb: usize, count: usize, ch: usize) -> Value {
    let mut sum_a = 0.0;
    let mut sum_b = 0.0;
    let mut aa = 0.0;
    let mut bb = 0.0;
    let mut ab = 0.0;
    for i in 0..count {
        let x = a.sample(sa + i, ch);
        let y = b.sample(sb + i, ch);
        sum_a += x;
        sum_b += y;
        aa += x * x;
        bb += y * y;
        ab += x * y;
    }
    let mean_a = sum_a / count as f64;
    let mean_b = sum_b / count as f64;
    let mut variance_a = 0.0;
    let mut variance_b = 0.0;
    let mut covariance = 0.0;
    for i in 0..count {
        let x = a.sample(sa + i, ch) - mean_a;
        let y = b.sample(sb + i, ch) - mean_b;
        variance_a += x * x;
        variance_b += y * y;
        covariance += x * y;
    }
    let correlation = (variance_a > 0.0 && variance_b > 0.0)
        .then(|| (covariance / (variance_a.sqrt() * variance_b.sqrt())).clamp(-1.0, 1.0));
    // Fit right ~= gain * left through the origin. A nonpositive fit is not
    // a positive-gain explanation; silence has no identifiable gain.
    let gain = (aa > 0.0 && bb > 0.0 && ab > 0.0).then(|| ab / aa);
    let residual = gain.map(|gain| {
        let mut squares = 0.0;
        for i in 0..count {
            let error = b.sample(sb + i, ch) - gain * a.sample(sa + i, ch);
            squares += error * error;
        }
        (squares / count as f64).sqrt()
    });
    json!({"channelIndex":ch,"left":a.levels(sa,count,ch),"right":b.levels(sb,count,ch),
        "correlation":correlation,"positiveGain":gain,"positiveGainResidualRms":residual,
        "positiveGainResidualRelativeToRightRms":residual.map(|r| r / (bb / count as f64).sqrt())})
}

fn shadow_coverage(pcm: &Pcm, start: usize, count: usize) -> Value {
    let mut unmatched = Vec::new();
    for (from, to) in [(0, start), (start + count, pcm.frames())] {
        if from < to {
            unmatched.push(json!({"startFrame":from,"endFrame":to,
                "nonzeroFrames":(from..to).filter(|i| !pcm.zero(*i)).count(),
                "channels":(0..pcm.channels).map(|ch| pcm.levels(from,to-from,ch)).collect::<Vec<_>>()}));
        }
    }
    json!({"totalFrames":pcm.frames(),"matchedInterval":{"startFrame":start,"endFrame":start+count},
        "matchedFrames":count,"unmatchedFrames":pcm.frames()-count,
        "wholeTimelineFraction":count as f64 / pcm.frames() as f64,"unmatched":unmatched})
}

fn shadow_measurements(left: &Observation, right: &Observation, offset: i64) -> Value {
    let unavailable = |reason: &str| json!({"status":"unavailable","reason":reason});
    let (Some(a), Some(b)) = (&left.pcm, &right.pcm) else {
        return unavailable("incomplete source observation");
    };
    if (a.rate, a.channels, &a.format) != (b.rate, b.channels, &b.format) {
        return unavailable("incompatible native PCM interpretation; no conversion attempted");
    }
    // Bounds precede negation so even i64::MIN cannot overflow.
    let overlap = offset > -(b.frames() as i64) && offset < a.frames() as i64;
    let (sa, sb, count) = if overlap {
        let sa = offset.max(0) as usize;
        let sb = (-offset).max(0) as usize;
        (sa, sb, (a.frames() - sa).min(b.frames() - sb))
    } else {
        (0, 0, 0)
    };
    let mut windows = Vec::new();
    let mut differing = 0;
    for from in (0..count).step_by(SHADOW_WINDOW_FRAMES) {
        let frames = SHADOW_WINDOW_FRAMES.min(count - from);
        let differences = (from..from + frames)
            .filter(|i| {
                a.bytes[(sa + i) * a.frame_bytes..(sa + i + 1) * a.frame_bytes]
                    != b.bytes[(sb + i) * b.frame_bytes..(sb + i + 1) * b.frame_bytes]
            })
            .count();
        differing += differences;
        windows.push(json!({"leftStartFrame":sa+from,"rightStartFrame":sb+from,"frames":frames,
            "differingFrames":differences,"exactEqualFrames":frames-differences,
            "channels":(0..a.channels).map(|ch| shadow_channel(a,b,sa+from,sb+from,frames,ch)).collect::<Vec<_>>()}));
    }
    json!({"version":1,"status":if overlap {"observed"} else {"no_overlap"},
        "offsetFrames":offset,"offsetConvention":"left frame = right frame + offsetFrames",
        "windowFrames":SHADOW_WINDOW_FRAMES,"sampleRate":a.rate,
        "measurementScope":"Diagnostic measurement only, not acceptance calibration. Consecutive overlap windows, final partial included. Matched means aligned at the fixed offset, not equal or accepted. Exact counts compare native frame bytes. Levels and residuals are full-scale units (integer PCM divided by 2^(bits-1), float unchanged). Pearson correlation is null for zero variance; positiveGain fits right = gain * left through the origin and is null without a positive identifiable fit. Channel order is native; no channel mixing.",
        "matchedFrames":count,"differingFrames":differing,"exactEqualFrames":count-differing,
        "coverage":{"left":shadow_coverage(a,sa,count),"right":shadow_coverage(b,sb,count)},
        "windows":windows})
}
