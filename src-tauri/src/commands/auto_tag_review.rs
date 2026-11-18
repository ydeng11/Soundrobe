//! Session auto-tag evidence and exact-file recovery.
use super::{
    mutations::replace_file_atomic,
    tracks::{read_album, read_track_metadata, try_read_extra_tags},
};
use crate::error::ApiError;
use crate::state::worker::WorkerStore;
#[cfg(any(feature = "desktop", test))]
use crate::state::write_queue::WriteQueue;
#[cfg(feature = "desktop")]
use base64::Engine;
use lofty::file::TaggedFileExt;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
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
}
struct ReviewData {
    detail: ReviewDetail,
    files: Vec<FileChange>,
    images: HashMap<String, PathBuf>,
    trusted_assisted_genre_proposals: HashSet<String>,
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
        return Ok(fs::canonicalize(path)?);
    }
    let parent = path
        .parent()
        .ok_or_else(|| failure("Recovery path has no parent"))?;
    Ok(fs::canonicalize(parent)?.join(
        path.file_name()
            .ok_or_else(|| failure("Recovery path has no filename"))?,
    ))
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
        let assisted_genre_write = patch
            .map(|patch| self.record_assisted_genre_obligation(&data, path, patch))
            .transpose()?
            .unwrap_or(false);
        let before = fingerprint(path)?;
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
            recovery
                .supersede_prior_write_intents(path, &format!("{}::", data.detail.id), &intent_id)
                .map_err(failure)?;
            Some((intent_id, intent))
        } else {
            None
        };
        let index = if let Some(index) = data.files.iter().position(|f| f.path == path) {
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
                target: canonical_target(path)?,
                backup,
                before: before.clone(),
                after: before.clone(),
                restored: false,
                verified: false,
                tracked: false,
                restoring: false,
                intent_outcomes: Vec::new(),
            });
            data.files.len() - 1
        };
        if let (Some(recovery), Some((intent_id, _))) = (&self.recovery, &durable_intent) {
            recovery
                .begin_write_intent(path, intent_id)
                .map_err(failure)?;
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
        self.records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.data.lock().unwrap().detail.id == id)
            .cloned()
            .ok_or_else(|| failure("Review is unavailable in this session"))
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
        Ok(self.journal(id)?.data.lock().unwrap().detail.clone())
    }
    pub fn list(&self) -> Vec<ReviewDetail> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .rev()
            .map(|r| r.data.lock().unwrap().detail.clone())
            .collect()
    }
    pub fn keep(&self, id: &str) -> Result<ReviewDetail, ApiError> {
        let journal = self.journal(id)?;
        let mut data = journal.data.lock().unwrap();
        if data.detail.outcome == "running" || data.detail.decision == "reverted" {
            return Err(failure("This review cannot be marked kept"));
        }
        data.detail.decision = "kept".into();
        Ok(data.detail.clone())
    }
    pub fn revert(&self, id: &str) -> Result<ReviewDetail, ApiError> {
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
        Ok(data.detail.clone())
    }
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn auto_tag_reviews_clear(store: State<'_, ReviewStore>) -> Result<(), ApiError> {
    store.clear()
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub fn auto_tag_reviews_list(store: State<'_, ReviewStore>) -> Vec<ReviewDetail> {
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
pub fn auto_tag_review_keep(
    review_id: String,
    store: State<'_, ReviewStore>,
) -> Result<ReviewDetail, ApiError> {
    store.keep(&review_id)
}
#[cfg(feature = "desktop")]
#[tauri::command]
pub async fn auto_tag_review_revert(
    review_id: String,
    store: State<'_, ReviewStore>,
    queue: State<'_, WriteQueue>,
) -> Result<ReviewDetail, ApiError> {
    queue
        .run_exclusive(async { store.revert(&review_id) })
        .await
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
                let (status, body) = if request.starts_with("GET /releases/42 ") {
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
                &request, &base, &base,
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
        let obligations = reopened.field_review_obligations(&track).unwrap();
        assert_eq!(obligations.len(), 1);
        assert_eq!(obligations[0].field, "genre");
        assert_eq!(obligations[0].state, "open");
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
            store.list().is_empty(),
            "previous run must not appear in Results"
        );
        assert!(
            store.get("first").is_err(),
            "previous run cannot be recovered after reset"
        );
        let second = store.begin("second", &root).unwrap();
        store.finish("second", "applied", json!({})).unwrap();
        assert_eq!(store.list().len(), 1, "only the new run is retained");
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
