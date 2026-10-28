//! Headless HTTP runtime.

use axum::{
    body::{to_bytes, Body},
    extract::{rejection::JsonRejection, Json, Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{any, get, post},
    Router,
};
use base64::Engine;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    convert::Infallible,
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tower_http::services::{ServeDir, ServeFile};
use url::Url;
#[cfg(test)]
use uuid::Uuid;

use crate::commands::album_search::{
    album_preview_release_match, apply_search_candidate, discogs_token, normalise_page,
    normalise_page_size, release_track_count_inner, resolve_release_inner, search_releases_inner,
    ApplyCandidateRequest, PreviewMatchRequest, ResolveReleaseRequest, SearchReleasesRequest,
};
use crate::commands::audit::{
    apply_audit_fixes_for_album_results, audit_album_with_services, audit_clients,
    audit_specific_albums, discover_album_dirs, finish_audit_run, start_audit, AuditAlbumResult,
};
use crate::commands::auto_tag::{
    auto_tag_completion_message, auto_tag_event, resolve_and_apply_album_with_review_intent,
    AutoTagOutcome, AutoTagRetryContexts, AutoTagServices, LookupSource,
};
use crate::commands::configuration::test_llm_connection_at;
use crate::commands::covers::{
    download_album_artwork_at, download_artist_artwork_at, remote_client, ArtistArtResult,
};
use crate::commands::dataset::dataset_status_at;
use crate::commands::lyrics::{download_album_lyrics_at, fetch_lyrics_at, resolve_lyrics_base_url};
use crate::commands::mutations::{
    batch_write_with_readback, delete_files_queued, rename_track_queued,
    write_extra_tags_batch_with_readback, write_extra_tags_with_readback,
    write_track_with_readback, ExtraTagBatchUpdate, ExtraTagUpdate, TrackPatch, TrackUpdate,
};
use crate::commands::organizer::{sort_by_album, SortByAlbumOptions};
use crate::commands::tracks::{
    read_album, read_extra_tags, read_track_metadata, unreadable_track_data, TrackData,
};
use crate::error::ApiError;
use crate::infra::tag_catalog::suggestions as suggest_tags;
use crate::state::{
    album::{
        cover_data_url, read_album_with_cancellation, remove_cover, write_cover_upload,
        MAX_COVER_UPLOAD_BYTES,
    },
    audit::AuditState,
    background::{
        local_now, wall_clock, BackgroundRun, BackgroundState, ReleaseHealth, ReleaseOutcome,
        RunStatus, RunTrigger, INTERRUPTED_RUN_MESSAGE,
    },
    config::{ConfigSetError, ConfigState},
    conversation::ConversationState,
    events::{EventBus, EventEnvelope},
    library::{
        discover_library_roots, is_audio_file, list_directory_entries,
        scan_directory_with_cancellation, ConfinedPath, DirectoryEntry, LibraryRoots,
        PathSecurityError,
    },
    operation::{OperationCoordinator, OperationKind},
    paths::AppDataPaths,
    sqlite::CacheState,
    tasks::{TaskProgress, TaskRegistry, TaskStatus},
    worker::WorkerStore,
    write_queue::WriteQueue,
};

#[derive(Clone)]
pub struct ServerConfig {
    listen_addr: SocketAddr,
    data_dir: PathBuf,
    library_root_dir: PathBuf,
    web_root: PathBuf,
    web: WebConfig,
}

impl ServerConfig {
    fn from_env() -> anyhow::Result<Self> {
        let listen_addr = std::env::var("SOUNDROBE_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse::<SocketAddr>()?;
        let data_dir = std::env::var_os("SOUNDROBE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/config"));
        let library_root_dir = std::env::var_os("SOUNDROBE_LIBRARY_ROOT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/libraries"));
        let web_root = std::env::var_os("SOUNDROBE_WEB_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/app/dist"));
        let web = WebConfig::from_sources(
            std::env::var("SOUNDROBE_PUBLIC_URL").ok().as_deref(),
            std::env::var("SOUNDROBE_ADDITIONAL_ORIGINS")
                .ok()
                .as_deref(),
        )?;

        Ok(Self {
            listen_addr,
            data_dir,
            library_root_dir,
            web_root,
            web,
        })
    }

    #[cfg(test)]
    fn for_tests(public_url: &str) -> Self {
        Self {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            data_dir: PathBuf::from("/tmp/soundrobe-test-config"),
            library_root_dir: PathBuf::from("/tmp/soundrobe-test-libraries"),
            web_root: PathBuf::from("/tmp/soundrobe-test-web"),
            web: WebConfig::from_sources(Some(public_url), None).unwrap(),
        }
    }
}

#[derive(Clone)]
struct WebConfig {
    allowed_origins: Vec<Arc<str>>,
    secure_transport: bool,
}

impl WebConfig {
    fn from_sources(
        public_url: Option<&str>,
        additional_origins: Option<&str>,
    ) -> anyhow::Result<Self> {
        let public_url =
            public_url.ok_or_else(|| anyhow::anyhow!("SOUNDROBE_PUBLIC_URL is required"))?;
        let (public_origin, secure_transport) = parse_public_origin(public_url)?;
        let mut allowed_origins = vec![Arc::<str>::from(public_origin.as_str())];
        for value in additional_origins.unwrap_or_default().split(',') {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let (origin, _) = parse_public_origin(value).map_err(|_| {
                anyhow::anyhow!(
                    "SOUNDROBE_ADDITIONAL_ORIGINS must contain absolute HTTP(S) origins"
                )
            })?;
            if !allowed_origins
                .iter()
                .any(|allowed| allowed.as_ref() == origin)
            {
                allowed_origins.push(Arc::from(origin));
            }
        }

        Ok(Self {
            allowed_origins,
            secure_transport,
        })
    }

    fn origin_matches(&self, origin: &str) -> bool {
        self.allowed_origins
            .iter()
            .any(|allowed| allowed.as_ref() == origin)
    }
}

fn parse_public_origin(public_url: &str) -> anyhow::Result<(String, bool)> {
    let url = Url::parse(public_url)
        .map_err(|_| anyhow::anyhow!("SOUNDROBE_PUBLIC_URL must be an absolute HTTP(S) origin"))?;
    let scheme = url.scheme();
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(anyhow::anyhow!(
            "SOUNDROBE_PUBLIC_URL must use http or https"
        ));
    }
    if url.host_str().is_none() {
        return Err(anyhow::anyhow!("SOUNDROBE_PUBLIC_URL must include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow::anyhow!(
            "SOUNDROBE_PUBLIC_URL must not include credentials"
        ));
    }
    if url.path() != "" && url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(anyhow::anyhow!(
            "SOUNDROBE_PUBLIC_URL must contain only an origin"
        ));
    }

    Ok((
        url.origin().ascii_serialization(),
        scheme.eq_ignore_ascii_case("https"),
    ))
}

#[derive(Clone)]
struct ServerState {
    web: WebConfig,
    config: ConfigState,
    providers: Arc<crate::state::providers::ProviderState>,
    libraries: Result<LibraryRoots, String>,
    events: EventBus,
    operations: OperationCoordinator,
    lifecycle: ServerLifecycle,
    debug: WebDebugState,
    write_queue: WriteQueue,
    audit: Arc<AuditState>,
    cache: Arc<CacheState>,
    tasks: Arc<TaskRegistry>,
    health: Arc<RwLock<HashMap<String, ReleaseHealth>>>,
    conversations: Arc<ConversationState>,
    background: BackgroundState,
    worker: Arc<WorkerStore>,
    worker_error: Option<String>,
}

fn release_is_healthy(path: &Path) -> Result<bool, String> {
    let album = read_album(path).map_err(|error| error.to_string())?;
    let tracks = album.tracks;
    Ok(!tracks.is_empty()
        && tracks.iter().all(|track| {
            let has_identity = track
                .title
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && track
                    .artist
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                && track
                    .album
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty());
            has_identity
                && (track
                    .musicbrainz_album_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    || track
                        .discogs_release_id
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()))
        }))
}

// ---------------------------------------------------------------------------
// Background tagging: configurable, scheduled, and observable
// ---------------------------------------------------------------------------

/// How often the scheduler evaluates its crontab and the watched directories.
/// Short enough that a `*/1 * * * *` schedule lands inside its own minute.
const BACKGROUND_POLL: Duration = Duration::from_secs(20);

/// The stored name of a tag source, taken from its serialized form so the
/// ledger, the events and the API cannot drift apart.
fn lookup_source_name(source: LookupSource) -> Option<String> {
    serde_json::to_value(source)
        .ok()?
        .as_str()
        .map(str::to_string)
}

/// Directories a background run covers: the configured ones, or every mounted
/// library root when none are configured. A configured directory that has gone
/// missing is dropped rather than failing the whole run.
fn background_directories(state: &ServerState) -> Vec<PathBuf> {
    let configured = state.background.settings().directories;
    if !configured.is_empty() {
        return configured
            .into_iter()
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect();
    }
    match &state.libraries {
        Ok(roots) => roots
            .roots()
            .into_iter()
            .map(|root| PathBuf::from(root.path))
            .collect(),
        Err(error) => {
            tracing::error!(
                %error,
                "mounted library discovery failed; background tagging has no directories"
            );
            Vec::new()
        }
    }
}

/// Albums under `directories`, optionally narrowed to the ones whose core tags
/// are incomplete — the same judgement the startup sweep always made.
fn background_albums(directories: &[PathBuf], skip_unchanged: bool) -> Vec<PathBuf> {
    let mut albums = directories
        .iter()
        .flat_map(|directory| crate::state::library::scan_directory(directory))
        .map(|album| PathBuf::from(album.path))
        .collect::<Vec<_>>();
    albums.sort();
    albums.dedup();
    if skip_unchanged {
        albums.retain(|album| matches!(release_is_healthy(album), Ok(false)));
    }
    albums
}

/// Albums a retry should cover: the ones the run left needing action and that
/// are still on disk. A release that has since been deleted or moved is not a
/// retry, it is noise.
fn retryable_albums(run: &BackgroundRun) -> Vec<PathBuf> {
    run.items
        .iter()
        .filter(|item| item.status.is_retryable())
        .map(|item| PathBuf::from(&item.path))
        .filter(|path| path.is_dir())
        .collect()
}

/// Start one background tagging run over the covered directories.
fn start_background_run(state: &ServerState, trigger: RunTrigger) -> Result<String, String> {
    let settings = state.background.settings();
    if !settings.enabled {
        return Err("background tagging is disabled".to_string());
    }
    let directories = background_directories(state);
    if directories.is_empty() {
        return Err("no directories are available for background tagging".to_string());
    }
    let albums = background_albums(&directories, settings.skip_unchanged);
    start_background_run_for(state, trigger, albums)
}

fn run_trigger_name(trigger: RunTrigger) -> &'static str {
    match trigger {
        RunTrigger::Schedule => "schedule",
        RunTrigger::Manual => "manual",
        RunTrigger::Watch => "watch",
    }
}

/// Enqueue one background request durably, then start it immediately when the
/// single auto-tag operation is available. A busy operation leaves accepted
/// albums pending for the next scheduler pass rather than dropping them.
fn start_background_run_for(
    state: &ServerState,
    trigger: RunTrigger,
    albums: Vec<PathBuf>,
) -> Result<String, String> {
    if albums.is_empty() {
        return Err("no directories are available for background tagging".to_string());
    }
    let albums = state.worker.enqueue(&albums, run_trigger_name(trigger))?;
    if albums.is_empty() {
        return Err("all requested albums are already queued or running".to_string());
    }
    start_background_batch(state, trigger, albums)
}

/// Start one background tagging run over already-enqueued `albums` and return
/// its task id. Exactly one run holds the [`OperationKind::AutoTag`] lock.
fn start_background_batch(
    state: &ServerState,
    trigger: RunTrigger,
    albums: Vec<PathBuf>,
) -> Result<String, String> {
    if albums.is_empty() {
        let run = state.background.record_run_checked(trigger, None)?;
        state.background.finish_run_checked(
            &run.id,
            RunStatus::Completed,
            0,
            0,
            "every album already has complete tags",
            Vec::new(),
        )?;
        return Err("every album already has complete tags".to_string());
    }

    let token = state
        .operations
        .try_start(OperationKind::AutoTag)
        .map_err(|error| error.to_string())?;

    let total = albums.len() as u64;
    let task_id = state.tasks.create(
        "background-auto-tag",
        total,
        "Starting background tagging...",
    );
    let run = match state
        .background
        .record_run_checked(trigger, Some(task_id.clone()))
    {
        Ok(run) => run,
        Err(error) => {
            state.operations.finish(token);
            state.tasks.finish(
                &task_id,
                TaskStatus::Failed,
                "Background run could not be persisted",
                serde_json::json!({ "error": error }),
            );
            return Err(error);
        }
    };
    let run_id = run.id.clone();

    let tasks = state.tasks.clone();
    let events = state.events.clone();
    let background = state.background.clone();
    let operations = state.operations.clone();
    let providers = state.providers.clone();
    let cache = state.cache.clone();
    let queue = state.write_queue.clone();
    let config = state.config.clone();
    let health = state.health.clone();
    let worker = state.worker.clone();
    let task_id_for_work = task_id.clone();

    tokio::spawn(async move {
        let Some(cancelled) = tasks.cancellation(&task_id_for_work) else {
            operations.finish(token);
            if let Err(error) = background.finish_run_checked(
                &run_id,
                RunStatus::Skipped,
                0,
                0,
                "the run was replaced before it started",
                Vec::new(),
            ) {
                tracing::error!(%error, %run_id, "skipped background run could not be persisted");
            }
            return;
        };

        let config_values = config.raw();
        let alias_file = config.alias_file_path();
        let mut tagged = 0u64;
        let mut failed = 0u64;
        let mut needing_review = 0u64;
        let mut last_message = String::new();
        // What this run did per release. The reference product keeps this in a
        // database row per file so an operator can see which file failed and
        // why after the fact; the run history is where that lives here.
        let mut items = Vec::new();

        for (index, album) in albums.iter().enumerate() {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            let key = album.to_string_lossy().into_owned();
            let name = album
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let message = format!("Tagging {name} ({}/{total})", index + 1);
            if let Err(error) = worker.mark_running(album, &run_id) {
                failed += 1;
                last_message = format!("worker checkpoint failed: {error}");
                items.push(ReleaseOutcome {
                    path: key.clone(),
                    status: ReleaseHealth::Failed,
                    reason: Some(last_message.clone()),
                    authority: None,
                });
                health.write().await.insert(key, ReleaseHealth::Failed);
                continue;
            }
            tasks.update(&task_id_for_work, index as u64, &message);
            health
                .write()
                .await
                .insert(key.clone(), ReleaseHealth::Tagging);
            let _ = events.publish(
                "background:event",
                &serde_json::json!({
                    "taskId": task_id_for_work,
                    "kind": "progress",
                    "message": message,
                    "progress": index,
                    "total": total,
                }),
            );

            let progress_tasks = tasks.clone();
            let progress_task_id = task_id_for_work.clone();
            let review_worker = worker.clone();
            let review_album = album.clone();
            let review_run_id = run_id.clone();
            let result = resolve_and_apply_album_with_review_intent(
                album,
                &config_values,
                AutoTagServices {
                    providers: providers.as_ref(),
                    cache: cache.as_ref(),
                    queue: &queue,
                    alias_file: &alias_file,
                },
                &cancelled,
                Some(AutoTagRetryContexts::new(Arc::clone(&cancelled))),
                move |_candidate| {
                    review_worker
                        .record_review_flag(&review_album, "ai_fallback", &review_run_id)
                        .map_err(|error| {
                            ApiError::Message(format!(
                                "AI review checkpoint failed before tag writes: {error}"
                            ))
                        })
                },
                move |_step, message| {
                    progress_tasks.update(&progress_task_id, index as u64, message);
                },
                |_kind, _message, _data| {},
            )
            .await;

            let mut release_health = match &result {
                Ok(result) if result.outcome == AutoTagOutcome::Applied => {
                    tagged += 1;
                    ReleaseHealth::Healthy
                }
                Ok(result) => {
                    needing_review += 1;
                    last_message = result
                        .reason_code
                        .clone()
                        .unwrap_or_else(|| "needs review".to_string());
                    ReleaseHealth::NeedsReview
                }
                Err(error) => {
                    failed += 1;
                    last_message = error.to_string();
                    ReleaseHealth::Failed
                }
            };
            let reason = match &result {
                Ok(result) => result.reason_code.clone(),
                Err(error) => Some(error.to_string()),
            };
            let authority = result
                .as_ref()
                .ok()
                .and_then(|result| result.authority)
                .and_then(lookup_source_name);
            if !cancelled.load(Ordering::Acquire) {
                let worker_status = match release_health {
                    ReleaseHealth::Healthy => "completed",
                    ReleaseHealth::NeedsReview => "needs_review",
                    ReleaseHealth::Tagging => "running",
                    ReleaseHealth::NeedsTagging => "pending",
                    ReleaseHealth::Unreadable | ReleaseHealth::Failed => "failed",
                };
                if let Err(error) = worker.mark_finished(
                    album,
                    worker_status,
                    reason.as_deref(),
                    authority.as_deref(),
                ) {
                    failed += 1;
                    last_message = format!("worker completion checkpoint failed: {error}");
                    release_health = ReleaseHealth::Failed;
                }
            }
            items.push(ReleaseOutcome {
                path: key.clone(),
                status: release_health,
                reason,
                authority,
            });
            health.write().await.insert(key, release_health);
        }

        let cancellation_error = if cancelled.load(Ordering::Acquire) {
            worker
                .cancel_paths(&albums, "operator_cancelled")
                .err()
                .map(|error| format!("worker cancellation checkpoint failed: {error}"))
        } else {
            None
        };
        if let Some(error) = &cancellation_error {
            failed += 1;
            last_message = error.clone();
        }

        let (status, message) = if let Some(error) = cancellation_error {
            (RunStatus::Failed, error)
        } else if cancelled.load(Ordering::Acquire) {
            (RunStatus::Cancelled, "Cancelled".to_string())
        } else if failed > 0 {
            (
                RunStatus::Failed,
                format!(
                    "Tagged {tagged}, {needing_review} need review, {failed} failed: {last_message}"
                ),
            )
        } else if needing_review > 0 {
            (
                RunStatus::Completed,
                format!("Tagged {tagged}, {needing_review} need review"),
            )
        } else {
            (RunStatus::Completed, format!("Tagged {tagged} of {total}"))
        };

        let task_status = match status {
            RunStatus::Cancelled => TaskStatus::Cancelled,
            RunStatus::Failed => TaskStatus::Failed,
            _ if needing_review > 0 => TaskStatus::NeedsReview,
            _ => TaskStatus::Completed,
        };
        let task_result = serde_json::json!({
            "tagged": tagged,
            "failed": failed,
            "needsReview": needing_review,
        });
        if task_status == TaskStatus::Failed && cancelled.load(Ordering::Acquire) {
            tasks.finish_cancellation_failure(&task_id_for_work, &message, task_result);
        } else {
            tasks.finish(&task_id_for_work, task_status, &message, task_result);
        }
        if let Err(error) =
            background.finish_run_checked(&run_id, status, tagged, failed, &message, items)
        {
            tracing::error!(%error, %run_id, "background run completion could not be persisted");
        }
        let _ = events.publish(
            "background:event",
            &serde_json::json!({
                "taskId": task_id_for_work,
                "kind": "finished",
                "message": message,
                "progress": total,
                "total": total,
            }),
        );
        operations.finish(token);
    });

    Ok(task_id)
}

fn start_pending_background_run(state: &ServerState) -> Result<String, String> {
    let albums = state.worker.pending_result()?;
    if albums.is_empty() {
        return Err("no pending background albums".to_string());
    }
    start_background_batch(state, RunTrigger::Manual, albums)
}

/// Which albums are new or changed compared with the previous poll.
///
/// The first poll after watching starts records only what already exists. That
/// baseline matters: without it every album in the library would look new and
/// the whole library would be re-tagged. After the baseline, an album that has
/// not been seen before *is* new, which is how music added while the service is
/// running gets noticed, as is an album whose folder changed.
fn changed_albums(
    scanned: Vec<(PathBuf, std::time::SystemTime)>,
    seen: &mut HashMap<PathBuf, std::time::SystemTime>,
    baseline: bool,
) -> Vec<PathBuf> {
    let mut changed = Vec::new();
    for (path, modified) in scanned {
        let is_new = match seen.get(&path) {
            Some(previous) => modified > *previous,
            None => !baseline,
        };
        seen.insert(path.clone(), modified);
        if is_new {
            changed.push(path);
        }
    }
    changed
}

/// Albums whose folder changed since the last poll, so newly added music is
/// picked up without re-tagging the whole library on every tick.
fn watched_albums(
    state: &ServerState,
    seen: &mut HashMap<PathBuf, std::time::SystemTime>,
    baseline: bool,
) -> Vec<PathBuf> {
    let directories = background_directories(state);
    let mut scanned = Vec::new();
    for directory in directories {
        for album in crate::state::library::scan_directory(&directory) {
            let path = PathBuf::from(&album.path);
            if let Ok(modified) = fs::metadata(&path).and_then(|metadata| metadata.modified()) {
                scanned.push((path, modified));
            }
        }
    }
    changed_albums(scanned, seen, baseline)
}

/// Albums the watcher has discovered but has not started a run for yet.
///
/// The reference product hands a newly found file to a broker queue, so a file
/// that arrives while a run is already in progress waits its turn. This is the
/// same guarantee without a broker: an album discovered by the watcher stays
/// here until a run actually starts for it, so a busy tag lock delays the album
/// instead of forgetting it until its folder is touched again.
#[derive(Debug, Default)]
struct WatchQueue {
    pending: Vec<PathBuf>,
}

impl WatchQueue {
    /// Bound on the backlog. A library growing faster than it can be tagged is
    /// a real problem, but it is not this queue's job to page someone about it;
    /// the scheduled sweep covers everything that is dropped from here.
    const LIMIT: usize = 500;

    /// Queue newly discovered albums, oldest first, without duplicates.
    fn enqueue(&mut self, discovered: impl IntoIterator<Item = PathBuf>) {
        for album in discovered {
            if self.pending.len() >= Self::LIMIT {
                tracing::warn!(
                    album = %album.display(),
                    limit = Self::LIMIT,
                    "watched album backlog is full; leaving it to the next scheduled run"
                );
                continue;
            }
            if !self.pending.contains(&album) {
                self.pending.push(album);
            }
        }
    }

    /// Take everything to attempt now. Whatever fails to start is put back.
    fn take(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.pending)
    }

    /// Put an album back at the front, so the next poll tries it first.
    fn requeue(&mut self, album: PathBuf) {
        if !self.pending.contains(&album) {
            self.pending.insert(0, album);
        }
    }
}

/// Keep background tagging running: one sweep at startup (the behaviour that
/// predates any settings), then every due schedule slot, plus watched
/// directories when that is enabled.
fn start_background_library_tagging(state: ServerState) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        if let Some(error) = &state.worker_error {
            tracing::error!(%error, "background tagging disabled because durable state is unavailable");
            return;
        }
        // Record what every covered release looks like before touching
        // anything, so Monitoring can show which ones need tagging even when a
        // run has nothing to do.
        let directories = background_directories(&state);
        let surveyed = tokio::task::spawn_blocking(move || {
            directories
                .iter()
                .flat_map(|directory| crate::state::library::scan_directory(directory))
                .map(|album| {
                    let path = PathBuf::from(&album.path);
                    let status = match release_is_healthy(&path) {
                        Ok(true) => ReleaseHealth::Healthy,
                        Ok(false) => ReleaseHealth::NeedsTagging,
                        Err(_) => ReleaseHealth::Unreadable,
                    };
                    (path.to_string_lossy().into_owned(), status)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        for (path, status) in surveyed {
            state.health.write().await.insert(path, status);
        }

        if !state.background.settings().enabled {
            tracing::info!("background tagging is disabled by configuration");
        } else if state
            .worker
            .pending_result()
            .is_ok_and(|pending| !pending.is_empty())
        {
            match start_pending_background_run(&state) {
                Ok(task_id) => tracing::info!(%task_id, "recovered background tagging started"),
                Err(error) => tracing::debug!(%error, "recovered background tagging did not start"),
            }
        } else {
            match start_background_run(&state, RunTrigger::Manual) {
                Ok(task_id) => tracing::info!(%task_id, "startup background tagging started"),
                Err(error) => {
                    tracing::debug!(%error, "startup background tagging did not start")
                }
            }
        }

        let mut seen = HashMap::new();
        let mut watch_baseline = true;
        let mut watch_queue = WatchQueue::default();
        loop {
            tokio::time::sleep(BACKGROUND_POLL).await;
            let now = wall_clock(local_now());

            if state.background.settings().enabled
                && state
                    .worker
                    .pending_result()
                    .is_ok_and(|pending| !pending.is_empty())
            {
                if let Err(error) = start_pending_background_run(&state) {
                    tracing::debug!(%error, "pending background tagging did not start");
                }
            }

            if let Some(slot) = state.background.due(now) {
                tracing::info!(?slot, "background tagging schedule came due");
                match start_background_run(&state, RunTrigger::Schedule) {
                    Ok(task_id) => {
                        tracing::info!(%task_id, "scheduled background tagging started")
                    }
                    // `due` already retired the slot. Accepted albums remain in
                    // the durable queue, so a busy lock defers work without
                    // creating an unbounded in-memory retry.
                    Err(error) => {
                        tracing::warn!(%error, "scheduled background tagging did not start")
                    }
                }
            }

            if state.background.settings().watch_enabled {
                watch_queue.enqueue(watched_albums(&state, &mut seen, watch_baseline));
                watch_baseline = false;
                for album in watch_queue.take() {
                    match start_background_run_for(&state, RunTrigger::Watch, vec![album.clone()]) {
                        Ok(task_id) => {
                            tracing::info!(%task_id, album = %album.display(), "watch background tagging started")
                        }
                        Err(error) => {
                            tracing::debug!(%error, album = %album.display(), "watch background tagging did not start");
                            if !state.worker.is_active(&album) {
                                watch_queue.requeue(album);
                            }
                        }
                    }
                }
            } else {
                // Re-arm while watching is off, so switching it back on takes a
                // fresh baseline instead of treating the whole library as new.
                seen.clear();
                watch_baseline = true;
                watch_queue = WatchQueue::default();
            }
        }
    });
}

async fn list_tasks(State(state): State<ServerState>) -> Json<Vec<TaskProgress>> {
    Json(state.tasks.list())
}

#[derive(Clone, Default)]
struct WebDebugState {
    enabled: Arc<AtomicBool>,
}

impl WebDebugState {
    fn new(enabled: bool) -> Self {
        let state = Self::default();
        state.set_enabled(enabled);
        state
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
}

#[derive(Clone, Default)]
struct ServerLifecycle {
    shutting_down: Arc<AtomicBool>,
    cancellation: CancellationToken,
}

impl ServerLifecycle {
    fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
        self.cancellation.cancel();
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyCommandRequest {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WrappedCommandRequest<T> {
    request: T,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanCommandRequest {
    #[serde(rename = "dirPath")]
    dir_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryListCommandRequest {
    #[serde(rename = "dirPath")]
    dir_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileExistsCommandRequest {
    #[serde(rename = "filePath")]
    file_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigSetCommandRequest {
    key: String,
    value: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TagSuggestionsCommandRequest {
    #[serde(default)]
    query: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestLlmConnectionCommandRequest {
    #[serde(rename = "apiKey", default)]
    api_key: String,
    model: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(rename = "baseUrl", default)]
    base_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DebugSetModeCommandRequest {
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackWriteCommandRequest {
    path: String,
    fields: TrackPatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackBatchWriteCommandRequest {
    updates: Vec<TrackUpdate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtraTagsReadCommandRequest {
    #[serde(rename = "trackPath")]
    track_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtraTagsWriteCommandRequest {
    #[serde(rename = "trackPath")]
    track_path: String,
    tags: Vec<ExtraTagUpdate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtraTagsBatchWriteCommandRequest {
    updates: Vec<ExtraTagBatchUpdate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackRenameCommandRequest {
    #[serde(rename = "oldPath")]
    old_path: String,
    #[serde(rename = "newPath")]
    new_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackDeleteCommandRequest {
    #[serde(rename = "filePaths")]
    file_paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LyricsFetchCommandRequest {
    #[serde(rename = "trackName")]
    track_name: String,
    #[serde(rename = "artistName")]
    artist_name: String,
    #[serde(rename = "albumName", default)]
    album_name: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlbumReadCommandRequest {
    #[serde(rename = "albumPath")]
    album_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditRunCommandRequest {
    #[serde(rename = "libraryPath")]
    library_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditSpecifiedCommandRequest {
    #[serde(rename = "trackPaths", default)]
    track_paths: Option<Vec<String>>,
    #[serde(rename = "albumPaths", default)]
    album_paths: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditApplyFixesCommandRequest {
    #[serde(rename = "albumResults")]
    album_results: Vec<AuditAlbumResult>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskProgressCommandRequest {
    #[serde(rename = "taskId")]
    task_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssistantListSessionsCommandRequest {
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssistantSessionCommandRequest {
    #[serde(rename = "sessionUuidOrNumber")]
    session_uuid_or_number: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrganizerCommandRequest {
    #[serde(rename = "sourceDir")]
    source_dir: String,
    options: Option<SortByAlbumOptions>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverDataCommandRequest {
    #[serde(rename = "albumPath")]
    album_path: String,
    #[serde(rename = "preferredTrackPath", default)]
    preferred_track_path: Option<String>,
}

#[derive(Serialize)]
struct WebAppInfo {
    identifier: &'static str,
    version: &'static str,
    runtime: &'static str,
    dev: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn api_not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "not found")
}

fn safe_web_root(path: &Path) -> PathBuf {
    let invalid = PathBuf::from("/__soundrobe_invalid_web_root__");
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return path.to_path_buf();
    };
    if metadata.file_type().is_symlink() {
        tracing::error!(path = %path.display(), "web root symlink is not allowed");
        return invalid;
    }
    let Ok(root) = path.canonicalize() else {
        return path.to_path_buf();
    };
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            return invalid;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                return invalid;
            };
            if file_type.is_symlink() {
                tracing::error!(path = %entry.path().display(), "web asset symlink is not allowed");
                return invalid;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    root
}

async fn libraries(State(state): State<ServerState>) -> Response {
    match &state.libraries {
        Ok(roots) => Json(roots.roots()).into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "library roots unavailable",
        ),
    }
}

#[derive(Debug, Deserialize, Default)]
struct EventsQuery {
    after: Option<String>,
    channels: Option<String>,
}

fn event_channels(query: &EventsQuery) -> Vec<String> {
    query
        .channels
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .filter(|channel| !channel.is_empty())
        .map(str::to_string)
        .collect()
}

fn event_matches(event: &EventEnvelope, channels: &[String]) -> bool {
    channels.is_empty() || channels.iter().any(|channel| channel == &event.channel)
}

#[derive(Clone)]
struct EventCursor {
    generation: String,
    sequence: u64,
}

fn parse_event_cursor(value: &str) -> Option<EventCursor> {
    let (generation, sequence) = value.rsplit_once(':')?;
    Some(EventCursor {
        generation: generation.to_string(),
        sequence: sequence.parse().ok()?,
    })
}

fn last_event_id(headers: &HeaderMap) -> Option<EventCursor> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_event_cursor)
}

fn sse_event(envelope: EventEnvelope) -> Result<Event, Infallible> {
    Ok(Event::default()
        .id(format!("{}:{}", envelope.generation, envelope.sequence))
        .event(envelope.channel)
        .json_data(envelope.payload)
        .expect("event payloads are serialized JSON values"))
}

async fn events(
    State(state): State<ServerState>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let channels = event_channels(&query);
    let channel_refs = channels.iter().map(String::as_str).collect::<Vec<_>>();
    let cursor = query
        .after
        .as_deref()
        .and_then(parse_event_cursor)
        .or_else(|| last_event_id(&headers));
    let after = cursor.as_ref().map_or(0, |cursor| cursor.sequence);
    let generation = cursor.as_ref().map(|cursor| cursor.generation.as_str());
    let (replay, receiver, gap) =
        state
            .events
            .replay_and_subscribe(after, generation, &channel_refs);
    let stream_state = EventStreamState {
        replay: replay.into(),
        receiver,
        channels,
        last_sequence: after,
        cancellation: state.lifecycle.cancellation.clone(),
        generation: state.events.generation().to_string(),
        gap,
        terminated: false,
    };
    let stream = futures_util::stream::unfold(stream_state, next_event);
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

struct EventStreamState {
    replay: VecDeque<EventEnvelope>,
    receiver: tokio::sync::broadcast::Receiver<EventEnvelope>,
    channels: Vec<String>,
    last_sequence: u64,
    generation: String,
    cancellation: CancellationToken,
    gap: bool,
    terminated: bool,
}

async fn next_event(
    mut state: EventStreamState,
) -> Option<(Result<Event, Infallible>, EventStreamState)> {
    if state.terminated || state.cancellation.is_cancelled() {
        return None;
    }
    if state.gap {
        state.gap = false;
        state.terminated = true;
        let payload = serde_json::json!({
            "after": state.last_sequence,
            "message": "event replay window was exceeded; resync required",
        });
        return Some((
            Ok(Event::default()
                .event("soundrobe:replay-gap")
                .id(format!("{}:0", state.generation))
                .data(serde_json::to_string(&payload).expect("replay gap payload is JSON"))),
            state,
        ));
    }
    if let Some(event) = state.replay.pop_front() {
        state.last_sequence = event.sequence;
        return Some((sse_event(event), state));
    }

    loop {
        let received = tokio::select! {
            _ = state.cancellation.cancelled() => return None,
            received = state.receiver.recv() => received,
        };
        match received {
            Ok(event) => {
                if event.sequence <= state.last_sequence || !event_matches(&event, &state.channels)
                {
                    continue;
                }
                state.last_sequence = event.sequence;
                return Some((sse_event(event), state));
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                // Closing makes EventSource reconnect with Last-Event-ID so the
                // bounded replay window can be applied deterministically.
                return None;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        }
    }
}

/// `directory:read` response. Field names match the desktop command so the
/// browser adapter can use one payload shape across both runtimes.
#[derive(Debug, Clone, Serialize)]
struct DirectoryReadResponse {
    path: String,
    name: String,
    subdirs: Vec<DirectoryEntry>,
    tracks: Vec<TrackData>,
    #[serde(rename = "audioCount")]
    audio_count: usize,
}

/// Read direct audio children plus subdirectories for the browser workspace.
///
/// The browser workspace browses folders from a tree and lists only the files
/// of the selected directory, so it needs subdirectories and parsed tracks in
/// one round trip. This mirrors `commands::directories::read_directory`, which
/// is gated behind the desktop feature; keep the two in step, because the
/// payload shape is part of the `DesktopAPI` contract both runtimes share.
///
/// Missing or unreadable directories yield empty subdirs/tracks rather than
/// rejecting, matching the desktop boundary so an unreadable folder cannot
/// blank the folder tree.
fn read_directory_for_web(dir_path: &Path) -> DirectoryReadResponse {
    let subdirs = list_directory_entries(dir_path);
    let mut audio_files: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(dir_path) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let full_path = entry.path();
            if file_type.is_file() && is_audio_file(&full_path) {
                audio_files.push(full_path);
            }
        }
    }
    audio_files.sort();

    // Read errors stay per-file: one unreadable track must not blank the rest.
    let tracks = audio_files
        .into_iter()
        .map(|path| match read_track_metadata(&path) {
            Ok(mut track) => {
                if track.codec == "unknown" && track.title.is_none() {
                    track.title = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                }
                track
            }
            Err(error) => {
                tracing::warn!("failed to read directory track {}: {error}", path.display());
                let size = fs::metadata(&path).map(|stat| stat.len()).unwrap_or(0);
                let title = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                unreadable_track_data(&path, size, title)
            }
        })
        .collect::<Vec<_>>();

    let name = dir_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    DirectoryReadResponse {
        path: dir_path.to_string_lossy().into_owned(),
        name,
        subdirs,
        audio_count: tracks.len(),
        tracks,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BackgroundSettingsPatch {
    enabled: Option<bool>,
    schedule: Option<String>,
    directories: Option<Vec<String>>,
    watch_enabled: Option<bool>,
    skip_unchanged: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BackgroundRetryRequest {
    /// Which run's releases to retry. Omitted means the newest run that has
    /// finished.
    #[serde(default)]
    run_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MonitoringReviewRequest {
    album_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MonitoringLogsRequest {
    source: Option<String>,
    lines: Option<usize>,
}

/// Largest log slice Monitoring will return, so a huge debug file cannot be
/// turned into an equally huge response.
const MAX_LOG_LINES: usize = 2000;

/// Settings, schedule state, and recent runs in one payload — what the
/// Monitoring and Settings views both need to render.
async fn background_status(state: &ServerState) -> serde_json::Value {
    let settings = state.background.settings();
    let now = wall_clock(local_now());
    serde_json::json!({
        "enabled": settings.enabled,
        "schedule": settings.schedule,
        "directories": settings.directories,
        "watchEnabled": settings.watch_enabled,
        "skipUnchanged": settings.skip_unchanged,
        "nextRunAt": state.background.peek_next(now).map(|slot| slot.to_string()),
        "effectiveDirectories": background_directories(state)
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        "running": state
            .tasks
            .list()
            .into_iter()
            .filter(|task| task.status == TaskStatus::Running)
            .collect::<Vec<_>>(),
        "recentRuns": state.background.runs(),
    })
}

/// Per-release tagging state, newest survey plus durable review flags.
async fn monitoring_releases(state: &ServerState) -> Result<serde_json::Value, String> {
    let review_paths = match state.worker.actionable_review_paths() {
        Ok(paths) => paths,
        Err(error) => return Err(format!("durable worker review state unavailable: {error}")),
    }
    .into_iter()
    .map(|path| path.to_string_lossy().into_owned())
    .collect::<HashSet<_>>();
    let health = state.health.read().await;
    let mut known = HashSet::new();
    let mut releases = health
        .iter()
        .map(|(path, status)| {
            known.insert(path.clone());
            let status = if review_paths.contains(path) {
                ReleaseHealth::NeedsReview
            } else {
                *status
            };
            serde_json::json!({
                "path": path,
                "status": status,
            })
        })
        .collect::<Vec<_>>();
    for path in review_paths {
        if known.insert(path.clone()) {
            releases.push(serde_json::json!({
                "path": path,
                "status": ReleaseHealth::NeedsReview,
            }));
        }
    }
    releases.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    Ok(serde_json::json!({ "releases": releases }))
}

/// The newest `auto-tag-debug-*.log`, which is the file the desktop and server
/// share for verbose auto-tag tracing.
fn newest_debug_log(data_dir: &Path) -> Option<PathBuf> {
    let mut candidates = fs::read_dir(data_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("auto-tag-debug-") && name.ends_with(".log"))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.pop()
}

/// The last `lines` lines of a log source, mirroring the reference product's
/// source-selectable service log viewer using the logs this app actually writes.
fn read_log_tail(path: &Path, lines: usize) -> Result<serde_json::Value, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let lines = lines.clamp(1, MAX_LOG_LINES);
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    Ok(serde_json::json!({
        "path": path.to_string_lossy().into_owned(),
        "totalLines": all.len(),
        "lines": &all[start..],
    }))
}

fn supported_web_command(command: &str) -> bool {
    matches!(
        command,
        "app:info"
            | "background:get-settings"
            | "background:set-settings"
            | "background:list-runs"
            | "background:run-now"
            | "background:retry-failed"
            | "monitoring:status"
            | "monitoring:releases"
            | "monitoring:mark-reviewed"
            | "monitoring:logs"
            | "library:list-roots"
            | "library:scan"
            | "album:read"
            | "album:search-releases"
            | "album:resolve-release"
            | "album:release-track-count"
            | "album:preview-release-match"
            | "album:search-apply-candidate"
            | "files:sort-by-album"
            | "album:auto-tag"
            | "task:progress"
            | "task:cancel"
            | "dataset:status"
            | "assistant:list-sessions"
            | "assistant:get-conversation"
            | "assistant:get-session"
            | "assistant:current-session"
            | "audit:run"
            | "audit:run-specified"
            | "audit:run-album"
            | "audit:apply-fixes"
            | "audit:cancel"
            | "album:refresh"
            | "track:write"
            | "tracks:batch-write"
            | "track:extra-tags:read"
            | "track:extra-tags:write"
            | "tracks:batch-write-extra-tags"
            | "track:rename"
            | "track:delete-files"
            | "lyrics:fetch"
            | "album:download-lyrics"
            | "cover:data-url"
            | "cover:remove"
            | "cover:download"
            | "cover:download-artist-art"
            | "directory:list"
            | "directory:read"
            | "file:exists"
            | "config:get"
            | "config:set"
            | "tags:suggest"
            | "test-llm-connection"
            | "debug:set-mode"
    )
}

fn decode_command_payload<T: DeserializeOwned>(payload: serde_json::Value) -> Result<T, ()> {
    serde_json::from_value(payload).map_err(|_| ())
}

fn scan_path_error(error: PathSecurityError) -> Response {
    match error {
        PathSecurityError::RelativePath | PathSecurityError::Traversal => {
            error_response(StatusCode::BAD_REQUEST, "invalid library path")
        }
        PathSecurityError::OutsideRoots | PathSecurityError::CrossRoot => {
            error_response(StatusCode::FORBIDDEN, "path outside library roots")
        }
        PathSecurityError::Unresolvable(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "library path unavailable",
        ),
    }
}

fn media_error(error: crate::error::ApiError) -> Response {
    match error {
        crate::error::ApiError::NotImplemented(_)
        | crate::error::ApiError::UnsupportedFormat(_)
        | crate::error::ApiError::Message(_) => {
            error_response(StatusCode::BAD_REQUEST, "unsupported media operation")
        }
        crate::error::ApiError::Io(_)
        | crate::error::ApiError::Lofty(_)
        | crate::error::ApiError::MediaSafety(_)
        | crate::error::ApiError::WriteTask(_)
        | crate::error::ApiError::ReadTask(_) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "media operation failed")
        }
        #[cfg(feature = "desktop")]
        crate::error::ApiError::Tauri(_) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "media operation failed")
        }
        crate::error::ApiError::ContextMenuAlreadyActive
        | crate::error::ApiError::ContextMenuStatePoisoned => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "media operation failed")
        }
    }
}

fn resolve_existing_track(
    roots: &LibraryRoots,
    supplied: &str,
) -> Result<ConfinedPath, Box<Response>> {
    let resolved = roots
        .resolve_path(Path::new(supplied))
        .map_err(|error| Box::new(scan_path_error(error)))?;
    if !resolved.path.is_file() {
        return Err(Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "track path not found",
        )));
    }
    Ok(resolved)
}

async fn execute_audit(state: &ServerState, paths: Vec<String>) -> Response {
    let token = match start_audit(state.audit.as_ref()) {
        Ok(token) => token,
        Err(error) => return media_error(error),
    };
    let (client, remote) = audit_clients(&state.providers, &state.config);
    let events = state.events.clone();
    let emit = move |event| crate::commands::audit::emit_audit(&events, event);
    let operation = audit_specific_albums(
        &emit,
        paths,
        client,
        remote,
        state.config.alias_file_path(),
        &token,
    );
    match finish_audit_run(&emit, state.audit.as_ref(), token.clone(), operation).await {
        Ok(summary) => Json(summary).into_response(),
        Err(error) => media_error(error),
    }
}

fn start_auto_tag_task(state: &ServerState, album_path: PathBuf) -> Response {
    let token = match state.operations.try_start(OperationKind::AutoTag) {
        Ok(token) => token,
        Err(_) => return error_response(StatusCode::CONFLICT, "another operation is busy"),
    };
    match state
        .worker
        .enqueue(std::slice::from_ref(&album_path), "manual")
    {
        Ok(accepted) if accepted.is_empty() => {
            state.operations.finish(token);
            return error_response(StatusCode::CONFLICT, "album is already queued or running");
        }
        Ok(_) => {}
        Err(error) => {
            state.operations.finish(token);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("durable worker queue unavailable: {error}"),
            );
        }
    }
    let task_id = state.tasks.create("auto-tag", 9, "Starting...");
    let task_id_for_work = task_id.clone();
    let task_state = state.tasks.clone();
    let event_bus = state.events.clone();
    let providers = state.providers.clone();
    let cache = state.cache.clone();
    let queue = state.write_queue.clone();
    let config = state.config.clone();
    let operations = state.operations.clone();
    let worker = state.worker.clone();
    tokio::spawn(async move {
        let Some(cancelled) = task_state.cancellation(&task_id_for_work) else {
            operations.finish(token);
            return;
        };
        let config_values = config.raw();
        let alias_file = config.alias_file_path();
        let progress_tasks = task_state.clone();
        let progress_events = event_bus.clone();
        let progress_task_id = task_id_for_work.clone();
        let report_tasks = task_state.clone();
        let report_events = event_bus.clone();
        let report_task_id = task_id_for_work.clone();
        if let Err(error) = worker.mark_running(&album_path, &task_id_for_work) {
            let message = format!("durable worker checkpoint failed: {error}");
            task_state.finish(
                &task_id_for_work,
                TaskStatus::Failed,
                &message,
                serde_json::json!({"error": message}),
            );
            operations.finish(token);
            return;
        }
        let review_worker = worker.clone();
        let review_album = album_path.clone();
        let review_run_id = task_id_for_work.clone();
        let result = resolve_and_apply_album_with_review_intent(
            &album_path,
            &config_values,
            AutoTagServices {
                providers: providers.as_ref(),
                cache: cache.as_ref(),
                queue: &queue,
                alias_file: &alias_file,
            },
            &cancelled,
            Some(AutoTagRetryContexts::new(Arc::clone(&cancelled))),
            move |_candidate| {
                review_worker
                    .record_review_flag(&review_album, "ai_fallback", &review_run_id)
                    .map_err(|error| {
                        ApiError::Message(format!(
                            "AI review checkpoint failed before tag writes: {error}"
                        ))
                    })
            },
            move |step, message| {
                if progress_tasks.update(&progress_task_id, step, message) {
                    let _ = progress_events.publish(
                        "auto-tag:event",
                        &auto_tag_event(&progress_task_id, "progress", message, step, None),
                    );
                }
            },
            move |kind, message, data| {
                if let Some(task) = report_tasks.get(&report_task_id) {
                    if task.status != TaskStatus::Cancelled {
                        let _ = report_events.publish(
                            "auto-tag:event",
                            &auto_tag_event(&report_task_id, kind, message, task.progress, data),
                        );
                    }
                }
            },
        )
        .await;

        match result {
            Ok(result) if result.outcome == AutoTagOutcome::Applied => {
                let candidate = result
                    .candidate
                    .as_ref()
                    .expect("applied auto-tag has a candidate");
                let data = serde_json::to_value(&result).unwrap_or_default();
                let message = auto_tag_completion_message(candidate);
                if let Err(error) = worker.mark_finished(
                    &album_path,
                    "completed",
                    result.reason_code.as_deref(),
                    result.authority.and_then(lookup_source_name).as_deref(),
                ) {
                    let message = format!("durable worker completion failed: {error}");
                    task_state.finish(
                        &task_id_for_work,
                        TaskStatus::Failed,
                        &message,
                        serde_json::json!({"error": message}),
                    );
                    operations.finish(token);
                    return;
                }
                if !task_state.finish(
                    &task_id_for_work,
                    TaskStatus::Completed,
                    message,
                    data.clone(),
                ) {
                    operations.finish(token);
                    return;
                }
                let _ = event_bus.publish(
                    "auto-tag:event",
                    &auto_tag_event(&task_id_for_work, "completed", message, 9, Some(data)),
                );
            }
            Ok(result) => {
                let data = serde_json::to_value(&result).unwrap_or_default();
                let message = result
                    .reason_code
                    .as_deref()
                    .map(|reason| format!("Needs review: {reason}"))
                    .unwrap_or_else(|| "Needs review".to_string());
                if let Err(error) = worker.mark_finished(
                    &album_path,
                    "needs_review",
                    result.reason_code.as_deref(),
                    result.authority.and_then(lookup_source_name).as_deref(),
                ) {
                    let message = format!("durable worker completion failed: {error}");
                    task_state.finish(
                        &task_id_for_work,
                        TaskStatus::Failed,
                        &message,
                        serde_json::json!({"error": message}),
                    );
                    operations.finish(token);
                    return;
                }
                if task_state.finish(
                    &task_id_for_work,
                    TaskStatus::NeedsReview,
                    &message,
                    data.clone(),
                ) {
                    let _ = event_bus.publish(
                        "auto-tag:event",
                        &auto_tag_event(&task_id_for_work, "needs_review", message, 9, Some(data)),
                    );
                }
            }
            Err(error) if cancelled.load(Ordering::Acquire) => {
                let progress = task_state
                    .get(&task_id_for_work)
                    .map(|task| task.progress)
                    .unwrap_or(0);
                match worker.cancel_paths(std::slice::from_ref(&album_path), "operator_cancelled") {
                    Ok(_) => {
                        task_state.finish(
                            &task_id_for_work,
                            TaskStatus::Cancelled,
                            "Cancelled",
                            serde_json::Value::Null,
                        );
                        let _ = event_bus.publish(
                            "auto-tag:event",
                            &auto_tag_event(
                                &task_id_for_work,
                                "cancelled",
                                "Cancelled",
                                progress,
                                None,
                            ),
                        );
                    }
                    Err(checkpoint_error) => {
                        let message = format!(
                            "Cancellation could not be persisted; worker scheduling is halted: {checkpoint_error}"
                        );
                        task_state.finish_cancellation_failure(
                            &task_id_for_work,
                            &message,
                            serde_json::json!({"error": message}),
                        );
                        let _ = event_bus.publish(
                            "auto-tag:event",
                            &auto_tag_event(&task_id_for_work, "failed", message, progress, None),
                        );
                    }
                }
                tracing::debug!(%error, "web auto-tag task cancelled");
            }
            Err(error) => {
                let message = error.to_string();
                if let Err(checkpoint_error) =
                    worker.mark_finished(&album_path, "failed", Some(&message), None)
                {
                    tracing::error!(%checkpoint_error, "manual auto-tag failure could not be persisted");
                }
                let data = serde_json::json!({"error": message});
                if !task_state.finish(
                    &task_id_for_work,
                    TaskStatus::Failed,
                    &message,
                    data.clone(),
                ) {
                    operations.finish(token);
                    return;
                }
                let _ = event_bus.publish(
                    "auto-tag:event",
                    &auto_tag_event(&task_id_for_work, "failed", message, 0, Some(data)),
                );
            }
        }
        operations.finish(token);
    });
    Json(task_id).into_response()
}

async fn command(
    AxumPath(command): AxumPath<String>,
    State(state): State<ServerState>,
    payload: Result<Json<serde_json::Value>, JsonRejection>,
) -> Response {
    if !supported_web_command(&command) {
        return error_response(StatusCode::NOT_FOUND, "unsupported command");
    }
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
    };

    match command.as_str() {
        "app:info" => {
            if decode_command_payload::<EmptyCommandRequest>(payload).is_err() {
                return error_response(StatusCode::BAD_REQUEST, "invalid command request");
            }
            Json(WebAppInfo {
                identifier: "com.ihelio.soundrobe",
                version: env!("CARGO_PKG_VERSION"),
                runtime: "web",
                dev: cfg!(debug_assertions),
            })
            .into_response()
        }
        "library:list-roots" => {
            if decode_command_payload::<EmptyCommandRequest>(payload).is_err() {
                return error_response(StatusCode::BAD_REQUEST, "invalid command request");
            }
            libraries(State(state)).await
        }
        "library:scan" => {
            let request = match decode_command_payload::<ScanCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.dir_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.exists() {
                return error_response(StatusCode::BAD_REQUEST, "library path not found");
            }
            let cancellation = state.lifecycle.cancellation.clone();
            let scan = tokio::task::spawn_blocking(move || {
                scan_directory_with_cancellation(&resolved.path, &|| cancellation.is_cancelled())
            });
            tokio::select! {
                _ = state.lifecycle.cancellation.cancelled() => {
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "server shutting down")
                }
                result = scan => match result {
                    Ok(Some(albums)) => Json(albums).into_response(),
                    Ok(None) => error_response(StatusCode::SERVICE_UNAVAILABLE, "server shutting down"),
                    Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "library scan failed"),
                },
            }
        }
        "directory:list" => {
            let request = match decode_command_payload::<DirectoryListCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.dir_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            Json(if resolved.path.is_dir() {
                list_directory_entries(&resolved.path)
            } else {
                Vec::new()
            })
            .into_response()
        }
        "directory:read" => {
            let request = match decode_command_payload::<DirectoryListCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.dir_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            // Reading a directory parses tags for every audio child, so it runs
            // on the blocking pool like the other tag-reading commands.
            let directory =
                match tokio::task::spawn_blocking(move || read_directory_for_web(&resolved.path))
                    .await
                {
                    Ok(directory) => directory,
                    Err(_) => {
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "directory read failed",
                        )
                    }
                };
            Json(directory).into_response()
        }
        "file:exists" => {
            let request = match decode_command_payload::<FileExistsCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.file_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            Json(resolved.path.is_file()).into_response()
        }
        "config:get" => {
            if decode_command_payload::<EmptyCommandRequest>(payload).is_err() {
                return error_response(StatusCode::BAD_REQUEST, "invalid command request");
            }
            Json(state.config.redacted()).into_response()
        }
        "tags:suggest" => {
            let request = match decode_command_payload::<TagSuggestionsCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            Json(suggest_tags(&request.query)).into_response()
        }
        "config:set" => {
            let request = match decode_command_payload::<ConfigSetCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            match state.config.try_set(&request.key, &request.value) {
                Ok(()) => Json(serde_json::Value::Null).into_response(),
                Err(ConfigSetError::UnsupportedKey(key)) => error_response(
                    StatusCode::BAD_REQUEST,
                    format!("unsupported config key: {key}"),
                ),
                Err(ConfigSetError::InvalidValue(message)) => {
                    error_response(StatusCode::BAD_REQUEST, message)
                }
                Err(ConfigSetError::Persistence) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "config persistence failed",
                ),
            }
        }
        "test-llm-connection" => {
            let request = match decode_command_payload::<TestLlmConnectionCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            match test_llm_connection_at(
                &request.api_key,
                &request.model,
                request.provider.as_deref(),
                request.base_url.as_deref(),
                &state.config,
            )
            .await
            {
                Ok(result) => Json(result).into_response(),
                Err(crate::error::ApiError::Message(message))
                    if message.starts_with("No API key provided")
                        || message.starts_with("No model provided") =>
                {
                    error_response(StatusCode::BAD_REQUEST, message)
                }
                Err(crate::error::ApiError::Message(_)) | Err(_) => {
                    error_response(StatusCode::BAD_GATEWAY, "LLM connection failed")
                }
            }
        }
        "debug:set-mode" => {
            let request = match decode_command_payload::<DebugSetModeCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            match state.config.try_set("debug", &request.enabled.into()) {
                Ok(()) => {
                    state.debug.set_enabled(request.enabled);
                    if state.debug.enabled() {
                        let _ = state.events.publish(
                            "debug:log",
                            &serde_json::json!({
                                "tag": "debug",
                                "level": "info",
                                "message": "Debug logging enabled"
                            }),
                        );
                    }
                    Json(serde_json::Value::Null).into_response()
                }
                Err(ConfigSetError::Persistence) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "config persistence failed",
                ),
                Err(ConfigSetError::UnsupportedKey(_)) | Err(ConfigSetError::InvalidValue(_)) => {
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "config persistence failed",
                    )
                }
            }
        }
        "album:search-releases" => {
            let WrappedCommandRequest { request } = match decode_command_payload::<
                WrappedCommandRequest<SearchReleasesRequest>,
            >(payload)
            {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let page = normalise_page(request.page);
            let page_size = normalise_page_size(request.page_size);
            match search_releases_inner(
                &request.provider,
                request.artist,
                request.album,
                request.year,
                request.country,
                request.format,
                request.catalog_number,
                request.barcode,
                page,
                page_size,
                state.providers.as_ref(),
                discogs_token(&state.config),
            )
            .await
            {
                Ok(result) => Json(result).into_response(),
                Err(error) => provider_error_response(error),
            }
        }
        "album:resolve-release" => {
            let WrappedCommandRequest { request } = match decode_command_payload::<
                WrappedCommandRequest<ResolveReleaseRequest>,
            >(payload)
            {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            match resolve_release_inner(&request, state.providers.as_ref(), &state.config).await {
                Ok(result) => Json(result).into_response(),
                Err(error) => provider_error_response(error),
            }
        }
        "album:release-track-count" => {
            let WrappedCommandRequest { request } = match decode_command_payload::<
                WrappedCommandRequest<ResolveReleaseRequest>,
            >(payload)
            {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            match release_track_count_inner(&request, state.providers.as_ref(), &state.config).await
            {
                Ok(count) => Json(count).into_response(),
                Err(error) => provider_error_response(error),
            }
        }
        "album:preview-release-match" => {
            let mut request =
                match decode_command_payload::<WrappedCommandRequest<PreviewMatchRequest>>(payload)
                {
                    Ok(request) => request.request,
                    Err(_) => {
                        return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                    }
                };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            request.album_path = resolved.path.to_string_lossy().into_owned();
            match album_preview_release_match(request).await {
                Ok(result) => Json(result).into_response(),
                Err(error) => search_operation_error_response(error),
            }
        }
        "album:search-apply-candidate" => {
            let mut request = match decode_command_payload::<
                WrappedCommandRequest<ApplyCandidateRequest>,
            >(payload)
            {
                Ok(request) => request.request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            request.album_path = resolved.path.to_string_lossy().into_owned();
            match apply_search_candidate(&request, &state.config, &state.write_queue).await {
                Ok(written) => Json(written).into_response(),
                Err(error) => search_operation_error_response(error),
            }
        }
        "album:read" | "album:refresh" => {
            let request = match decode_command_payload::<AlbumReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let cancellation = state.lifecycle.cancellation.clone();
            let scan = tokio::task::spawn_blocking(move || {
                read_album_with_cancellation(&resolved.path, &|| cancellation.is_cancelled())
            });
            tokio::select! {
                _ = state.lifecycle.cancellation.cancelled() => {
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "server shutting down")
                }
                result = scan => match result {
                    Ok(Ok(Some(album))) => Json(album).into_response(),
                    Ok(Ok(None)) => error_response(StatusCode::SERVICE_UNAVAILABLE, "server shutting down"),
                    Ok(Err(_)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "album read failed"),
                    Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "album read failed"),
                },
            }
        }
        "track:write" => {
            let request = match decode_command_payload::<TrackWriteCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let path = match resolve_existing_track(roots, &request.path) {
                Ok(path) => path.path,
                Err(response) => return *response,
            };
            match write_track_with_readback(&state.write_queue, path, request.fields).await {
                Ok(track) => Json(track).into_response(),
                Err(error) => media_error(error),
            }
        }
        "tracks:batch-write" => {
            let request = match decode_command_payload::<TrackBatchWriteCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let mut updates = request.updates;
            for update in &mut updates {
                let resolved = match resolve_existing_track(roots, &update.path) {
                    Ok(path) => path,
                    Err(response) => return *response,
                };
                update.path = resolved.path.to_string_lossy().into_owned();
            }
            let sink =
                Some(Arc::new(state.events.clone()) as Arc<dyn crate::state::events::EventSink>);
            match batch_write_with_readback(&state.write_queue, updates, sink).await {
                Ok(result) => Json(result).into_response(),
                Err(error) => media_error(error),
            }
        }
        "track:extra-tags:read" => {
            let request = match decode_command_payload::<ExtraTagsReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let path = match resolve_existing_track(roots, &request.track_path) {
                Ok(path) => path.path,
                Err(response) => return *response,
            };
            match tokio::task::spawn_blocking(move || read_extra_tags(&path)).await {
                Ok(tags) => Json(tags).into_response(),
                Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "media read failed"),
            }
        }
        "track:extra-tags:write" => {
            let request = match decode_command_payload::<ExtraTagsWriteCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let path = match resolve_existing_track(roots, &request.track_path) {
                Ok(path) => path.path,
                Err(response) => return *response,
            };
            match write_extra_tags_with_readback(&state.write_queue, path, request.tags).await {
                Ok(track) => Json(track).into_response(),
                Err(error) => media_error(error),
            }
        }
        "tracks:batch-write-extra-tags" => {
            let request = match decode_command_payload::<ExtraTagsBatchWriteCommandRequest>(payload)
            {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let mut updates = request.updates;
            for update in &mut updates {
                let resolved = match resolve_existing_track(roots, &update.path) {
                    Ok(path) => path,
                    Err(response) => return *response,
                };
                update.path = resolved.path.to_string_lossy().into_owned();
            }
            match write_extra_tags_batch_with_readback(&state.write_queue, updates).await {
                Ok(tracks) => Json(tracks).into_response(),
                Err(error) => media_error(error),
            }
        }
        "track:rename" => {
            let request = match decode_command_payload::<TrackRenameCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let source = match resolve_existing_track(roots, &request.old_path) {
                Ok(path) => path,
                Err(response) => return *response,
            };
            let destination = match roots.resolve_path(Path::new(&request.new_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if source.root_id != destination.root_id {
                return error_response(StatusCode::FORBIDDEN, "path outside library roots");
            }
            match rename_track_queued(&state.write_queue, source.path, destination.path).await {
                Ok(track) => Json(track).into_response(),
                Err(error) => media_error(error),
            }
        }
        "track:delete-files" => {
            let request = match decode_command_payload::<TrackDeleteCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let mut paths = Vec::with_capacity(request.file_paths.len());
            for supplied in request.file_paths {
                let resolved = match roots.resolve_path(Path::new(&supplied)) {
                    Ok(path) => path,
                    Err(error) => return scan_path_error(error),
                };
                paths.push(resolved.path.to_string_lossy().into_owned());
            }
            Json(delete_files_queued(&state.write_queue, paths).await).into_response()
        }
        "files:sort-by-album" => {
            let request = match decode_command_payload::<OrganizerCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.source_dir)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "source directory not found");
            }
            let copy = request
                .options
                .and_then(|options| options.copy)
                .unwrap_or(true);
            let source_dir = resolved.path;
            let result = state
                .write_queue
                .run(async move {
                    tokio::task::spawn_blocking(move || sort_by_album(&source_dir, copy))
                        .await
                        .map_err(|error| crate::error::ApiError::WriteTask(error.to_string()))?
                })
                .await;
            match result {
                Ok(result) => Json(result).into_response(),
                Err(error) => media_error(error),
            }
        }
        "audit:run" => {
            let request = match decode_command_payload::<AuditRunCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.library_path)) {
                Ok(path) => path.path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "library path not found");
            }
            let paths = discover_album_dirs(&resolved)
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            execute_audit(&state, paths).await
        }
        "audit:run-specified" => {
            let request = match decode_command_payload::<AuditSpecifiedCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let paths = if let Some(track_paths) =
                request.track_paths.filter(|paths| !paths.is_empty())
            {
                let mut albums = Vec::new();
                for track_path in track_paths {
                    let track = match resolve_existing_track(roots, &track_path) {
                        Ok(track) => track.path,
                        Err(response) => return *response,
                    };
                    let Some(album) = track.parent() else {
                        return error_response(StatusCode::BAD_REQUEST, "track path not found");
                    };
                    let album = album.to_path_buf();
                    if !albums.contains(&album) {
                        albums.push(album);
                    }
                }
                albums
            } else if let Some(album_paths) = request.album_paths.filter(|paths| !paths.is_empty())
            {
                let mut albums = Vec::new();
                for album_path in album_paths {
                    let album = match roots.resolve_path(Path::new(&album_path)) {
                        Ok(album) => album.path,
                        Err(error) => return scan_path_error(error),
                    };
                    if !album.is_dir() {
                        return error_response(StatusCode::BAD_REQUEST, "album path not found");
                    }
                    if !albums.contains(&album) {
                        albums.push(album);
                    }
                }
                albums
            } else {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "no tracks or albums specified for audit",
                );
            };
            execute_audit(
                &state,
                paths
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            )
            .await
        }
        "audit:run-album" => {
            let request = match decode_command_payload::<AlbumReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let album = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(album) => album.path,
                Err(error) => return scan_path_error(error),
            };
            if !album.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let token = match start_audit(state.audit.as_ref()) {
                Ok(token) => token,
                Err(error) => return media_error(error),
            };
            let (client, remote) = audit_clients(&state.providers, &state.config);
            let findings = audit_album_with_services(
                &album,
                &token,
                client.as_deref(),
                remote.as_deref(),
                &state.config.alias_file_path(),
            )
            .await;
            state.audit.finish(&token);
            Json(findings).into_response()
        }
        "audit:apply-fixes" => {
            let mut request = match decode_command_payload::<AuditApplyFixesCommandRequest>(payload)
            {
                Ok(request) => request.album_results,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            for album_result in &mut request {
                let album = match roots.resolve_path(Path::new(&album_result.album_path)) {
                    Ok(album) => album.path,
                    Err(error) => return scan_path_error(error),
                };
                if !album.is_dir() {
                    return error_response(StatusCode::BAD_REQUEST, "album path not found");
                }
                album_result.album_path = album.to_string_lossy().into_owned();
            }
            match apply_audit_fixes_for_album_results(&state.write_queue, request).await {
                Ok(summary) => Json(summary).into_response(),
                Err(error) => media_error(error),
            }
        }
        "audit:cancel" => {
            if decode_command_payload::<EmptyCommandRequest>(payload).is_err() {
                return error_response(StatusCode::BAD_REQUEST, "invalid command request");
            }
            state.audit.cancel();
            Json(serde_json::Value::Null).into_response()
        }
        "album:auto-tag" => {
            let request = match decode_command_payload::<AlbumReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let album = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(album) => album.path,
                Err(error) => return scan_path_error(error),
            };
            if !album.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            start_auto_tag_task(&state, album)
        }
        "background:get-settings" => Json(state.background.settings()).into_response(),
        "background:set-settings" => {
            let patch = match decode_command_payload::<BackgroundSettingsPatch>(payload) {
                Ok(patch) => patch,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let current = state.background.settings();
            let updated = crate::state::background::BackgroundSettings {
                enabled: patch.enabled.unwrap_or(current.enabled),
                // An empty schedule would leave background tagging unable to
                // ever run, so it is rejected rather than stored.
                schedule: patch.schedule.unwrap_or(current.schedule),
                directories: patch.directories.unwrap_or(current.directories),
                watch_enabled: patch.watch_enabled.unwrap_or(current.watch_enabled),
                skip_unchanged: patch.skip_unchanged.unwrap_or(current.skip_unchanged),
            };
            match state.background.set_settings(updated) {
                Ok(()) => Json(state.background.settings()).into_response(),
                Err(error) => error_response(StatusCode::BAD_REQUEST, &error),
            }
        }
        "background:list-runs" => Json(state.background.runs()).into_response(),
        "background:run-now" => match start_background_run(&state, RunTrigger::Manual) {
            Ok(task_id) => Json(serde_json::json!({ "taskId": task_id })).into_response(),
            Err(error) => error_response(StatusCode::CONFLICT, &error),
        },
        "background:retry-failed" => {
            let request = match decode_command_payload::<BackgroundRetryRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let runs = state.background.runs();
            let run = match request.run_id.as_deref() {
                Some(run_id) => runs.iter().find(|run| run.id == run_id),
                // A run that is still going is not a retry subject: it has no
                // settled outcome to retry, and while it holds the tag lock a
                // retry would be refused anyway. Retrying is about a run that
                // finished and left releases behind, so answering "nothing left
                // to retry" while a run is plainly in progress is wrong.
                None => runs.iter().find(|run| {
                    run.status != RunStatus::Running && run.message != INTERRUPTED_RUN_MESSAGE
                }),
            };
            let Some(run) = run else {
                return error_response(StatusCode::NOT_FOUND, "no background run to retry");
            };
            let albums = retryable_albums(run);
            if albums.is_empty() {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "that run has no release left to retry",
                );
            }
            let albums = match state.worker.requeue(&albums, "retry") {
                Ok(albums) => albums,
                Err(error) => {
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, &error);
                }
            };
            match start_background_batch(&state, RunTrigger::Manual, albums) {
                Ok(task_id) => Json(serde_json::json!({ "taskId": task_id })).into_response(),
                Err(error) => error_response(StatusCode::CONFLICT, &error),
            }
        }
        "monitoring:status" => Json(background_status(&state).await).into_response(),
        "monitoring:releases" => match monitoring_releases(&state).await {
            Ok(releases) => Json(releases).into_response(),
            Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &error),
        },
        "monitoring:mark-reviewed" => {
            let request = match decode_command_payload::<MonitoringReviewRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let album = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(album) => album.path,
                Err(error) => return scan_path_error(error),
            };
            if !album.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            if let Err(error) = state.worker.mark_reviewed(&album) {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, &error);
            }
            let status = match release_is_healthy(&album) {
                Ok(true) => ReleaseHealth::Healthy,
                Ok(false) => ReleaseHealth::NeedsTagging,
                Err(_) => ReleaseHealth::Unreadable,
            };
            state
                .health
                .write()
                .await
                .insert(album.to_string_lossy().into_owned(), status);
            Json(serde_json::json!({
                "path": album,
                "status": status,
                "reviewed": true,
            }))
            .into_response()
        }
        "monitoring:logs" => {
            let request = match decode_command_payload::<MonitoringLogsRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let data_dir = state.config.data_dir();
            let path = match request.source.as_deref().unwrap_or("app") {
                "app" => data_dir.join("auto-tagger.log"),
                "auto-tag" => match newest_debug_log(data_dir) {
                    Some(path) => path,
                    None => return error_response(StatusCode::NOT_FOUND, "no auto-tag log yet"),
                },
                other => {
                    let message = format!("unknown log source: {other}");
                    return error_response(StatusCode::BAD_REQUEST, &message);
                }
            };
            match read_log_tail(&path, request.lines.unwrap_or(400)) {
                Ok(payload) => Json(payload).into_response(),
                Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &error),
            }
        }
        "task:progress" => {
            let request = match decode_command_payload::<TaskProgressCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            Json(state.tasks.get(&request.task_id)).into_response()
        }
        "task:cancel" => {
            let request = match decode_command_payload::<TaskProgressCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            state.tasks.cancel(&request.task_id);
            Json(serde_json::Value::Null).into_response()
        }
        "dataset:status" => {
            if decode_command_payload::<EmptyCommandRequest>(payload).is_err() {
                return error_response(StatusCode::BAD_REQUEST, "invalid command request");
            }
            let path = state
                .config
                .raw()
                .dataset_path
                .map(PathBuf::from)
                .unwrap_or_else(|| state.config.data_file("dataset-index.sqlite"));
            match tokio::task::spawn_blocking(move || dataset_status_at(&path)).await {
                Ok(status) => Json(status).into_response(),
                Err(_) => {
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "dataset status failed")
                }
            }
        }
        "assistant:list-sessions" => {
            let request =
                match decode_command_payload::<AssistantListSessionsCommandRequest>(payload) {
                    Ok(request) => request,
                    Err(_) => {
                        return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                    }
                };
            let conversations = state.conversations.clone();
            match tokio::task::spawn_blocking(move || {
                conversations.sessions(request.limit.unwrap_or(50))
            })
            .await
            {
                Ok(sessions) => Json(sessions).into_response(),
                Err(_) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "conversation query failed",
                ),
            }
        }
        "assistant:get-conversation" => {
            let request = match decode_command_payload::<AssistantSessionCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let conversations = state.conversations.clone();
            match tokio::task::spawn_blocking(move || {
                conversations.conversation(&request.session_uuid_or_number)
            })
            .await
            {
                Ok(entries) => Json(entries).into_response(),
                Err(_) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "conversation query failed",
                ),
            }
        }
        "assistant:get-session" => {
            let request = match decode_command_payload::<AssistantSessionCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let conversations = state.conversations.clone();
            match tokio::task::spawn_blocking(move || {
                conversations.session(&request.session_uuid_or_number)
            })
            .await
            {
                Ok(session) => Json(session).into_response(),
                Err(_) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "conversation query failed",
                ),
            }
        }
        "assistant:current-session" => {
            if decode_command_payload::<EmptyCommandRequest>(payload).is_err() {
                return error_response(StatusCode::BAD_REQUEST, "invalid command request");
            }
            let conversations = state.conversations.clone();
            match tokio::task::spawn_blocking(move || conversations.current()).await {
                Ok(session) => Json(session).into_response(),
                Err(_) => error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "conversation query failed",
                ),
            }
        }
        "lyrics:fetch" => {
            let request = match decode_command_payload::<LyricsFetchCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let base_url = resolve_lyrics_base_url(&state.config.raw());
            Json(
                fetch_lyrics_at(
                    &base_url,
                    &request.track_name,
                    &request.artist_name,
                    request.album_name.as_deref(),
                    request.duration,
                )
                .await,
            )
            .into_response()
        }
        "album:download-lyrics" => {
            let request = match decode_command_payload::<AlbumReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let base_url = resolve_lyrics_base_url(&state.config.raw());
            let report =
                download_album_lyrics_at(&resolved.path, &base_url, &state.write_queue).await;
            Json(report).into_response()
        }
        "cover:data-url" => {
            let request = match decode_command_payload::<CoverDataCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let preferred_track_path = request
                .preferred_track_path
                .as_deref()
                .and_then(|path| roots.resolve_path(Path::new(path)).ok())
                .filter(|path| path.root_id == resolved.root_id && path.path.is_file())
                .map(|path| path.path);
            let cancellation = state.lifecycle.cancellation.clone();
            let cover = tokio::task::spawn_blocking(move || {
                if cancellation.is_cancelled() {
                    return None;
                }
                cover_data_url(&resolved.path, preferred_track_path.as_deref())
                    .ok()
                    .flatten()
            });
            tokio::select! {
                _ = state.lifecycle.cancellation.cancelled() => {
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "server shutting down")
                }
                result = cover => match result {
                    Ok(data_url) => Json(data_url).into_response(),
                    Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "cover read failed"),
                },
            }
        }
        "cover:remove" => {
            let request = match decode_command_payload::<AlbumReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let result = state
                .write_queue
                .run(async move {
                    tokio::task::spawn_blocking(move || remove_cover(&resolved.path))
                        .await
                        .map_err(|error| io::Error::other(error.to_string()))
                        .and_then(|result| result)
                })
                .await;
            match result {
                Ok(removed) => Json(removed).into_response(),
                Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "cover removal failed"),
            }
        }
        "cover:download" => {
            let request = match decode_command_payload::<AlbumReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let remote = remote_client(state.providers.as_ref(), &state.config);
            let result =
                download_album_artwork_at(&resolved.path, &remote, &state.write_queue).await;
            let data_url = result.map(|(bytes, _, _)| {
                format!(
                    "data:image/jpeg;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )
            });
            Json(data_url).into_response()
        }
        "cover:download-artist-art" => {
            let request = match decode_command_payload::<AlbumReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return error_response(StatusCode::BAD_REQUEST, "invalid command request")
                }
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "library roots unavailable",
                    )
                }
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let remote = remote_client(state.providers.as_ref(), &state.config);
            let result = download_artist_artwork_at(&resolved.path, &remote, &state.write_queue)
                .await
                .map(|(_, source, path)| ArtistArtResult {
                    path: path.to_string_lossy().into_owned(),
                    source: source.to_string(),
                });
            Json(result).into_response()
        }
        _ => unreachable!("supported_web_command and command dispatch diverged"),
    }
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}

fn provider_error_response(error: String) -> Response {
    let status = if error == "Artist or album is required" || error.starts_with("Unknown provider:")
    {
        StatusCode::BAD_REQUEST
    } else if error.to_ascii_lowercase().contains("not found") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    error_response(status, error)
}

fn search_operation_error_response(error: String) -> Response {
    let status = if error.starts_with("Unknown provider:")
        || error.starts_with("Album directory does not exist:")
        || error.starts_with("Track count mismatch:")
        || error.starts_with("Selected track index")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    error_response(status, error)
}

fn normalize_error_response(mut response: Response) -> Response {
    if response.status() == StatusCode::NOT_FOUND
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value.as_bytes().starts_with(b"application/json"))
    {
        return response;
    }
    let status = response.status();
    let Some(message) = (match status {
        StatusCode::NOT_FOUND => Some("not found"),
        StatusCode::METHOD_NOT_ALLOWED => Some("method not allowed"),
        StatusCode::INTERNAL_SERVER_ERROR => Some("internal server error"),
        _ => None,
    }) else {
        return response;
    };
    let allow = response.headers().get(header::ALLOW).cloned();
    response = error_response(status, message);
    if let Some(allow) = allow {
        response.headers_mut().insert(header::ALLOW, allow);
    }
    response
}

fn normalized_origin(value: &str) -> Option<String> {
    parse_public_origin(value).ok().map(|(origin, _)| origin)
}

fn is_api_mutation(method: &Method, path: &str) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) && path.starts_with("/api/v1/")
}

fn operation_kind(method: &Method, path: &str) -> Option<OperationKind> {
    if method != Method::POST {
        return None;
    }
    if path == "/api/v1/covers" {
        return Some(OperationKind::Mutation);
    }
    let command = path.strip_prefix("/api/v1/commands/")?;
    match command {
        "assistant:send"
        | "assistant:clear"
        | "assistant:apply-actions"
        | "assistant:complete-task-actions"
        | "assistant:reject-actions"
        | "assistant:init-runtime"
        | "assistant:init-services" => Some(OperationKind::Assistant),
        "audit:run" | "audit:run-specified" | "audit:run-album" | "audit:apply-fixes" => {
            Some(OperationKind::Audit)
        }
        "album:auto-tag" => Some(OperationKind::AutoTag),
        "files:sort-by-album" => Some(OperationKind::Organizer),
        "lyrics:fetch" | "album:download-lyrics" => Some(OperationKind::Lyrics),
        "track:write"
        | "tracks:batch-write"
        | "track:extra-tags:write"
        | "tracks:batch-write-extra-tags"
        | "track:rename"
        | "track:delete-files"
        | "cover:set"
        | "cover:remove"
        | "cover:download"
        | "cover:download-artist-art"
        | "config:set"
        | "volume:probe-write"
        | "volume:probe-write-real"
        | "debug:set-mode"
        | "album:search-apply-candidate" => Some(OperationKind::Mutation),
        _ => None,
    }
}

fn request_operation_kind(method: &Method, path: &str) -> Option<OperationKind> {
    if path == "/api/v1/commands/album:auto-tag" {
        // This command returns a task ID before its spawned work completes.
        // Its future handler must retain an explicit OperationToken instead
        // of using the request-scoped guard below.
        return None;
    }
    operation_kind(method, path)
}

async fn enforce_web_security(
    State(state): State<ServerState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if state.lifecycle.is_shutting_down() && path.starts_with("/api/") {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "server shutting down");
    }
    if is_api_mutation(request.method(), path) {
        let origin_matches = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .and_then(normalized_origin)
            .is_some_and(|origin| state.web.origin_matches(&origin));
        if !origin_matches {
            return error_response(StatusCode::FORBIDDEN, "same-origin request required");
        }
    }
    let _operation_guard = match request_operation_kind(request.method(), path) {
        Some(kind) => match state.operations.try_acquire(kind) {
            Ok(guard) => Some(guard),
            Err(_) => {
                return error_response(StatusCode::CONFLICT, "another operation is busy");
            }
        },
        None => None,
    };
    next.run(request).await
}

async fn security_headers(
    State(state): State<ServerState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mut response = normalize_error_response(next.run(request).await);
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), geolocation=(), microphone=()"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if state.web.secure_transport {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

#[derive(Deserialize)]
struct CoverUploadQuery {
    #[serde(rename = "albumPath")]
    album_path: String,
}

async fn upload_cover(
    State(state): State<ServerState>,
    Query(query): Query<CoverUploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or_default();
    if !matches!(content_type, "image/jpeg" | "image/png" | "image/webp") {
        return error_response(StatusCode::BAD_REQUEST, "unsupported cover image type");
    }
    let body = match to_bytes(body, MAX_COVER_UPLOAD_BYTES + 1).await {
        Ok(body) if body.len() <= MAX_COVER_UPLOAD_BYTES => body,
        Ok(_) | Err(_) => {
            return error_response(StatusCode::PAYLOAD_TOO_LARGE, "cover image is too large")
        }
    };

    let roots = match &state.libraries {
        Ok(roots) => roots,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "library roots unavailable",
            )
        }
    };
    let resolved = match roots.resolve_path(Path::new(&query.album_path)) {
        Ok(path) => path,
        Err(error) => return scan_path_error(error),
    };
    if !resolved.path.is_dir() {
        return error_response(StatusCode::BAD_REQUEST, "album path not found");
    }

    let album_path = resolved.path;
    let bytes = body.to_vec();
    let result = state
        .write_queue
        .run(async move {
            tokio::task::spawn_blocking(move || write_cover_upload(&album_path, &bytes))
                .await
                .map_err(|error| io::Error::other(error.to_string()))
                .and_then(|result| result)
        })
        .await;
    match result {
        Ok(data_url) => Json(data_url).into_response(),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            let message = if error.to_string() == "cover image is too large" {
                "cover image is too large"
            } else {
                "invalid cover image"
            };
            error_response(StatusCode::BAD_REQUEST, message)
        }
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "cover upload failed"),
    }
}

pub fn router(config: ServerConfig) -> Router {
    router_with_runtime(config, ServerLifecycle::default(), WriteQueue::default())
}

fn router_with_runtime(
    config: ServerConfig,
    lifecycle: ServerLifecycle,
    write_queue: WriteQueue,
) -> Router {
    router_with_runtime_and_events(config, lifecycle, write_queue, EventBus::default())
}

fn router_with_runtime_and_events(
    config: ServerConfig,
    lifecycle: ServerLifecycle,
    write_queue: WriteQueue,
    event_bus: EventBus,
) -> Router {
    let config_state = ConfigState::init_in(config.data_dir.clone());
    let cache = Arc::new(CacheState::new_in(config.data_dir.clone()));
    let _ = cache.initialize(config_state.raw().cache_path.as_deref());
    let worker = Arc::new(WorkerStore::new_in(config.data_dir.clone()));
    let worker_error = if !worker.initialize() {
        Some("durable background worker store could not be initialized".to_string())
    } else {
        worker
            .recover_interrupted()
            .map(|_| None)
            .unwrap_or_else(|error| Some(format!("worker recovery failed: {error}")))
    };
    if let Some(error) = &worker_error {
        tracing::error!(%error);
    }
    let background = BackgroundState::init_in(&config.data_dir);
    if let Err(error) = background.recover_interrupted_runs() {
        tracing::error!(%error, "background run history recovery failed");
    }
    let tasks = Arc::new(TaskRegistry::default());
    let conversations = Arc::new(ConversationState::new_in(config.data_dir.clone()));
    let _ = conversations.initialize(config_state.raw().cache_path.as_deref());
    let web_root = safe_web_root(&config.web_root);
    let spa = ServeDir::new(&web_root).fallback(ServeFile::new(web_root.join("index.html")));
    let state = ServerState {
        web: config.web.clone(),
        debug: WebDebugState::new(config_state.raw().debug.unwrap_or(false)),
        config: config_state,
        providers: Arc::new(crate::state::providers::ProviderState::default()),
        libraries: discover_library_roots(&config.library_root_dir)
            .map_err(|error| error.to_string()),
        audit: Arc::new(AuditState::default()),
        events: event_bus,
        operations: OperationCoordinator::default(),
        lifecycle,
        write_queue,
        cache,
        tasks,
        health: Arc::new(RwLock::new(HashMap::new())),
        conversations,
        background,
        worker,
        worker_error,
    };
    start_background_library_tagging(state.clone());
    let cover_upload_route = Router::new().route("/api/v1/covers", post(upload_cover));

    Router::new()
        .route("/healthz", get(health))
        .route("/api/v1/libraries", get(libraries))
        .route("/api/v1/events", get(events))
        .route("/api/v1/tasks", get(list_tasks))
        .route("/api/v1/commands/{command}", post(command))
        .route("/api/{*path}", any(api_not_found))
        .merge(cover_upload_route)
        .fallback_service(spa)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_web_security,
        ))
        .layer(middleware::from_fn_with_state(state, security_headers))
}

pub async fn run() -> anyhow::Result<()> {
    let config = ServerConfig::from_env()?;
    AppDataPaths::server(config.data_dir.clone()).prepare()?;
    // So the Monitoring view can show this process's own log, and an operator
    // can read what the service did without attaching to stdout.
    crate::infra::logging::init_tracing(Some(&config.data_dir));

    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    let lifecycle = ServerLifecycle::default();
    let write_queue = WriteQueue::default();
    axum::serve(
        listener,
        router_with_runtime(config, lifecycle.clone(), write_queue.clone()),
    )
    .with_graceful_shutdown(shutdown_signal(lifecycle))
    .await?;
    wait_for_active_writes(write_queue).await;
    Ok(())
}

async fn wait_for_active_writes(queue: WriteQueue) {
    while queue.is_active() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn shutdown_signal(lifecycle: ServerLifecycle) {
    #[cfg(unix)]
    {
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl-C handler");
        };
        let terminate = async {
            let mut signal =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to install SIGTERM handler");
            signal.recv().await;
        };
        tokio::select! {
            _ = ctrl_c => {}
            _ = terminate => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl-C handler");
    lifecycle.begin_shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::auto_tag::LookupRequest;
    use crate::state::config::{EnvMap, ProcessEnv};
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use futures_util::StreamExt;
    use serde_json::json;
    use std::time::Duration;
    use tokio::sync::{Barrier, Notify};
    use tower::ServiceExt;

    fn test_config() -> ServerConfig {
        ServerConfig::for_tests("https://soundrobe.test")
    }

    /// A `ServerState` for tests: no library roots, an injected configuration
    /// environment, and an injected provider service.
    ///
    /// Three tests need the same fifteen fields. A state missing one of them
    /// fails in a way that reads like the code under test rather than like the
    /// fixture, so the construction lives in one place.
    fn test_server_state(
        config: &ServerConfig,
        env: Arc<dyn crate::state::config::Env>,
        providers: Arc<crate::state::providers::ProviderState>,
    ) -> ServerState {
        ServerState {
            web: config.web.clone(),
            config: ConfigState::init_in_with_env(config.data_dir.clone(), env),
            providers,
            libraries: Ok(LibraryRoots::default()),
            events: EventBus::default(),
            operations: OperationCoordinator::default(),
            lifecycle: ServerLifecycle::default(),
            debug: WebDebugState::default(),
            write_queue: WriteQueue::default(),
            audit: Arc::new(AuditState::default()),
            cache: Arc::new(CacheState::new_in(config.data_dir.clone())),
            tasks: Arc::new(TaskRegistry::default()),
            health: Arc::new(RwLock::new(HashMap::new())),
            background: BackgroundState::init_in(&config.data_dir),
            conversations: Arc::new(ConversationState::new_in(config.data_dir.clone())),
            worker: {
                let worker = Arc::new(WorkerStore::new_in(config.data_dir.clone()));
                assert!(worker.initialize());
                worker
            },
            worker_error: None,
        }
    }

    fn origin_request(request: Request<Body>) -> Request<Body> {
        origin_request_with(request, "https://soundrobe.test")
    }

    fn origin_request_with(request: Request<Body>, origin: &str) -> Request<Body> {
        let (mut parts, body) = request.into_parts();
        parts
            .headers
            .insert(header::ORIGIN, origin.parse().unwrap());
        Request::from_parts(parts, body)
    }

    async fn test_cookie(_router: Router, _password: &str) -> axum::response::Response {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::SET_COOKIE, "soundrobe_session=test")
            .body(Body::empty())
            .unwrap()
    }

    async fn assert_json_error(response: axum::response::Response, status: StatusCode, body: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let actual = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(actual.as_ref(), body.as_bytes());
    }

    async fn delete_response_body(response: axum::response::Response) -> serde_json::Value {
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn health_is_the_public_server_readiness_endpoint() {
        let response = router(test_config())
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert_eq!(
            response.headers()["permissions-policy"],
            "camera=(), geolocation=(), microphone=()"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'"
        );
        assert_eq!(
            response.headers()[header::STRICT_TRANSPORT_SECURITY],
            "max-age=31536000; includeSubDomains"
        );
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }

    #[tokio::test]
    async fn browser_routes_serve_the_built_spa_shell() {
        let web_root =
            std::env::temp_dir().join(format!("soundrobe-web-static-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&web_root).unwrap();
        std::fs::write(web_root.join("index.html"), "<main>Soundrobe</main>").unwrap();
        let mut config = test_config();
        config.web_root = web_root.clone();

        let response = router(config)
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .as_ref(),
            b"<main>Soundrobe</main>"
        );

        let browser_route = router({
            let mut config = test_config();
            config.web_root = web_root.clone();
            config
        })
        .oneshot(
            Request::builder()
                .uri("/library/example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(browser_route.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(browser_route.into_body(), usize::MAX)
                .await
                .unwrap()
                .as_ref(),
            b"<main>Soundrobe</main>"
        );

        let api_app = router({
            let mut config = test_config();
            config.web_root = web_root.clone();
            config
        });
        let cookie_response = test_cookie(api_app.clone(), "correct horse battery staple").await;
        let api_response = api_app
            .oneshot(origin_request(
                Request::builder()
                    .uri("/api/v1/not-known")
                    .header(
                        header::COOKIE,
                        cookie_response.headers()[header::SET_COOKIE].clone(),
                    )
                    .body(Body::empty())
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_json_error(
            api_response,
            StatusCode::NOT_FOUND,
            r#"{"error":"not found"}"#,
        )
        .await;
        std::fs::remove_dir_all(web_root).unwrap();
    }

    #[tokio::test]
    async fn unknown_routes_are_not_exposed() {
        let response = router(test_config())
            .oneshot(Request::builder().uri("/api").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_json_error(response, StatusCode::NOT_FOUND, r#"{"error":"not found"}"#).await;
    }

    #[tokio::test]
    async fn api_routes_are_available_without_login() {
        let response = router(test_config())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/tasks")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let legacy_login = router(test_config())
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .body(Body::empty())
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_json_error(
            legacy_login,
            StatusCode::NOT_FOUND,
            r#"{"error":"not found"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn web_command_transport_exposes_web_app_info() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/app%3Ainfo")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["identifier"], "com.ihelio.soundrobe");
        assert_eq!(value["runtime"], "web");
    }

    #[tokio::test]
    async fn web_command_transport_validates_llm_test_payload_without_network() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/test-llm-connection")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"apiKey":"x","model":""}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"No model provided and none configured. Set LLM_MODEL in Settings or env."}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn web_command_transport_reports_unavailable_dataset_status() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/dataset%3Astatus")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let status: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status["available"], false);
        assert_eq!(status["totalRecords"], 0);
    }

    #[tokio::test]
    async fn web_command_transport_runs_a_confined_audit() {
        let base =
            std::env::temp_dir().join(format!("soundrobe-web-audit-{}", Uuid::new_v4().simple()));
        let library = base.join("Artist");
        let album = library.join("Album");
        std::fs::create_dir_all(&album).unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.mp3");
        std::fs::copy(fixture, album.join("01.mp3")).unwrap();

        let mut config = test_config();
        config.library_root_dir = base;
        let root_dir = config.library_root_dir.clone();
        let app = router(config.clone());
        let outside_app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();
        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/audit%3Arun")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "libraryPath": library.canonicalize().unwrap()
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let summary: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(summary["albums"], 1);
        assert_eq!(
            summary["albumResults"][0]["albumPath"],
            album.canonicalize().unwrap().to_string_lossy().as_ref()
        );

        let outside_cookie = test_cookie(outside_app.clone(), "correct horse battery staple").await;
        let outside_response = outside_app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/audit%3Arun")
                    .header(
                        header::COOKIE,
                        outside_cookie.headers()[header::SET_COOKIE].clone(),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"libraryPath":"/tmp/not-mounted"}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_json_error(
            outside_response,
            StatusCode::FORBIDDEN,
            r#"{"error":"path outside library roots"}"#,
        )
        .await;

        std::fs::remove_dir_all(root_dir).unwrap();
    }

    #[tokio::test]
    async fn web_command_transport_starts_confined_auto_tag_tasks() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-auto-tag-{}",
            Uuid::new_v4().simple()
        ));
        let library = base.join("Artist");
        let album = library.join("Empty Album");
        std::fs::create_dir_all(&album).unwrap();

        let mut config = test_config();
        config.library_root_dir = base;
        let root_dir = config.library_root_dir.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();
        let progress_app = app.clone();
        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Aauto-tag")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "albumPath": album.canonicalize().unwrap()
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let task_id: String = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(task_id.starts_with("auto-tag-"));

        let progress_response = progress_app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/task%3Aprogress")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({"taskId": task_id})).unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(progress_response.status(), StatusCode::OK);
        let progress: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(progress_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(progress["taskId"].is_string());
        std::fs::remove_dir_all(root_dir).unwrap();
    }

    #[tokio::test]
    async fn web_command_transport_scans_a_confined_library_root() {
        let base =
            std::env::temp_dir().join(format!("soundrobe-web-scan-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(base.join("Artist/Album")).unwrap();
        std::fs::write(base.join("Artist/Album/01.flac"), b"fixture").unwrap();
        let canonical_base = base.canonicalize().unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/library%3Ascan")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"dirPath":"{}"}}"#,
                        canonical_base.join("Artist").display()
                    )))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let albums: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(albums[0]["name"], "Album");
        assert_eq!(albums[0]["trackCount"], 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn web_command_transport_reads_a_confined_album() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-album-read-{}",
            Uuid::new_v4().simple()
        ));
        let album = base.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join("01.flac"), b"fixture").unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Arefresh")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"albumPath":"{}"}}"#,
                        album.canonicalize().unwrap().display()
                    )))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let detail: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(detail["name"], "Album");
        assert_eq!(detail["tracks"].as_array().unwrap().len(), 1);
        assert_eq!(detail["tracks"][0]["title"], "01.flac");
        std::fs::remove_dir_all(base).unwrap();
    }

    /// The browser workspace browses folders and lists the files of the
    /// selected directory in one round trip, so `directory:read` must return
    /// the same `{ path, name, subdirs, tracks, audioCount }` payload the
    /// desktop command returns.
    #[tokio::test]
    async fn web_directory_read_lists_confined_tracks_and_subdirs() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-directory-read-{}",
            Uuid::new_v4().simple()
        ));
        let album = base.join("Artist Alpha/Reference Album One");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::create_dir_all(album.join("Extras")).unwrap();
        std::fs::write(album.join("01.flac"), b"fixture").unwrap();
        std::fs::write(album.join("01.lrc"), b"sidecar").unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/directory%3Aread")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"dirPath":"{}"}}"#,
                        album.canonicalize().unwrap().display()
                    )))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let directory: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(directory["name"], "Reference Album One");
        assert_eq!(directory["tracks"].as_array().unwrap().len(), 1);
        assert_eq!(directory["audioCount"], 1);
        assert_eq!(directory["subdirs"].as_array().unwrap().len(), 1);
        assert_eq!(directory["subdirs"][0]["name"], "Extras");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn web_directory_read_rejects_paths_outside_library_roots() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-directory-confined-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let outside = std::env::temp_dir().join(format!(
            "soundrobe-web-directory-outside-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&outside).unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/directory%3Aread")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"dirPath":"{}"}}"#,
                        outside.canonicalize().unwrap().display()
                    )))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::FORBIDDEN,
            r#"{"error":"path outside library roots"}"#,
        )
        .await;
        std::fs::remove_dir_all(base).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[tokio::test]
    async fn web_track_mutations_use_shared_writers_and_stay_confined() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-track-write-{}",
            Uuid::new_v4().simple()
        ));
        let album = base.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let track = album.join("01.mp3");
        let batch_track = album.join("02.mp3");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.mp3");
        std::fs::copy(&fixture, &track).unwrap();
        std::fs::copy(&fixture, &batch_track).unwrap();

        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();
        let track_path = track.canonicalize().unwrap().to_string_lossy().into_owned();

        let write_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/track%3Awrite")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "path": track_path.clone(),
                            "fields": { "title": "Web title" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(write_response.status(), StatusCode::OK);
        let written: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(write_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(written["title"], "Web title");

        let extra_write_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/track%3Aextra-tags%3Awrite")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "trackPath": track_path.clone(),
                            "tags": [{ "key": "CUSTOM_WEB", "value": "yes" }]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(extra_write_response.status(), StatusCode::OK);
        let extra_read_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/track%3Aextra-tags%3Aread")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "trackPath": track_path.clone() })).unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(extra_read_response.status(), StatusCode::OK);
        let extra_tags: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(extra_read_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(extra_tags
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag["key"] == "CUSTOM_WEB" && tag["value"] == "yes"));

        let batch_path = batch_track
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let batch_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/tracks%3Abatch-write")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "updates": [{ "path": batch_path, "fields": { "title": "Batch title" } }]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(batch_response.status(), StatusCode::OK);
        let batch: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(batch_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(batch["tracks"][0]["title"], "Batch title");
        assert_eq!(batch["failures"].as_array().unwrap().len(), 0);

        let renamed = album.canonicalize().unwrap().join("renamed.mp3");
        let rename_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/track%3Arename")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "oldPath": track_path,
                            "newPath": renamed.to_string_lossy()
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(
            rename_response.status(),
            StatusCode::OK,
            "rename response: {}",
            String::from_utf8_lossy(
                &axum::body::to_bytes(rename_response.into_body(), usize::MAX)
                    .await
                    .unwrap()
            )
        );
        assert!(renamed.is_file());
        assert!(!track.is_file());

        let delete_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/track%3Adelete-files")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "filePaths": [renamed.to_string_lossy()]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::OK);
        assert_eq!(
            delete_response_body(delete_response).await[0]["success"],
            true
        );
        assert!(!renamed.exists());

        let outside = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/track%3Awrite")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "path": "/tmp/not-mounted.mp3",
                            "fields": { "title": "escape" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(outside.status(), StatusCode::FORBIDDEN);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn web_lyrics_commands_preserve_typed_results_and_confinement() {
        let base =
            std::env::temp_dir().join(format!("soundrobe-web-lyrics-{}", Uuid::new_v4().simple()));
        let album = base.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let fetch_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/lyrics%3Afetch")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"trackName":"","artistName":"Artist"}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(fetch_response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(fetch_response.into_body(), usize::MAX)
                .await
                .unwrap()
                .as_ref(),
            b"null"
        );

        let album_path = album.canonicalize().unwrap().to_string_lossy().into_owned();
        let download_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Adownload-lyrics")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({ "albumPath": album_path })).unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(download_response.status(), StatusCode::OK);
        let report: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(download_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(report["total"], 0);

        let outside_response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Adownload-lyrics")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"albumPath":"/tmp/not-mounted-album"}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(outside_response.status(), StatusCode::FORBIDDEN);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn web_command_transport_reads_a_confined_external_cover() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-cover-read-{}",
            Uuid::new_v4().simple()
        ));
        let album = base.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let mut cover_bytes = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(
                &mut std::io::Cursor::new(&mut cover_bytes),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        std::fs::write(album.join("cover.jpg"), cover_bytes).unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/cover%3Adata-url")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"albumPath":"{}"}}"#,
                        album.canonicalize().unwrap().display()
                    )))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let data_url: String = serde_json::from_slice(&body).unwrap();
        assert!(data_url.starts_with("data:image/jpeg;base64,"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn web_cover_upload_normalizes_and_writes_inside_the_album() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-cover-upload-{}",
            Uuid::new_v4().simple()
        ));
        let album = base.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let mut image_bytes = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(
                &mut std::io::Cursor::new(&mut image_bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();
        let album_path = album.canonicalize().unwrap().to_string_lossy().into_owned();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("albumPath", &album_path)
            .finish();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/covers?{query}"))
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "image/png")
                    .body(Body::from(image_bytes))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let data_url: String = serde_json::from_slice(&body).unwrap();
        assert!(data_url.starts_with("data:image/jpeg;base64,"));
        assert!(album.join("cover.jpg").is_file());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn web_cover_remove_deletes_external_art_and_suppresses_reappearance() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-cover-remove-{}",
            Uuid::new_v4().simple()
        ));
        let album = base.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join("cover.jpg"), b"cover").unwrap();
        std::fs::write(album.join("front.png"), b"front").unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/cover%3Aremove")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"albumPath":"{}"}}"#,
                        album.canonicalize().unwrap().display()
                    )))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"true");
        assert!(!album.join("cover.jpg").exists());
        assert!(album.join("front.png").exists());
        assert!(album.join(".auto-tagger-cover-removed").exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn web_directory_listing_and_file_exists_stay_inside_roots() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-directory-reads-{}",
            Uuid::new_v4().simple()
        ));
        let artist = base.join("Artist");
        let album = artist.join("Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::create_dir_all(artist.join("Second")).unwrap();
        std::fs::create_dir_all(artist.join(".hidden")).unwrap();
        let track = album.join("01.flac");
        std::fs::write(&track, b"fixture").unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();
        let artist_path = artist.canonicalize().unwrap().display().to_string();

        let listing = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/directory%3Alist")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(r#"{{"dirPath":"{artist_path}"}}"#)))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(listing.status(), StatusCode::OK);
        let listing_body = axum::body::to_bytes(listing.into_body(), usize::MAX)
            .await
            .unwrap();
        let entries: serde_json::Value = serde_json::from_slice(&listing_body).unwrap();
        assert_eq!(entries.as_array().unwrap().len(), 2);
        assert_eq!(entries[0]["name"], "Album");
        assert_eq!(entries[1]["name"], "Second");

        let track_path = track.canonicalize().unwrap().display().to_string();
        let exists = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/file%3Aexists")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(r#"{{"filePath":"{track_path}"}}"#)))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(exists.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(exists.into_body(), usize::MAX)
                .await
                .unwrap()
                .as_ref(),
            b"true"
        );

        let missing_path = artist
            .canonicalize()
            .unwrap()
            .join("missing.flac")
            .display()
            .to_string();
        let missing = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/file%3Aexists")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(r#"{{"filePath":"{missing_path}"}}"#)))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(missing.into_body(), usize::MAX)
                .await
                .unwrap()
                .as_ref(),
            b"false"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn web_config_commands_return_redacted_settings_and_persist_safe_values() {
        let base =
            std::env::temp_dir().join(format!("soundrobe-web-config-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&base).unwrap();
        let mut config = test_config();
        config.data_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let get_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/config%3Aget")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_body = axum::body::to_bytes(get_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let settings: serde_json::Value = serde_json::from_slice(&get_body).unwrap();
        assert_eq!(settings["llmApiKey"], serde_json::Value::Null);
        assert_eq!(settings["llmApiKeyConfigured"], false);

        let set_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/config%3Aset")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"key":"debug","value":true}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(set_response.status(), StatusCode::OK);
        assert!(std::fs::read_to_string(base.join("config.yaml"))
            .unwrap()
            .contains("debug: true"));

        let debug_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/debug%3Aset-mode")
                    .header(
                        header::COOKIE,
                        cookie_response.headers()[header::SET_COOKIE].clone(),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"enabled":false}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(debug_response.status(), StatusCode::OK);
        assert!(std::fs::read_to_string(base.join("config.yaml"))
            .unwrap()
            .contains("debug: false"));

        let assistant_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/config%3Aset")
                    .header(
                        header::COOKIE,
                        cookie_response.headers()[header::SET_COOKIE].clone(),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"key":"assistantAutonomous","value":true}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(assistant_response.status(), StatusCode::OK);
        assert!(std::fs::read_to_string(base.join("config.yaml"))
            .unwrap()
            .contains("assistant_autonomous: true"));

        let clear_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/config%3Aset")
                    .header(
                        header::COOKIE,
                        cookie_response.headers()[header::SET_COOKIE].clone(),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"key":"llmProvider","value":null}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(clear_response.status(), StatusCode::OK);
        let refreshed = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/config%3Aget")
                    .header(
                        header::COOKIE,
                        cookie_response.headers()[header::SET_COOKIE].clone(),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            ))
            .await
            .unwrap();
        let refreshed_settings: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(refreshed.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(refreshed_settings["llmProvider"], serde_json::Value::Null);

        let unknown_response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/config%3Aset")
                    .header(
                        header::COOKIE,
                        cookie_response.headers()[header::SET_COOKIE].clone(),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"key":"notARealKey","value":true}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_json_error(
            unknown_response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"unsupported config key: notARealKey"}"#,
        )
        .await;
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn cover_upload_rejects_unsupported_invalid_and_oversized_payloads() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-cover-errors-{}",
            Uuid::new_v4().simple()
        ));
        let album = base.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let album_path = album.canonicalize().unwrap().to_string_lossy().into_owned();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("albumPath", &album_path)
            .finish();
        let request = |content_type: &str, bytes: Vec<u8>| {
            origin_request(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/covers?{query}"))
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::from(bytes))
                    .unwrap(),
            )
        };

        assert_json_error(
            app.clone()
                .oneshot(request("text/plain", b"cover".to_vec()))
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            r#"{"error":"unsupported cover image type"}"#,
        )
        .await;
        assert_json_error(
            app.clone()
                .oneshot(request("image/png", b"not-an-image".to_vec()))
                .await
                .unwrap(),
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid cover image"}"#,
        )
        .await;
        assert_json_error(
            app.oneshot(request("image/png", vec![0; MAX_COVER_UPLOAD_BYTES + 1]))
                .await
                .unwrap(),
            StatusCode::PAYLOAD_TOO_LARGE,
            r#"{"error":"cover image is too large"}"#,
        )
        .await;
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn scan_path_errors_preserve_stable_security_statuses() {
        assert_json_error(
            scan_path_error(PathSecurityError::RelativePath),
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid library path"}"#,
        )
        .await;
        assert_json_error(
            scan_path_error(PathSecurityError::OutsideRoots),
            StatusCode::FORBIDDEN,
            r#"{"error":"path outside library roots"}"#,
        )
        .await;
        assert_json_error(
            scan_path_error(PathSecurityError::Unresolvable("io".to_string())),
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"library path unavailable"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn web_events_replay_sequenced_channel_data() {
        let event_bus = EventBus::with_capacity(4);
        event_bus
            .publish("audit:event", &json!({ "message": "started" }))
            .unwrap();
        let app = router_with_runtime_and_events(
            test_config(),
            ServerLifecycle::default(),
            WriteQueue::default(),
            event_bus,
        );
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/events?channels=audit%3Aevent")
                    .header(header::COOKIE, cookie)
                    .header("last-event-id", "test:0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        let mut body = response.into_body().into_data_stream();
        let frame = body.next().await.unwrap().unwrap();
        let text = String::from_utf8_lossy(&frame);
        assert!(text.contains("id: test:1\n"));
        assert!(text.contains("event: audit:event\n"));
        assert!(text.contains(r#"data: {"message":"started"}"#));
    }

    #[tokio::test]
    async fn active_event_stream_stops_when_lifecycle_is_cancelled() {
        let lifecycle = ServerLifecycle::default();
        let event_bus = EventBus::default();
        let (_, receiver, _) = event_bus.replay_and_subscribe(0, Some("test"), &[]);
        let state = EventStreamState {
            replay: VecDeque::new(),
            receiver,
            channels: Vec::new(),
            last_sequence: 0,
            generation: event_bus.generation().to_string(),
            cancellation: lifecycle.cancellation.clone(),
            gap: false,
            terminated: false,
        };
        let task = tokio::spawn(next_event(state));
        tokio::task::yield_now().await;
        lifecycle.begin_shutdown();

        assert!(task.await.unwrap().is_none());
    }

    #[tokio::test]
    async fn command_transport_rejects_unknown_payload_fields() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/app%3Ainfo")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"unexpected":true}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid command request"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn unsupported_web_commands_fail_loudly() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/unknown")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::NOT_FOUND,
            r#"{"error":"unsupported command"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn provider_commands_are_recognized_before_provider_validation() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Asearch-releases")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"request":{"provider":"unknown","album":"Test"}}"#,
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"Unknown provider: unknown"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn provider_command_payloads_reject_unknown_nested_fields() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Aresolve-release")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"request":{"provider":"musicbrainz","releaseId":"id","unexpected":true}}"#,
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid command request"}"#,
        )
        .await;
    }

    #[test]
    fn provider_search_pages_are_bounded_before_multiplication() {
        assert_eq!(normalise_page(None), 1);
        assert_eq!(normalise_page(Some(0)), 1);
        assert_eq!(normalise_page(Some(u32::MAX)), 10_000);
    }

    #[test]
    fn provider_upstream_failures_are_not_reported_as_bad_requests() {
        assert_eq!(
            provider_error_response("MusicBrainz request failed: offline".to_string()).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            provider_error_response("MusicBrainz release not found: id".to_string()).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            provider_error_response("Unknown provider: other".to_string()).status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn the_web_service_can_read_a_release_track_count() {
        // The browser search loads per-release track counts on request and filters
        // on them. Without this route the tool can only fall back to "unavailable",
        // so the command being registered is the contract that matters here.
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Arelease-track-count")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "request": { "provider": "unknown", "releaseId": "id" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();

        // An unknown provider is a provider error, not a missing route.
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn provider_preview_and_apply_routes_keep_the_album_inside_a_mount() {
        let base = std::env::temp_dir().join(format!("soundrobe-web-provider-{}", Uuid::new_v4()));
        let album = base.join("album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.flac"),
            album.join("01.flac"),
        )
        .unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();
        let album_path = album.canonicalize().unwrap().to_string_lossy().into_owned();

        let preview_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Apreview-release-match")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "request": {
                                "albumPath": album_path,
                                "provider": "unknown",
                                "release": { "id": "id", "title": "Album", "artists": [], "tracks": [] }
                            }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_json_error(
            preview_response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"Unknown provider: unknown"}"#,
        )
        .await;

        let apply_response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/album%3Asearch-apply-candidate")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "request": {
                                "albumPath": album_path,
                                "candidate": { "artists": [], "albumArtists": [], "tracks": [], "source": "musicbrainz" },
                                "selectedTrackIndices": [0]
                            }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_json_error(
            apply_response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"Track count mismatch: album has 1 audio files but candidate has 0 tracks"}"#,
        )
        .await;
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn provider_cover_download_commands_are_confined_to_mounted_albums() {
        let base = std::env::temp_dir().join(format!("soundrobe-web-cover-{}", Uuid::new_v4()));
        std::fs::create_dir_all(base.join("album")).unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/cover%3Adownload")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"albumPath":"/tmp/not-mounted"}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_json_error(
            response,
            StatusCode::FORBIDDEN,
            r#"{"error":"path outside library roots"}"#,
        )
        .await;
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn organizer_sort_commands_are_confined_to_mounted_roots() {
        let base = std::env::temp_dir().join(format!("soundrobe-web-organizer-{}", Uuid::new_v4()));
        let library = base.join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../test/fixtures/tauri/media-corpus/minimal.mp3"),
            library.join("song.mp3"),
        )
        .unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let valid_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/files%3Asort-by-album")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "sourceDir": library.canonicalize().unwrap(),
                            "options": { "copy": true }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(valid_response.status(), StatusCode::OK);
        let valid_body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(valid_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(valid_body["totalFiles"], 1);
        assert!(library.join("Corpus Album/song.mp3").is_file());

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/files%3Asort-by-album")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"sourceDir":"/tmp/not-mounted","options":{"copy":true}}"#,
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_json_error(
            response,
            StatusCode::FORBIDDEN,
            r#"{"error":"path outside library roots"}"#,
        )
        .await;
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn shutdown_lifecycle_rejects_new_api_requests() {
        let lifecycle = ServerLifecycle::default();
        let app = router_with_runtime(test_config(), lifecycle.clone(), WriteQueue::default());
        lifecycle.begin_shutdown();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/libraries")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            r#"{"error":"server shutting down"}"#,
        )
        .await;
        assert!(lifecycle.cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn shutdown_waits_for_active_write_queue_operations() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());
        let worker_queue = queue.clone();
        let worker_entered = entered.clone();
        let worker_release = release.clone();
        let worker = tokio::spawn(async move {
            worker_queue
                .run(async move {
                    worker_entered.wait().await;
                    worker_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let drain = tokio::spawn(wait_for_active_writes(queue));
        tokio::task::yield_now().await;
        assert!(!drain.is_finished());
        release.notify_one();
        drain.await.unwrap();
        worker.await.unwrap();
    }

    #[test]
    fn existing_mutating_command_ids_are_coordinated_and_reads_are_not() {
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/tracks"),
            None
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/assistant:apply-actions"),
            Some(OperationKind::Assistant)
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/audit:apply-fixes"),
            Some(OperationKind::Audit)
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/album:auto-tag"),
            Some(OperationKind::AutoTag)
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/assistant:cancel"),
            None
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/audit:cancel"),
            None
        );
        assert_eq!(
            request_operation_kind(&Method::POST, "/api/v1/commands/album:auto-tag"),
            None
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/files:sort-by-album"),
            Some(OperationKind::Organizer)
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/lyrics:fetch"),
            Some(OperationKind::Lyrics)
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/tracks:batch-write"),
            Some(OperationKind::Mutation)
        );
        assert_eq!(
            operation_kind(&Method::POST, "/api/v1/commands/album:read"),
            None
        );
    }

    #[test]
    fn web_single_album_audits_register_a_cancellable_state_token() {
        let state = AuditState::default();
        let token = start_audit(&state).unwrap();
        assert!(!token.load(Ordering::Acquire));
        state.cancel();
        assert!(token.load(Ordering::Acquire));
        state.finish(&token);
    }

    #[test]
    fn web_command_allowlist_includes_browser_album_refresh() {
        assert!(supported_web_command("album:refresh"));
    }

    #[test]
    fn web_command_allowlist_includes_dataset_status() {
        assert!(supported_web_command("dataset:status"));
    }

    #[tokio::test]
    async fn web_tag_suggestions_use_the_supported_catalog() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/tags%3Asuggest")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"query":"desc"}"#))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let suggestions: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(suggestions[0]["id"], "description");
        assert_eq!(suggestions[0]["source"], "ID3 / Navidrome");
    }

    #[tokio::test]
    async fn web_command_transport_exposes_current_assistant_session() {
        let app = router(test_config());
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/assistant%3Acurrent-session")
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let session: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(session["sessionId"].is_string());
        assert!(session["sessionNumber"].is_string());
    }

    #[test]
    fn web_debug_state_changes_for_the_running_server() {
        let state = WebDebugState::new(false);
        assert!(!state.enabled());
        state.set_enabled(true);
        assert!(state.enabled());
        state.set_enabled(false);
        assert!(!state.enabled());
    }

    #[tokio::test]
    async fn an_active_command_operation_returns_conflict() {
        let config = test_config();
        let state = test_server_state(
            &config,
            Arc::new(ProcessEnv),
            Arc::new(crate::state::providers::ProviderState::default()),
        );
        let _active = state
            .operations
            .try_acquire(OperationKind::AutoTag)
            .unwrap();
        let app = Router::new()
            .route(
                "/api/v1/commands/assistant:apply-actions",
                post(|| async { StatusCode::NO_CONTENT }),
            )
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, enforce_web_security));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/assistant:apply-actions")
                    .header(header::ORIGIN, "https://soundrobe.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_json_error(
            response,
            StatusCode::CONFLICT,
            r#"{"error":"another operation is busy"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn web_library_listing_returns_only_mounted_roots() {
        let base =
            std::env::temp_dir().join(format!("soundrobe-http-libraries-{}", Uuid::new_v4()));
        std::fs::create_dir_all(base.join("alpha/nested")).unwrap();
        std::fs::create_dir_all(base.join("beta")).unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/libraries")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let roots: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(roots.as_array().unwrap().len(), 2);
        assert_eq!(roots[0]["id"], "alpha");
        assert_eq!(roots[1]["id"], "beta");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn library_listing_reports_discovery_errors_instead_of_empty_success() {
        let path = std::env::temp_dir().join(format!("soundrobe-library-file-{}", Uuid::new_v4()));
        std::fs::write(&path, b"not a directory").unwrap();
        let mut config = test_config();
        config.library_root_dir = path.clone();
        let app = router(config);
        let cookie_response = test_cookie(app.clone(), "correct horse battery staple").await;
        let cookie = cookie_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/libraries")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_json_error(
            response,
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"internal server error"}"#,
        )
        .await;
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn startup_requires_public_origin_but_not_a_password() {
        assert!(WebConfig::from_sources(Some("https://soundrobe.test"), None).is_ok());
        assert!(WebConfig::from_sources(None, None).is_err());
        assert_eq!(
            parse_public_origin("https://soundrobe.test/").unwrap(),
            ("https://soundrobe.test".to_string(), true)
        );
        assert_eq!(
            parse_public_origin("http://soundrobe.test").unwrap(),
            ("http://soundrobe.test".to_string(), false)
        );
        assert_eq!(
            parse_public_origin("HTTPS://SOUNDROBE.TEST:443/").unwrap(),
            ("https://soundrobe.test".to_string(), true)
        );
        assert!(parse_public_origin("https://soundrobe.test/app").is_err());
    }

    #[tokio::test]
    async fn explicitly_configured_additional_origins_are_allowed_for_mutations() {
        let mut config = test_config();
        config.web =
            WebConfig::from_sources(Some("http://127.0.0.1:8080"), Some("http://localhost:8080"))
                .unwrap();

        let response = router(config)
            .oneshot(origin_request_with(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/scan")
                    .body(Body::empty())
                    .unwrap(),
                "http://localhost:8080",
            ))
            .await
            .unwrap();

        assert_ne!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn normalized_same_origin_headers_are_accepted() {
        let config = ServerConfig::for_tests("HTTPS://SOUNDROBE.TEST:443/");
        let response = router(config)
            .oneshot(origin_request_with(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/scan")
                    .body(Body::empty())
                    .unwrap(),
                "HTTPS://SOUNDROBE.TEST:443",
            ))
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn mutation_requests_require_the_configured_same_origin() {
        let app = router(test_config());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/scan")
                    .header(header::ORIGIN, "https://evil.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::FORBIDDEN,
            r#"{"error":"same-origin request required"}"#,
        )
        .await;

        let missing_origin = router(test_config())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/scan")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_json_error(
            missing_origin,
            StatusCode::FORBIDDEN,
            r#"{"error":"same-origin request required"}"#,
        )
        .await;

        let accepted = router(test_config())
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/scan")
                    .body(Body::empty())
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_ne!(accepted.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn method_failures_use_the_stable_json_error_contract() {
        let response = router(test_config())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::METHOD_NOT_ALLOWED,
            r#"{"error":"method not allowed"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn internal_failures_use_the_stable_json_error_contract() {
        let config = test_config();
        let state = test_server_state(
            &config,
            Arc::new(ProcessEnv),
            Arc::new(crate::state::providers::ProviderState::default()),
        );
        let app = Router::new()
            .route("/boom", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }))
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, security_headers));
        let response = app
            .oneshot(Request::builder().uri("/boom").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"internal server error"}"#,
        )
        .await;
    }

    // -----------------------------------------------------------------------
    // Background tagging: settings, schedule, monitoring
    // -----------------------------------------------------------------------

    /// Post a command the way the browser does, returning status and JSON body.
    async fn post_command(
        app: Router,
        cookie: header::HeaderValue,
        command: &str,
        payload: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let encoded = command.replace(':', "%3A");
        let response = app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/commands/{encoded}"))
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            ))
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    /// A throwaway data directory, because `test_config` shares one path across
    /// every test and background settings are persisted.
    fn isolated_config(tag: &str) -> ServerConfig {
        let mut config = test_config();
        config.data_dir =
            std::env::temp_dir().join(format!("soundrobe-bg-{tag}-{}", Uuid::new_v4().simple()));
        config
    }

    async fn signed_in(app: Router) -> header::HeaderValue {
        test_cookie(app, "correct horse battery staple")
            .await
            .headers()[header::SET_COOKIE]
            .clone()
    }

    #[tokio::test]
    async fn background_schedule_is_persisted_and_validated() {
        let config = isolated_config("schedule");
        let app = router(config);
        let cookie = signed_in(app.clone()).await;

        // The default keeps the startup sweep that predates this setting.
        let (status, value) = post_command(
            app.clone(),
            cookie.clone(),
            "background:get-settings",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["enabled"], json!(true));
        assert_eq!(value["schedule"], json!("0 0 * * *"));

        // An unparseable crontab is refused with a reason...
        let (status, value) = post_command(
            app.clone(),
            cookie.clone(),
            "background:set-settings",
            json!({"schedule": "every midnight"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            value["error"].as_str().unwrap().contains("5 fields"),
            "unexpected error: {value}"
        );

        // ...and the stored schedule is untouched, so a typo cannot leave
        // background tagging unable to ever run.
        let (_, value) = post_command(
            app.clone(),
            cookie.clone(),
            "background:get-settings",
            json!({}),
        )
        .await;
        assert_eq!(value["schedule"], json!("0 0 * * *"));

        // A valid patch merges rather than replacing the whole record.
        let (status, value) = post_command(
            app.clone(),
            cookie.clone(),
            "background:set-settings",
            json!({"schedule": "*/15 * * * *", "watchEnabled": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["schedule"], json!("*/15 * * * *"));
        assert_eq!(value["watchEnabled"], json!(true));
        assert_eq!(
            value["enabled"],
            json!(true),
            "unpatched field is preserved"
        );
        assert_eq!(value["skipUnchanged"], json!(true));

        // It survives a restart of the state.
        let (_, value) = post_command(app, cookie, "background:get-settings", json!({})).await;
        assert_eq!(value["schedule"], json!("*/15 * * * *"));
    }

    #[test]
    fn background_run_persistence_failure_finishes_the_created_task() {
        let config = isolated_config("run-persist-failure");
        fs::create_dir_all(&config.data_dir).unwrap();
        fs::create_dir_all(
            config
                .data_dir
                .join(crate::state::background::SETTINGS_FILE),
        )
        .unwrap();
        let state = test_server_state(
            &config,
            Arc::new(ProcessEnv),
            Arc::new(crate::state::providers::ProviderState::default()),
        );

        assert!(start_background_batch(
            &state,
            RunTrigger::Manual,
            vec![config.data_dir.join("Album")],
        )
        .is_err());
        let task = state
            .tasks
            .latest_with_prefix("background-auto-tag")
            .unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(state.background.runs().is_empty());
        assert!(state.operations.try_start(OperationKind::AutoTag).is_ok());
        fs::remove_dir_all(config.data_dir).unwrap();
    }

    #[tokio::test]
    async fn a_retry_defaults_to_the_newest_run_that_finished() {
        // The newest run is usually the retry that is already going. "Retry the
        // newest run" would then mean retrying a run that has no settled outcome
        // yet, and answering "nothing left to retry" while a run is plainly in
        // progress is the wrong thing to tell an operator.
        let config = isolated_config("retry-default");
        fs::create_dir_all(&config.data_dir).unwrap();
        let album = config.data_dir.join("Album A");
        fs::create_dir_all(&album).unwrap();
        let runs = json!([
            {
                "id": "bg-run-2-0",
                "trigger": "manual",
                "startedAt": "2026-09-19T03:00:00Z",
                "finishedAt": null,
                "status": "running",
                "tagged": 0,
                "failed": 0,
                "message": "",
                "taskId": "background-auto-tag-9",
                "items": [],
                "itemsTruncated": false,
            },
            {
                "id": "bg-run-1-0",
                "trigger": "schedule",
                "startedAt": "2026-09-19T00:00:00Z",
                "finishedAt": "2026-09-19T00:01:00Z",
                "status": "failed",
                "tagged": 0,
                "failed": 1,
                "message": "Tagged 0, 1 failed",
                "taskId": null,
                "items": [
                    {
                        "path": album.to_string_lossy(),
                        "status": "failed",
                        "reason": "permission denied",
                    },
                ],
                "itemsTruncated": false,
            }
        ]);
        fs::write(
            config.data_dir.join("background-tagging.json"),
            serde_json::to_vec(&json!({
                "settings": {
                    "enabled": true,
                    "schedule": "0 0 * * *",
                    "directories": [config.data_dir.to_string_lossy()],
                    "watchEnabled": false,
                    "skipUnchanged": true,
                },
                "runs": runs,
            }))
            .unwrap(),
        )
        .unwrap();

        let app = router(config);
        let cookie = signed_in(app.clone()).await;
        let (status, value) = post_command(
            app.clone(),
            cookie.clone(),
            "background:retry-failed",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "expected a retry, got {value}");
        let task_id = value["taskId"].as_str().unwrap().to_string();
        let (_, progress) =
            post_command(app, cookie, "task:progress", json!({ "taskId": task_id })).await;
        // The finished run covered exactly one release, so the retry covers one.
        assert_eq!(progress["total"], json!(1));
    }

    #[tokio::test]
    async fn background_retry_failed_reruns_only_the_releases_that_need_action() {
        // A run that failed on two of three releases has to be actionable after
        // the fact: the ledger says which releases, and the retry covers those
        // and only those. Retrying a healthy release or a deleted path would
        // either re-tag working music or chase a path that no longer exists.
        let config = isolated_config("retry-failed");
        fs::create_dir_all(&config.data_dir).unwrap();
        let album = config.data_dir.join("Album A");
        fs::create_dir_all(&album).unwrap();
        let gone = config.data_dir.join("Album Gone");
        let album_path = album.to_string_lossy().into_owned();
        let runs = json!([
            {
                "id": "bg-run-2-0",
                "trigger": "manual",
                "startedAt": "2026-09-19T02:00:00Z",
                "finishedAt": "2026-09-19T02:00:20Z",
                "status": "completed",
                "tagged": 1,
                "failed": 0,
                "message": "Tagged 1 of 1",
                "taskId": null,
                "items": [
                    {
                        "path": album_path,
                        "status": "healthy",
                        "authority": "discogs",
                    },
                ],
                "itemsTruncated": false,
            },
            {
                "id": "bg-run-1-0",
                "trigger": "schedule",
                "startedAt": "2026-09-19T00:00:00Z",
                "finishedAt": "2026-09-19T00:01:00Z",
                "status": "failed",
                "tagged": 1,
                "failed": 2,
                "message": "Tagged 1, 2 failed",
                "taskId": null,
                "items": [
                    {
                        "path": album_path,
                        "status": "failed",
                        "reason": "permission denied",
                    },
                    {
                        "path": gone.to_string_lossy(),
                        "status": "failed",
                        "reason": "permission denied",
                    },
                ],
                "itemsTruncated": false,
            }
        ]);
        fs::write(
            config.data_dir.join("background-tagging.json"),
            serde_json::to_vec(&json!({
                "settings": {
                    "enabled": true,
                    "schedule": "0 0 * * *",
                    "directories": [config.data_dir.to_string_lossy()],
                    "watchEnabled": false,
                    "skipUnchanged": true,
                },
                "runs": runs,
            }))
            .unwrap(),
        )
        .unwrap();

        let app = router(config);
        let cookie = signed_in(app.clone()).await;

        // The ledger is part of what the API reports, not just what is on disk.
        let (status, runs) = post_command(
            app.clone(),
            cookie.clone(),
            "background:list-runs",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(runs[0]["items"][0]["status"], json!("healthy"));
        assert_eq!(runs[1]["items"][0]["reason"], json!("permission denied"));

        // The newest run has nothing left to retry, and saying so is better
        // than starting an empty run that looks like work.
        let (status, value) = post_command(
            app.clone(),
            cookie.clone(),
            "background:retry-failed",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            value["error"].as_str().unwrap().contains("no release"),
            "unexpected error: {value}"
        );

        // An unknown run is a 404 rather than a silent retry of something else.
        let (status, _) = post_command(
            app.clone(),
            cookie.clone(),
            "background:retry-failed",
            json!({ "runId": "bg-run-404-0" }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Retrying the failed run covers its one surviving release: the deleted
        // path is skipped and the healthy release is not repeated.
        let (status, value) = post_command(
            app.clone(),
            cookie.clone(),
            "background:retry-failed",
            json!({ "runId": "bg-run-1-0" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let task_id = value["taskId"].as_str().unwrap().to_string();
        let (_, progress) = post_command(
            app.clone(),
            cookie.clone(),
            "task:progress",
            json!({ "taskId": task_id }),
        )
        .await;
        assert_eq!(progress["total"], json!(1));

        let (_, runs) = post_command(app, cookie, "background:list-runs", json!({})).await;
        assert_eq!(runs[0]["trigger"], json!("manual"));
        assert_eq!(runs[0]["status"], json!("running"));
    }

    // -----------------------------------------------------------------------
    // Background tagging smoke over the reviewed synthetic album
    // -----------------------------------------------------------------------

    /// An HTTP provider stub that serves the reviewed Discogs answers for one
    /// synthetic album and empty MusicBrainz answers, so a background run can
    /// resolve end to end with no network and no credentials.
    ///
    /// It stops on a flag rather than a fixed request count, because how many
    /// provider round trips a run makes is the resolver's business, not
    /// something a test should pin.
    struct StubProvider {
        base: String,
        shutdown: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl StubProvider {
        fn start() -> Self {
            use std::io::{Read, Write};
            use std::net::TcpListener;
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let shutdown = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&shutdown);
            let handle = std::thread::spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let mut buffer = [0; 8192];
                            let size = stream.read(&mut buffer).unwrap_or(0);
                            let request = String::from_utf8_lossy(&buffer[..size]).into_owned();
                            let target = request.split_whitespace().nth(1).unwrap_or("/");
                            let body = stub_provider_body(target);
                            let _ = write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                base,
                shutdown,
                handle: Some(handle),
            }
        }
    }

    impl Drop for StubProvider {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::Release);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    /// The path of a request target. The offline client proxies every request,
    /// so an HTTP request arrives in absolute form (`GET http://host/discogs/...`)
    /// rather than as an origin-form path.
    fn request_path(target: &str) -> &str {
        match target.split_once("://") {
            Some((_, rest)) => match rest.find('/') {
                Some(index) => &rest[index..],
                None => "/",
            },
            None => target,
        }
    }

    /// The reviewed Discogs answers for `Relapse (Deluxe)`, plus empty
    /// MusicBrainz answers so the resolver falls through to Discogs.
    fn stub_provider_body(target: &str) -> String {
        let path = request_path(target);
        if path.starts_with("/musicbrainz/") {
            return json!({"count": 0, "releases": [], "artists": [], "recordings": []})
                .to_string();
        }
        if path.starts_with("/discogs/database/search") {
            let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");
            if query.contains("type=master") {
                return json!({
                    "pagination": {"items": 1, "pages": 1},
                    "results": [{"id": 100118, "type": "master", "title": "Eminem - Relapse"}]
                })
                .to_string();
            }
            if query.contains("type=artist") {
                return json!({"pagination": {"items": 0, "pages": 1}, "results": []}).to_string();
            }
            return json!({
                "pagination": {"items": 1, "pages": 1},
                "results": [{
                    "id": 36441795,
                    "type": "release",
                    "title": "Eminem - Relapse (Deluxe)",
                    "year": 2009,
                    "country": "US",
                    "format": ["CD"],
                    "catno": "",
                    "barcode": [],
                }]
            })
            .to_string();
        }
        if path.starts_with("/discogs/masters/") && path.contains("/versions") {
            return json!({
                "pagination": {"pages": 1},
                "versions": [
                    {"id": 36441795, "title": "Relapse (Deluxe)", "country": "US", "released": "2009"}
                ]
            })
            .to_string();
        }
        if path.starts_with("/discogs/releases/") {
            return include_str!("../../test/fixtures/tauri/relapse-deluxe/release-36441795.json")
                .to_string();
        }
        json!({"error": "not_found", "path": path}).to_string()
    }

    #[test]
    fn a_proxied_request_target_still_resolves_to_its_path() {
        // Requests from the offline client arrive proxied, so the stub has to
        // see through the absolute form or every provider call misses and the
        // smoke would report a provider outage instead of a match.
        assert_eq!(
            request_path("http://127.0.0.1:9/discogs/releases/36441795?x=1"),
            "/discogs/releases/36441795?x=1"
        );
        assert_eq!(request_path("/discogs/releases/1"), "/discogs/releases/1");
        assert_eq!(request_path("http://127.0.0.1:9"), "/");
        assert_eq!(
            request_path("unreachable.invalid:443"),
            "unreachable.invalid:443"
        );
    }

    /// Write one silent FLAC per reviewed fixture track, tagged exactly as the
    /// reviewed album is, and return the album folder. The media is generated so
    /// the smoke never touches real music; the metadata comes from
    /// `test/fixtures/tauri/relapse-deluxe/local.json`.
    async fn materialize_synthetic_album(root: &Path, request: &LookupRequest) -> PathBuf {
        use crate::commands::mutations::{write_track_queued, Patch, StringList, TrackPatch};
        let album = root.join("Eminem/2009 - Relapse (Deluxe)");
        fs::create_dir_all(&album).unwrap();
        let queue = WriteQueue::default();
        for track in &request.tracks {
            let stem = track
                .filename
                .clone()
                .expect("fixture tracks carry filenames");
            let duration = track.length.expect("fixture tracks carry lengths");
            let path = album.join(format!("{stem}.flac"));
            let generated = std::process::Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=r=44100:cl=mono",
                    "-t",
                    &format!("{duration:.6}"),
                ])
                .arg(&path)
                .status()
                .expect("ffmpeg available for synthetic media");
            assert!(generated.success(), "ffmpeg failed for {}", path.display());

            let mut patch = TrackPatch::default();
            if let Some(title) = track.title.clone() {
                patch.title = Patch::Value(title);
            }
            if let Some(artist) = track.artist.clone() {
                patch.artist = Patch::Value(artist);
            }
            if !track.artists.is_empty() {
                patch.artists = Patch::Value(StringList::Many(track.artists.clone()));
            }
            if let Some(album_hint) = request.album_hint.clone() {
                patch.album = Patch::Value(album_hint);
            }
            if let Some(artist_hint) = request.artist_hint.clone() {
                patch.album_artist = Patch::Value(artist_hint);
            }
            if let Some(year) = request.year_hint.clone() {
                patch.year = Patch::Value(year);
            }
            patch.track_number = Patch::Value(track.track_number.unwrap_or(1));
            if let Some(total) = track.track_total {
                patch.track_total = Patch::Value(total);
            }
            if let Some(disc) = track.disc_number {
                patch.disc_number = Patch::Value(disc);
            }
            if let Some(total) = track.disc_total {
                patch.disc_total = Patch::Value(total);
            }
            write_track_queued(&queue, path, patch).await.unwrap();
        }
        album
    }

    /// The state a background run needs, wired to the offline provider service
    /// and to a data directory nothing else uses.
    fn smoke_state(root: &Path, base: &str) -> ServerState {
        let mut config = test_config();
        config.data_dir = root.join("data");
        fs::create_dir_all(&config.data_dir).unwrap();
        let state = test_server_state(
            &config,
            Arc::new(EnvMap::new()),
            Arc::new(
                crate::commands::auto_tag::eval_tests::offline_eval_providers(base)
                    .expect("offline provider service"),
            ),
        );
        assert!(state.cache.initialize(None));
        state
    }

    /// Wait for a background run's task to reach a terminal state.
    ///
    /// The budget is generous because the whole suite runs in parallel and this
    /// task does real file and HTTP work: a two-minute ceiling still fails a
    /// genuine hang, and it fails loudly rather than reporting a slow machine as
    /// a broken run.
    async fn await_task(state: &ServerState, task_id: &str) {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(120) {
            match state.tasks.get(task_id) {
                Some(progress) if progress.status != TaskStatus::Running => return,
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        let last = state
            .tasks
            .get(task_id)
            .map(|progress| {
                format!(
                    "{:?} at {}/{}",
                    progress.status, progress.progress, progress.total
                )
            })
            .unwrap_or_else(|| "unregistered".to_string());
        panic!("background task {task_id} never finished; still {last}");
    }

    #[tokio::test]
    #[ignore = "end-to-end smoke; run with `just smoke-background`"]
    async fn background_smoke_over_the_synthetic_album_records_every_release() {
        // The end-to-end version of the run ledger: a real resolver over
        // generated media, a real write, and a real failure. The unit tests
        // prove the ledger stores what it is handed; this proves the run hands
        // it the right things - the source that supplied the tags for a release
        // that worked, and a reason for one that did not.
        let root =
            std::env::temp_dir().join(format!("soundrobe-bg-smoke-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let request: LookupRequest = serde_json::from_str(include_str!(
            "../../test/fixtures/tauri/relapse-deluxe/local.json"
        ))
        .unwrap();
        let stub = StubProvider::start();
        let state = smoke_state(&root, &stub.base);
        let album = materialize_synthetic_album(&root, &request).await;

        // A release the resolver cannot place at all. It has to come back as an
        // outcome with a reason: a run that reports only successes cannot be
        // acted on.
        let unmatched = root.join("Eminem/2019 - Not In Any Provider");
        fs::create_dir_all(&unmatched).unwrap();
        fs::write(unmatched.join("01 Untitled.flac"), b"this is not a flac").unwrap();

        // A release that vanished between the survey and the run. It is the one
        // outcome a retry must *not* pick up: there is nothing left to tag.
        let vanished = root.join("Eminem/1999 - Deleted Since The Survey");

        let task_id = start_background_run_for(
            &state,
            RunTrigger::Manual,
            vec![album.clone(), unmatched.clone(), vanished.clone()],
        )
        .expect("background run starts");
        await_task(&state, &task_id).await;

        let runs = state.background.runs();
        let run = runs.first().expect("the run was recorded");
        assert_eq!(
            run.status,
            RunStatus::Failed,
            "one release failed, so the run needs attention: {}",
            run.message
        );
        assert_eq!(run.items.len(), 3, "one row per release: {:?}", run.items);
        let item = |path: &Path| {
            run.items
                .iter()
                .find(|item| item.path == path.to_string_lossy())
                .unwrap_or_else(|| panic!("{} is missing from the ledger", path.display()))
        };

        let tagged = item(&album);
        assert_eq!(tagged.status, ReleaseHealth::Healthy, "{}", run.message);
        assert_eq!(tagged.authority.as_deref(), Some("discogs"));

        let reviewed = item(&unmatched);
        assert_eq!(reviewed.status, ReleaseHealth::NeedsReview);
        assert!(
            reviewed
                .reason
                .as_deref()
                .is_some_and(|reason| !reason.is_empty()),
            "a release needing review without a reason cannot be acted on: {reviewed:?}"
        );

        let missing = item(&vanished);
        assert_eq!(missing.status, ReleaseHealth::Failed);
        assert!(
            missing
                .reason
                .as_deref()
                .is_some_and(|reason| !reason.is_empty()),
            "a failed release without a reason cannot be acted on: {missing:?}"
        );

        // The ledger has to describe a write that actually happened, not just a
        // provider match.
        let written =
            read_track_metadata(&album.join("01 Dr. West.flac")).expect("synthetic track readable");
        assert_eq!(written.discogs_release_id.as_deref(), Some("36441795"));

        // A retry re-runs what still needs action and drops the stale path.
        assert_eq!(retryable_albums(run), vec![unmatched.clone()]);

        // A smoke whose evidence nobody sees is a test that happens to pass.
        println!(
            "background smoke: {} -> {:?} via {}\n                {} -> {:?} ({})\n                {} -> {:?} (stale path)\n                retry covers: {}",
            album.display(),
            tagged.status,
            tagged.authority.as_deref().unwrap_or("unknown"),
            unmatched.display(),
            reviewed.status,
            reviewed.reason.as_deref().unwrap_or("no reason"),
            vanished.display(),
            missing.status,
            unmatched.display(),
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn monitoring_status_reports_the_schedule_and_drops_missing_directories() {
        let config = isolated_config("status");
        let app = router(config);
        let cookie = signed_in(app.clone()).await;

        let missing = std::env::temp_dir().join("soundrobe-does-not-exist-12345");
        let (status, _) = post_command(
            app.clone(),
            cookie.clone(),
            "background:set-settings",
            json!({"directories": [missing.to_string_lossy()]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, value) =
            post_command(app.clone(), cookie.clone(), "monitoring:status", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["enabled"], json!(true));
        assert_eq!(value["schedule"], json!("0 0 * * *"));
        // A configured directory that no longer exists must not be offered as
        // coverage, or the run would fail on every tick.
        assert_eq!(value["effectiveDirectories"], json!([]));
        assert_eq!(value["recentRuns"], json!([]));
        // Enabled + daily schedule means a next run is always known.
        assert!(
            value["nextRunAt"].is_string(),
            "expected a next run, got {value}"
        );
    }

    #[tokio::test]
    async fn background_run_now_refuses_when_no_directory_can_be_covered() {
        let config = isolated_config("runnow");
        let app = router(config);
        let cookie = signed_in(app.clone()).await;

        let (status, value) =
            post_command(app.clone(), cookie.clone(), "background:run-now", json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            value["error"]
                .as_str()
                .unwrap()
                .contains("no directories are available"),
            "unexpected error: {value}"
        );

        // A refused run must not be recorded as history, or Monitoring would
        // show attempts that never did anything.
        let (status, runs) = post_command(app, cookie, "background:list-runs", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(runs, json!([]));
    }

    #[tokio::test]
    async fn background_run_now_is_refused_while_disabled() {
        let config = isolated_config("disabled");
        let app = router(config);
        let cookie = signed_in(app.clone()).await;

        let (_, value) = post_command(
            app.clone(),
            cookie.clone(),
            "background:set-settings",
            json!({"enabled": false}),
        )
        .await;
        assert_eq!(value["enabled"], json!(false));

        let (status, value) =
            post_command(app.clone(), cookie, "background:run-now", json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(value["error"].as_str().unwrap().contains("disabled"));
    }

    #[tokio::test]
    async fn monitoring_logs_serve_the_app_log_and_reject_unknown_sources() {
        let config = isolated_config("logs");
        let library = config.data_dir.join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(
            config.data_dir.join("auto-tagger.log"),
            "first line\nsecond line\nthird line\n",
        )
        .unwrap();
        let mut config = config;
        config.library_root_dir = library;
        let app = router(config);
        let cookie = signed_in(app.clone()).await;

        let (status, value) = post_command(
            app.clone(),
            cookie.clone(),
            "monitoring:logs",
            json!({"source": "app", "lines": 2}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["lines"], json!(["second line", "third line"]));
        assert_eq!(value["totalLines"], json!(3));

        // Verbose tracing has not run yet in a fresh install; that is a
        // distinct, explainable state rather than an empty success.
        let (status, _) = post_command(
            app.clone(),
            cookie.clone(),
            "monitoring:logs",
            json!({"source": "auto-tag"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, value) =
            post_command(app, cookie, "monitoring:logs", json!({"source": "celery"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(value["error"]
            .as_str()
            .unwrap()
            .contains("unknown log source"));
    }

    #[tokio::test]
    async fn monitoring_releases_keep_ai_flags_until_explicit_acknowledgement() {
        let config = isolated_config("review-flags");
        let data_dir = config.data_dir.clone();
        let album_path = config.library_root_dir.join("Library/Artist/Album");
        fs::create_dir_all(&album_path).unwrap();
        let album = fs::canonicalize(album_path).unwrap();
        let worker = WorkerStore::new_in(config.data_dir.clone());
        assert!(worker.initialize());
        worker
            .record_review_flag(&album, "ai_fallback", "run-1")
            .unwrap();

        let app = router(config);
        let cookie = signed_in(app.clone()).await;
        let (status, value) = post_command(
            app.clone(),
            cookie.clone(),
            "monitoring:releases",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["releases"][0]["status"], json!("needs_review"));

        let (status, response) = post_command(
            app.clone(),
            cookie.clone(),
            "monitoring:mark-reviewed",
            json!({"albumPath": album}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "review command response: {status} {response}"
        );
        let (status, value) = post_command(app, cookie, "monitoring:releases", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert!(value["releases"]
            .as_array()
            .unwrap()
            .iter()
            .all(|release| release["status"] != json!("needs_review")));
        fs::remove_dir_all(data_dir).unwrap();
    }

    #[tokio::test]
    async fn monitoring_releases_start_empty_and_are_ordered() {
        let config = isolated_config("releases");
        let app = router(config);
        let cookie = signed_in(app.clone()).await;

        let (status, value) = post_command(app, cookie, "monitoring:releases", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["releases"], json!([]));
    }

    #[tokio::test]
    async fn monitoring_releases_restore_provider_deferrals_from_worker_state() {
        let config = isolated_config("provider-review-state");
        let album = config.library_root_dir.join("Library/Artist/Album");
        fs::create_dir_all(&album).unwrap();
        let worker = WorkerStore::new_in(config.data_dir.clone());
        assert!(worker.initialize());
        worker
            .enqueue(std::slice::from_ref(&album), "manual")
            .unwrap();
        worker
            .mark_finished(&album, "needs_review", Some("provider_incomplete"), None)
            .unwrap();

        let app = router(config);
        let cookie = signed_in(app.clone()).await;
        let (status, value) = post_command(app, cookie, "monitoring:releases", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["releases"][0]["path"], json!(album));
        assert_eq!(value["releases"][0]["status"], json!("needs_review"));
    }

    // -----------------------------------------------------------------------
    // Watched folders
    // -----------------------------------------------------------------------

    fn stamp(seconds: u64) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds)
    }

    #[test]
    fn a_watch_baseline_records_what_exists_without_tagging_any_of_it() {
        let mut seen = HashMap::new();
        let existing = vec![
            (PathBuf::from("/music/A"), stamp(100)),
            (PathBuf::from("/music/B"), stamp(100)),
        ];

        // Without a baseline pass, switching watching on would schedule the
        // whole library for re-tagging.
        assert!(changed_albums(existing, &mut seen, true).is_empty());
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn an_album_added_after_the_baseline_is_new() {
        let mut seen = HashMap::new();
        let _ = changed_albums(
            vec![(PathBuf::from("/music/A"), stamp(100))],
            &mut seen,
            true,
        );

        // This is the regression that motivated the baseline flag: a folder
        // created while the service runs must not be treated as a first
        // sighting and silently skipped.
        let changed = changed_albums(
            vec![
                (PathBuf::from("/music/A"), stamp(100)),
                (PathBuf::from("/music/NEW"), stamp(200)),
            ],
            &mut seen,
            false,
        );
        assert_eq!(changed, vec![PathBuf::from("/music/NEW")]);
    }

    #[test]
    fn a_folder_that_changes_again_is_reported_once_per_change() {
        let mut seen = HashMap::new();
        let _ = changed_albums(
            vec![(PathBuf::from("/music/A"), stamp(100))],
            &mut seen,
            true,
        );

        // Unchanged: an idle poll must not re-tag anything.
        assert!(changed_albums(
            vec![(PathBuf::from("/music/A"), stamp(100))],
            &mut seen,
            false
        )
        .is_empty());
        // A newer folder mtime means music was added or replaced.
        assert_eq!(
            changed_albums(
                vec![(PathBuf::from("/music/A"), stamp(150))],
                &mut seen,
                false
            ),
            vec![PathBuf::from("/music/A")]
        );
        // ...and is not reported again while it stays put.
        assert!(changed_albums(
            vec![(PathBuf::from("/music/A"), stamp(150))],
            &mut seen,
            false
        )
        .is_empty());
    }

    #[test]
    fn a_watched_album_is_retried_instead_of_being_dropped_when_the_tag_lock_is_busy() {
        // Discovery is one shot: once an album is marked seen, only another
        // folder mtime change brings it back. If a scheduled or manual run holds
        // the tag lock at that moment, dropping the album loses it silently - the
        // reference product cannot lose it because the broker queue already owns
        // it. So a start that fails has to put the album back.
        let mut queue = WatchQueue::default();
        let album = PathBuf::from("/music/NEW");
        queue.enqueue(vec![album.clone()]);

        let attempted = queue.take();
        assert_eq!(attempted, vec![album.clone()]);
        // The run was refused ("another auto-tag operation is already running").
        queue.requeue(attempted[0].clone());

        assert_eq!(queue.take(), vec![album]);
    }

    #[test]
    fn a_watched_album_is_not_queued_or_retried_twice() {
        let mut queue = WatchQueue::default();
        let album = PathBuf::from("/music/A");

        // A folder whose mtime keeps moving is still one album.
        queue.enqueue(vec![album.clone()]);
        queue.enqueue(vec![album.clone()]);
        assert_eq!(queue.take(), vec![album.clone()]);

        // A requeue after a successful start would tag the album twice.
        queue.requeue(album.clone());
        queue.requeue(album.clone());
        assert_eq!(queue.take(), vec![album]);
    }

    #[test]
    fn the_watch_backlog_is_bounded() {
        // A library that grows faster than it can be tagged is not a reason to
        // hold unbounded work in memory; the scheduled sweep still covers
        // everything the backlog refuses.
        let mut queue = WatchQueue::default();
        queue.enqueue(
            (0..WatchQueue::LIMIT + 10).map(|index| PathBuf::from(format!("/music/{index}"))),
        );
        assert_eq!(queue.take().len(), WatchQueue::LIMIT);
        assert!(queue.take().is_empty());
    }

    #[test]
    fn a_retry_covers_only_the_releases_that_still_need_action() {
        let root =
            std::env::temp_dir().join(format!("soundrobe-retry-{}", Uuid::new_v4().simple()));
        let present = root.join("Album A");
        fs::create_dir_all(&present).unwrap();
        let gone = root.join("Album B");
        let review = root.join("Album C");
        fs::create_dir_all(&review).unwrap();
        let run = BackgroundRun {
            id: "bg-run-1-0".to_string(),
            trigger: RunTrigger::Manual,
            started_at: String::new(),
            finished_at: None,
            status: RunStatus::Failed,
            tagged: 1,
            failed: 1,
            message: String::new(),
            task_id: None,
            items: vec![
                ReleaseOutcome {
                    path: present.to_string_lossy().into_owned(),
                    status: ReleaseHealth::Failed,
                    reason: Some("permission denied".to_string()),
                    authority: None,
                },
                ReleaseOutcome {
                    path: gone.to_string_lossy().into_owned(),
                    status: ReleaseHealth::Failed,
                    reason: Some("permission denied".to_string()),
                    authority: None,
                },
                ReleaseOutcome {
                    path: review.to_string_lossy().into_owned(),
                    status: ReleaseHealth::NeedsReview,
                    reason: Some("no_authoritative_match".to_string()),
                    authority: None,
                },
                ReleaseOutcome {
                    path: root.join("Album D").to_string_lossy().into_owned(),
                    status: ReleaseHealth::Healthy,
                    reason: None,
                    authority: Some("discogs".to_string()),
                },
            ],
            items_truncated: false,
        };

        let albums = retryable_albums(&run);
        // The healthy release is excluded, or a retry would re-tag what already
        // worked; the deleted one is excluded because it is not a retry, it is a
        // stale path.
        assert_eq!(albums, vec![present, review]);
        fs::remove_dir_all(&root).unwrap();
    }
}
