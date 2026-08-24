//! Headless HTTP runtime.

use axum::{
    body::{to_bytes, Body},
    extract::{
        rejection::JsonRejection, DefaultBodyLimit, Json, Path as AxumPath, Query, State,
    },
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse, Response},
    routing::{any, get, post},
    Router,
};
use base64::Engine;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    fs,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tower_http::services::{ServeDir, ServeFile};
use url::Url;
use uuid::Uuid;

use crate::commands::album_search::{
    album_preview_release_match, apply_search_candidate, discogs_token, normalise_page,
    normalise_page_size, resolve_release_inner, search_releases_inner, ApplyCandidateRequest,
    PreviewMatchRequest, ResolveReleaseRequest, SearchReleasesRequest,
};
use crate::commands::covers::{
    download_album_artwork_at, download_artist_artwork_at, remote_client, ArtistArtResult,
};
use crate::commands::configuration::test_llm_connection_at;
use crate::commands::audit::{
    apply_audit_fixes_for_album_results, audit_album_with_services, audit_clients,
    audit_specific_albums, discover_album_dirs, finish_audit_run, start_audit, AuditAlbumResult,
};
use crate::commands::auto_tag::{
    auto_tag_completion_message, auto_tag_event, resolve_and_apply_album, AutoTagServices,
};
use crate::commands::organizer::{sort_by_album, SortByAlbumOptions};
use crate::commands::mutations::{
    batch_write_with_readback, delete_files_queued, rename_track_queued,
    write_extra_tags_batch_with_readback, write_extra_tags_with_readback,
    write_track_with_readback, ExtraTagBatchUpdate, ExtraTagUpdate, TrackPatch, TrackUpdate,
};
use crate::commands::lyrics::{
    download_album_lyrics_at, fetch_lyrics_at, resolve_lyrics_base_url,
};
use crate::commands::tracks::read_extra_tags;
use crate::state::{
    album::{
        cover_data_url, read_album_with_cancellation, remove_cover, write_cover_upload,
        MAX_COVER_UPLOAD_BYTES,
    },
    config::{ConfigSetError, ConfigState},
    audit::AuditState,
    events::{EventBus, EventEnvelope},
    library::{
        discover_library_roots, list_directory_entries, scan_directory_with_cancellation,
        ConfinedPath, LibraryRoots, PathSecurityError,
    },
    operation::{OperationCoordinator, OperationKind},
    paths::AppDataPaths,
    sqlite::CacheState,
    tasks::{TaskRegistry, TaskStatus},
    write_queue::WriteQueue,
};

const SESSION_COOKIE: &str = "soundrobe_session";
const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_LOGIN_WINDOW: Duration = Duration::from_secs(60);
const DEFAULT_MAX_FAILED_LOGINS: u32 = 5;

#[derive(Clone)]
pub struct ServerConfig {
    listen_addr: SocketAddr,
    data_dir: PathBuf,
    library_root_dir: PathBuf,
    web_root: PathBuf,
    auth: AuthConfig,
    session_ttl: Duration,
    login_window: Duration,
    max_failed_logins: u32,
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
        let password_file = std::env::var_os("SOUNDROBE_AUTH_PASSWORD_FILE").map(PathBuf::from);
        let auth = AuthConfig::from_sources(
            password_file.as_deref(),
            std::env::var("SOUNDROBE_AUTH_PASSWORD").ok().as_deref(),
            std::env::var("SOUNDROBE_PUBLIC_URL").ok().as_deref(),
        )?;

        Ok(Self {
            listen_addr,
            data_dir,
            library_root_dir,
            web_root,
            auth,
            session_ttl: DEFAULT_SESSION_TTL,
            login_window: DEFAULT_LOGIN_WINDOW,
            max_failed_logins: DEFAULT_MAX_FAILED_LOGINS,
        })
    }

    #[cfg(test)]
    fn for_tests(password: &str, public_url: &str) -> Self {
        Self {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            data_dir: PathBuf::from("/tmp/soundrobe-test-config"),
            library_root_dir: PathBuf::from("/tmp/soundrobe-test-libraries"),
            web_root: PathBuf::from("/tmp/soundrobe-test-web"),
            auth: AuthConfig::from_sources(None, Some(password), Some(public_url)).unwrap(),
            session_ttl: DEFAULT_SESSION_TTL,
            login_window: DEFAULT_LOGIN_WINDOW,
            max_failed_logins: DEFAULT_MAX_FAILED_LOGINS,
        }
    }
}

#[derive(Clone)]
struct AuthConfig {
    password: Arc<str>,
    public_origin: Arc<str>,
    secure_cookie: bool,
}

impl AuthConfig {
    fn from_sources(
        password_file: Option<&Path>,
        fallback_password: Option<&str>,
        public_url: Option<&str>,
    ) -> anyhow::Result<Self> {
        let public_url =
            public_url.ok_or_else(|| anyhow::anyhow!("SOUNDROBE_PUBLIC_URL is required"))?;
        let (public_origin, secure_cookie) = parse_public_origin(public_url)?;
        let password = match password_file {
            Some(path) => std::fs::read_to_string(path)
                .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
                .map_err(|error| {
                    anyhow::anyhow!("unable to read SOUNDROBE_AUTH_PASSWORD_FILE: {error}")
                })?,
            None => fallback_password.unwrap_or_default().to_string(),
        };
        if password.is_empty() {
            return Err(anyhow::anyhow!(
                "SOUNDROBE_AUTH_PASSWORD_FILE or SOUNDROBE_AUTH_PASSWORD is required"
            ));
        }

        Ok(Self {
            password: Arc::from(password),
            public_origin: Arc::from(public_origin),
            secure_cookie,
        })
    }

    fn password_matches(&self, candidate: &str) -> bool {
        self.password.as_bytes().ct_eq(candidate.as_bytes()).into()
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
    auth: AuthService,
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

#[derive(Clone)]
struct AuthService {
    config: AuthConfig,
    sessions: Arc<RwLock<HashMap<String, Instant>>>,
    failed_logins: Arc<Mutex<FailedLogins>>,
    session_ttl: Duration,
    login_window: Duration,
    max_failed_logins: u32,
}

struct FailedLogins {
    window_started: Instant,
    count: u32,
}

enum LoginAttempt {
    Authenticated,
    Invalid,
    Throttled,
}

impl AuthService {
    fn new(config: &ServerConfig) -> Self {
        Self {
            config: config.auth.clone(),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            failed_logins: Arc::new(Mutex::new(FailedLogins {
                window_started: Instant::now(),
                count: 0,
            })),
            session_ttl: config.session_ttl,
            login_window: config.login_window,
            max_failed_logins: config.max_failed_logins,
        }
    }

    async fn verify_password(&self, candidate: &str) -> LoginAttempt {
        let mut failed_logins = self.failed_logins.lock().await;
        if failed_logins.window_started.elapsed() >= self.login_window {
            failed_logins.window_started = Instant::now();
            failed_logins.count = 0;
        }
        if failed_logins.count >= self.max_failed_logins {
            return LoginAttempt::Throttled;
        }
        if self.config.password_matches(candidate) {
            failed_logins.window_started = Instant::now();
            failed_logins.count = 0;
            LoginAttempt::Authenticated
        } else {
            failed_logins.count = failed_logins.count.saturating_add(1);
            LoginAttempt::Invalid
        }
    }

    async fn create_session(&self) -> String {
        let token = Uuid::new_v4().simple().to_string();
        self.sessions
            .write()
            .await
            .insert(token.clone(), Instant::now() + self.session_ttl);
        token
    }

    async fn has_valid_session(&self, token: Option<&str>) -> bool {
        let Some(token) = token else {
            return false;
        };
        let mut sessions = self.sessions.write().await;
        let Some(expires_at) = sessions.get(token).copied() else {
            return false;
        };
        if expires_at <= Instant::now() {
            sessions.remove(token);
            return false;
        }
        true
    }

    async fn remove_session(&self, token: Option<&str>) {
        if let Some(token) = token {
            self.sessions.write().await.remove(token);
        }
    }

    fn session_cookie(&self, token: &str) -> String {
        let secure = if self.config.secure_cookie {
            "; Secure"
        } else {
            ""
        };
        format!(
            "{SESSION_COOKIE}={token}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict{secure}",
            self.session_ttl.as_secs()
        )
    }

    fn clear_session_cookie(&self) -> String {
        let secure = if self.config.secure_cookie {
            "; Secure"
        } else {
            ""
        };
        format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict{secure}")
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

#[derive(Serialize)]
struct SessionResponse {
    authenticated: bool,
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
    let (replay, receiver, gap) = state
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
                if event.sequence <= state.last_sequence || !event_matches(&event, &state.channels) {
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

fn supported_web_command(command: &str) -> bool {
    matches!(
        command,
        "app:info"
            | "library:list-roots"
            | "library:scan"
            | "album:read"
            | "album:search-releases"
            | "album:resolve-release"
            | "album:preview-release-match"
            | "album:search-apply-candidate"
            | "files:sort-by-album"
            | "album:auto-tag"
            | "task:progress"
            | "task:cancel"
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
            | "file:exists"
            | "config:get"
            | "config:set"
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
        PathSecurityError::Unresolvable(_) => {
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "library path unavailable")
        }
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
    let task_id = state.tasks.create("auto-tag", 9, "Starting...");
    let task_id_for_work = task_id.clone();
    let task_state = state.tasks.clone();
    let event_bus = state.events.clone();
    let providers = state.providers.clone();
    let cache = state.cache.clone();
    let queue = state.write_queue.clone();
    let config = state.config.clone();
    let operations = state.operations.clone();
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
        let result = resolve_and_apply_album(
            &album_path,
            &config_values,
            AutoTagServices {
                providers: providers.as_ref(),
                cache: cache.as_ref(),
                queue: &queue,
                alias_file: &alias_file,
            },
            &cancelled,
            move |step, message| {
                if progress_tasks.update(&progress_task_id, step, message) {
                    let _ = progress_events.publish(
                        "auto-tag:event",
                        &auto_tag_event(&progress_task_id, "progress", message, step, None),
                    );
                }
            },
            move |kind, message, data| {
                let progress = report_tasks
                    .get(&report_task_id)
                    .map(|task| task.progress)
                    .unwrap_or(0);
                let _ = report_events.publish(
                    "auto-tag:event",
                    &auto_tag_event(&report_task_id, kind, message, progress, data),
                );
            },
        )
        .await;

        match result {
            Ok(result) => {
                let data = serde_json::to_value(&result.candidate).unwrap_or_default();
                let message = auto_tag_completion_message(&result.candidate);
                task_state.finish(
                    &task_id_for_work,
                    TaskStatus::Completed,
                    message,
                    data.clone(),
                );
                let _ = event_bus.publish(
                    "auto-tag:event",
                    &auto_tag_event(&task_id_for_work, "completed", message, 9, Some(data)),
                );
            }
            Err(error) if cancelled.load(Ordering::Acquire) => {
                let progress = task_state
                    .get(&task_id_for_work)
                    .map(|task| task.progress)
                    .unwrap_or(0);
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
                tracing::debug!(%error, "web auto-tag task cancelled");
            }
            Err(error) => {
                let message = error.to_string();
                let data = serde_json::json!({"error": message});
                task_state.finish(
                    &task_id_for_work,
                    TaskStatus::Failed,
                    &message,
                    data.clone(),
                );
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
        "file:exists" => {
            let request = match decode_command_payload::<FileExistsCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
        "config:set" => {
            let request = match decode_command_payload::<ConfigSetCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            match state.config.try_set(&request.key, &request.value) {
                Ok(()) => Json(serde_json::Value::Null).into_response(),
                Err(ConfigSetError::UnsupportedKey(key)) => error_response(
                    StatusCode::BAD_REQUEST,
                    format!("unsupported config key: {key}"),
                ),
                Err(ConfigSetError::Persistence) => {
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "config persistence failed")
                }
            }
        }
        "test-llm-connection" => {
            let request = match decode_command_payload::<TestLlmConnectionCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
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
                Err(ConfigSetError::Persistence) => {
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "config persistence failed")
                }
                Err(ConfigSetError::UnsupportedKey(_)) => {
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "config persistence failed")
                }
            }
        }
        "album:search-releases" => {
            let WrappedCommandRequest { request } =
                match decode_command_payload::<WrappedCommandRequest<SearchReleasesRequest>>(payload)
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
            let WrappedCommandRequest { request } =
                match decode_command_payload::<WrappedCommandRequest<ResolveReleaseRequest>>(payload)
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
            let mut request =
                match decode_command_payload::<WrappedCommandRequest<ApplyCandidateRequest>>(payload)
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
            };
            let mut updates = request.updates;
            for update in &mut updates {
                let resolved = match resolve_existing_track(roots, &update.path) {
                    Ok(path) => path,
                    Err(response) => return *response,
                };
                update.path = resolved.path.to_string_lossy().into_owned();
            }
            let sink = Some(Arc::new(state.events.clone()) as Arc<dyn crate::state::events::EventSink>);
            match batch_write_with_readback(&state.write_queue, updates, sink).await {
                Ok(result) => Json(result).into_response(),
                Err(error) => media_error(error),
            }
        }
        "track:extra-tags:read" => {
            let request = match decode_command_payload::<ExtraTagsReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
            let request = match decode_command_payload::<ExtraTagsBatchWriteCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
            };
            let resolved = match roots.resolve_path(Path::new(&request.source_dir)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "source directory not found");
            }
            let copy = request.options.and_then(|options| options.copy).unwrap_or(true);
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
            };
            let paths = if let Some(track_paths) = request.track_paths.filter(|paths| !paths.is_empty()) {
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
            } else if let Some(album_paths) = request.album_paths.filter(|paths| !paths.is_empty()) {
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
                return error_response(StatusCode::BAD_REQUEST, "no tracks or albums specified for audit");
            };
            execute_audit(
                &state,
                paths.into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            )
            .await
        }
        "audit:run-album" => {
            let request = match decode_command_payload::<AlbumReadCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
            let mut request = match decode_command_payload::<AuditApplyFixesCommandRequest>(payload) {
                Ok(request) => request.album_results,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
        "task:progress" => {
            let request = match decode_command_payload::<TaskProgressCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            Json(state.tasks.get(&request.task_id)).into_response()
        }
        "task:cancel" => {
            let request = match decode_command_payload::<TaskProgressCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            state.tasks.cancel(&request.task_id);
            Json(serde_json::Value::Null).into_response()
        }
        "lyrics:fetch" => {
            let request = match decode_command_payload::<LyricsFetchCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let base_url = resolve_lyrics_base_url(&state.config.raw());
            let report = download_album_lyrics_at(
                &resolved.path,
                &base_url,
                &state.write_queue,
            )
            .await;
            Json(report).into_response()
        }
        "cover:data-url" => {
            let request = match decode_command_payload::<CoverDataCommandRequest>(payload) {
                Ok(request) => request,
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
            };
            let resolved = match roots.resolve_path(Path::new(&request.album_path)) {
                Ok(path) => path,
                Err(error) => return scan_path_error(error),
            };
            if !resolved.path.is_dir() {
                return error_response(StatusCode::BAD_REQUEST, "album path not found");
            }
            let remote = remote_client(state.providers.as_ref(), &state.config);
            let result = download_album_artwork_at(&resolved.path, &remote, &state.write_queue).await;
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
                Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid command request"),
            };
            let roots = match &state.libraries {
                Ok(roots) => roots,
                Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
    let status = if error == "Artist or album is required" || error.starts_with("Unknown provider:") {
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

fn is_public_auth_endpoint(path: &str) -> bool {
    matches!(
        path,
        "/api/v1/auth/login" | "/api/v1/auth/logout" | "/api/v1/auth/session"
    )
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
            .is_some_and(|origin| origin == state.auth.config.public_origin.as_ref());
        if !origin_matches {
            return error_response(StatusCode::FORBIDDEN, "same-origin request required");
        }
    }
    if path.starts_with("/api/v1/")
        && !is_public_auth_endpoint(path)
        && !state
            .auth
            .has_valid_session(session_token(request.headers()))
            .await
    {
        return error_response(StatusCode::UNAUTHORIZED, "authentication required");
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
    if state.auth.config.secure_cookie {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == SESSION_COOKIE && !value.is_empty()).then_some(value)
            })
        })
}

async fn login(
    State(state): State<ServerState>,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Response {
    let Json(payload) = match payload {
        Ok(payload) => payload,
        Err(rejection) => {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                return error_response(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
            }
            return error_response(StatusCode::BAD_REQUEST, "invalid login request");
        }
    };
    match state.auth.verify_password(&payload.password).await {
        LoginAttempt::Throttled => {
            return error_response(StatusCode::TOO_MANY_REQUESTS, "login temporarily blocked")
        }
        LoginAttempt::Invalid => {
            return error_response(StatusCode::UNAUTHORIZED, "invalid credentials")
        }
        LoginAttempt::Authenticated => {}
    }

    let token = state.auth.create_session().await;
    let mut response = Json(SessionResponse {
        authenticated: true,
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::try_from(state.auth.session_cookie(&token)).unwrap(),
    );
    response
}

async fn session(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    let authenticated = state.auth.has_valid_session(session_token(&headers)).await;
    Json(SessionResponse { authenticated }).into_response()
}

async fn logout(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    state.auth.remove_session(session_token(&headers)).await;
    let mut response = Json(SessionResponse {
        authenticated: false,
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::try_from(state.auth.clear_session_cookie()).unwrap(),
    );
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
        Err(_) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, "library roots unavailable"),
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
    let tasks = Arc::new(TaskRegistry::default());
    let web_root = safe_web_root(&config.web_root);
    let spa = ServeDir::new(&web_root).fallback(ServeFile::new(web_root.join("index.html")));
    let state = ServerState {
        auth: AuthService::new(&config),
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
    };
    let login_route = Router::new()
        .route("/api/v1/auth/login", post(login))
        .layer(DefaultBodyLimit::max(64 * 1024));
    let cover_upload_route = Router::new()
        .route("/api/v1/covers", post(upload_cover));

    Router::new()
        .route("/healthz", get(health))
        .route("/api/v1/auth/session", get(session))
        .route("/api/v1/libraries", get(libraries))
        .route("/api/v1/events", get(events))
        .route("/api/v1/commands/{command}", post(command))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/{*path}", any(api_not_found))
        .merge(login_route)
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
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use futures_util::StreamExt;
    use serde_json::json;
    use std::time::Duration;
    use tokio::sync::{Barrier, Notify};
    use tower::ServiceExt;

    fn test_config() -> ServerConfig {
        ServerConfig::for_tests("correct horse battery staple", "https://soundrobe.test")
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

    async fn login(router: Router, password: &str) -> axum::response::Response {
        login_with_origin(router, password, "https://soundrobe.test").await
    }

    async fn login_with_origin(
        router: Router,
        password: &str,
        origin: &str,
    ) -> axum::response::Response {
        router
            .oneshot(origin_request_with(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(r#"{{"password":"{password}"}}"#)))
                    .unwrap(),
                origin,
            ))
            .await
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
        let web_root = std::env::temp_dir().join(format!(
            "soundrobe-web-static-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&web_root).unwrap();
        std::fs::write(&web_root.join("index.html"), "<main>Soundrobe</main>").unwrap();
        let mut config = test_config();
        config.web_root = web_root.clone();

        let response = router(config)
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
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
        let login_response = login(api_app.clone(), "correct horse battery staple").await;
        let api_response = api_app
            .oneshot(
                origin_request(
                    Request::builder()
                        .uri("/api/v1/not-known")
                        .header(header::COOKIE, login_response.headers()[header::SET_COOKIE].clone())
                        .body(Body::empty())
                        .unwrap(),
                ),
            )
            .await
            .unwrap();
        assert_json_error(api_response, StatusCode::NOT_FOUND, r#"{"error":"not found"}"#)
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
    async fn api_routes_require_a_session_except_for_auth_endpoints() {
        let response = router(test_config())
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
            StatusCode::UNAUTHORIZED,
            r#"{"error":"authentication required"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn authenticated_command_transport_exposes_web_app_info() {
        let app = router(test_config());
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/app%3Ainfo")
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                ),
            )
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
    async fn authenticated_command_transport_validates_llm_test_payload_without_network() {
        let app = router(test_config());
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/test-llm-connection")
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"apiKey":"x","model":""}"#))
                        .unwrap(),
                ),
            )
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
    async fn authenticated_command_transport_runs_a_confined_audit() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-audit-{}",
            Uuid::new_v4().simple()
        ));
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();
        let response = app
            .oneshot(
                origin_request(
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
                ),
            )
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

        let outside_login = login(outside_app.clone(), "correct horse battery staple").await;
        let outside_response = outside_app
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/audit%3Arun")
                        .header(
                            header::COOKIE,
                            outside_login.headers()[header::SET_COOKIE].clone(),
                        )
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"libraryPath":"/tmp/not-mounted"}"#))
                        .unwrap(),
                ),
            )
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
    async fn authenticated_command_transport_starts_confined_auto_tag_tasks() {
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();
        let progress_app = app.clone();
        let response = app
            .oneshot(
                origin_request(
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
                ),
            )
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
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/task%3Aprogress")
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&json!({"taskId": task_id})).unwrap(),
                        ))
                        .unwrap(),
                ),
            )
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
    async fn authenticated_command_transport_scans_a_confined_library_root() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-scan-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(base.join("Artist/Album")).unwrap();
        std::fs::write(base.join("Artist/Album/01.flac"), b"fixture").unwrap();
        let canonical_base = base.canonicalize().unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
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
                ),
            )
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
    async fn authenticated_command_transport_reads_a_confined_album() {
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
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
                ),
            )
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

    #[tokio::test]
    async fn authenticated_track_mutations_use_shared_writers_and_stay_confined() {
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();
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

        let batch_path = batch_track.canonicalize().unwrap().to_string_lossy().into_owned();
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
        assert_eq!(delete_response_body(delete_response).await[0]["success"], true);
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
    async fn authenticated_lyrics_commands_preserve_typed_results_and_confinement() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-lyrics-{}",
            Uuid::new_v4().simple()
        ));
        let album = base.join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let fetch_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/commands/lyrics%3Afetch")
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"trackName":"","artistName":"Artist"}"#,
                    ))
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
                    .body(Body::from(
                        r#"{"albumPath":"/tmp/not-mounted-album"}"#,
                    ))
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(outside_response.status(), StatusCode::FORBIDDEN);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn authenticated_command_transport_reads_a_confined_external_cover() {
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
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
                ),
            )
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
    async fn authenticated_cover_upload_normalizes_and_writes_inside_the_album() {
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();
        let album_path = album.canonicalize().unwrap().to_string_lossy().into_owned();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("albumPath", &album_path)
            .finish();

        let response = app
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/v1/covers?{query}"))
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "image/png")
                        .body(Body::from(image_bytes))
                        .unwrap(),
                ),
            )
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
    async fn authenticated_cover_remove_deletes_external_art_and_suppresses_reappearance() {
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
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
                ),
            )
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
    async fn authenticated_directory_listing_and_file_exists_stay_inside_roots() {
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();
        let artist_path = artist.canonicalize().unwrap().display().to_string();

        let listing = app
            .clone()
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/directory%3Alist")
                        .header(header::COOKIE, cookie.clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(r#"{{"dirPath":"{artist_path}"}}"#)))
                        .unwrap(),
                ),
            )
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
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/file%3Aexists")
                        .header(header::COOKIE, cookie.clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(r#"{{"filePath":"{track_path}"}}"#)))
                        .unwrap(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(exists.status(), StatusCode::OK);
        assert_eq!(axum::body::to_bytes(exists.into_body(), usize::MAX).await.unwrap().as_ref(), b"true");

        let missing_path = artist
            .canonicalize()
            .unwrap()
            .join("missing.flac")
            .display()
            .to_string();
        let missing = app
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/file%3Aexists")
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(format!(r#"{{"filePath":"{missing_path}"}}"#)))
                        .unwrap(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::OK);
        assert_eq!(axum::body::to_bytes(missing.into_body(), usize::MAX).await.unwrap().as_ref(), b"false");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn authenticated_config_commands_return_redacted_settings_and_persist_safe_values() {
        let base = std::env::temp_dir().join(format!(
            "soundrobe-web-config-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let mut config = test_config();
        config.data_dir = base.clone();
        let app = router(config);
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let get_response = app
            .clone()
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/config%3Aget")
                        .header(header::COOKIE, cookie.clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                ),
            )
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
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/config%3Aset")
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"key":"debug","value":true}"#))
                        .unwrap(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(set_response.status(), StatusCode::OK);
        assert!(std::fs::read_to_string(base.join("config.yaml"))
            .unwrap()
            .contains("debug: true"));

        let debug_response = app
            .clone()
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/debug%3Aset-mode")
                        .header(header::COOKIE, login_response.headers()[header::SET_COOKIE].clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"enabled":false}"#))
                        .unwrap(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(debug_response.status(), StatusCode::OK);
        assert!(std::fs::read_to_string(base.join("config.yaml"))
            .unwrap()
            .contains("debug: false"));

        let assistant_response = app
            .clone()
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/config%3Aset")
                        .header(header::COOKIE, login_response.headers()[header::SET_COOKIE].clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"key":"assistantAutonomous","value":true}"#))
                        .unwrap(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(assistant_response.status(), StatusCode::OK);
        assert!(std::fs::read_to_string(base.join("config.yaml"))
            .unwrap()
            .contains("assistant_autonomous: true"));

        let clear_response = app
            .clone()
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/config%3Aset")
                        .header(header::COOKIE, login_response.headers()[header::SET_COOKIE].clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"key":"llmProvider","value":null}"#))
                        .unwrap(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(clear_response.status(), StatusCode::OK);
        let refreshed = app
            .clone()
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/config%3Aget")
                        .header(header::COOKIE, login_response.headers()[header::SET_COOKIE].clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                ),
            )
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
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/config%3Aset")
                        .header(header::COOKIE, login_response.headers()[header::SET_COOKIE].clone())
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"key":"notARealKey","value":true}"#))
                        .unwrap(),
                ),
            )
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response
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
            app.clone().oneshot(request("text/plain", b"cover".to_vec())).await.unwrap(),
            StatusCode::BAD_REQUEST,
            r#"{"error":"unsupported cover image type"}"#,
        )
        .await;
        assert_json_error(
            app.clone().oneshot(request("image/png", b"not-an-image".to_vec())).await.unwrap(),
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
    async fn authenticated_events_replay_sequenced_channel_data() {
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/app%3Ainfo")
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"unexpected":true}"#))
                        .unwrap(),
                ),
            )
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/unknown")
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                ),
            )
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

        let response = app
            .oneshot(
                origin_request(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/commands/album%3Asearch-releases")
                        .header(header::COOKIE, cookie)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            r#"{"request":{"provider":"unknown","album":"Test"}}"#,
                        ))
                        .unwrap(),
                ),
            )
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response.headers()[header::SET_COOKIE].clone();

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
                    .body(Body::from(r#"{"sourceDir":"/tmp/not-mounted","options":{"copy":true}}"#))
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
        let state = ServerState {
            auth: AuthService::new(&config),
            config: ConfigState::init_in(config.data_dir.clone()),
            providers: Arc::new(crate::state::providers::ProviderState::default()),
            libraries: Ok(LibraryRoots::default()),
            events: EventBus::default(),
            operations: OperationCoordinator::default(),
            lifecycle: ServerLifecycle::default(),
            debug: WebDebugState::default(),
            write_queue: WriteQueue::default(),
            audit: Arc::new(AuditState::default()),
            cache: Arc::new(CacheState::new_in(config.data_dir.clone())),
            tasks: Arc::new(TaskRegistry::default()),
        };
        let _active = state
            .operations
            .try_acquire(OperationKind::AutoTag)
            .unwrap();
        let token = state.auth.create_session().await;
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
                    .header(header::COOKIE, format!("{SESSION_COOKIE}={token}"))
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
    async fn authenticated_library_listing_returns_only_mounted_roots() {
        let base =
            std::env::temp_dir().join(format!("soundrobe-http-libraries-{}", Uuid::new_v4()));
        std::fs::create_dir_all(base.join("alpha/nested")).unwrap();
        std::fs::create_dir_all(base.join("beta")).unwrap();
        let mut config = test_config();
        config.library_root_dir = base.clone();
        let app = router(config);
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response
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
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response
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
    fn startup_requires_public_origin_and_password() {
        let missing_password = AuthConfig::from_sources(None, None, Some("https://soundrobe.test"));
        assert!(missing_password.is_err());

        let missing_origin = AuthConfig::from_sources(None, Some("secret"), None);
        assert!(missing_origin.is_err());
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
    async fn normalized_same_origin_headers_are_accepted() {
        let config = ServerConfig::for_tests(
            "correct horse battery staple",
            "HTTPS://SOUNDROBE.TEST:443/",
        );
        assert_eq!(
            login_with_origin(
                router(config),
                "correct horse battery staple",
                "HTTPS://SOUNDROBE.TEST:443",
            )
            .await
            .status(),
            StatusCode::OK
        );
    }

    #[test]
    fn password_file_takes_precedence_and_removes_only_trailing_newlines() {
        let path = std::env::temp_dir().join(format!("soundrobe-auth-{}", Uuid::new_v4()));
        std::fs::write(&path, "file-secret  \n").unwrap();

        let config = AuthConfig::from_sources(
            Some(&path),
            Some("environment-secret"),
            Some("https://soundrobe.test"),
        )
        .unwrap();

        std::fs::remove_file(path).unwrap();
        assert!(config.password_matches("file-secret  "));
        assert!(!config.password_matches("environment-secret"));
    }

    #[test]
    fn unreadable_password_file_fails_startup_instead_of_using_the_fallback() {
        let path = std::env::temp_dir().join(format!("soundrobe-missing-auth-{}", Uuid::new_v4()));
        let result = AuthConfig::from_sources(
            Some(&path),
            Some("environment-secret"),
            Some("https://soundrobe.test"),
        );

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn login_sets_opaque_strict_session_cookie_without_returning_the_password() {
        let response = login(router(test_config()), "correct horse battery staple").await;

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.starts_with("soundrobe_session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("; Secure"));
        assert!(cookie.contains("Max-Age=86400"));
        assert!(!cookie.contains("correct horse"));
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("correct horse"));
    }

    #[tokio::test]
    async fn session_endpoint_reports_authenticated_cookie_and_logout_expires_it() {
        let app = router(test_config());
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        let cookie_pair = cookie.split(';').next().unwrap();

        let session_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session_response.status(), StatusCode::OK);
        let session_body = axum::body::to_bytes(session_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(session_body.as_ref(), b"{\"authenticated\":true}");

        let logout_response = app
            .clone()
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/logout")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(logout_response.status(), StatusCode::OK);
        assert!(logout_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0"));
        let session_after_logout = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(session_after_logout.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"{\"authenticated\":false}");
        let fresh_session = router(test_config())
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let fresh_body = axum::body::to_bytes(fresh_session.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(fresh_body.as_ref(), b"{\"authenticated\":false}");
    }

    #[tokio::test]
    async fn expired_sessions_are_rejected_without_exposing_session_state() {
        let mut config = test_config();
        config.session_ttl = Duration::from_millis(1);
        let app = router(config);
        let login_response = login(app.clone(), "correct horse battery staple").await;
        let cookie = login_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/session")
                    .header(header::COOKIE, cookie.split(';').next().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"{\"authenticated\":false}");
        let protected_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/libraries")
                    .header(header::COOKIE, cookie.split(';').next().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_json_error(
            protected_response,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"authentication required"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn failed_logins_are_rate_limited_and_success_resets_the_window() {
        let mut config = test_config();
        config.max_failed_logins = 2;
        config.login_window = Duration::from_secs(60);
        let app = router(config);

        assert_json_error(
            login(app.clone(), "wrong").await,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"invalid credentials"}"#,
        )
        .await;
        assert_eq!(
            login(app.clone(), "correct horse battery staple")
                .await
                .status(),
            StatusCode::OK
        );
        assert_json_error(
            login(app.clone(), "wrong").await,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"invalid credentials"}"#,
        )
        .await;
        assert_json_error(
            login(app.clone(), "wrong").await,
            StatusCode::UNAUTHORIZED,
            r#"{"error":"invalid credentials"}"#,
        )
        .await;
        assert_json_error(
            login(app, "wrong").await,
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":"login temporarily blocked"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn concurrent_failed_logins_share_one_atomic_throttle() {
        let mut config = test_config();
        config.max_failed_logins = 1;
        let app = router(config);
        let (first, second) = tokio::join!(login(app.clone(), "wrong"), login(app, "wrong"));
        let statuses = [first.status(), second.status()];

        assert!(statuses.contains(&StatusCode::UNAUTHORIZED));
        assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS));
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

        let accepted_app = router(test_config());
        let login_response = login(accepted_app.clone(), "correct horse battery staple").await;
        let cookie_pair = login_response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let accepted = accepted_app
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/logout")
                    .header(header::COOKIE, cookie_pair)
                    .body(Body::empty())
                    .unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn malformed_login_requests_return_stable_json_errors() {
        let response = router(test_config())
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("not-json"))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid login request"}"#,
        )
        .await;
    }

    #[tokio::test]
    async fn oversized_login_requests_return_payload_too_large() {
        let oversized_password = "x".repeat(64 * 1024);
        let response = router(test_config())
            .oneshot(origin_request(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"password":"{oversized_password}"}}"#
                    )))
                    .unwrap(),
            ))
            .await
            .unwrap();

        assert_json_error(
            response,
            StatusCode::PAYLOAD_TOO_LARGE,
            r#"{"error":"request body too large"}"#,
        )
        .await;
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
        let state = ServerState {
            auth: AuthService::new(&config),
            config: ConfigState::init_in(config.data_dir.clone()),
            providers: Arc::new(crate::state::providers::ProviderState::default()),
            libraries: Ok(LibraryRoots::default()),
            events: EventBus::default(),
            operations: OperationCoordinator::default(),
            lifecycle: ServerLifecycle::default(),
            debug: WebDebugState::default(),
            write_queue: WriteQueue::default(),
            audit: Arc::new(AuditState::default()),
            cache: Arc::new(CacheState::new_in(config.data_dir.clone())),
            tasks: Arc::new(TaskRegistry::default()),
        };
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
}
