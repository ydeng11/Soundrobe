//! Session auto-tag evidence and exact-file recovery.
use super::{
    mutations::replace_file_atomic,
    tracks::{read_album, read_track_metadata, try_read_extra_tags},
};
use crate::{error::ApiError, state::write_queue::WriteQueue};
use base64::Engine;
use lofty::file::TaggedFileExt;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
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
fn process_alive(pid: u32) -> bool {
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
fn process_alive(pid: u32) -> bool {
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
}
struct ReviewData {
    detail: ReviewDetail,
    files: Vec<FileChange>,
    images: HashMap<String, PathBuf>,
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
}
pub struct ReviewJournal {
    data: Mutex<ReviewData>,
    files: Arc<SessionFiles>,
    owners: Arc<Mutex<HashMap<PathBuf, Vec<String>>>>,
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
fn track_view(path: &Path) -> Result<Value, ApiError> {
    let mut track =
        serde_json::to_value(read_track_metadata(path)?).map_err(|e| failure(e.to_string()))?;
    match try_read_extra_tags(path) {
        Ok(tags) => track["extraTags"] = json!(tags),
        Err(error) => track["extraTagsError"] = json!(error.to_string()),
    }
    Ok(track)
}
impl ReviewJournal {
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
        let mut data = self.data.lock().unwrap();
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(failure(format!(
                "Cannot journal symlink: {}",
                path.display()
            )));
        }
        let before = fingerprint(path)?;
        let index = if let Some(index) = data.files.iter().position(|f| f.path == path) {
            index
        } else {
            let backup = if before.is_some() {
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
            });
            data.files.len() - 1
        };
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
        data.files[index].tracked =
            result.is_ok() || data.files[index].before != data.files[index].after;
        if data.files[index].tracked {
            let mut owners = self.owners.lock().unwrap();
            let history = owners.entry(data.files[index].target.clone()).or_default();
            if history.last() != Some(&data.detail.id) {
                history.push(data.detail.id.clone());
            }
        }
        result
    }
}
impl ReviewStore {
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
            }),
            files: Arc::clone(&self.files),
            owners: Arc::clone(&self.owners),
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
        match journal.view(&mut data) {
            Ok(view) => data.detail.after = view,
            Err(error) => data
                .detail
                .after
                .errors
                .push(format!("Readback failed: {error}")),
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
#[tauri::command]
pub fn auto_tag_reviews_list(store: State<'_, ReviewStore>) -> Vec<ReviewDetail> {
    store.list()
}
#[tauri::command]
pub fn auto_tag_review_get(
    review_id: String,
    store: State<'_, ReviewStore>,
) -> Result<ReviewDetail, ApiError> {
    store.get(&review_id)
}
#[tauri::command]
pub fn auto_tag_review_keep(
    review_id: String,
    store: State<'_, ReviewStore>,
) -> Result<ReviewDetail, ApiError> {
    store.keep(&review_id)
}
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
