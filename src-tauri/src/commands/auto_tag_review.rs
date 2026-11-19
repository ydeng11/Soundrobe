//! Session auto-tag evidence and exact-file recovery.
use super::{
    mutations::replace_file_atomic,
    tracks::{read_album, read_track_metadata, try_read_extra_tags},
};
use crate::error::ApiError;
use crate::state::worker::{FieldReviewObligation, WorkerStore};
#[cfg(any(feature = "desktop", test))]
use crate::state::write_queue::WriteQueue;
#[cfg(feature = "desktop")]
use base64::Engine;
use lofty::file::TaggedFileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[cfg(feature = "desktop")]
use tauri::State;

/// Delete only journals belonging to processes that are definitely no longer
/// alive. PID reuse conservatively retains an old journal until a later startup.
pub fn cleanup_abandoned_sessions() {
    cleanup_abandoned_at(&std::env::temp_dir());
}
fn cleanup_abandoned_at(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(suffix) = name
            .to_str()
            .and_then(|name| name.strip_prefix("soundrobe-review-"))
        else {
            continue;
        };
        let Some((pid, id)) = suffix.split_once('-') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err()
            || process_alive(pid)
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        if let Err(error) = fs::remove_dir_all(entry.path()) {
            tracing::warn!(%error, "abandoned review cleanup failed");
        }
    }
}
#[cfg(unix)]
pub(crate) fn process_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return true;
    }
    // Signal zero checks existence without sending a signal. Permission errors
    // mean the process may still own its journal.
    unsafe {
        libc::kill(pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}
#[cfg(windows)]
pub(crate) fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER},
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        CloseHandle(handle);
        true
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewView {
    pub tracks: Vec<Value>,
    pub artworks: Vec<ReviewArtwork>,
    pub errors: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewArtwork {
    pub id: String,
    pub label: String,
    pub source: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDetail {
    pub id: String,
    pub album_path: String,
    pub outcome: String,
    pub decision: String,
    pub result: Value,
    pub before: ReviewView,
    pub after: ReviewView,
    pub can_revert: bool,
    pub errors: Vec<String>,
    pub field_reviews: Vec<ReviewField>,
    pub recovered: bool,
    pub recovery_required: bool,
    pub recovery_files: Vec<RecoveryFile>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryFile {
    pub path: PathBuf,
    pub current_sha256: Option<String>,
    pub current_metadata: Option<Value>,
    pub unavailable_reason: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewField {
    pub path: PathBuf,
    pub field: String,
    pub before: Value,
    pub after: Value,
    pub source: &'static str,
    pub state: String,
}

impl From<FieldReviewObligation> for ReviewField {
    fn from(field: FieldReviewObligation) -> Self {
        let source = match field.provenance["sourceObservation"]["origin"].as_str() {
            Some("discogs") => "Discogs release notes",
            Some("synthetic_loopback") => "Synthetic release notes",
            _ => "Source evidence",
        };
        Self {
            path: field.path,
            field: field.field,
            before: field.before,
            after: field.after,
            source,
            state: field.state,
        }
    }
}
struct FileChange {
    path: PathBuf,
    target: PathBuf,
    backup: Option<PathBuf>,
    before: Option<String>,
    after: Option<String>,
    restored: bool,
    verified: bool,
    tracked: bool,
    restoring: bool,
    intent_outcomes: Vec<(String, bool)>,
    original_mode: Option<u32>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct ReversalFile {
    path: PathBuf,
    target: PathBuf,
    before: Option<String>,
    after: Option<String>,
    backup: Option<PathBuf>,
    original_mode: Option<u32>,
    phase: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct ReversalManifest {
    files: Vec<ReversalFile>,
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode())
}
#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
    None
}

#[cfg(unix)]
fn restore_mode(path: &Path, mode: Option<u32>) -> Result<(), ApiError> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}
#[cfg(not(unix))]
fn restore_mode(_path: &Path, _mode: Option<u32>) -> Result<(), ApiError> {
    Ok(())
}

fn verify_reversal_file(file: &ReversalFile) -> Result<(), ApiError> {
    if canonical_target(&file.path)? != file.target
        || fs::symlink_metadata(&file.path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(failure(format!(
            "Reversal target changed: {}",
            file.path.display()
        )));
    }
    let current = fingerprint(&file.path)?;
    let valid = match file.phase.as_str() {
        "pending" => current == file.after,
        "restoring" => current == file.after || current == file.before,
        "restored" => current == file.before,
        _ => false,
    };
    if !valid {
        return Err(failure(format!(
            "Reversal target changed: {}",
            file.path.display()
        )));
    }
    match (&file.before, &file.backup) {
        (Some(expected), Some(backup))
            if fs::symlink_metadata(backup)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
                && fingerprint(backup)?.as_deref() == Some(expected) => {}
        (None, None) => {}
        _ => return Err(failure("Reversal backup is missing or damaged")),
    }
    Ok(())
}
struct ReviewData {
    detail: ReviewDetail,
    files: Vec<FileChange>,
    images: HashMap<String, PathBuf>,
    trusted_assisted_genre_proposals: HashSet<String>,
}
pub(crate) struct TargetWriteLease {
    worker: Arc<WorkerStore>,
    revision: String,
    target: PathBuf,
    token: String,
    inserted_owner: bool,
    discard_owner: bool,
    active: bool,
}
impl TargetWriteLease {
    pub(crate) fn mutation_started(&mut self) {
        self.discard_owner = false;
    }
    fn release(&mut self) -> Result<(), ApiError> {
        self.worker
            .release_review_target(
                &self.revision,
                &self.target,
                &self.token,
                self.inserted_owner && self.discard_owner,
            )
            .map_err(failure)?;
        self.active = false;
        Ok(())
    }
}
impl Drop for TargetWriteLease {
    fn drop(&mut self) {
        if self.active {
            if let Err(error) = self.release() {
                tracing::error!(%error, "failed to release review target ownership");
            }
        }
    }
}
struct SessionFiles(PathBuf);
impl Default for SessionFiles {
    fn default() -> Self {
        Self(std::env::temp_dir().join(format!(
            "soundrobe-review-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        )))
    }
}
impl Drop for SessionFiles {
    fn drop(&mut self) {
        if self.0.exists() {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                tracing::warn!(%error, "review backup cleanup failed");
            }
        }
    }
}
#[derive(Default)]
pub struct ReviewStore {
    records: Mutex<Vec<Arc<ReviewJournal>>>,
    files: Arc<SessionFiles>,
    owners: Arc<Mutex<HashMap<PathBuf, Vec<String>>>>,
    recovery: Option<Arc<WorkerStore>>,
}
pub struct ReviewJournal {
    data: Mutex<ReviewData>,
    files: Arc<SessionFiles>,
    owners: Arc<Mutex<HashMap<PathBuf, Vec<String>>>>,
    recovery: Option<Arc<WorkerStore>>,
}
tokio::task_local! { pub static ACTIVE_REVIEW: Arc<ReviewJournal>; }
pub fn active_review() -> Option<Arc<ReviewJournal>> {
    ACTIVE_REVIEW.try_with(Arc::clone).ok()
}
fn failure(message: impl Into<String>) -> ApiError {
    ApiError::Message(message.into())
}
fn canonical_target(path: &Path) -> Result<PathBuf, ApiError> {
    if path.exists() {
        let target = fs::canonicalize(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = fs::metadata(&target)?;
            if metadata.is_file() && metadata.nlink() > 1 {
                return Err(failure(format!(
                    "Hard-linked target cannot be safely owned: {}",
                    target.display()
                )));
            }
        }
        return Ok(target);
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(failure(
            "Recovery path contains unresolved parent traversal",
        ));
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    while !ancestor.exists() {
        if fs::symlink_metadata(ancestor).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(failure(format!(
                "Cannot own a dangling symlink: {}",
                ancestor.display()
            )));
        }
        missing.push(
            ancestor
                .file_name()
                .ok_or_else(|| failure("Recovery path has no filename"))?
                .to_os_string(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| failure("Recovery path has no parent"))?;
    }
    let mut canonical = fs::canonicalize(ancestor)?;
    for name in missing.into_iter().rev() {
        canonical.push(name);
    }
    Ok(canonical)
}

pub(crate) fn ordinary_target_lease(
    worker: Arc<WorkerStore>,
    path: &Path,
) -> Result<TargetWriteLease, ApiError> {
    let mut lease = staged_ordinary_target_lease(worker, path)?;
    lease.mutation_started();
    Ok(lease)
}

pub(crate) fn staged_ordinary_target_lease(
    worker: Arc<WorkerStore>,
    path: &Path,
) -> Result<TargetWriteLease, ApiError> {
    let target = canonical_target(path)?;
    let revision = format!("ordinary-write-{}", uuid::Uuid::new_v4());
    let token = uuid::Uuid::new_v4().to_string();
    let inserted_owner = worker
        .register_ordinary_target(&revision, &target, &token)
        .map_err(failure)?;
    Ok(TargetWriteLease {
        worker,
        revision,
        target,
        token,
        inserted_owner,
        discard_owner: true,
        active: true,
    })
}

pub(crate) fn stage_ordinary_targets(
    worker: Option<Arc<WorkerStore>>,
    paths: &[PathBuf],
) -> Result<Vec<TargetWriteLease>, ApiError> {
    let Some(worker) = worker else {
        return Ok(Vec::new());
    };
    let targets = paths
        .iter()
        .map(|path| canonical_target(path))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    targets
        .into_iter()
        .map(|target| staged_ordinary_target_lease(worker.clone(), &target))
        .collect()
}
fn fingerprint(path: &Path) -> Result<Option<String>, ApiError> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(Some(format!("{:x}", digest.finalize())))
}
fn private_directory(path: &Path) -> Result<(), ApiError> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub(crate) fn track_view(path: &Path) -> Result<Value, ApiError> {
    let mut track =
        serde_json::to_value(read_track_metadata(path)?).map_err(|e| failure(e.to_string()))?;
    match try_read_extra_tags(path) {
        Ok(tags) => track["extraTags"] = json!(tags),
        Err(error) => track["extraTagsError"] = json!(error.to_string()),
    }
    Ok(track)
}
impl ReviewJournal {
    pub(crate) fn record_candidate(&self, candidate: &super::auto_tag::AlbumCandidate) -> bool {
        let trusted_proposal = candidate.validated_assisted_genre_proposal_hash();
        let candidate = serde_json::to_value(candidate).unwrap_or(Value::Null);
        let mut data = self.data.lock().unwrap();
        data.detail.result["candidate"] = candidate;
        if let Some(proposal) = trusted_proposal {
            data.trusted_assisted_genre_proposals.insert(proposal);
            true
        } else {
            false
        }
    }

    pub fn evidence(&self, key: &str, value: Value) {
        let mut data = self.data.lock().unwrap();
        data.detail.result[key] = value;
    }
    pub fn event(&self, kind: &str, message: &str, value: Option<&Value>) {
        let mut data = self.data.lock().unwrap();
        if !data.detail.result["events"].is_array() {
            data.detail.result["events"] = json!([]);
        }
        data.detail.result["events"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind": kind, "message": message, "detail": value}));
    }
    pub fn report_error(&self, message: String) {
        self.data.lock().unwrap().detail.errors.push(message);
    }
    pub(crate) fn is_assisted_genre_patch(&self, genre: &str) -> Result<bool, ApiError> {
        let data = self.data.lock().unwrap();
        let Some(assisted) = data.detail.result["candidate"].get("assisted_genre") else {
            return Ok(false);
        };
        if assisted["value"].as_str() != Some(genre) {
            return Ok(false);
        }
        let proposal = assisted["proposal_sha256"]
            .as_str()
            .filter(|proposal| proposal.len() == 64)
            .ok_or_else(|| failure("Assisted genre proposal is not source-validated"))?;
        if !data.trusted_assisted_genre_proposals.contains(proposal) {
            return Err(failure("Assisted genre proposal is not source-validated"));
        }
        let request_input = assisted
            .get("request_input")
            .filter(|input| input.is_object())
            .ok_or_else(|| failure("Assisted genre request input is unavailable"))?;
        let expected = assisted["input_sha256"]
            .as_str()
            .ok_or_else(|| failure("Assisted genre request hash is unavailable"))?;
        let observed = format!("{:x}", Sha256::digest(request_input.to_string().as_bytes()));
        if observed != expected {
            return Err(failure("Assisted genre request input hash does not match"));
        }
        Ok(true)
    }
    fn record_assisted_genre_obligation(
        &self,
        data: &ReviewData,
        path: &Path,
        patch: &Value,
    ) -> Result<bool, ApiError> {
        let Some(assisted) = data.detail.result["candidate"].get("assisted_genre") else {
            return Ok(false);
        };
        let Some(genre) = assisted["value"].as_str() else {
            return Ok(false);
        };
        if patch["genre"].as_str() != Some(genre) {
            return Ok(false);
        }
        let proposal = assisted["proposal_sha256"]
            .as_str()
            .filter(|proposal| proposal.len() == 64)
            .ok_or_else(|| failure("Assisted genre proposal is not source-validated"))?;
        if !data.trusted_assisted_genre_proposals.contains(proposal) {
            return Err(failure("Assisted genre proposal is not source-validated"));
        }
        let request_input = assisted
            .get("request_input")
            .filter(|input| input.is_object())
            .ok_or_else(|| failure("Assisted genre request input is unavailable"))?;
        let request_input_sha256 = assisted["input_sha256"]
            .as_str()
            .ok_or_else(|| failure("Assisted genre request hash is unavailable"))?;
        let observed_sha256 = format!("{:x}", Sha256::digest(request_input.to_string().as_bytes()));
        if observed_sha256 != request_input_sha256 {
            return Err(failure("Assisted genre request input hash does not match"));
        }
        let before = track_view(path)?
            .get("genre")
            .cloned()
            .unwrap_or(Value::Null);
        if before
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
        {
            let Some(recovery) = &self.recovery else {
                return Ok(false);
            };
            let obligations = recovery.field_review_obligations(path).map_err(failure)?;
            return Ok(obligations.iter().any(|obligation| {
                obligation.field == "genre"
                    && obligation.revision == data.detail.id
                    && obligation.after == Value::String(genre.to_string())
                    && obligation.state == "open"
            }));
        }
        let recovery = self
            .recovery
            .as_ref()
            .ok_or_else(|| failure("Assisted genre writes require durable review storage"))?;
        recovery
            .record_field_review_obligation(
                path,
                "genre",
                &data.detail.id,
                &before,
                &Value::String(genre.to_string()),
                &json!({
                    "kind":"assisted_inference",
                    "grounded":assisted["source_observation"]["origin"] == "discogs",
                    "sourceMatch":"exact_phrase_in_source_text",
                    "sourceObservation":assisted["source_observation"],
                    "proposalSha256":proposal,
                    "model":assisted["model"],
                    "rawValue":assisted["raw_value"],
                    "appliedValue":genre,
                    "requestInput":request_input,
                    "requestInputSha256":request_input_sha256
                }),
                &[assisted["source_observation"]["evidenceId"]
                    .as_str()
                    .ok_or_else(|| failure("Assisted genre source evidence is unavailable"))?
                    .to_string()],
            )
            .map_err(failure)?;
        Ok(true)
    }
    fn image(
        &self,
        data: &mut ReviewData,
        bytes: &[u8],
        label: String,
        source: &str,
    ) -> Result<ReviewArtwork, ApiError> {
        let id = format!("{:x}", Sha256::digest(bytes));
        let path = self.files.0.join(&id);
        if !path.exists() {
            fs::write(&path, bytes)?;
        }
        data.images.insert(id.clone(), path);
        let dimensions =
            image::load_from_memory(bytes).map(|image| (image.width(), image.height()));
        Ok(ReviewArtwork {
            id,
            label,
            source: source.into(),
            width: dimensions.as_ref().ok().map(|d| d.0),
            height: dimensions.as_ref().ok().map(|d| d.1),
            error: dimensions.err().map(|e| e.to_string()),
        })
    }
    fn view(&self, data: &mut ReviewData) -> Result<ReviewView, ApiError> {
        let album = PathBuf::from(&data.detail.album_path);
        let detail = read_album(&album)?;
        let mut view = ReviewView::default();
        for track in detail.tracks {
            let path = Path::new(&track.path);
            match track_view(path) {
                Ok(track) => view.tracks.push(track),
                Err(error) => {
                    view.errors.push(format!("{}: {error}", path.display()));
                    view.tracks.push(json!(track));
                }
            }
            match lofty::read_from_path(path) {
                Ok(tagged) => {
                    for (index, picture) in tagged
                        .tags()
                        .iter()
                        .flat_map(|tag| tag.pictures())
                        .enumerate()
                    {
                        match self.image(
                            data,
                            picture.data(),
                            format!(
                                "{} · embedded image {}",
                                path.file_name().unwrap_or_default().to_string_lossy(),
                                index + 1
                            ),
                            "embedded",
                        ) {
                            Ok(image) => view.artworks.push(image),
                            Err(error) => view.errors.push(error.to_string()),
                        }
                    }
                }
                Err(error) if track.has_cover => view
                    .errors
                    .push(format!("{} artwork: {error}", path.display())),
                Err(_) => {}
            }
        }
        let mut paths = fs::read_dir(&album)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && matches!(
                        p.extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .as_str(),
                        "jpg" | "jpeg" | "png" | "webp"
                    )
            })
            .collect::<Vec<_>>();
        if let Some(parent) = album.parent() {
            for name in ["artist.jpg", "artist.png"] {
                let path = parent.join(name);
                if path.is_file() {
                    paths.push(path);
                }
            }
        }
        paths.sort();
        for path in paths {
            match fs::read(&path).map_err(ApiError::from).and_then(|bytes| {
                self.image(
                    data,
                    &bytes,
                    path.to_string_lossy().into_owned(),
                    if path.parent() == album.parent() {
                        "artist"
                    } else {
                        "external"
                    },
                )
            }) {
                Ok(image) => view.artworks.push(image),
                Err(error) => view.errors.push(format!("{}: {error}", path.display())),
            }
        }
        Ok(view)
    }
    /// Caller holds the shared queue's exclusive coordination guard. A failed
    /// backup aborts before mutation; even failed writes receive post-state evidence.
    pub fn mutate<T>(
        &self,
        path: &Path,
        write: impl FnOnce() -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        self.mutate_inner(path, None, write)
    }

    /// Persists a matching assisted genre obligation and write intent before
    /// invoking the existing queued writer. Other patches retain session-only
    /// review recovery.
    pub fn mutate_with_patch<T>(
        &self,
        path: &Path,
        patch: &Value,
        write: impl FnOnce() -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        self.mutate_inner(path, Some(patch), write)
    }

    fn mutate_inner<T>(
        &self,
        path: &Path,
        patch: Option<&Value>,
        write: impl FnOnce() -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let mut data = self.data.lock().unwrap();
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(failure(format!(
                "Cannot journal symlink: {}",
                path.display()
            )));
        }
        let before = fingerprint(path)?;
        let target = canonical_target(path)?;
        if let Some(file) = data.files.iter().find(|file| file.target == target) {
            if !file.verified
                || file.after != before
                || self
                    .owners
                    .lock()
                    .unwrap()
                    .get(&target)
                    .and_then(|history| history.last())
                    .is_some_and(|owner| owner != &data.detail.id)
            {
                return Err(failure("File changed or belongs to a newer review"));
            }
        }
        let mut target_lease = if let Some(recovery) = &self.recovery {
            let token = uuid::Uuid::new_v4().to_string();
            let inserted_owner = recovery
                .register_review_target(&data.detail.id, &target, &token)
                .map_err(failure)?;
            Some(TargetWriteLease {
                worker: Arc::clone(recovery),
                revision: data.detail.id.clone(),
                target: target.clone(),
                token,
                inserted_owner,
                discard_owner: true,
                active: true,
            })
        } else {
            None
        };
        let assisted_genre_write = patch
            .map(|patch| self.record_assisted_genre_obligation(&data, path, patch))
            .transpose()?
            .unwrap_or(false);
        let durable_intent = if assisted_genre_write {
            let intent_id = format!("{}::{}", data.detail.id, uuid::Uuid::new_v4());
            let recovery = self
                .recovery
                .as_ref()
                .ok_or_else(|| failure("Assisted genre writes require durable review storage"))?;
            let patch = patch.expect("assisted genre write has a patch");
            let intent = recovery
                .prepare_write_intent(path, &intent_id, patch)
                .map_err(failure)?;
            Some((intent_id, intent))
        } else {
            None
        };
        let index = if let Some(index) = data.files.iter().position(|f| f.target == target) {
            index
        } else {
            let backup = if let Some((_, intent)) = durable_intent.as_ref() {
                intent.backup_path.clone()
            } else if before.is_some() {
                let backup = self.files.0.join(uuid::Uuid::new_v4().to_string());
                fs::copy(path, &backup)?;
                fs::File::open(&backup)?.sync_all()?;
                if fingerprint(&backup)? != before || fingerprint(path)? != before {
                    return Err(failure("File changed while preparing recovery backup"));
                }
                Some(backup)
            } else {
                None
            };
            if let Ok(track) = track_view(path) {
                if let Some(row) = data
                    .detail
                    .before
                    .tracks
                    .iter_mut()
                    .find(|row| row["path"].as_str() == path.to_str())
                {
                    *row = track;
                }
            }
            let label = path.to_string_lossy().into_owned();
            if super::library::is_audio_file(path) {
                let prefix = format!(
                    "{} · embedded image ",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                data.detail
                    .before
                    .artworks
                    .retain(|art| !art.label.starts_with(&prefix));
                if let Ok(tagged) = lofty::read_from_path(path) {
                    for (index, picture) in tagged
                        .tags()
                        .iter()
                        .flat_map(|tag| tag.pictures())
                        .enumerate()
                    {
                        let art = self.image(
                            &mut data,
                            picture.data(),
                            format!("{prefix}{}", index + 1),
                            "embedded",
                        )?;
                        data.detail.before.artworks.push(art);
                    }
                }
            } else {
                data.detail.before.artworks.retain(|art| art.label != label);
                if before.is_some() {
                    let bytes = fs::read(path)?;
                    let source = if path.parent() == Path::new(&data.detail.album_path).parent() {
                        "artist"
                    } else {
                        "external"
                    };
                    let art = self.image(&mut data, &bytes, label, source)?;
                    data.detail.before.artworks.push(art);
                }
            }
            data.files.push(FileChange {
                path: path.to_path_buf(),
                target: target.clone(),
                backup,
                before: before.clone(),
                after: before.clone(),
                restored: false,
                verified: false,
                tracked: false,
                restoring: false,
                intent_outcomes: Vec::new(),
                original_mode: file_mode(path),
            });
            data.files.len() - 1
        };
        if let (Some(recovery), Some((intent_id, _))) = (&self.recovery, &durable_intent) {
            recovery
                .supersede_prior_write_intents(path, &format!("{}::", data.detail.id), intent_id)
                .map_err(failure)?;
            recovery
                .begin_write_intent(path, intent_id)
                .map_err(failure)?;
        }
        if let Some(lease) = &mut target_lease {
            lease.discard_owner = false;
        }
        let result = write();
        match fingerprint(path) {
            Ok(after) => {
                data.files[index].after = after;
                data.files[index].verified = true;
            }
            Err(error) => {
                data.detail.errors.push(format!(
                    "Post-write verification failed for {}: {error}",
                    path.display()
                ));
                return Err(error);
            }
        }
        let write_succeeded = result.is_ok();
        if let Some(lease) = &mut target_lease {
            lease.discard_owner = !write_succeeded && before == data.files[index].after;
        }
        data.files[index].tracked =
            write_succeeded || data.files[index].before != data.files[index].after;
        if data.files[index].tracked {
            let mut owners = self.owners.lock().unwrap();
            let history = owners.entry(data.files[index].target.clone()).or_default();
            if history.last() != Some(&data.detail.id) {
                history.push(data.detail.id.clone());
            }
        }
        if let (Some(recovery), Some((intent_id, _))) = (&self.recovery, &durable_intent) {
            data.files[index]
                .intent_outcomes
                .push((intent_id.clone(), write_succeeded));
            if let Err(error) =
                recovery.record_write_intent_applied(path, &intent_id, write_succeeded)
            {
                data.detail.errors.push(format!(
                    "Write recovery state could not be persisted for {}: {error}",
                    path.display()
                ));
                return Err(failure(error));
            }
        }
        if let Some(lease) = &mut target_lease {
            lease.release()?;
        }
        result
    }
}
impl ReviewStore {
    /// Creates a review store that can persist assisted genre writes under the
    /// app's existing data directory.
    pub fn new_in(data_dir: PathBuf) -> Result<Self, ApiError> {
        Self::new_with_worker(WorkerStore::new_in(data_dir))
    }

    /// Shares the app's initialized worker database for assisted field receipts.
    pub fn new_with_worker(recovery: WorkerStore) -> Result<Self, ApiError> {
        if !recovery.initialize() {
            return Err(failure("Could not initialize durable auto-tag recovery"));
        }
        Ok(Self {
            records: Mutex::new(Vec::new()),
            files: Arc::new(SessionFiles::default()),
            owners: Arc::new(Mutex::new(HashMap::new())),
            recovery: Some(Arc::new(recovery)),
        })
    }

    /// Results and their recovery journals are scoped to one auto-tag batch.
    /// A running review must finish before its batch can be replaced.
    pub fn clear(&self) -> Result<(), ApiError> {
        let mut records = self.records.lock().unwrap();
        if records
            .iter()
            .any(|record| record.data.lock().unwrap().detail.outcome == "running")
        {
            return Err(failure(
                "Cannot clear auto-tag results while tagging is running",
            ));
        }
        records.clear();
        self.owners.lock().unwrap().clear();
        if self.files.0.exists() {
            fs::remove_dir_all(&self.files.0)?;
        }
        Ok(())
    }
    pub fn cleanup(&self) {
        if self.files.0.exists() {
            if let Err(error) = fs::remove_dir_all(&self.files.0) {
                tracing::warn!(%error, "review backup cleanup failed");
            }
        }
    }
    pub fn begin(&self, id: &str, album: &Path) -> Result<Arc<ReviewJournal>, ApiError> {
        private_directory(&self.files.0)?;
        let earlier_attempts = self.records.lock().unwrap().iter().filter_map(|record| {
            let data = record.data.lock().unwrap();
            if data.detail.album_path != album.to_string_lossy() { return None; }
            let mut result = data.detail.result.clone();
            if let Some(object) = result.as_object_mut() { object.remove("earlierAttempts"); }
            Some(json!({"reviewId": data.detail.id, "outcome": data.detail.outcome, "result": result}))
        }).collect::<Vec<_>>();
        let journal = Arc::new(ReviewJournal {
            data: Mutex::new(ReviewData {
                detail: ReviewDetail {
                    id: id.into(),
                    album_path: album.to_string_lossy().into_owned(),
                    outcome: "running".into(),
                    decision: "pending".into(),
                    result: json!({"earlierAttempts": earlier_attempts}),
                    before: ReviewView::default(),
                    after: ReviewView::default(),
                    can_revert: false,
                    errors: vec![],
                    field_reviews: vec![],
                    recovered: false,
                    recovery_required: false,
                    recovery_files: vec![],
                },
                files: vec![],
                images: HashMap::new(),
                trusted_assisted_genre_proposals: HashSet::new(),
            }),
            files: Arc::clone(&self.files),
            owners: Arc::clone(&self.owners),
            recovery: self.recovery.as_ref().map(Arc::clone),
        });
        {
            let mut data = journal.data.lock().unwrap();
            data.detail.before = journal.view(&mut data)?;
        }
        self.records.lock().unwrap().push(Arc::clone(&journal));
        Ok(journal)
    }
    fn journal(&self, id: &str) -> Result<Arc<ReviewJournal>, ApiError> {
        self.find_journal(id)
            .ok_or_else(|| failure("Review is unavailable in this session"))
    }
    fn find_journal(&self, id: &str) -> Option<Arc<ReviewJournal>> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.data.lock().unwrap().detail.id == id)
            .cloned()
    }
    pub fn finish(&self, id: &str, outcome: &str, result: Value) -> Result<(), ApiError> {
        let journal = self.journal(id)?;
        let mut data = journal.data.lock().unwrap();
        data.detail.outcome = outcome.into();
        if let Some(fields) = result.as_object() {
            for (key, value) in fields {
                data.detail.result[key] = value.clone();
            }
        }
        let readback_succeeded = match journal.view(&mut data) {
            Ok(view) => {
                data.detail.after = view;
                true
            }
            Err(error) => {
                data.detail
                    .after
                    .errors
                    .push(format!("Readback failed: {error}"));
                false
            }
        };
        if readback_succeeded {
            if let Some(recovery) = &self.recovery {
                let latest = data
                    .files
                    .iter()
                    .filter_map(|file| {
                        file.intent_outcomes
                            .last()
                            .map(|(run_id, success)| (file.path.clone(), run_id.clone(), *success))
                    })
                    .collect::<Vec<_>>();
                for (path, run_id, success) in latest {
                    if success {
                        if let Err(error) = recovery.verify_write_intent(&path, &run_id) {
                            data.detail.errors.push(format!(
                                "Independent write readback validation failed for {}: {error}",
                                path.display()
                            ));
                        }
                    }
                }
            }
        }
        data.detail.can_revert = data
            .files
            .iter()
            .any(|f| !f.restored && (f.tracked || !f.verified));
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<ReviewDetail, ApiError> {
        let journal = self.find_journal(id);
        let Some(worker) = &self.recovery else {
            return Ok(self.journal(id)?.data.lock().unwrap().detail.clone());
        };
        if let Some((album, _, status)) = worker.reversal_manifest(id).map_err(failure)? {
            let pending = status == "pending";
            if pending || journal.is_none() {
                return Ok(ReviewDetail {
                    id: id.into(),
                    album_path: album,
                    outcome: if pending { "needs_review" } else { "applied" }.into(),
                    decision: if pending { "pending" } else { "reverted" }.into(),
                    result: Value::Null,
                    before: ReviewView::default(),
                    after: ReviewView::default(),
                    can_revert: pending,
                    errors: vec![],
                    field_reviews: vec![],
                    recovered: true,
                    recovery_required: pending,
                    recovery_files: vec![],
                });
            }
        }
        let fields = worker.verified_field_reviews(Some(id)).map_err(failure)?;
        let conflicts = worker
            .write_recovery_conflicts()
            .map_err(failure)?
            .into_iter()
            .filter_map(|(revision, path)| (revision == id).then_some(path))
            .collect::<Vec<_>>();
        if let Some(journal) = journal {
            let data = journal.data.lock().unwrap();
            let mut detail = data.detail.clone();
            detail.field_reviews = fields
                .into_iter()
                .map(|field| {
                    let mut field = ReviewField::from(field);
                    if let Some(file) = data.files.iter().find(|file| file.target == field.path) {
                        field.path = file.path.clone();
                    }
                    field
                })
                .collect();
            add_recovery_conflicts(worker, &mut detail, conflicts)?;
            return Ok(detail);
        }
        recovered_review(worker, id, fields, conflicts)?
            .ok_or_else(|| failure("Review is unavailable in this session"))
    }
    pub fn list(&self) -> Result<Vec<ReviewDetail>, ApiError> {
        let mut reviews: Vec<_> = self
            .records
            .lock()
            .unwrap()
            .iter()
            .rev()
            .map(|r| r.data.lock().unwrap().detail.clone())
            .collect();
        let Some(worker) = &self.recovery else {
            return Ok(reviews);
        };
        let mut recovered = BTreeMap::<String, (Vec<_>, Vec<_>)>::new();
        for field in worker.verified_field_reviews(None).map_err(failure)? {
            recovered
                .entry(field.revision.clone())
                .or_default()
                .0
                .push(field);
        }
        for (revision, path) in worker.write_recovery_conflicts().map_err(failure)? {
            recovered.entry(revision).or_default().1.push(path);
        }
        for (id, (fields, conflicts)) in recovered {
            if let Some(detail) = reviews.iter_mut().find(|detail| detail.id == id) {
                add_recovery_conflicts(worker, detail, conflicts)?;
            } else if let Some(detail) = recovered_review(worker, &id, fields, conflicts)? {
                reviews.push(detail);
            }
        }
        for id in worker.pending_reversals().map_err(failure)? {
            let detail = self.get(&id)?;
            if let Some(existing) = reviews.iter_mut().find(|review| review.id == id) {
                *existing = detail;
            } else {
                reviews.push(detail);
            }
        }
        Ok(reviews)
    }
    pub fn keep(&self, id: &str) -> Result<ReviewDetail, ApiError> {
        let review = self.get(id)?;
        if review.outcome == "running" || review.decision == "reverted" || review.recovery_required
        {
            return Err(failure("This review cannot be marked kept"));
        }
        if let Some(worker) = &self.recovery {
            worker
                .acknowledge_verified_field_reviews(id)
                .map_err(failure)?;
        }
        if let Some(journal) = self.find_journal(id) {
            journal.data.lock().unwrap().detail.decision = "kept".into();
        }
        self.get(id)
    }
    pub fn keep_recovery_file(
        &self,
        id: &str,
        path: &Path,
        expected_sha256: &str,
    ) -> Result<Option<ReviewDetail>, ApiError> {
        let worker = self
            .recovery
            .as_ref()
            .ok_or_else(|| failure("Recovery storage unavailable"))?;
        worker
            .keep_recovery_file(id, path, expected_sha256)
            .map_err(failure)?;
        if self.find_journal(id).is_some() || self.list()?.iter().any(|detail| detail.id == id) {
            Ok(Some(self.get(id)?))
        } else {
            Ok(None)
        }
    }
    pub fn revert(&self, id: &str) -> Result<ReviewDetail, ApiError> {
        self.revert_with_limit(id, None)
    }

    fn revert_with_limit(&self, id: &str, limit: Option<usize>) -> Result<ReviewDetail, ApiError> {
        let Some(worker) = &self.recovery else {
            return self.revert_session(id);
        };
        let existing = worker.reversal_manifest(id).map_err(failure)?;
        if let Some((_, _, status)) = &existing {
            if status == "completed" {
                return self.get(id);
            }
            if status != "pending" {
                return Err(failure("Invalid reversal state"));
            }
        }
        if existing.is_none() {
            let journal = self.journal(id)?;
            let data = journal.data.lock().unwrap();
            if data.detail.outcome == "running" || !data.detail.can_revert {
                return Err(failure("No completed changes to revert"));
            }
            let owners = self.owners.lock().unwrap();
            let mut files = Vec::new();
            for file in data
                .files
                .iter()
                .filter(|file| !file.restored && (file.tracked || !file.verified))
            {
                if canonical_target(&file.path)? != file.target
                    || fs::symlink_metadata(&file.path).is_ok_and(|m| m.file_type().is_symlink())
                    || !file.verified
                    || fingerprint(&file.path)? != file.after
                    || owners
                        .get(&file.target)
                        .and_then(|history| history.last())
                        .is_some_and(|owner| owner != id)
                {
                    return Err(failure(format!("Cannot revert: {} changed after this auto-tag run, or could not be verified. Undo newer changes first.", file.path.display())));
                }
                let backup = match (&file.before, &file.backup) {
                    (Some(expected), Some(source)) => {
                        if fingerprint(source)?.as_deref() != Some(expected) {
                            return Err(failure("Recovery backup is missing or damaged"));
                        }
                        Some(
                            worker
                                .copy_reversal_backup(source, expected)
                                .map_err(failure)?,
                        )
                    }
                    (None, None) => None,
                    _ => return Err(failure("Recovery backup is missing or damaged")),
                };
                files.push(ReversalFile {
                    path: file.path.clone(),
                    target: file.target.clone(),
                    before: file.before.clone(),
                    after: file.after.clone(),
                    backup,
                    original_mode: file.original_mode,
                    phase: "pending".into(),
                });
            }
            if files.is_empty() {
                return Err(failure("No completed changes to revert"));
            }
            let manifest = ReversalManifest { files };
            worker
                .create_reversal_manifest(
                    id,
                    &data.detail.album_path,
                    &serde_json::to_value(&manifest).map_err(|e| failure(e.to_string()))?,
                )
                .map_err(failure)?;
        }
        self.resume_reversal(id, limit, || {})
    }

    fn resume_reversal(
        &self,
        id: &str,
        limit: Option<usize>,
        before_completion: impl FnOnce(),
    ) -> Result<ReviewDetail, ApiError> {
        let worker = self
            .recovery
            .as_ref()
            .ok_or_else(|| failure("Recovery storage unavailable"))?;
        let token = uuid::Uuid::new_v4().to_string();
        worker.claim_reversal(id, &token).map_err(failure)?;
        let result = (|| -> Result<(), ApiError> {
            let (_, value, status) = worker
                .reversal_manifest(id)
                .map_err(failure)?
                .ok_or_else(|| failure("Reversal is unavailable"))?;
            if status != "pending" {
                return Err(failure("Reversal is already complete"));
            }
            let mut manifest: ReversalManifest =
                serde_json::from_value(value).map_err(|e| failure(e.to_string()))?;
            for file in &manifest.files {
                verify_reversal_file(file)?;
            }
            let mut restored = 0usize;
            for index in 0..manifest.files.len() {
                if manifest.files[index].phase == "restored" {
                    continue;
                }
                let file = &manifest.files[index];
                let already_restored =
                    file.phase == "restoring" && fingerprint(&file.path)? == file.before;
                if !already_restored {
                    manifest.files[index].phase = "restoring".into();
                    worker
                        .update_reversal(
                            id,
                            &token,
                            &serde_json::to_value(&manifest).map_err(|e| failure(e.to_string()))?,
                            false,
                        )
                        .map_err(failure)?;
                    let file = &manifest.files[index];
                    verify_reversal_file(file)?;
                    if file.before != file.after {
                        if let Some(backup) = &file.backup {
                            let staging = file.path.with_file_name(format!(
                                ".soundrobe-restore-{}",
                                uuid::Uuid::new_v4()
                            ));
                            let operation = (|| -> Result<(), ApiError> {
                                let mut input = fs::File::open(backup)?;
                                let mut output = fs::OpenOptions::new()
                                    .write(true)
                                    .create_new(true)
                                    .open(&staging)?;
                                std::io::copy(&mut input, &mut output)?;
                                restore_mode(&staging, file.original_mode)?;
                                output.sync_all()?;
                                if fingerprint(&staging)? != file.before {
                                    return Err(failure("Restore staging validation failed"));
                                }
                                verify_reversal_file(file)?;
                                replace_file_atomic(&staging, &file.path)?;
                                Ok(())
                            })();
                            if staging.exists() {
                                let _ = fs::remove_file(staging);
                            }
                            operation?;
                        } else {
                            fs::remove_file(&file.path)?;
                        }
                    }
                }
                restored += 1;
                if limit == Some(restored) {
                    #[cfg(test)]
                    if std::env::var_os("SOUNDROBE_TEST_REVERSAL_CRASH").is_some() {
                        std::process::exit(70);
                    }
                    return Err(failure("Injected interruption after media replacement"));
                }
                let file = &manifest.files[index];
                if fingerprint(&file.path)? != file.before {
                    return Err(failure(format!(
                        "Restore readback failed: {}",
                        file.path.display()
                    )));
                }
                WorkerStore::sync_reversal_parent(&file.path).map_err(failure)?;
                manifest.files[index].phase = "restored".into();
                worker
                    .update_reversal(
                        id,
                        &token,
                        &serde_json::to_value(&manifest).map_err(|e| failure(e.to_string()))?,
                        false,
                    )
                    .map_err(failure)?;
            }
            before_completion();
            for file in &manifest.files {
                verify_reversal_file(file)?;
            }
            worker
                .update_reversal(
                    id,
                    &token,
                    &serde_json::to_value(&manifest).map_err(|e| failure(e.to_string()))?,
                    true,
                )
                .map_err(failure)?;
            Ok(())
        })();
        let released = worker.release_reversal(id, &token).map_err(failure);
        match (result, released) {
            (Err(error), Err(release)) => {
                return Err(failure(format!(
                    "{error}; reversal ownership could not be released: {release}"
                )))
            }
            (Err(error), _) | (_, Err(error)) => return Err(error),
            _ => {}
        }
        if let Some(journal) = self.find_journal(id) {
            let mut data = journal.data.lock().unwrap();
            data.detail.decision = "reverted".into();
            data.detail.can_revert = false;
            let mut owners = self.owners.lock().unwrap();
            for file in &mut data.files {
                file.restored = true;
                if let Some(history) = owners.get_mut(&file.target) {
                    if history.last().is_some_and(|owner| owner == id) {
                        history.pop();
                    }
                }
            }
        }
        let detail = self.get(id)?;
        super::covers::cover_cache_invalidate(&detail.album_path);
        Ok(detail)
    }

    fn revert_session(&self, id: &str) -> Result<ReviewDetail, ApiError> {
        let journal = self.journal(id)?;
        let mut data = journal.data.lock().unwrap();
        if data.detail.decision == "reverted" {
            return Ok(data.detail.clone());
        }
        if data.detail.outcome == "running" || !data.detail.can_revert {
            return Err(failure("No completed changes to revert"));
        }
        let mut owners = self.owners.lock().unwrap();
        // A replace may have succeeded even when its readback failed. On retry,
        // recognize only the exact original, never an arbitrary external edit.
        for file in data
            .files
            .iter_mut()
            .filter(|file| !file.restored && file.restoring)
        {
            if canonical_target(&file.path)? == file.target
                && fingerprint(&file.path)? == file.before
            {
                if let Some(recovery) = &self.recovery {
                    recovery
                        .mark_review_write_intents_reverted(&file.path, &format!("{id}::"))
                        .map_err(failure)?;
                }
                file.restored = true;
                if let Some(history) = owners.get_mut(&file.target) {
                    if history.last().is_some_and(|owner| owner == id) {
                        history.pop();
                    }
                }
            }
        }
        for file in data
            .files
            .iter()
            .filter(|f| !f.restored && (f.tracked || !f.verified))
        {
            if canonical_target(&file.path)? != file.target
                || fs::symlink_metadata(&file.path).is_ok_and(|m| m.file_type().is_symlink())
                || !file.verified
                || fingerprint(&file.path)? != file.after
                || owners
                    .get(&file.target)
                    .and_then(|h| h.last())
                    .is_some_and(|owner| owner != id)
            {
                return Err(failure(format!("Cannot revert: {} changed after this auto-tag run, or could not be verified. Undo newer changes first.", file.path.display())));
            }
            if let Some(backup) = &file.backup {
                if fingerprint(backup)? != file.before {
                    return Err(failure("Recovery backup is missing or damaged"));
                }
            }
        }
        if let Some(recovery) = &self.recovery {
            for file in data
                .files
                .iter()
                .filter(|file| !file.restored && file.tracked)
            {
                recovery
                    .mark_review_write_intents_reverting(&file.path, &format!("{id}::"))
                    .map_err(failure)?;
            }
        }
        for file in data.files.iter_mut().filter(|f| !f.restored && f.tracked) {
            if file.before != file.after {
                file.restoring = true;
                if let Some(backup) = &file.backup {
                    let staging = file
                        .path
                        .with_file_name(format!(".soundrobe-restore-{}", uuid::Uuid::new_v4()));
                    let restored = (|| -> Result<(), ApiError> {
                        fs::copy(backup, &staging)?;
                        fs::File::open(&staging)?.sync_all()?;
                        if fingerprint(&staging)? != file.before {
                            return Err(failure("Restore staging validation failed"));
                        }
                        replace_file_atomic(&staging, &file.path)?;
                        Ok(())
                    })();
                    if staging.exists() {
                        let _ = fs::remove_file(&staging);
                    }
                    restored?;
                } else {
                    fs::remove_file(&file.path)?;
                }
            }
            if fingerprint(&file.path)? != file.before {
                return Err(failure(format!(
                    "Restore readback failed: {}",
                    file.path.display()
                )));
            }
            if let Some(recovery) = &self.recovery {
                recovery
                    .mark_review_write_intents_reverted(&file.path, &format!("{id}::"))
                    .map_err(failure)?;
            }
            file.restored = true;
            if let Some(history) = owners.get_mut(&file.target) {
                if history.last().is_some_and(|owner| owner == id) {
                    history.pop();
                }
            }
        }
        data.detail.decision = "reverted".into();
        data.detail.can_revert = false;
        super::covers::cover_cache_invalidate(&data.detail.album_path);
        drop(owners);
        drop(data);
        self.get(id)
    }
}

fn add_recovery_conflicts(
    worker: &WorkerStore,
    detail: &mut ReviewDetail,
    mut paths: Vec<PathBuf>,
) -> Result<(), ApiError> {
    paths.sort();
    paths.dedup();
    detail.recovery_required = !paths.is_empty();
    detail.recovery_files = paths
        .iter()
        .map(|path| {
            let digest = WorkerStore::recovery_file_digest(path);
            let metadata = track_view(path).ok();
            let stable = digest.is_some() && digest == WorkerStore::recovery_file_digest(path);
            let can_keep = worker
                .recovery_file_can_be_kept(&detail.id, path)
                .map_err(failure)?;
            Ok(RecoveryFile {
                path: path.clone(),
                current_sha256: if stable && can_keep { digest } else { None },
                current_metadata: if stable { metadata } else { None },
                unavailable_reason: if !can_keep {
                    Some("This interrupted revert needs further recovery.")
                } else if !stable {
                    Some("Current file is unavailable or changed while loading.")
                } else {
                    None
                },
            })
        })
        .collect::<Result<_, ApiError>>()?;
    detail.errors.extend(paths.into_iter().map(|path| {
        format!(
            "Could not verify the interrupted write to {}",
            path.display()
        )
    }));
    Ok(())
}

fn recovered_review(
    worker: &WorkerStore,
    id: &str,
    fields: Vec<FieldReviewObligation>,
    conflicts: Vec<PathBuf>,
) -> Result<Option<ReviewDetail>, ApiError> {
    let Some(path) = fields
        .first()
        .map(|field| &field.path)
        .or_else(|| conflicts.first())
    else {
        return Ok(None);
    };
    let album = worker
        .album_for_write_review(id)
        .map_err(failure)?
        .unwrap_or_else(|| path.parent().unwrap_or(path).to_path_buf());
    let pending = !conflicts.is_empty() || fields.iter().any(|field| field.state == "open");
    let field_reviews: Vec<ReviewField> = fields.into_iter().map(Into::into).collect();
    let mut tracks = BTreeMap::new();
    for field in &field_reviews {
        let (before, after) = tracks
            .entry(&field.path)
            .or_insert_with(|| (json!({"path":field.path}), json!({"path":field.path})));
        before[&field.field] = field.before.clone();
        after[&field.field] = field.after.clone();
    }
    let (before, after) = tracks.into_values().unzip();
    let mut detail = ReviewDetail {
        id: id.into(),
        album_path: album.to_string_lossy().into_owned(),
        outcome: if conflicts.is_empty() {
            "applied"
        } else {
            "needs_review"
        }
        .into(),
        decision: if pending { "pending" } else { "kept" }.into(),
        result: Value::Null,
        before: ReviewView {
            tracks: before,
            ..Default::default()
        },
        after: ReviewView {
            tracks: after,
            ..Default::default()
        },
        can_revert: false,
        errors: vec![],
        field_reviews,
        recovered: true,
        recovery_required: false,
        recovery_files: vec![],
    };
    add_recovery_conflicts(worker, &mut detail, conflicts)?;
    Ok(Some(detail))
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub fn auto_tag_reviews_clear(store: State<'_, ReviewStore>) -> Result<(), ApiError> {
    store.clear()
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn auto_tag_reviews_list(store: State<'_, ReviewStore>) -> Result<Vec<ReviewDetail>, ApiError> {
    store.list()
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn auto_tag_review_get(
    review_id: String,
    store: State<'_, ReviewStore>,
) -> Result<ReviewDetail, ApiError> {
    store.get(&review_id)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn auto_tag_review_keep(
    review_id: String,
    store: State<'_, ReviewStore>,
    queue: State<'_, WriteQueue>,
) -> Result<ReviewDetail, ApiError> {
    queue
        .run_exclusive_checked(async { store.keep(&review_id) })
        .await?
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn auto_tag_review_revert(
    review_id: String,
    store: State<'_, ReviewStore>,
    queue: State<'_, WriteQueue>,
) -> Result<ReviewDetail, ApiError> {
    queue
        .run_exclusive_checked(async { store.revert(&review_id) })
        .await?
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn auto_tag_review_keep_recovery_file(
    review_id: String,
    path: PathBuf,
    expected_sha256: String,
    store: State<'_, ReviewStore>,
    queue: State<'_, WriteQueue>,
) -> Result<Option<ReviewDetail>, ApiError> {
    queue
        .run_exclusive_checked(async {
            store.keep_recovery_file(&review_id, &path, &expected_sha256)
        })
        .await?
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn auto_tag_review_artwork(
    review_id: String,
    artwork_id: String,
    store: State<'_, ReviewStore>,
) -> Result<String, ApiError> {
    let journal = store.journal(&review_id)?;
    let data = journal.data.lock().unwrap();
    let path = data
        .images
        .get(&artwork_id)
        .ok_or_else(|| failure("Artwork unavailable"))?;
    let bytes = fs::read(path)?;
    let format = image::guess_format(&bytes).map_err(|e| failure(e.to_string()))?;
    let mime = format.to_mime_type();
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_genre_server(genre: &str) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let genre = genre.to_string();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 4096];
                let request_end;
                loop {
                    let count = stream.read(&mut chunk).unwrap();
                    assert!(count > 0, "loopback request closed before headers");
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(header_end) =
                        bytes.windows(4).position(|value| value == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&bytes[..header_end]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        let body_end = header_end + 4 + content_length;
                        if bytes.len() >= body_end {
                            request_end = body_end;
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes[..request_end].to_vec()).unwrap();
                let (status, body) = if request.starts_with("GET /discogs/releases/42 ") {
                    ("200 OK", json!({
                        "id":42,
                        "title":"Synthetic Album",
                        "artists":[{"id":7,"name":"Synthetic Artist","anv":""}],
                        "notes":format!("Genre: {genre}"),
                        "genres":[],
                        "styles":[],
                        "tracklist":[{"position":"1","title":"Synthetic Track","duration":"03:00"}]
                    }).to_string())
                } else {
                    assert!(
                        request.starts_with("POST /chat/completions "),
                        "unexpected route: {request}"
                    );
                    (
                        "200 OK",
                        json!({
                            "id":"synthetic-response",
                            "model":"scripted/genre-v1",
                            "choices":[{"finish_reason":"stop","message":{"content":
                                json!({"genre":genre,"confidence":0.99}).to_string()
                            }}],
                            "usage":{"prompt_tokens":10,"completion_tokens":4,"total_tokens":14}
                        })
                        .to_string(),
                    )
                };
                write!(stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()).unwrap();
                stream.flush().unwrap();
            }
        });
        (base, server)
    }

    async fn record_assisted_genre_candidate(
        journal: &ReviewJournal,
        album_path: &Path,
        genre: &str,
    ) -> (Value, String) {
        let (base, server) = synthetic_genre_server(genre);
        let request = crate::commands::auto_tag::LookupRequest {
            path: album_path.to_string_lossy().into_owned(),
            artist_hint: Some("Synthetic Artist".into()),
            album_hint: Some("Synthetic Album".into()),
            tracks: vec![crate::commands::auto_tag::TrackCandidate {
                title: Some("Synthetic Track".into()),
                track_number: Some(1),
                length: Some(180.0),
                ..Default::default()
            }],
            ..Default::default()
        };
        let candidate =
            crate::commands::auto_tag::capture_runtime_assisted_genre_candidate_for_test(
                &request,
                &format!("{base}/discogs"),
                &base,
            )
            .await
            .unwrap();
        server.join().unwrap();
        let assisted = candidate.assisted_genre.as_ref().unwrap();
        let request_input = assisted.request_input.clone();
        let input_sha256 = assisted.input_sha256.clone();
        assert!(
            journal.record_candidate(&candidate),
            "runtime evidence should register the proposal"
        );
        (request_input, input_sha256)
    }

    fn record_forged_assisted_genre_candidate(journal: &ReviewJournal, genre: &str) {
        let request_input = json!({
            "artist":"Synthetic Artist",
            "album":"Synthetic Album",
            "tracks":["Synthetic Track"]
        });
        let input_sha256 = format!("{:x}", Sha256::digest(request_input.to_string().as_bytes()));
        journal.evidence(
            "candidate",
            json!({
                "genre":genre,
                "assisted_genre":{
                    "value":genre,
                    "model":"scripted/offline",
                    "request_input":request_input,
                    "input_sha256":input_sha256
                }
            }),
        );
    }

    #[test]
    fn caller_supplied_genre_hash_does_not_authorize_assisted_write() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let store = ReviewStore::default();
        let journal = store.begin("forged", &root).unwrap();
        record_forged_assisted_genre_candidate(&journal, "Dream pop");

        assert!(
            journal.is_assisted_genre_patch("Dream pop").is_err(),
            "a matching caller-supplied value and hash cannot create write authority"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_assisted_review_mutation_does_not_create_durable_intent() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        let file = root.join("01.bin");
        fs::write(&file, b"before").unwrap();
        let store = ReviewStore::new_in(state.clone()).unwrap();
        let journal = store.begin("run", &root).unwrap();
        journal
            .mutate(&file, || {
                fs::write(&file, b"after")?;
                Ok(())
            })
            .unwrap();
        drop(journal);
        drop(store);

        let worker = crate::state::worker::WorkerStore::new_in(state);
        assert!(worker.initialize());
        assert!(worker.reconcile_write_intents().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn queued_assisted_genre_write_persists_input_and_readback_after_restart() {
        use crate::commands::mutations::{write_track_queued, TrackPatch};
        use crate::state::{worker::WriteRecoveryAction, write_queue::WriteQueue};

        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let track = root.join("01.flac");
        let provider_track = root.join("02.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.flac"),
            &track,
        )
        .unwrap();
        fs::copy(&track, &provider_track).unwrap();
        let payload_before = crate::commands::mutations::track_payload_digest(&track).unwrap();
        let state = root.join("state");
        let worker = WorkerStore::new_in(state.clone());
        assert!(worker.initialize());
        let store = ReviewStore::new_with_worker(worker.clone()).unwrap();
        let journal = store.begin("assisted-run", &root).unwrap();
        let (source_input, _input_sha256) =
            record_assisted_genre_candidate(&journal, &root, "Acoustic Pop").await;

        ACTIVE_REVIEW
            .scope(
                journal,
                write_track_queued(
                    &WriteQueue::default(),
                    track.clone(),
                    serde_json::from_value::<TrackPatch>(json!({"genre":"Acoustic Pop"})).unwrap(),
                ),
            )
            .await
            .unwrap();
        store.finish("assisted-run", "applied", json!({})).unwrap();
        let displayed = store.get("assisted-run").unwrap();
        assert_eq!(displayed.field_reviews.len(), 1);
        assert_eq!(displayed.field_reviews[0].field, "genre");
        assert_eq!(displayed.field_reviews[0].state, "open");
        let provider_journal = store.begin("provider-run", &root).unwrap();
        ACTIVE_REVIEW
            .scope(
                provider_journal,
                write_track_queued(
                    &WriteQueue::default(),
                    provider_track.clone(),
                    serde_json::from_value::<TrackPatch>(json!({"genre":"Pop"})).unwrap(),
                ),
            )
            .await
            .unwrap();
        store.finish("provider-run", "applied", json!({})).unwrap();
        assert_eq!(
            crate::commands::mutations::track_payload_digest(&track).unwrap(),
            payload_before
        );
        drop(store);

        let reopened = crate::state::worker::WorkerStore::new_in(state);
        assert!(reopened.initialize());
        let desktop = ReviewStore::new_with_worker(reopened.clone()).unwrap();
        let recovered = desktop.get("assisted-run").unwrap();
        assert_eq!(recovered.field_reviews.len(), 1);
        assert!(recovered.recovered);
        assert!(
            !recovered.can_revert,
            "only the assisted field survived, not full album undo"
        );
        desktop.clear().unwrap();
        assert_eq!(
            desktop.list().unwrap().len(),
            1,
            "clearing session results retains open field review"
        );
        let reviewed_bytes = fs::read(&track).unwrap();
        write_track_queued(
            &WriteQueue::default(),
            track.clone(),
            serde_json::from_value::<TrackPatch>(json!({"genre":"Manual edit"})).unwrap(),
        )
        .await
        .unwrap();
        assert!(desktop
            .keep("assisted-run")
            .unwrap_err()
            .to_string()
            .contains("File changed"));
        assert_eq!(
            desktop.get("assisted-run").unwrap().field_reviews[0].state,
            "open"
        );
        fs::write(&track, reviewed_bytes).unwrap();
        let kept = desktop.keep("assisted-run").unwrap();
        assert_eq!(kept.field_reviews[0].state, "acknowledged");
        assert!(desktop.list().unwrap().is_empty());
        let obligations = reopened.field_review_obligations(&track).unwrap();
        assert_eq!(obligations.len(), 1);
        assert_eq!(obligations[0].field, "genre");
        assert_eq!(obligations[0].state, "acknowledged");
        assert_eq!(obligations[0].after, json!("Acoustic Pop"));
        assert_eq!(obligations[0].evidence_ids.len(), 1);
        assert_eq!(obligations[0].provenance["grounded"], false);
        assert_eq!(
            obligations[0].provenance["sourceObservation"]["origin"],
            "synthetic_loopback"
        );
        assert_eq!(obligations[0].provenance["requestInput"], source_input);
        assert!(reopened
            .field_review_obligations(&provider_track)
            .unwrap()
            .is_empty());
        let recovery = reopened.reconcile_write_intents().unwrap();
        assert_eq!(recovery.len(), 1, "{recovery:#?}");
        assert_eq!(
            recovery[0].intent.path,
            fs::canonicalize(&track).unwrap(),
            "only the assisted genre write gets a durable recovery intent"
        );
        assert_eq!(recovery[0].action, WriteRecoveryAction::ReceiptRecoverable);
        fs::remove_file(&track).unwrap();
        reopened.reconcile_writes_at_startup().unwrap();
        assert!(desktop.list().unwrap().is_empty());
        assert!(reopened.claim_desktop_run(&root, "new-run").is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn results_reopen_an_interrupted_write_without_claiming_it_was_applied() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let file = root.join("01.bin");
        fs::write(&file, b"before").unwrap();
        let worker = WorkerStore::new_in(root.join("state"));
        assert!(worker.initialize());
        worker
            .prepare_write_intent(&file, "interrupted::attempt", &json!({"genre":"Jazz"}))
            .unwrap();
        worker
            .begin_write_intent(&file, "interrupted::attempt")
            .unwrap();
        fs::write(&file, b"unknown").unwrap();
        worker.reconcile_writes_at_startup().unwrap();

        let store = ReviewStore::new_with_worker(worker.clone()).unwrap();
        let reviews = store.list().unwrap();
        assert_eq!(reviews.len(), 1);
        let review = store.get("interrupted").unwrap();
        assert!(review.recovery_required);
        assert!(review.recovered);
        assert!(!review.can_revert);
        assert!(review.after.tracks.is_empty());
        assert!(review.field_reviews.is_empty());
        assert!(store.keep("interrupted").is_err());
        assert_eq!(fs::read(&file).unwrap(), b"unknown");
        let choice = &review.recovery_files[0];
        assert_eq!(choice.path, fs::canonicalize(&file).unwrap());
        let digest = choice.current_sha256.as_deref().unwrap();
        fs::write(&file, b"new edit").unwrap();
        assert!(store
            .keep_recovery_file("interrupted", &choice.path, digest)
            .is_err());
        assert_eq!(fs::read(&file).unwrap(), b"new edit");
        fs::write(&file, b"unknown").unwrap();
        store
            .keep_recovery_file("interrupted", &choice.path, digest)
            .unwrap();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(fs::read(&file).unwrap(), b"unknown");
        assert_eq!(
            worker.reconcile_write_intents().unwrap()[0].action,
            crate::state::worker::WriteRecoveryAction::KeptCurrent
        );
        assert!(store
            .keep_recovery_file("interrupted", &choice.path, digest)
            .is_err());
        drop(store);
        drop(worker);
        let reopened = ReviewStore::new_in(root.join("state")).unwrap();
        assert!(reopened.list().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn receipt_persistence_failure_keeps_the_changed_file_revertible() {
        use crate::commands::mutations::{write_track_queued, TrackPatch};
        use crate::state::write_queue::WriteQueue;

        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        let file = root.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.flac"),
            &file,
        )
        .unwrap();
        let before = fs::read(&file).unwrap();
        let worker = WorkerStore::new_in(state.clone());
        assert!(worker.initialize());
        let store = ReviewStore::new_with_worker(worker).unwrap();
        let journal = store.begin("run", &root).unwrap();
        record_assisted_genre_candidate(&journal, &root, "Jazz").await;
        let database = rusqlite::Connection::open(state.join("worker.db")).unwrap();
        database
            .execute_batch(
                "CREATE TRIGGER fail_intent_receipt BEFORE UPDATE OF after_sha256 ON write_intents
                 BEGIN SELECT RAISE(FAIL, 'injected receipt persistence failure'); END;",
            )
            .unwrap();
        assert!(ACTIVE_REVIEW
            .scope(
                journal,
                write_track_queued(
                    &WriteQueue::default(),
                    file.clone(),
                    serde_json::from_value::<TrackPatch>(json!({"genre":"Jazz"})).unwrap(),
                ),
            )
            .await
            .is_err());
        store
            .finish("run", "failed", json!({"reason":"receipt_failed"}))
            .unwrap();
        assert!(
            store.get("run").unwrap().can_revert,
            "a successful media mutation remains revertible if receipt persistence fails"
        );
        store.revert("run").unwrap();
        assert_eq!(fs::read(&file).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn writer_error_after_partial_change_needs_recovery_not_a_receipt() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        let file = root.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.flac"),
            &file,
        )
        .unwrap();
        let store = ReviewStore::new_in(state.clone()).unwrap();
        let journal = store.begin("run", &root).unwrap();
        record_assisted_genre_candidate(&journal, &root, "Jazz").await;
        assert!(journal
            .mutate_with_patch(&file, &json!({"genre":"Jazz"}), || {
                fs::write(&file, b"partial")?;
                Err::<(), _>(failure("injected writer error"))
            })
            .is_err());
        drop(journal);
        drop(store);

        let worker = WorkerStore::new_in(state);
        assert!(worker.initialize());
        let recovery = worker.reconcile_write_intents().unwrap();
        assert_eq!(recovery.len(), 1);
        assert_eq!(
            recovery[0].action,
            crate::state::worker::WriteRecoveryAction::NeedsRecovery,
            "a post-write hash cannot substitute for a successful write and readback"
        );
        assert_eq!(fs::read(&file).unwrap(), b"partial");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn successful_revert_makes_all_repeated_file_intents_terminal() {
        use crate::commands::mutations::{write_track_queued, TrackPatch};
        use crate::state::{worker::WriteRecoveryAction, write_queue::WriteQueue};

        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let state = root.join("state");
        let file = root.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.flac"),
            &file,
        )
        .unwrap();
        let original = fs::read(&file).unwrap();
        let store = ReviewStore::new_in(state.clone()).unwrap();
        let journal = store.begin("run", &root).unwrap();
        record_assisted_genre_candidate(&journal, &root, "Jazz").await;
        for _ in 0..2 {
            ACTIVE_REVIEW
                .scope(
                    Arc::clone(&journal),
                    write_track_queued(
                        &WriteQueue::default(),
                        file.clone(),
                        serde_json::from_value::<TrackPatch>(json!({"genre":"Jazz"})).unwrap(),
                    ),
                )
                .await
                .unwrap();
        }
        store.finish("run", "applied", json!({})).unwrap();
        store.revert("run").unwrap();
        assert_eq!(fs::read(&file).unwrap(), original);
        drop(store);

        let worker = WorkerStore::new_in(state);
        assert!(worker.initialize());
        let recovery = worker.reconcile_write_intents().unwrap();
        assert_eq!(recovery.len(), 2);
        assert!(recovery
            .iter()
            .all(|entry| entry.action == WriteRecoveryAction::Reverted));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_restores_exact_media_and_removes_only_created_artwork() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let track = root.join("01.mp3");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            &track,
        )
        .unwrap();
        let original = std::fs::read(&track).unwrap();
        let store = ReviewStore::default();
        let review = store.begin("run", &root).unwrap();
        review
            .mutate(&track, || {
                std::fs::write(&track, b"changed")?;
                Ok(())
            })
            .unwrap();
        let cover = root.join("cover.jpg");
        review
            .mutate(&cover, || {
                std::fs::write(&cover, b"new art")?;
                Ok(())
            })
            .unwrap();
        store
            .finish(
                "run",
                "failed",
                serde_json::json!({"error":"partial write"}),
            )
            .unwrap();
        store.revert("run").unwrap();
        assert_eq!(std::fs::read(&track).unwrap(), original);
        assert!(!cover.exists());
        assert_eq!(store.get("run").unwrap().decision, "reverted");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_album_revert_resumes_all_files_after_restart() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one.bin");
        let second = root.join("two.bin");
        let created = root.join("cover.jpg");
        fs::write(&first, b"first before").unwrap();
        fs::write(&second, b"second before").unwrap();
        let state = root.join("state");
        let store = ReviewStore::new_in(state.clone()).unwrap();
        let journal = store.begin("run", &root).unwrap();
        for (path, bytes) in [
            (&first, b"first after".as_slice()),
            (&second, b"second after".as_slice()),
            (&created, b"new cover".as_slice()),
        ] {
            journal
                .mutate(path, || {
                    fs::write(path, bytes)?;
                    Ok(())
                })
                .unwrap();
        }
        store.finish("run", "applied", json!({})).unwrap();
        assert!(store.revert_with_limit("run", Some(1)).is_err());
        assert_eq!(fs::read(&first).unwrap(), b"first before");
        assert_eq!(fs::read(&second).unwrap(), b"second after");
        assert_eq!(fs::read(&created).unwrap(), b"new cover");
        drop(journal);
        drop(store);

        let reopened = ReviewStore::new_in(state).unwrap();
        let detail = reopened.get("run").unwrap();
        assert!(detail.can_revert);
        assert!(detail.recovery_required);
        fs::write(&second, b"external edit").unwrap();
        assert!(reopened.revert("run").is_err());
        assert_eq!(fs::read(&first).unwrap(), b"first before");
        assert_eq!(fs::read(&second).unwrap(), b"external edit");
        assert_eq!(fs::read(&created).unwrap(), b"new cover");
        fs::write(&second, b"second after").unwrap();
        reopened.clear().unwrap();
        assert_eq!(reopened.list().unwrap().len(), 1);
        let worker =
            WorkerStore::new_in(reopened.recovery.as_ref().unwrap().data_dir().to_path_buf());
        assert!(worker.initialize());
        let (_, value, _) = worker.reversal_manifest("run").unwrap().unwrap();
        let manifest: ReversalManifest = serde_json::from_value(value).unwrap();
        let backup = manifest
            .files
            .iter()
            .find(|file| file.path == second)
            .unwrap()
            .backup
            .as_ref()
            .unwrap();
        let original_backup = fs::read(backup).unwrap();
        fs::write(backup, b"damaged").unwrap();
        assert!(reopened.revert("run").is_err());
        assert_eq!(fs::read(&second).unwrap(), b"second after");
        assert_eq!(fs::read(&created).unwrap(), b"new cover");
        fs::write(backup, original_backup).unwrap();
        reopened.revert("run").unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"first before");
        assert_eq!(fs::read(&second).unwrap(), b"second before");
        assert!(!created.exists());
        assert!(reopened.list().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn newer_shared_art_owner_in_another_store_blocks_an_older_reversal() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();
        let art = root.join("artist.jpg");
        let first_store = ReviewStore::new_in(root.join("state")).unwrap();
        let second_store = ReviewStore::new_in(root.join("state")).unwrap();
        let first = first_store.begin("first", &root.join("one")).unwrap();
        first
            .mutate(&art, || {
                fs::write(&art, b"same art")?;
                Ok(())
            })
            .unwrap();
        first_store.finish("first", "applied", json!({})).unwrap();
        let second = second_store.begin("second", &root.join("two")).unwrap();
        second
            .mutate(&art, || {
                fs::write(&art, b"same art")?;
                Ok(())
            })
            .unwrap();
        second_store.finish("second", "applied", json!({})).unwrap();
        assert!(first_store.revert("first").is_err());
        assert_eq!(fs::read(&art).unwrap(), b"same art");
        second_store.revert("second").unwrap();
        first_store.revert("first").unwrap();
        assert!(!art.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_unchanged_write_does_not_claim_newer_ownership() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();
        let art = root.join("artist.jpg");
        let first_store = ReviewStore::new_in(root.join("state")).unwrap();
        let second_store = ReviewStore::new_in(root.join("state")).unwrap();
        let first = first_store.begin("first", &root.join("one")).unwrap();
        first
            .mutate(&art, || {
                fs::write(&art, b"art")?;
                Ok(())
            })
            .unwrap();
        first_store.finish("first", "applied", json!({})).unwrap();
        let second = second_store.begin("second", &root.join("two")).unwrap();
        assert!(second
            .mutate::<()>(&art, || Err(failure("injected failure")))
            .is_err());
        second_store.finish("second", "failed", json!({})).unwrap();
        first_store.revert("first").unwrap();
        assert!(!art.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn live_target_writer_blocks_reversal_from_another_store() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();
        let art = root.join("artist.jpg");
        let first_store = ReviewStore::new_in(root.join("state")).unwrap();
        let second_store = ReviewStore::new_in(root.join("state")).unwrap();
        fs::write(&art, b"original").unwrap();
        let first = first_store.begin("first", &root.join("one")).unwrap();
        let second = second_store.begin("second", &root.join("two")).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let target = art.clone();
        let writer = std::thread::spawn(move || {
            first.mutate(&target, || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                fs::write(&target, b"new art")?;
                Ok(())
            })
        });
        started_rx.recv().unwrap();
        let competing = second.mutate(&art, || {
            fs::write(&art, b"competing")?;
            Ok(())
        });
        release_tx.send(()).unwrap();
        writer.join().unwrap().unwrap();
        assert!(
            competing.is_err(),
            "Another review must not write while the target is owned by a live writer"
        );
        assert_eq!(fs::read(&art).unwrap(), b"new art");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "subprocess entry for the interrupted reversal test"]
    fn reversal_crash_child() {
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_TEST_REVERSAL_ROOT").unwrap());
        let first = root.join("first.bin");
        let second = root.join("second.bin");
        let cover = root.join("cover.jpg");
        fs::write(&first, b"first before").unwrap();
        fs::write(&second, b"second before").unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let journal = store.begin("crash-run", &root).unwrap();
        for (path, bytes) in [
            (&first, b"first after".as_slice()),
            (&second, b"second after".as_slice()),
            (&cover, b"new cover".as_slice()),
        ] {
            journal
                .mutate(path, || {
                    fs::write(path, bytes)?;
                    Ok(())
                })
                .unwrap();
        }
        store.finish("crash-run", "applied", json!({})).unwrap();
        store.revert_with_limit("crash-run", Some(1)).unwrap();
    }

    #[test]
    fn resumed_reversal_rechecks_all_restored_files_before_completion() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let file = root.join("track.bin");
        fs::write(&file, b"before").unwrap();
        let state = root.join("state");
        let store = ReviewStore::new_in(state.clone()).unwrap();
        let journal = store.begin("run", &root).unwrap();
        journal
            .mutate(&file, || {
                fs::write(&file, b"after")?;
                Ok(())
            })
            .unwrap();
        store.finish("run", "applied", json!({})).unwrap();
        assert!(store.revert_with_limit("run", Some(1)).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"before");
        drop(journal);
        drop(store);
        let reopened = ReviewStore::new_in(state).unwrap();
        assert!(reopened
            .resume_reversal("run", None, || {
                fs::write(&file, b"later edit").unwrap();
            })
            .is_err());
        assert_eq!(fs::read(&file).unwrap(), b"later edit");
        assert!(reopened.get("run").unwrap().recovery_required);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn process_exit_after_replacement_can_resume_from_durable_manifest() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::auto_tag_review::tests::reversal_crash_child",
                "--ignored",
            ])
            .env("SOUNDROBE_TEST_REVERSAL_ROOT", &root)
            .env("SOUNDROBE_TEST_REVERSAL_CRASH", "1")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(70));
        assert_eq!(fs::read(root.join("first.bin")).unwrap(), b"first before");
        assert_eq!(fs::read(root.join("second.bin")).unwrap(), b"second after");
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        assert!(store.get("crash-run").unwrap().can_revert);
        store.revert("crash-run").unwrap();
        assert_eq!(store.revert("crash-run").unwrap().decision, "reverted");
        assert_eq!(fs::read(root.join("first.bin")).unwrap(), b"first before");
        assert_eq!(fs::read(root.join("second.bin")).unwrap(), b"second before");
        assert!(!root.join("cover.jpg").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn ordinary_same_byte_write_blocks_older_album_reversal() {
        use crate::commands::mutations::write_track_queued;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let track = album.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus/padded.flac"),
            &track,
        )
        .unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let journal = store.begin("old-review", &album).unwrap();
        let patch = || serde_json::from_value(json!({"title":"Reviewed"})).unwrap();
        ACTIVE_REVIEW
            .scope(
                journal.clone(),
                write_track_queued(&queue, track.clone(), patch()),
            )
            .await
            .unwrap();
        store.finish("old-review", "applied", json!({})).unwrap();
        let reviewed = fs::read(&track).unwrap();
        write_track_queued(&queue, track.clone(), patch())
            .await
            .unwrap();
        assert_eq!(
            fs::read(&track).unwrap(),
            reviewed,
            "the later write must be byte-identical to exercise ownership"
        );
        assert!(
            store.revert("old-review").is_err(),
            "an ordinary later write must supersede the older review"
        );
        assert_eq!(fs::read(track).unwrap(), reviewed);
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "subprocess entry for the ordinary ownership crash test"]
    fn ordinary_owner_crash_child() {
        let root = PathBuf::from(std::env::var_os("SOUNDROBE_TEST_ORDINARY_OWNER_ROOT").unwrap());
        let worker = WorkerStore::new_in(root.join("state"));
        assert!(worker.initialize());
        let _owner =
            ordinary_target_lease(Arc::new(worker), &root.join("album/track.bin")).unwrap();
        std::process::exit(71);
    }

    #[test]
    #[cfg(unix)]
    fn dangling_symlink_is_not_treated_as_a_new_target() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let alias = root.join("alias");
        symlink(root.join("missing"), &alias).unwrap();
        assert!(canonical_target(&alias).is_err());
        assert!(canonical_target(&alias.join("child")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn reviewed_write_rejects_hardlinked_target_before_mutation() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target.bin");
        let alias = root.join("alias.bin");
        fs::write(&target, b"before").unwrap();
        fs::hard_link(&target, &alias).unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let journal = store.begin("hardlink-review", &root).unwrap();
        let result = journal.mutate(&target, || {
            fs::write(&target, b"changed")?;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read(target).unwrap(), b"before");
        assert_eq!(fs::read(alias).unwrap(), b"before");
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ordinary_owner_survives_process_crash_before_write() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let track = album.join("track.bin");
        fs::write(&track, b"before").unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let journal = store.begin("crash-owner-review", &album).unwrap();
        journal
            .mutate(&track, || {
                fs::write(&track, b"reviewed")?;
                Ok(())
            })
            .unwrap();
        store
            .finish("crash-owner-review", "applied", json!({}))
            .unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::auto_tag_review::tests::ordinary_owner_crash_child",
                "--ignored",
            ])
            .env("SOUNDROBE_TEST_ORDINARY_OWNER_ROOT", &root)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(71));
        assert!(store.revert("crash-owner-review").is_err());
        assert_eq!(fs::read(track).unwrap(), b"reviewed");
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rejected_rename_does_not_supersede_untouched_source_review() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let source_album = root.join("source");
        let destination_album = root.join("destination");
        fs::create_dir_all(&source_album).unwrap();
        fs::create_dir_all(&destination_album).unwrap();
        let source = source_album.join("track.flac");
        let destination = destination_album.join("track.flac");
        let sibling = destination_album.join("cover.jpg");
        fs::write(&source, b"source before").unwrap();
        fs::write(&destination, b"destination before").unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let first = store.begin("source-review", &source_album).unwrap();
        first
            .mutate(&source, || {
                fs::write(&source, b"source reviewed")?;
                Ok(())
            })
            .unwrap();
        store.finish("source-review", "applied", json!({})).unwrap();
        let second = store
            .begin("destination-review", &destination_album)
            .unwrap();
        second
            .mutate(&destination, || {
                fs::write(&destination, b"destination reviewed")?;
                Ok(())
            })
            .unwrap();
        second
            .mutate(&sibling, || {
                fs::write(&sibling, b"cover")?;
                Ok(())
            })
            .unwrap();
        store
            .finish("destination-review", "applied", json!({}))
            .unwrap();
        assert!(store
            .revert_with_limit("destination-review", Some(1))
            .is_err());
        assert!(crate::commands::mutations::rename_track_queued(
            &queue,
            source.clone(),
            destination
        )
        .await
        .is_err());
        store
            .revert("source-review")
            .expect("an untouched source review remains reversible");
        assert_eq!(fs::read(source).unwrap(), b"source before");
        drop(first);
        drop(second);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "desktop")]
    #[tokio::test]
    async fn invalid_cover_selection_does_not_supersede_review() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let cover = album.join("cover.jpg");
        let invalid = root.join("invalid.jpg");
        fs::write(&cover, b"before").unwrap();
        fs::write(&invalid, b"not an image").unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let journal = store.begin("cover-invalid", &album).unwrap();
        journal
            .mutate(&cover, || {
                fs::write(&cover, b"reviewed")?;
                Ok(())
            })
            .unwrap();
        store.finish("cover-invalid", "applied", json!({})).unwrap();
        assert!(crate::commands::covers::set_cover_from_path_queued(
            &queue,
            album.clone(),
            invalid
        )
        .await
        .unwrap()
        .is_none());
        store
            .revert("cover-invalid")
            .expect("an invalid image did not write the cover");
        assert_eq!(fs::read(cover).unwrap(), b"before");
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn protected_batch_target_reports_failure_and_keeps_other_successes() {
        use crate::commands::mutations::{
            batch_write_with_readback, write_track_queued, TrackUpdate,
        };
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let protected = album.join("02.flac");
        let cover = album.join("cover.jpg");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/writer-corpus/padded.flac");
        fs::copy(&fixture, &protected).unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let journal = store.begin("pending-batch", &album).unwrap();
        ACTIVE_REVIEW
            .scope(
                journal.clone(),
                write_track_queued(
                    &queue,
                    protected.clone(),
                    serde_json::from_value(json!({"title":"Reviewed"})).unwrap(),
                ),
            )
            .await
            .unwrap();
        journal
            .mutate(&cover, || {
                fs::write(&cover, b"cover")?;
                Ok(())
            })
            .unwrap();
        store.finish("pending-batch", "applied", json!({})).unwrap();
        assert!(store.revert_with_limit("pending-batch", Some(1)).is_err());
        let protected_before = fs::read(&protected).unwrap();
        let first = album.join("01.flac");
        let last = album.join("03.flac");
        fs::copy(&fixture, &first).unwrap();
        fs::copy(&fixture, &last).unwrap();
        let updates = [&first, &protected, &last]
            .iter()
            .map(|path| TrackUpdate {
                path: path.to_string_lossy().into_owned(),
                fields: serde_json::from_value(json!({"title":"Changed"})).unwrap(),
            })
            .collect();
        let result = batch_write_with_readback(&queue, updates, None)
            .await
            .unwrap();
        assert_eq!(result.tracks.len(), 2);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].path, protected.to_string_lossy());
        assert!(result.failures[0].error.contains("pending reversal"));
        assert_eq!(fs::read(&protected).unwrap(), protected_before);
        assert_eq!(
            crate::commands::tracks::read_track_metadata(&first)
                .unwrap()
                .title
                .as_deref(),
            Some("Changed")
        );
        assert_eq!(
            crate::commands::tracks::read_track_metadata(&last)
                .unwrap()
                .title
                .as_deref(),
            Some("Changed")
        );
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn audit_target_preflight_preserves_all_files_when_one_is_protected() {
        use crate::commands::mutations::write_track_queued;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/writer-corpus/padded.flac");
        let protected = album.join("02.flac");
        let cover = album.join("cover.jpg");
        fs::copy(&fixture, &protected).unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let journal = store.begin("audit-pending", &album).unwrap();
        ACTIVE_REVIEW
            .scope(
                journal.clone(),
                write_track_queued(
                    &queue,
                    protected.clone(),
                    serde_json::from_value(json!({"title":"Reviewed"})).unwrap(),
                ),
            )
            .await
            .unwrap();
        journal
            .mutate(&cover, || {
                fs::write(&cover, b"cover")?;
                Ok(())
            })
            .unwrap();
        store.finish("audit-pending", "applied", json!({})).unwrap();
        assert!(store.revert_with_limit("audit-pending", Some(1)).is_err());
        let first = album.join("01.flac");
        fs::copy(&fixture, &first).unwrap();
        let before = fs::read(&first).unwrap();
        let findings = serde_json::from_value(json!([
            {"index":0,"field":"title","status":"error","message":"fix","corrected":{"title":"Other"},"source":"deterministic","confidence":0.99,"autoFixEligible":true,"autoFixed":false},
            {"index":1,"field":"title","status":"error","message":"fix","corrected":{"title":"Other"},"source":"deterministic","confidence":0.99,"autoFixEligible":true,"autoFixed":false}
        ])).unwrap();
        assert!(crate::commands::audit::apply_audit_fixes_for_album_results(
            &queue,
            vec![crate::commands::audit::AuditAlbumResult {
                album_path: album.to_string_lossy().into_owned(),
                results: findings
            }]
        )
        .await
        .is_err());
        assert_eq!(
            fs::read(first).unwrap(),
            before,
            "no earlier file may change before all target admissions succeed"
        );
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn extra_tag_batch_preflight_preserves_all_files_when_one_is_protected() {
        use crate::commands::mutations::{
            batch_write_extra_tags_queued, write_track_queued, ExtraTagBatchUpdate, ExtraTagUpdate,
        };
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/writer-corpus/padded.flac");
        let protected = album.join("02.flac");
        let cover = album.join("cover.jpg");
        fs::copy(&fixture, &protected).unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let journal = store.begin("extra-pending", &album).unwrap();
        ACTIVE_REVIEW
            .scope(
                journal.clone(),
                write_track_queued(
                    &queue,
                    protected.clone(),
                    serde_json::from_value(json!({"title":"Reviewed"})).unwrap(),
                ),
            )
            .await
            .unwrap();
        journal
            .mutate(&cover, || {
                fs::write(&cover, b"cover")?;
                Ok(())
            })
            .unwrap();
        store.finish("extra-pending", "applied", json!({})).unwrap();
        assert!(store.revert_with_limit("extra-pending", Some(1)).is_err());
        let first = album.join("01.flac");
        fs::copy(&fixture, &first).unwrap();
        let before = fs::read(&first).unwrap();
        let updates = [&first, &protected]
            .iter()
            .map(|path| ExtraTagBatchUpdate {
                path: path.to_string_lossy().into_owned(),
                tags: vec![ExtraTagUpdate {
                    key: "COMMENT".into(),
                    value: "Changed".into(),
                }],
            })
            .collect();
        assert!(batch_write_extra_tags_queued(&queue, updates)
            .await
            .is_err());
        assert_eq!(
            fs::read(first).unwrap(),
            before,
            "no earlier file may change before all target admissions succeed"
        );
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn ordinary_same_byte_batch_blocks_older_album_reversal() {
        use crate::commands::mutations::{
            batch_write_with_readback, write_track_queued, TrackUpdate,
        };
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let track = album.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus/padded.flac"),
            &track,
        )
        .unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let journal = store.begin("old-batch-review", &album).unwrap();
        let patch = || serde_json::from_value(json!({"title":"Reviewed"})).unwrap();
        ACTIVE_REVIEW
            .scope(
                journal.clone(),
                write_track_queued(&queue, track.clone(), patch()),
            )
            .await
            .unwrap();
        store
            .finish("old-batch-review", "applied", json!({}))
            .unwrap();
        let reviewed = fs::read(&track).unwrap();
        batch_write_with_readback(
            &queue,
            vec![TrackUpdate {
                path: track.to_string_lossy().into_owned(),
                fields: patch(),
            }],
            None,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&track).unwrap(), reviewed);
        assert!(
            store.revert("old-batch-review").is_err(),
            "the later batch write must supersede the review"
        );
        assert_eq!(fs::read(track).unwrap(), reviewed);
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn ordinary_extra_tag_write_blocks_older_album_reversal() {
        use crate::commands::mutations::write_extra_tags_queued;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let track = album.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus/padded.flac"),
            &track,
        )
        .unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let original = fs::read(&track).unwrap();
        write_extra_tags_queued(&queue, track.clone(), vec![])
            .await
            .unwrap();
        let normalized = fs::read(&track).unwrap();
        fs::write(&track, original).unwrap();
        let journal = store.begin("old-extra-review", &album).unwrap();
        journal
            .mutate(&track, || {
                fs::write(&track, &normalized)?;
                Ok(())
            })
            .unwrap();
        store
            .finish("old-extra-review", "applied", json!({}))
            .unwrap();
        let reviewed = fs::read(&track).unwrap();
        write_extra_tags_queued(&queue, track.clone(), vec![])
            .await
            .unwrap();
        assert_eq!(fs::read(&track).unwrap(), reviewed);
        assert!(store.revert("old-extra-review").is_err());
        assert_eq!(fs::read(track).unwrap(), reviewed);
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn pending_reversal_blocks_ordinary_track_write() {
        use crate::commands::mutations::write_track_queued;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("album");
        fs::create_dir_all(&album).unwrap();
        let track = album.join("01.flac");
        let cover = album.join("cover.jpg");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus/padded.flac"),
            &track,
        )
        .unwrap();
        let store = ReviewStore::new_in(root.join("state")).unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(store.recovery.as_ref().unwrap().clone())
            .unwrap();
        let journal = store.begin("pending-review", &album).unwrap();
        ACTIVE_REVIEW
            .scope(
                journal.clone(),
                write_track_queued(
                    &queue,
                    track.clone(),
                    serde_json::from_value(json!({"title":"Reviewed"})).unwrap(),
                ),
            )
            .await
            .unwrap();
        let mut image_bytes = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(
                &mut std::io::Cursor::new(&mut image_bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        journal
            .mutate(&cover, || {
                fs::write(&cover, &image_bytes)?;
                Ok(())
            })
            .unwrap();
        let cover_before = fs::read(&cover).unwrap();
        store
            .finish("pending-review", "applied", json!({}))
            .unwrap();
        assert!(store.revert_with_limit("pending-review", Some(1)).is_err());
        let interrupted = fs::read(&track).unwrap();
        let source = root.join("spare.flac");
        fs::copy(&track, &source).unwrap();
        let alias = album.join("missing/../cover.jpg");
        let error = crate::commands::mutations::rename_track_queued(&queue, source.clone(), alias)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("unresolved parent traversal"),
            "{error}"
        );
        assert_eq!(
            fs::read(&cover).unwrap(),
            cover_before,
            "path alias must not bypass pending target"
        );
        assert!(
            source.exists(),
            "rename must be rejected before touching source"
        );
        write_track_queued(
            &queue,
            source.clone(),
            serde_json::from_value(json!({"title":"Unrelated"})).unwrap(),
        )
        .await
        .unwrap();
        let error = write_track_queued(
            &queue,
            track.clone(),
            serde_json::from_value(json!({"title":"Other"})).unwrap(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("pending reversal"), "{error}");
        let providers = crate::state::providers::ProviderState::new();
        let remote =
            crate::state::providers::RemoteArtworkClient::new(providers.http(), None, None);
        let error = crate::commands::covers::download_album_artwork_at(&album, &remote, &queue)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("pending reversal"), "{error}");
        assert_eq!(fs::read(&cover).unwrap(), cover_before);
        #[cfg(all(feature = "desktop", unix))]
        {
            let alias_album = root.join("alias");
            fs::create_dir_all(&alias_album).unwrap();
            let alias_cover = alias_album.join("cover.jpg");
            fs::hard_link(&cover, &alias_cover).unwrap();
            let result = crate::commands::covers::set_cover_from_path_queued(
                &queue,
                alias_album,
                cover.clone(),
            )
            .await;
            assert!(
                result.is_err(),
                "hard-linked target must not bypass pending recovery"
            );
            assert_eq!(fs::read(&cover).unwrap(), cover_before);
            fs::remove_file(alias_cover).unwrap();
        }
        let error = crate::commands::mutations::batch_write_with_readback(
            &queue,
            vec![crate::commands::mutations::TrackUpdate {
                path: track.to_string_lossy().into_owned(),
                fields: serde_json::from_value(json!({"title":"Other"})).unwrap(),
            }],
            None,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("pending reversal"), "{error}");
        #[cfg(feature = "desktop")]
        {
            let error =
                crate::commands::mutations::remove_embedded_cover_queued(&queue, track.clone())
                    .await
                    .unwrap_err();
            assert!(error.to_string().contains("pending reversal"), "{error}");
            let error = crate::commands::covers::remove_cover_queued(&queue, album.clone())
                .await
                .unwrap_err();
            assert!(error.to_string().contains("pending reversal"), "{error}");
            let error = crate::commands::covers::set_cover_from_path_queued(
                &queue,
                album.clone(),
                cover.clone(),
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains("pending reversal"), "{error}");
        }
        let renamed = root.join("new/folder/renamed.flac");
        let error =
            crate::commands::mutations::rename_track_queued(&queue, track.clone(), renamed.clone())
                .await
                .unwrap_err();
        assert!(error.to_string().contains("pending reversal"), "{error}");
        assert!(!renamed.parent().unwrap().exists());
        #[cfg(feature = "desktop")]
        {
            let moved = root.join("assistant/new/track.flac");
            let error = crate::commands::assistant::relocate_file_queued(
                &queue,
                track.clone(),
                moved.clone(),
                &root.canonicalize().unwrap(),
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains("pending reversal"), "{error}");
            assert!(!moved.parent().unwrap().exists());
        }
        let findings = serde_json::from_value(json!([{"index":0,"field":"title","status":"error","message":"fix","corrected":{"title":"Other"},"source":"deterministic","confidence":0.99,"autoFixEligible":true,"autoFixed":false}])).unwrap();
        let error = crate::commands::audit::apply_audit_fixes_for_album_results(
            &queue,
            vec![crate::commands::audit::AuditAlbumResult {
                album_path: album.to_string_lossy().into_owned(),
                results: findings,
            }],
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("pending reversal"), "{error}");
        fs::write(track.with_extension("txt"), "Local lyrics").unwrap();
        let lyrics = crate::commands::lyrics::apply_album_lyrics_at(&album, None, &queue)
            .await
            .unwrap();
        assert!(lyrics.results.iter().any(|result| result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("pending reversal"))));
        let deleted = crate::commands::mutations::delete_files_queued(
            &queue,
            vec![track.to_string_lossy().into_owned()],
        )
        .await
        .unwrap();
        assert!(!deleted[0].success);
        assert!(deleted[0]
            .error
            .as_deref()
            .unwrap()
            .contains("pending reversal"));
        assert_eq!(fs::read(track).unwrap(), interrupted);
        drop(journal);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn ordinary_write_without_prior_review_does_not_accumulate_owner_history() {
        use crate::commands::mutations::write_track_queued;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let track = root.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus/padded.flac"),
            &track,
        )
        .unwrap();
        let worker = WorkerStore::new_in(root.join("state"));
        assert!(worker.initialize());
        let queue = WriteQueue::default()
            .with_recovery_worker(Arc::new(worker))
            .unwrap();
        write_track_queued(
            &queue,
            track,
            serde_json::from_value(json!({"title":"Changed"})).unwrap(),
        )
        .await
        .unwrap();
        let conn = rusqlite::Connection::open(root.join("state/worker.db")).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM review_target_owners", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count, 0,
            "a first ordinary write has no older review to supersede"
        );
        drop(conn);
        drop(queue);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn unresolved_write_intent_blocks_ordinary_track_edit() {
        use crate::commands::mutations::write_track_queued;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let track = root.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus/padded.flac"),
            &track,
        )
        .unwrap();
        let before = fs::read(&track).unwrap();
        let worker = WorkerStore::new_in(root.join("state"));
        assert!(worker.initialize());
        worker
            .prepare_write_intent(&track, "abandoned::attempt", &json!({"title":"First"}))
            .unwrap();
        worker
            .begin_write_intent(&track, "abandoned::attempt")
            .unwrap();
        let queue = WriteQueue::default()
            .with_recovery_worker(Arc::new(worker))
            .unwrap();
        let result = write_track_queued(
            &queue,
            track.clone(),
            serde_json::from_value(json!({"title":"Other"})).unwrap(),
        )
        .await;
        assert!(
            result.is_err(),
            "unknown prior write must be resolved before another edit"
        );
        assert_eq!(fs::read(track).unwrap(), before);
        drop(queue);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn resumed_album_reversal_restores_copied_audio_and_shared_artwork() {
        use crate::commands::mutations::{track_payload_digest, write_track_queued};
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        let track = album.join("01.flac");
        let art = root.join("Artist/artist.jpg");
        let cover = album.join("cover.jpg");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/writer-corpus/padded.flac"),
            &track,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&track, fs::Permissions::from_mode(0o640)).unwrap();
        }
        fs::write(&art, b"original artist art").unwrap();
        let original = fs::read(&track).unwrap();
        let payload = track_payload_digest(&track).unwrap();
        let state = root.join("state");
        let store = ReviewStore::new_in(state.clone()).unwrap();
        let journal = store.begin("copied-album", &album).unwrap();
        ACTIVE_REVIEW
            .scope(
                journal.clone(),
                write_track_queued(
                    &WriteQueue::default(),
                    track.clone(),
                    serde_json::from_value(json!({"title":"New title"})).unwrap(),
                ),
            )
            .await
            .unwrap();
        journal
            .mutate(&art, || {
                fs::write(&art, b"new artist art")?;
                Ok(())
            })
            .unwrap();
        journal
            .mutate(&cover, || {
                fs::write(&cover, b"new cover")?;
                Ok(())
            })
            .unwrap();
        store.finish("copied-album", "applied", json!({})).unwrap();
        assert!(store.revert_with_limit("copied-album", Some(1)).is_err());
        drop(journal);
        drop(store);
        let reopened = ReviewStore::new_in(state).unwrap();
        let album_key = album.to_string_lossy();
        super::super::covers::cover_cache_source(&album_key, "external", &cover.to_string_lossy());
        reopened.revert("copied-album").unwrap();
        assert!(super::super::covers::cover_source_cached(&album_key).is_none());
        assert_eq!(fs::read(&track).unwrap(), original);
        assert_eq!(track_payload_digest(&track).unwrap(), payload);
        assert_eq!(fs::read(&art).unwrap(), b"original artist art");
        assert!(!cover.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&track).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn queued_tag_and_lyrics_changes_restore_the_complete_original_file() {
        use crate::commands::mutations::{write_track_queued, TrackPatch};
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let track = root.join("01.flac");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.flac"),
            &track,
        )
        .unwrap();
        let original = fs::read(&track).unwrap();
        let store = ReviewStore::default();
        let journal = store.begin("run", &root).unwrap();
        let patch: TrackPatch = serde_json::from_value(json!({"title":"Reviewed title", "artists":["A", "B"], "lyrics":{"plainLyrics":"Verse", "syncedLyrics":null, "language":"eng"}})).unwrap();
        ACTIVE_REVIEW
            .scope(
                journal,
                write_track_queued(&WriteQueue::default(), track.clone(), patch),
            )
            .await
            .unwrap();
        store.finish("run", "applied", json!({})).unwrap();
        assert_eq!(
            store.get("run").unwrap().after.tracks[0]["title"],
            "Reviewed title"
        );
        assert_eq!(
            store.get("run").unwrap().after.tracks[0]["lyrics"]["plainLyrics"],
            "Verse"
        );
        assert!(store.keep("run").unwrap().can_revert);
        store.revert("run").unwrap();
        assert_eq!(
            fs::read(&track).unwrap(),
            original,
            "Full recovery preserves all metadata and the audio payload byte-for-byte"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skipped_review_has_no_recovery_action_and_keep_does_not_write() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let store = ReviewStore::default();
        store.begin("run", &root).unwrap();
        store
            .finish(
                "run",
                "needs_review",
                json!({"reasonCode":"provider_unavailable"}),
            )
            .unwrap();
        assert!(!store.keep("run").unwrap().can_revert);
        assert!(store.revert("run").is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn backup_failure_prevents_the_write_and_damaged_backup_prevents_restore() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let file = root.join("cover.jpg");
        fs::write(&file, b"before").unwrap();
        let store = ReviewStore::default();
        let journal = store.begin("run", &root).unwrap();
        fs::remove_dir_all(&store.files.0).unwrap();
        let mut wrote = false;
        assert!(journal
            .mutate(&file, || {
                wrote = true;
                Ok(())
            })
            .is_err());
        assert!(!wrote);
        private_directory(&store.files.0).unwrap();
        journal
            .mutate(&file, || {
                fs::write(&file, b"after")?;
                Ok(())
            })
            .unwrap();
        store.finish("run", "applied", json!({})).unwrap();
        let backup = journal.data.lock().unwrap().files[0]
            .backup
            .clone()
            .unwrap();
        fs::write(backup, b"corrupt").unwrap();
        assert!(store.revert("run").is_err());
        assert_eq!(fs::read(file).unwrap(), b"after");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_artwork_requires_newer_album_recovery_first_even_for_identical_writes() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(root.join("one")).unwrap();
        fs::create_dir_all(root.join("two")).unwrap();
        let art = root.join("artist.jpg");
        let store = ReviewStore::default();
        let first = store.begin("first", &root.join("one")).unwrap();
        first
            .mutate(&art, || {
                fs::write(&art, b"shared")?;
                Ok(())
            })
            .unwrap();
        store.finish("first", "applied", json!({})).unwrap();
        let second = store.begin("second", &root.join("two")).unwrap();
        second
            .mutate(&art, || {
                fs::write(&art, b"shared")?;
                Ok(())
            })
            .unwrap();
        store.finish("second", "applied", json!({})).unwrap();
        assert!(
            store.revert("first").is_err(),
            "The older review must not delete art also written by a newer album"
        );
        store.revert("second").unwrap();
        store.revert("first").unwrap();
        assert!(!art.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn shared_artwork_ownership_follows_directory_aliases() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let real = root.join("artist");
        let alias = root.join("alias");
        fs::create_dir_all(real.join("one")).unwrap();
        fs::create_dir_all(real.join("two")).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let first_art = real.join("artist.jpg");
        let second_art = alias.join("artist.jpg");
        let store = ReviewStore::default();
        let first = store.begin("first", &real.join("one")).unwrap();
        first
            .mutate(&first_art, || {
                fs::write(&first_art, b"art")?;
                Ok(())
            })
            .unwrap();
        store.finish("first", "applied", json!({})).unwrap();
        let second = store.begin("second", &alias.join("two")).unwrap();
        second
            .mutate(&second_art, || {
                fs::write(&second_art, b"art")?;
                Ok(())
            })
            .unwrap();
        store.finish("second", "applied", json!({})).unwrap();
        assert!(
            store.revert("first").is_err(),
            "Aliases must share the same ownership history"
        );
        store.revert("second").unwrap();
        store.revert("first").unwrap();
        assert!(!first_art.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restore_retry_continues_after_a_successful_replace_with_failed_readback() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let first = root.join("cover.jpg");
        let second = root.join("front.png");
        fs::write(&first, b"before").unwrap();
        let store = ReviewStore::default();
        let journal = store.begin("run", &root).unwrap();
        journal
            .mutate(&first, || {
                fs::write(&first, b"after")?;
                Ok(())
            })
            .unwrap();
        journal
            .mutate(&second, || {
                fs::write(&second, b"created")?;
                Ok(())
            })
            .unwrap();
        store
            .finish(
                "run",
                "cancelled",
                json!({"error":"cancelled after writes"}),
            )
            .unwrap();
        // Model replacement success followed by an unreadable verification read.
        fs::write(&first, b"before").unwrap();
        journal.data.lock().unwrap().files[0].restoring = true;
        store.revert("run").unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"before");
        assert!(!second.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_and_retried_runs_retain_evidence_and_explicit_attempt_links() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let store = ReviewStore::default();
        let first = store.begin("first", &root).unwrap();
        first.evidence("candidate", json!({"album":"Selected release"}));
        first.event("warning", "Genre remains missing", None);
        store
            .finish("first", "failed", json!({"error":"one file failed"}))
            .unwrap();
        assert_eq!(
            store.get("first").unwrap().result["candidate"]["album"],
            "Selected release"
        );
        store.begin("second", &root).unwrap();
        store.finish("second", "needs_review", json!({})).unwrap();
        let detail = store.get("second").unwrap();
        assert_eq!(detail.result["earlierAttempts"][0]["reviewId"], "first");
        assert_eq!(
            detail.result["earlierAttempts"][0]["result"]["events"][0]["message"],
            "Genre remains missing"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clear_discards_prior_run_results_and_recovery_records() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let store = ReviewStore::default();
        let first = store.begin("first", &root).unwrap();
        first.event("warning", "First run", None);
        store.finish("first", "needs_review", json!({})).unwrap();

        store.clear().unwrap();

        assert!(
            store.list().unwrap().is_empty(),
            "previous run must not appear in Results"
        );
        assert!(
            store.get("first").is_err(),
            "previous run cannot be recovered after reset"
        );
        let second = store.begin("second", &root).unwrap();
        store.finish("second", "applied", json!({})).unwrap();
        assert_eq!(
            store.list().unwrap().len(),
            1,
            "only the new run is retained"
        );
        drop(second);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn abandoned_cleanup_keeps_live_sessions_and_unrelated_directories() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let stale = root.join(format!(
            "soundrobe-review-{}-{}",
            u32::MAX,
            uuid::Uuid::new_v4()
        ));
        let live = root.join(format!(
            "soundrobe-review-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let other = root.join("unrelated");
        for path in [&stale, &live, &other] {
            fs::create_dir(path).unwrap();
        }
        cleanup_abandoned_at(&root);
        assert!(!stale.exists());
        assert!(live.exists());
        assert!(other.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conflict_preflight_prevents_reverting_any_file() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("cover.jpg");
        let second = root.join("front.png");
        std::fs::write(&first, b"original").unwrap();
        let store = ReviewStore::default();
        let review = store.begin("run", &root).unwrap();
        review
            .mutate(&first, || {
                std::fs::write(&first, b"tagged")?;
                Ok(())
            })
            .unwrap();
        review
            .mutate(&second, || {
                std::fs::write(&second, b"art")?;
                Ok(())
            })
            .unwrap();
        store
            .finish("run", "applied", serde_json::json!({}))
            .unwrap();
        std::fs::write(&second, b"later edit").unwrap();
        assert!(store
            .revert("run")
            .unwrap_err()
            .to_string()
            .contains("changed"));
        assert_eq!(std::fs::read(&first).unwrap(), b"tagged");
        std::fs::remove_dir_all(root).unwrap();
    }
}
