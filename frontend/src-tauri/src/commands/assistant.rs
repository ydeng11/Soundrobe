//! Assistant runtime and tool-service commands.

use crate::commands::{
    mutations::{
        rename_track_queued, write_extra_tags_queued, write_track_queued, ExtraTagUpdate,
        TrackPatch,
    },
    tracks::{read_extra_tags, read_track_metadata},
};
use crate::error::ApiError;
use crate::infra::openrouter::{ChatMessage, OpenRouterClient};
use crate::state::assistant::{
    AssistantActionBatch, AssistantRuntimeState, AssistantServicesConfig, AssistantServicesState,
};
use crate::state::config::ConfigState;
use crate::state::conversation::ConversationState;
use crate::state::write_queue::WriteQueue;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEvent {
    session_id: String,
    #[serde(rename = "type")]
    event_type: &'static str,
    message: String,
    data: Option<Value>,
}

#[tauri::command]
pub async fn assistant_send(
    input: AssistantSendInput,
    app: AppHandle,
    runtime: State<'_, AssistantRuntimeState>,
    services: State<'_, AssistantServicesState>,
    config: State<'_, ConfigState>,
    conversation: State<'_, ConversationState>,
) -> Result<AssistantEvent, ApiError> {
    let raw_config = config.raw();
    let contract = derive_assistant_task_contract(&input.message);
    if !conversation.initialize(raw_config.cache_path.as_deref()) || !runtime.initialize() {
        return Err(ApiError::Message(
            "Assistant runtime could not be initialized".into(),
        ));
    }
    let current = conversation
        .current()
        .ok_or_else(|| ApiError::Message("No active assistant session".into()))?;
    conversation.record("user_message", &input.message, None, 0, 0, 0);
    if let Some(route) = contract.route {
        let batch = deterministic_task_batch(&current.session_id, &input, route)?;
        if !runtime.add_batch(batch.clone()) {
            return Err(ApiError::Message(
                "Failed to store assistant action preview".into(),
            ));
        }
        let event = AssistantEvent {
            session_id: current.session_id,
            event_type: "action_batch_created",
            message: batch.summary.clone(),
            data: Some(serde_json::json!({
                "actionBatchId": batch.id,
                "actionBatch": batch,
                "routeSource": "deterministic",
                "contractReason": contract.reason,
            })),
        };
        conversation.record("assistant_message", &event.message, None, 0, 0, 0);
        let _ = app.emit("assistant:event", &event);
        return Ok(event);
    }
    let snapshot = services.snapshot().unwrap_or_default();
    let api_key = (!snapshot.api_key.is_empty())
        .then_some(snapshot.api_key)
        .or(raw_config.llm_api_key)
        .filter(|key| !key.is_empty());
    let model = (!snapshot.model.is_empty())
        .then_some(snapshot.model)
        .or(raw_config.llm_model)
        .filter(|model| !model.is_empty());
    let Some(api_key) = api_key else {
        return assistant_error_event(
            &app,
            conversation.current().map(|current| current.session_id),
            "LLM API key is not configured. Set it in Settings or via the LLM_API_KEY environment variable.",
        );
    };
    let Some(model) = model else {
        return assistant_error_event(
            &app,
            conversation.current().map(|current| current.session_id),
            "LLM model is not configured. Set it in Settings or via the LLM_MODEL environment variable.",
        );
    };
    let cancelled = runtime
        .begin_request()
        .ok_or_else(|| ApiError::Message("Assistant runtime is unavailable".into()))?;
    let step = AssistantEvent {
        session_id: current.session_id.clone(),
        event_type: "step",
        message: "Step 1/1".into(),
        data: None,
    };
    let _ = app.emit("assistant:event", step);

    let context = serde_json::json!({
        "libraryPath": input.library_path,
        "activeAlbumPath": input.active_album_path,
        "selectedTrackPaths": input.selected_track_paths,
        "tracks": input.tracks.iter().take(200).collect::<Vec<_>>(),
        "albums": input.albums.iter().take(100).collect::<Vec<_>>(),
        "autonomous": input.autonomous,
    });
    let messages = vec![
        ChatMessage::system(concat!(
            "You are the Auto Tagger desktop assistant. Answer music-library questions directly. ",
            "For any mutation, return an actionBatch preview; never claim a write already happened. ",
            "Allowed batch kinds: tag-update, extra-tag-update, metadata-update, auto-tag-run, audit-run. ",
            "Every action must use an exact trackPath from the supplied active scope. ",
            "Standard metadata actions use tagKind=standard, field, and newValue (null removes). ",
            "Custom tags use tagKind=extra. Auto-tag/audit actions need only trackPath. ",
            "Keep riskLevel low, medium, or high. Return concise user-facing text in message."
        )),
        ChatMessage::user(format!("App context:\n{context}\n\nUser request:\n{}", input.message)),
    ];
    let response = OpenRouterClient::new(&api_key, &model)
        .with_generation(0.2, 1400)
        .complete_json(
            messages,
            "AssistantResponse",
            assistant_response_schema(),
            &cancelled,
        )
        .await;
    if cancelled.load(Ordering::Acquire) {
        let event = AssistantEvent {
            session_id: current.session_id,
            event_type: "cancelled",
            message: "Cancelled".into(),
            data: None,
        };
        let _ = app.emit("assistant:event", &event);
        return Ok(event);
    }
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            conversation.record_system(&error.to_string());
            return assistant_error_event(&app, Some(current.session_id), &error.to_string());
        }
    };
    conversation.record(
        "api_response",
        &response.data.to_string(),
        Some(&response.model),
        response.usage.prompt_tokens,
        response.usage.completion_tokens,
        response.usage.total_tokens,
    );
    let draft: AssistantDraft = match serde_json::from_value(response.data) {
        Ok(draft) => draft,
        Err(error) => {
            return assistant_error_event(
                &app,
                Some(current.session_id),
                &format!("Invalid assistant response: {error}"),
            );
        }
    };
    validate_completion_evidence(&contract, draft.action_batch.is_some(), &draft.message)?;
    let event = if let Some(batch) = draft.action_batch {
        let batch = match validated_assistant_batch(&current.session_id, &input, batch) {
            Ok(batch) => batch,
            Err(error) => {
                return assistant_error_event(&app, Some(current.session_id), &error.to_string());
            }
        };
        if !runtime.add_batch(batch.clone()) {
            return Err(ApiError::Message(
                "Failed to store assistant action preview".into(),
            ));
        }
        AssistantEvent {
            session_id: current.session_id,
            event_type: "action_batch_created",
            message: draft.message,
            data: Some(serde_json::json!({
                "actionBatchId": batch.id,
                "actionBatch": batch
            })),
        }
    } else {
        AssistantEvent {
            session_id: current.session_id,
            event_type: "message",
            message: draft.message,
            data: None,
        }
    };
    conversation.record("assistant_message", &event.message, Some(&model), 0, 0, 0);
    let _ = app.emit("assistant:event", &event);
    Ok(event)
}

fn assistant_error_event(
    app: &AppHandle,
    session_id: Option<String>,
    message: &str,
) -> Result<AssistantEvent, ApiError> {
    let event = AssistantEvent {
        session_id: session_id.unwrap_or_else(|| "none".into()),
        event_type: "error",
        message: message.to_string(),
        data: None,
    };
    let _ = app.emit("assistant:event", &event);
    Ok(event)
}

fn assistant_response_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "message": {"type": "string"},
            "actionBatch": {
                "type": ["object", "null"],
                "properties": {
                    "kind": {"type": "string"},
                    "title": {"type": "string"},
                    "summary": {"type": "string"},
                    "riskLevel": {"type": "string", "enum": ["low", "medium", "high"]},
                    "actions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "tagKind": {"type": ["string", "null"]},
                                "trackPath": {"type": "string"},
                                "field": {"type": ["string", "null"]},
                                "newValue": {"type": ["string", "null"]},
                                "description": {"type": ["string", "null"]}
                            },
                            "required": ["trackPath"]
                        }
                    }
                },
                "required": ["kind", "title", "summary", "riskLevel", "actions"]
            }
        },
        "required": ["message", "actionBatch"]
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSendInput {
    pub message: String,
    #[serde(default)]
    pub library_path: Option<String>,
    #[serde(default)]
    pub active_album_path: Option<String>,
    #[serde(default)]
    pub selected_track_paths: Vec<String>,
    #[serde(default)]
    pub tracks: Vec<Value>,
    #[serde(default)]
    pub albums: Vec<Value>,
    #[serde(default)]
    pub autonomous: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssistantTaskContractKind {
    ReadOnlyAnswer,
    ActionPreviewRequired,
    ClarificationRequired,
    ChatOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssistantTaskRoute {
    AutoTag,
    Audit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AssistantTaskContract {
    kind: AssistantTaskContractKind,
    route: Option<AssistantTaskRoute>,
    reason: &'static str,
    requires_completion_evidence: bool,
}

fn derive_assistant_task_contract(message: &str) -> AssistantTaskContract {
    let text = message.trim().to_lowercase();
    if text.is_empty() {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ClarificationRequired,
            route: None,
            reason: "empty_user_message",
            requires_completion_evidence: false,
        };
    }
    if text.contains("auto-tag")
        || text.contains("auto tag")
        || text.contains("fill tags")
        || text.contains("fill missing tags")
        || text.contains("tag this")
    {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ActionPreviewRequired,
            route: Some(AssistantTaskRoute::AutoTag),
            reason: "auto_tag_intent",
            requires_completion_evidence: true,
        };
    }
    if text.contains("audit")
        || text.contains("check missing")
        || text.contains("check metadata")
        || text.contains("scan metadata")
    {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ActionPreviewRequired,
            route: Some(AssistantTaskRoute::Audit),
            reason: "audit_intent",
            requires_completion_evidence: true,
        };
    }
    let read_only = [
        "summarize",
        "summary",
        "find",
        "search",
        "list",
        "show",
        "inspect",
        "what ",
        "which ",
        "how many",
        "count",
        "missing",
        "duplicate",
    ]
    .iter()
    .any(|needle| text.contains(needle));
    let mutation = [
        "apply", "change", "fix", "update", "edit", "set ", "write", "number", "renumber", "infer",
        "parse", "strip", "organize", "organise", "move", "run ",
    ]
    .iter()
    .any(|needle| text.contains(needle))
        || text.starts_with("tag ");
    if mutation {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ActionPreviewRequired,
            route: None,
            reason: "general_action_intent",
            requires_completion_evidence: true,
        };
    }
    if read_only {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ReadOnlyAnswer,
            route: None,
            reason: "read_only_intent",
            requires_completion_evidence: false,
        };
    }
    AssistantTaskContract {
        kind: AssistantTaskContractKind::ChatOnly,
        route: None,
        reason: "no_action_or_read_only_intent",
        requires_completion_evidence: false,
    }
}

fn active_scope_paths(input: &AssistantSendInput) -> Vec<String> {
    if !input.selected_track_paths.is_empty() {
        return input.selected_track_paths.clone();
    }
    input
        .tracks
        .iter()
        .filter_map(|track| track.get("path").and_then(Value::as_str))
        .filter(|path| {
            input
                .active_album_path
                .as_deref()
                .is_none_or(|album| Path::new(path).parent() == Some(Path::new(album)))
        })
        .map(str::to_string)
        .collect()
}

fn deterministic_task_batch(
    session_id: &str,
    input: &AssistantSendInput,
    route: AssistantTaskRoute,
) -> Result<AssistantActionBatch, ApiError> {
    let paths = active_scope_paths(input);
    if paths.is_empty() {
        return Err(ApiError::Message(
            "No tracks are available in the current assistant scope".into(),
        ));
    }
    let (kind, title, summary) = match route {
        AssistantTaskRoute::AutoTag => (
            "auto-tag-run",
            "Run auto-tag",
            format!("Preview auto-tag for {} track(s)", paths.len()),
        ),
        AssistantTaskRoute::Audit => (
            "audit-run",
            "Run metadata audit",
            format!("Preview metadata audit for {} track(s)", paths.len()),
        ),
    };
    Ok(AssistantActionBatch {
        id: format!("batch-{}", uuid::Uuid::new_v4()),
        created_at: time::OffsetDateTime::now_utc().to_string(),
        session_id: session_id.to_string(),
        kind: kind.into(),
        title: title.into(),
        summary,
        risk_level: "low".into(),
        actions: paths
            .into_iter()
            .map(|track_path| crate::state::assistant::AssistantAction {
                track_path: Some(track_path),
                ..Default::default()
            })
            .collect(),
        reversible: true,
        status: "pending".into(),
    })
}

fn validate_completion_evidence(
    contract: &AssistantTaskContract,
    has_action_batch: bool,
    response_message: &str,
) -> Result<(), ApiError> {
    if contract.requires_completion_evidence && !has_action_batch {
        return Err(ApiError::Message(format!(
            "No action was performed. This request requires a preview batch, but the assistant only replied: {response_message}"
        )));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssistantDraft {
    message: String,
    #[serde(default)]
    action_batch: Option<AssistantDraftBatch>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssistantDraftBatch {
    kind: String,
    title: String,
    summary: String,
    risk_level: String,
    #[serde(default)]
    actions: Vec<crate::state::assistant::AssistantAction>,
}

fn allowed_assistant_paths(input: &AssistantSendInput) -> HashSet<String> {
    active_scope_paths(input).into_iter().collect()
}

fn validated_assistant_batch(
    session_id: &str,
    input: &AssistantSendInput,
    draft: AssistantDraftBatch,
) -> Result<AssistantActionBatch, ApiError> {
    const KINDS: &[&str] = &[
        "tag-update",
        "extra-tag-update",
        "metadata-update",
        "auto-tag-run",
        "audit-run",
    ];
    const STANDARD_FIELDS: &[&str] = &[
        "title",
        "artist",
        "artists",
        "album",
        "albumArtist",
        "albumArtists",
        "year",
        "genre",
        "composer",
        "comment",
        "description",
        "trackNumber",
        "trackTotal",
        "discNumber",
        "discTotal",
        "lyrics",
        "compilation",
        "musicbrainzTrackId",
        "musicbrainzAlbumId",
        "musicbrainzArtistId",
        "discogsArtistId",
        "discogsReleaseId",
    ];
    if !KINDS.contains(&draft.kind.as_str()) {
        return Err(ApiError::Message(format!(
            "Assistant proposed unsupported action kind: {}",
            draft.kind
        )));
    }
    if !matches!(draft.risk_level.as_str(), "low" | "medium" | "high") {
        return Err(ApiError::Message(format!(
            "Assistant proposed unsupported risk level: {}",
            draft.risk_level
        )));
    }
    let allowed_paths = allowed_assistant_paths(input);
    if draft.actions.is_empty() {
        return Err(ApiError::Message(
            "Assistant proposed an empty action batch".into(),
        ));
    }
    for action in &draft.actions {
        let path = action
            .track_path
            .as_deref()
            .ok_or_else(|| ApiError::Message("Assistant action is missing trackPath".into()))?;
        if !allowed_paths.contains(path) {
            return Err(ApiError::Message(format!(
                "Assistant action is outside the active scope: {path}"
            )));
        }
        if matches!(draft.kind.as_str(), "auto-tag-run" | "audit-run") {
            continue;
        }
        let tag_kind = action.tag_kind.as_deref().unwrap_or("standard");
        if !matches!(tag_kind, "standard" | "extra") {
            return Err(ApiError::Message(format!(
                "Assistant proposed unsupported tag kind: {tag_kind}"
            )));
        }
        let field = action.field.as_deref().ok_or_else(|| {
            ApiError::Message("Assistant metadata action is missing field".into())
        })?;
        if tag_kind == "standard" && !STANDARD_FIELDS.contains(&field) {
            return Err(ApiError::Message(format!(
                "Assistant proposed unsupported metadata field: {field}"
            )));
        }
        if draft.kind == "tag-update" && tag_kind == "extra" {
            return Err(ApiError::Message(
                "Assistant proposed an extra tag in a standard tag batch".into(),
            ));
        }
        if draft.kind == "extra-tag-update" && tag_kind != "extra" {
            return Err(ApiError::Message(
                "Assistant proposed a standard tag in an extra tag batch".into(),
            ));
        }
    }
    Ok(AssistantActionBatch {
        id: format!("batch-{}", uuid::Uuid::new_v4()),
        created_at: time::OffsetDateTime::now_utc().to_string(),
        session_id: session_id.to_string(),
        kind: draft.kind,
        title: draft.title,
        summary: draft.summary,
        risk_level: draft.risk_level,
        actions: draft.actions,
        reversible: true,
        status: "pending".into(),
    })
}

#[tauri::command]
pub fn assistant_init_services(
    config: AssistantServicesConfig,
    services: State<'_, AssistantServicesState>,
) -> Result<(), ApiError> {
    services
        .initialize(config)
        .then_some(())
        .ok_or_else(|| ApiError::Message("Failed to initialize assistant services".to_string()))
}

#[tauri::command]
pub fn assistant_cancel(
    app: AppHandle,
    runtime: State<'_, AssistantRuntimeState>,
    conversation: State<'_, ConversationState>,
) -> Result<(), ApiError> {
    if !runtime.cancel() {
        return Ok(());
    }
    let current = conversation
        .current()
        .ok_or_else(|| ApiError::Message("No active assistant session".to_string()))?;
    if !conversation.record_system("Session cancelled") {
        return Err(ApiError::Message(
            "Failed to record assistant cancellation".to_string(),
        ));
    }
    app.emit(
        "assistant:event",
        AssistantEvent {
            session_id: current.session_id,
            event_type: "cancelled",
            message: "Session cancelled".to_string(),
            data: None,
        },
    )?;
    Ok(())
}

#[tauri::command]
pub fn assistant_clear(
    runtime: State<'_, AssistantRuntimeState>,
    conversation: State<'_, ConversationState>,
) -> Result<(), ApiError> {
    if runtime.reset() && conversation.reset_session() {
        Ok(())
    } else if conversation.current().is_none() {
        // Electron is a no-op before runtime initialization.
        Ok(())
    } else {
        Err(ApiError::Message(
            "Failed to reset assistant session".to_string(),
        ))
    }
}

#[tauri::command]
pub fn assistant_reject_actions(
    app: AppHandle,
    action_batch_id: String,
    runtime: State<'_, AssistantRuntimeState>,
    conversation: State<'_, ConversationState>,
) -> Result<(), ApiError> {
    let Some(title) = runtime.reject_batch(&action_batch_id) else {
        return Ok(());
    };
    let current = conversation
        .current()
        .ok_or_else(|| ApiError::Message("No active assistant session".to_string()))?;
    app.emit(
        "assistant:event",
        AssistantEvent {
            session_id: current.session_id,
            event_type: "action_batch_rejected",
            message: format!("Rejected: {title}"),
            data: Some(serde_json::json!({ "batchId": action_batch_id })),
        },
    )?;
    Ok(())
}

#[tauri::command]
pub fn assistant_get_batches(
    runtime: State<'_, AssistantRuntimeState>,
) -> Vec<AssistantActionBatch> {
    runtime.pending_batches()
}

fn action_patch(field: &str, new_value: Option<&str>) -> Result<TrackPatch, ApiError> {
    let value = match (field, new_value) {
        (_, None) => Value::Null,
        ("trackNumber" | "trackTotal" | "discNumber" | "discTotal", Some(value)) => Value::Number(
            value
                .parse::<u32>()
                .map_err(|_| {
                    ApiError::Message(format!("Invalid numeric value for {field}: {value}"))
                })?
                .into(),
        ),
        ("compilation", Some(value)) => Value::Bool(value.parse::<bool>().map_err(|_| {
            ApiError::Message(format!("Invalid boolean value for compilation: {value}"))
        })?),
        (_, Some(value)) => Value::String(value.into()),
    };
    serde_json::from_value(serde_json::json!({ field: value }))
        .map_err(|error| ApiError::Message(format!("Invalid assistant tag update: {error}")))
}

async fn apply_standard_actions(
    runtime: &AssistantRuntimeState,
    queue: &WriteQueue,
    batch: &AssistantActionBatch,
    batch_id: &str,
    metadata_only: bool,
    mark_status: bool,
) -> Value {
    let mut updates: Vec<(String, TrackPatch)> = Vec::new();
    for action in &batch.actions {
        let (Some(path), Some(field)) = (action.track_path.as_ref(), action.field.as_deref())
        else {
            continue;
        };
        if metadata_only && action.tag_kind.as_deref() != Some("standard") {
            continue;
        }
        let path = path.clone();
        let patch = match action_patch(field, action.new_value.as_deref()) {
            Ok(patch) => patch,
            Err(error) => {
                if mark_status {
                    runtime.mark_batch_failed(batch_id, &error.to_string());
                }
                return serde_json::json!({ "success": false, "error": error.to_string() });
            }
        };
        if let Some((_, existing)) = updates.iter_mut().find(|(existing, _)| existing == &path) {
            merge_assistant_patch(existing, patch);
        } else {
            updates.push((path, patch));
        }
    }
    let mut undo = Vec::new();
    for (path, _) in &updates {
        match read_track_metadata(Path::new(path)) {
            Ok(track) => {
                undo.push(serde_json::json!({ "path": path, "metadata": undo_metadata(&track) }))
            }
            Err(error) => {
                if mark_status {
                    runtime.mark_batch_failed(batch_id, &error.to_string());
                }
                return serde_json::json!({ "success": false, "error": error.to_string(), "undoSnapshots": undo });
            }
        }
    }
    let mut results = Vec::new();
    for (path, patch) in updates {
        match write_track_queued(queue, PathBuf::from(&path), patch).await {
            Ok(()) => match read_track_metadata(Path::new(&path)) {
                Ok(track) => results.push(serde_json::json!({ "trackPath": path, "success": true, "updatedTrack": track })),
                Err(error) => results.push(serde_json::json!({ "trackPath": path, "success": false, "error": error.to_string() })),
            },
            Err(error) => results.push(serde_json::json!({ "trackPath": path, "success": false, "error": error.to_string() })),
        }
    }
    let failed = results
        .iter()
        .filter(|result| result["success"] == false)
        .count();
    if failed > 0 {
        let error = format!("Failed to update {failed} track(s)");
        if mark_status {
            runtime.mark_batch_failed(batch_id, &error);
        }
        serde_json::json!({ "success": false, "error": error, "results": results.into_iter().filter(|result| result["success"] == false).collect::<Vec<_>>(), "undoSnapshots": undo })
    } else {
        if mark_status {
            runtime.mark_batch_applied(batch_id);
        }
        serde_json::json!({ "success": true, "results": results, "undoSnapshots": undo })
    }
}

fn undo_metadata(track: &crate::commands::tracks::TrackData) -> Value {
    serde_json::json!({
        "title": track.title,
        "artist": track.artist,
        "artists": track.artists,
        "album": track.album,
        "albumArtist": track.album_artist,
        "albumArtists": track.album_artists,
        "year": track.year,
        "genre": track.genre,
        "composer": track.composer,
        "comment": track.comment,
        "description": track.description,
        "trackNumber": track.track_number,
        "trackTotal": track.track_total,
        "discNumber": track.disc_number,
        "discTotal": track.disc_total,
        "lyrics": track.lyrics,
        "compilation": track.compilation,
        "musicbrainzTrackId": track.musicbrainz_track_id,
        "musicbrainzAlbumId": track.musicbrainz_album_id,
        "musicbrainzArtistId": track.musicbrainz_artist_id,
    })
}

async fn apply_extra_actions(
    runtime: &AssistantRuntimeState,
    queue: &WriteQueue,
    batch: &AssistantActionBatch,
    batch_id: &str,
    mark_status: bool,
) -> Value {
    let mut paths = Vec::<String>::new();
    for action in &batch.actions {
        if action.tag_kind.as_deref() == Some("extra")
            && action.track_path.is_some()
            && action.field.is_some()
        {
            let Some(path) = action.track_path.as_ref() else {
                continue;
            };
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    }
    let mut undo = Vec::new();
    let mut results = Vec::new();
    for path in paths {
        let current = read_extra_tags(Path::new(&path));
        undo.push(serde_json::json!({ "path": path, "extraTags": current }));
        let mut final_tags = current
            .iter()
            .map(|tag| ExtraTagUpdate {
                key: tag.key.clone(),
                value: tag.value.clone(),
            })
            .collect::<Vec<_>>();
        for action in batch.actions.iter().filter(|action| {
            action.tag_kind.as_deref() == Some("extra")
                && action.track_path.as_deref() == Some(path.as_str())
                && action.field.is_some()
        }) {
            let Some(key) = action.field.as_deref() else {
                continue;
            };
            let key = key.trim();
            final_tags.retain(|tag| !tag.key.trim().eq_ignore_ascii_case(key));
            if action.operation.as_deref() != Some("remove") {
                if let Some(value) = action.new_value.as_deref() {
                    final_tags.push(ExtraTagUpdate {
                        key: key.into(),
                        value: value.trim().into(),
                    });
                }
            }
        }
        match write_extra_tags_queued(queue, PathBuf::from(&path), final_tags).await {
            Ok(()) => results.push(serde_json::json!({ "trackPath": path, "success": true })),
            Err(error) => results.push(serde_json::json!({ "trackPath": path, "success": false, "error": error.to_string() })),
        }
    }
    let failed = results
        .iter()
        .filter(|result| result["success"] == false)
        .count();
    if failed > 0 {
        let error = format!("Failed to update {failed} track(s)");
        if mark_status {
            runtime.mark_batch_failed(batch_id, &error);
        }
        serde_json::json!({ "success": false, "error": error, "results": results.into_iter().filter(|result| result["success"] == false).collect::<Vec<_>>(), "extraUndoSnapshots": undo })
    } else {
        if mark_status {
            runtime.mark_batch_applied(batch_id);
        }
        serde_json::json!({ "success": true, "results": results, "extraUndoSnapshots": undo })
    }
}

async fn apply_folder_moves(
    runtime: &AssistantRuntimeState,
    queue: &WriteQueue,
    batch: &AssistantActionBatch,
    batch_id: &str,
) -> Value {
    let mut results = Vec::new();
    for action in &batch.actions {
        let (Some(source), Some(destination)) = (
            action.source_path.as_ref(),
            action.destination_path.as_ref(),
        ) else {
            continue;
        };
        if action.skip_reason.is_some() {
            continue;
        }
        let source = source.clone();
        let destination = destination.clone();
        match rename_track_queued(
            queue,
            PathBuf::from(&source),
            PathBuf::from(&destination),
        )
        .await
        {
            Ok(_) => results.push(serde_json::json!({ "sourcePath": source, "destinationPath": destination, "success": true })),
            Err(error) => results.push(serde_json::json!({ "sourcePath": source, "destinationPath": destination, "success": false, "error": error.to_string() })),
        }
    }
    let failed = results
        .iter()
        .filter(|result| result["success"] == false)
        .count();
    if failed > 0 {
        let error = format!("Failed to move {failed} file(s)");
        runtime.mark_batch_failed(batch_id, &error);
        serde_json::json!({ "success": false, "error": error, "results": results.into_iter().filter(|result| result["success"] == false).collect::<Vec<_>>() })
    } else {
        runtime.mark_batch_applied(batch_id);
        let manifest = results.iter().map(|result| serde_json::json!({ "from": result["sourcePath"], "to": result["destinationPath"] })).collect::<Vec<_>>();
        serde_json::json!({ "success": true, "results": results, "manifest": manifest })
    }
}

fn merge_assistant_patch(target: &mut TrackPatch, incoming: TrackPatch) {
    macro_rules! merge {
        ($field:ident) => {
            if !incoming.$field.is_omitted() {
                target.$field = incoming.$field;
            }
        };
    }
    merge!(title);
    merge!(artist);
    merge!(artists);
    merge!(album);
    merge!(album_artist);
    merge!(album_artists);
    merge!(year);
    merge!(genre);
    merge!(composer);
    merge!(comment);
    merge!(description);
    merge!(track_number);
    merge!(track_total);
    merge!(disc_number);
    merge!(disc_total);
    merge!(lyrics);
    merge!(compilation);
    merge!(musicbrainz_track_id);
    merge!(musicbrainz_album_id);
    merge!(musicbrainz_artist_id);
    merge!(discogs_artist_id);
    merge!(discogs_release_id);
}

async fn apply_action_batch(
    runtime: &AssistantRuntimeState,
    queue: &WriteQueue,
    batch_id: &str,
) -> Value {
    if !runtime.is_active() {
        return serde_json::json!({ "success": false, "error": "No active assistant session" });
    }
    let Some(batch) = runtime.get_batch(batch_id) else {
        return serde_json::json!({ "success": false, "error": format!("Action batch not found: {batch_id}") });
    };
    if batch.status != "pending" {
        return serde_json::json!({ "success": false, "error": format!("Batch already {}", batch.status) });
    }
    match batch.kind.as_str() {
        "tag-update" => apply_standard_actions(runtime, queue, &batch, batch_id, false, true).await,
        "extra-tag-update" => apply_extra_actions(runtime, queue, &batch, batch_id, true).await,
        "metadata-update" => {
            let standard =
                apply_standard_actions(runtime, queue, &batch, batch_id, true, false).await;
            let extra = apply_extra_actions(runtime, queue, &batch, batch_id, false).await;
            let standard_failed = standard["success"] == false;
            let extra_failed = extra["success"] == false;
            if standard_failed || extra_failed {
                let failed_standard = if standard_failed {
                    standard["results"].clone()
                } else {
                    serde_json::json!([])
                };
                let failed_extra = if extra_failed {
                    extra["results"].clone()
                } else {
                    serde_json::json!([])
                };
                let failed = failed_standard.as_array().map_or(0, Vec::len)
                    + failed_extra.as_array().map_or(0, Vec::len);
                let error = format!("Failed to update {failed} track(s)");
                runtime.mark_batch_failed(batch_id, &error);
                serde_json::json!({ "success": false, "error": error, "results": { "standard": failed_standard, "extra": failed_extra }, "undoSnapshots": standard["undoSnapshots"], "extraUndoSnapshots": extra["extraUndoSnapshots"] })
            } else {
                runtime.mark_batch_applied(batch_id);
                serde_json::json!({ "success": true, "results": { "standard": standard["results"], "extra": extra["results"] }, "undoSnapshots": standard["undoSnapshots"], "extraUndoSnapshots": extra["extraUndoSnapshots"] })
            }
        }
        "folder-move" => apply_folder_moves(runtime, queue, &batch, batch_id).await,
        "auto-tag-run" | "audit-run" => {
            runtime.mark_batch_applied(batch_id);
            let task = if batch.kind == "auto-tag-run" {
                "auto_tag"
            } else {
                "audit"
            };
            let paths = batch
                .actions
                .iter()
                .filter_map(|action| action.track_path.clone())
                .collect::<Vec<_>>();
            serde_json::json!({ "success": true, "message": format!("{} will be triggered by the renderer", if task == "auto_tag" { "Auto-tag" } else { "Audit" }), "task": task, "trackPaths": paths })
        }
        _ => {
            let error = format!("Unknown batch kind: {}", batch.kind);
            runtime.mark_batch_failed(batch_id, &error);
            serde_json::json!({ "success": false, "error": error })
        }
    }
}

#[tauri::command]
pub async fn assistant_apply_actions(
    app: AppHandle,
    action_batch_id: String,
    runtime: State<'_, AssistantRuntimeState>,
    conversation: State<'_, ConversationState>,
    queue: State<'_, WriteQueue>,
) -> Result<Value, ApiError> {
    let result = apply_action_batch(&runtime, &queue, &action_batch_id).await;
    let Some(batch) = runtime.get_batch(&action_batch_id) else {
        return Ok(result);
    };
    let Some(current) = conversation.current() else {
        return Ok(result);
    };
    let (event_type, message, data) = match batch.status.as_str() {
        "applied" => (
            "action_batch_applied",
            format!("Applied: {}", batch.title),
            serde_json::json!({ "batchId": action_batch_id }),
        ),
        "failed" => {
            let error = runtime.batch_error(&action_batch_id).unwrap_or_default();
            (
                "action_batch_failed",
                format!("Failed: {}: {error}", batch.title),
                serde_json::json!({ "batchId": action_batch_id, "error": error }),
            )
        }
        _ => return Ok(result),
    };
    let _ = app.emit(
        "assistant:event",
        AssistantEvent {
            session_id: current.session_id,
            event_type,
            message,
            data: Some(data),
        },
    );
    Ok(result)
}

#[cfg(test)]
mod apply_contract_tests {
    use super::*;
    use crate::commands::mutations::{write_track_dispatch, Patch, TrackPatch};
    use crate::commands::tracks::read_track_metadata;
    use crate::state::assistant::AssistantAction;
    use crate::state::write_queue::WriteQueue;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn task_contract_routes_known_library_tasks_without_model_judgment() {
        let auto_tag = derive_assistant_task_contract("fill missing tags for this album");
        assert_eq!(
            auto_tag.kind,
            AssistantTaskContractKind::ActionPreviewRequired
        );
        assert_eq!(auto_tag.route, Some(AssistantTaskRoute::AutoTag));
        assert!(auto_tag.requires_completion_evidence);

        let audit = derive_assistant_task_contract("audit the selected tracks");
        assert_eq!(audit.kind, AssistantTaskContractKind::ActionPreviewRequired);
        assert_eq!(audit.route, Some(AssistantTaskRoute::Audit));
        assert!(audit.requires_completion_evidence);
    }

    #[test]
    fn task_contract_distinguishes_read_only_and_chat_requests() {
        let inspect = derive_assistant_task_contract("show tracks missing a title");
        assert_eq!(inspect.kind, AssistantTaskContractKind::ReadOnlyAnswer);
        assert!(!inspect.requires_completion_evidence);

        let chat = derive_assistant_task_contract("hello there");
        assert_eq!(chat.kind, AssistantTaskContractKind::ChatOnly);
        assert!(!chat.requires_completion_evidence);
    }

    #[test]
    fn deterministic_library_task_batch_uses_only_active_scope_paths() {
        let input = AssistantSendInput {
            message: "auto tag this".into(),
            selected_track_paths: vec!["/music/selected.mp3".into()],
            tracks: vec![serde_json::json!({"path": "/music/other.mp3"})],
            ..Default::default()
        };

        let batch =
            deterministic_task_batch("session", &input, AssistantTaskRoute::AutoTag).unwrap();

        assert_eq!(batch.kind, "auto-tag-run");
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(
            batch.actions[0].track_path.as_deref(),
            Some("/music/selected.mp3")
        );
    }

    #[test]
    fn mutating_contract_rejects_model_reply_without_preview_evidence() {
        let contract = derive_assistant_task_contract("change the album title");
        let error = validate_completion_evidence(&contract, false, "I can do that").unwrap_err();

        assert!(error.to_string().contains("requires a preview batch"));
    }

    #[test]
    fn assistant_preview_rejects_paths_outside_selected_scope() {
        let input = AssistantSendInput {
            message: "change title".into(),
            selected_track_paths: vec!["/music/selected.mp3".into()],
            tracks: vec![serde_json::json!({"path": "/music/other.mp3"})],
            ..Default::default()
        };
        let draft = AssistantDraftBatch {
            kind: "tag-update".into(),
            title: "Change title".into(),
            summary: "one change".into(),
            risk_level: "low".into(),
            actions: vec![AssistantAction {
                tag_kind: Some("standard".into()),
                track_path: Some("/music/other.mp3".into()),
                field: Some("title".into()),
                new_value: Some("New".into()),
                ..Default::default()
            }],
        };

        let error = validated_assistant_batch("session", &input, draft).unwrap_err();

        assert!(error.to_string().contains("outside the active scope"));
    }

    #[test]
    fn assistant_preview_accepts_supported_field_for_active_track() {
        let input = AssistantSendInput {
            message: "change title".into(),
            selected_track_paths: vec!["/music/selected.mp3".into()],
            ..Default::default()
        };
        let draft = AssistantDraftBatch {
            kind: "tag-update".into(),
            title: "Change title".into(),
            summary: "one change".into(),
            risk_level: "low".into(),
            actions: vec![AssistantAction {
                tag_kind: Some("standard".into()),
                track_path: Some("/music/selected.mp3".into()),
                field: Some("title".into()),
                new_value: Some("New".into()),
                ..Default::default()
            }],
        };

        let batch = validated_assistant_batch("session", &input, draft).unwrap();

        assert_eq!(batch.kind, "tag-update");
        assert_eq!(batch.actions[0].field.as_deref(), Some("title"));
        assert_eq!(batch.status, "pending");
    }

    #[test]
    fn assistant_preview_rejects_metadata_action_without_field() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/selected.mp3".into()],
            ..Default::default()
        };
        let draft = AssistantDraftBatch {
            kind: "tag-update".into(),
            title: "Broken preview".into(),
            summary: "missing field".into(),
            risk_level: "low".into(),
            actions: vec![AssistantAction {
                track_path: Some("/music/selected.mp3".into()),
                ..Default::default()
            }],
        };

        let error = validated_assistant_batch("session", &input, draft).unwrap_err();

        assert!(error.to_string().contains("missing field"));
    }

    #[tokio::test]
    async fn approved_standard_batch_returns_undo_and_uses_safe_writer() {
        let root = temp_dir();
        let path = root.join("track.mp3");
        fs::copy(media_fixture(), &path).unwrap();
        write_track_dispatch(
            &path,
            &TrackPatch {
                title: Patch::Value("Before".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-1".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "tag-update".into(),
            title: "Update title".into(),
            summary: "one".into(),
            risk_level: "low".into(),
            reversible: true,
            status: "pending".into(),
            actions: vec![AssistantAction {
                tag_kind: None,
                track_path: Some(path.to_string_lossy().into_owned()),
                field: Some("title".into()),
                new_value: Some("After".into()),
                ..Default::default()
            }],
        }));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), "batch-1").await;

        assert_eq!(result["success"], true);
        assert_eq!(result["undoSnapshots"][0]["metadata"]["title"], "Before");
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("After")
        );
        assert_eq!(runtime.get_batch("batch-1").unwrap().status, "applied");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn approved_metadata_batch_applies_standard_and_extra_with_both_undo_shapes() {
        let root = temp_dir();
        let path = root.join("track.mp3");
        fs::copy(media_fixture(), &path).unwrap();
        let queue = WriteQueue::default();
        write_track_dispatch(
            &path,
            &TrackPatch {
                title: Patch::Value("Before".into()),
                ..Default::default()
            },
        )
        .unwrap();
        crate::commands::mutations::write_extra_tags_queued(
            &queue,
            path.clone(),
            vec![crate::commands::mutations::ExtraTagUpdate {
                key: "MOOD".into(),
                value: "Calm".into(),
            }],
        )
        .await
        .unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-mixed".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "metadata-update".into(),
            title: "Mixed".into(),
            summary: "two".into(),
            risk_level: "low".into(),
            reversible: true,
            status: "pending".into(),
            actions: vec![
                AssistantAction {
                    tag_kind: Some("standard".into()),
                    track_path: Some(path.to_string_lossy().into_owned()),
                    field: Some("title".into()),
                    new_value: Some("After".into()),
                    ..Default::default()
                },
                AssistantAction {
                    tag_kind: Some("extra".into()),
                    track_path: Some(path.to_string_lossy().into_owned()),
                    field: Some("MOOD".into()),
                    new_value: Some("Energetic".into()),
                    operation: Some("upsert".into()),
                    ..Default::default()
                },
            ],
        }));

        let result = apply_action_batch(&runtime, &queue, "batch-mixed").await;

        assert_eq!(result["success"], true);
        assert_eq!(result["undoSnapshots"][0]["metadata"]["title"], "Before");
        assert_eq!(
            result["extraUndoSnapshots"][0]["extraTags"][0]["value"],
            "Calm"
        );
        assert_eq!(
            read_track_metadata(&path).unwrap().title.as_deref(),
            Some("After")
        );
        assert!(crate::commands::tracks::read_extra_tags(&path)
            .iter()
            .any(|tag| tag.key == "MOOD" && tag.value == "Energetic"));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn approved_folder_move_creates_parent_and_returns_manifest() {
        let root = temp_dir();
        let source = root.join("source.mp3");
        let destination = root.join("nested").join("destination.mp3");
        fs::copy(media_fixture(), &source).unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-move".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "folder-move".into(),
            title: "Move".into(),
            summary: "one".into(),
            risk_level: "medium".into(),
            reversible: true,
            status: "pending".into(),
            actions: vec![AssistantAction {
                source_path: Some(source.to_string_lossy().into_owned()),
                destination_path: Some(destination.to_string_lossy().into_owned()),
                ..Default::default()
            }],
        }));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), "batch-move").await;

        assert_eq!(result["success"], true);
        assert_eq!(
            result["manifest"][0]["from"],
            source.to_string_lossy().as_ref()
        );
        assert!(destination.exists());
        assert!(!source.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn mixed_batch_reports_only_failed_side_and_retains_partial_undo() {
        let root = temp_dir();
        let good = root.join("good.mp3");
        let unsupported = root.join("bad.aiff");
        fs::copy(media_fixture(), &good).unwrap();
        fs::write(&unsupported, b"FORM").unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-partial".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "metadata-update".into(),
            title: "Partial".into(),
            summary: "two".into(),
            risk_level: "medium".into(),
            reversible: true,
            status: "pending".into(),
            actions: vec![
                AssistantAction {
                    tag_kind: Some("standard".into()),
                    track_path: Some(good.to_string_lossy().into_owned()),
                    field: Some("title".into()),
                    new_value: Some("Updated".into()),
                    ..Default::default()
                },
                AssistantAction {
                    tag_kind: Some("extra".into()),
                    track_path: Some(unsupported.to_string_lossy().into_owned()),
                    field: Some("MOOD".into()),
                    new_value: Some("Calm".into()),
                    ..Default::default()
                },
            ],
        }));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), "batch-partial").await;

        assert_eq!(result["success"], false);
        assert_eq!(result["error"], "Failed to update 1 track(s)");
        assert_eq!(result["results"]["standard"], serde_json::json!([]));
        assert_eq!(result["results"]["extra"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            read_track_metadata(&good).unwrap().title.as_deref(),
            Some("Updated")
        );
        assert_eq!(runtime.get_batch("batch-partial").unwrap().status, "failed");
        fs::remove_dir_all(root).unwrap();
    }

    fn media_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.mp3")
    }

    fn temp_dir() -> PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "auto-tagger-assistant-apply-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
