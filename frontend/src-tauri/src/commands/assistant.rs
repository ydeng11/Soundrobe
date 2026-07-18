//! Assistant runtime and tool-service commands.

use crate::commands::{
    assistant_tools::{
        context_tool_catalog, execute_context_tool, prettify_tag, registered_tool_is_read_only,
        validate_registered_tool_args, AssistantToolResult,
    },
    dataset::dataset_status_at,
    lyrics::{fetch_lyrics_at, DEFAULT_BASE_URL},
    mutations::{
        rename_track_queued, write_extra_tags_queued, write_track_queued, ExtraTagUpdate,
        TrackPatch,
    },
    organizer::sanitize_dir_name,
    tracks::{read_extra_tags, read_track_metadata},
};
use crate::error::ApiError;
use crate::infra::openrouter::{ChatMessage, OpenRouterClient};
use crate::state::assistant::{
    AssistantAction, AssistantActionBatch, AssistantRuntimeState, AssistantServicesConfig,
    AssistantServicesSnapshot, AssistantServicesState,
};
use crate::state::config::ConfigState;
use crate::state::conversation::ConversationState;
use crate::state::providers::convert_chinese_text;
use crate::state::providers::{DiscogsClient, MusicBrainzClient, ProviderState};
use crate::state::write_queue::WriteQueue;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
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

#[derive(Clone, Copy)]
struct NativeAssistantToolServices<'a> {
    input: &'a AssistantSendInput,
    providers: &'a ProviderState,
    config: &'a crate::state::config::AutoTagConfig,
    assistant: &'a AssistantServicesSnapshot,
}

async fn execute_native_assistant_tool(
    name: &str,
    args: &Value,
    services: NativeAssistantToolServices<'_>,
) -> AssistantToolResult {
    if matches!(
        name,
        "library.summarize"
            | "tracks.search"
            | "tracks.inspect"
            | "albums.inspect"
            | "query.metadata"
    ) {
        return execute_context_tool(name, args, services.input);
    }
    match name {
        "query.datasetStatus" => {
            let path = services
                .config
                .dataset_path
                .as_deref()
                .map(PathBuf::from)
                .or_else(|| {
                    dirs::home_dir().map(|home| home.join(".auto-tagger/dataset-index.sqlite"))
                });
            let status = path
                .as_deref()
                .map(dataset_status_at)
                .unwrap_or_else(|| dataset_status_at(Path::new("")));
            AssistantToolResult {
                ok: true,
                summary: if status.available {
                    format!("Dataset available with {} record(s).", status.total_records)
                } else {
                    "Local dataset is unavailable; online providers remain available.".into()
                },
                data: serde_json::to_value(status).ok(),
                error: None,
            }
        }
        "api.musicbrainzSearch" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (artist, album) = parse_musicbrainz_tool_query(query);
            if artist.is_empty() || album.is_empty() {
                return assistant_tool_error(
                    "MusicBrainz query must include artist: and album: fields".into(),
                );
            }
            let limit = tool_limit(args, 5);
            let albums = MusicBrainzClient::new(services.providers.http())
                .search_album(&artist, &album, limit)
                .await;
            AssistantToolResult {
                ok: true,
                summary: format!("MusicBrainz returned {} release(s).", albums.len()),
                data: serde_json::to_value(albums).ok(),
                error: None,
            }
        }
        "api.discogsSearch" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let token = services
                .assistant
                .discogs_token
                .clone()
                .or_else(|| services.config.discogs_token.clone());
            if token.as_deref().is_none_or(str::is_empty) {
                return assistant_tool_error("Discogs token is not configured".into());
            }
            let albums = DiscogsClient::new(services.providers.http(), token)
                .search_album("", query, tool_limit(args, 5))
                .await;
            AssistantToolResult {
                ok: true,
                summary: format!("Discogs returned {} release(s).", albums.len()),
                data: serde_json::to_value(albums).ok(),
                error: None,
            }
        }
        "api.lyricsSearch" => {
            let artist = args
                .get("artist")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let host = services
                .assistant
                .lyrics_host
                .as_deref()
                .or(services.config.lyrics_api_url.as_deref())
                .unwrap_or(DEFAULT_BASE_URL);
            let lyrics = fetch_lyrics_at(host, title, artist, None, None).await;
            AssistantToolResult {
                ok: true,
                summary: lyrics
                    .as_ref()
                    .map(|lyrics| format!("Found lyrics ({} characters).", lyrics.len()))
                    .unwrap_or_else(|| "No lyrics found.".into()),
                data: lyrics.map(|lyrics| serde_json::json!({"lyrics": lyrics})),
                error: None,
            }
        }
        _ => execute_context_tool(name, args, services.input),
    }
}

fn assistant_tool_error(error: String) -> AssistantToolResult {
    AssistantToolResult {
        ok: false,
        summary: error.clone(),
        data: None,
        error: Some(error),
    }
}

fn tool_result_prompt(result: &AssistantToolResult) -> String {
    let Some(data) = &result.data else {
        return format!("Tool result: {}", result.summary);
    };
    let serialized = data.to_string();
    let truncated = serialized.chars().count() > 8_000;
    let evidence = serialized.chars().take(8_000).collect::<String>();
    format!(
        "Tool result: {}\nStructured evidence: {}{}",
        result.summary,
        evidence,
        if truncated { "…[truncated]" } else { "" }
    )
}

fn tool_limit(args: &Value, fallback: usize) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(fallback)
        .clamp(1, 25)
}

fn parse_musicbrainz_tool_query(query: &str) -> (String, String) {
    (
        query_field(query, "artist").unwrap_or_default(),
        query_field(query, "album").unwrap_or_default(),
    )
}

fn query_field(query: &str, field: &str) -> Option<String> {
    let lower = query.to_lowercase();
    let marker = format!("{field}:");
    let start = lower.find(&marker)? + marker.len();
    let tail = query[start..].trim_start();
    if let Some(quoted) = tail.strip_prefix('"') {
        return quoted.split_once('"').map(|(value, _)| value.to_string());
    }
    let lower_tail = tail.to_lowercase();
    let end = [" artist:", " album:"]
        .iter()
        .filter(|candidate| !candidate.trim_start().starts_with(field))
        .filter_map(|candidate| lower_tail.find(candidate))
        .min()
        .unwrap_or(tail.len());
    let value = tail[..end].trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[tauri::command]
pub async fn assistant_send(
    input: AssistantSendInput,
    app: AppHandle,
    runtime: State<'_, AssistantRuntimeState>,
    services: State<'_, AssistantServicesState>,
    providers: State<'_, ProviderState>,
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
        let Some(batch) = deterministic_task_batch(&current.session_id, &input, route)? else {
            let event = AssistantEvent {
                session_id: current.session_id,
                event_type: "message",
                message: route.no_changes_message().into(),
                data: Some(serde_json::json!({
                    "outcome": "no_changes",
                    "routeSource": "deterministic",
                    "contractReason": contract.reason,
                })),
            };
            conversation.record("assistant_message", &event.message, None, 0, 0, 0);
            let _ = app.emit("assistant:event", &event);
            return Ok(event);
        };
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
        .then_some(snapshot.api_key.clone())
        .or(raw_config.llm_api_key.clone())
        .filter(|key| !key.is_empty());
    let model = (!snapshot.model.is_empty())
        .then_some(snapshot.model.clone())
        .or(raw_config.llm_model.clone())
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
    let session_id = current.session_id.clone();
    let context = serde_json::json!({
        "libraryPath": input.library_path,
        "activeAlbumPath": input.active_album_path,
        "selectedTrackPaths": input.selected_track_paths,
        "tracks": input.tracks.iter().take(200).collect::<Vec<_>>(),
        "albums": input.albums.iter().take(100).collect::<Vec<_>>(),
        "autonomous": input.autonomous,
    });
    let tools = context_tool_catalog();
    let mut messages = vec![
        ChatMessage::system(format!(
            concat!(
                "You are the Soundrobe desktop assistant. Answer music-library questions directly. ",
                "For library facts, call one of the supplied read-only tools, then use its result. ",
                "For mutations, call the supplied mutating tool so the app creates a native preview; never claim a write already happened. ",
                "Allowed batch kinds: tag-update, extra-tag-update, metadata-update, folder-move, auto-tag-run, audit-run. ",
                "Every action must use an exact trackPath from the supplied active scope. ",
                "Standard metadata actions use tagKind=standard, field, and newValue (null removes). ",
                "Custom tags use tagKind=extra. Auto-tag/audit actions need only trackPath. ",
                "Keep riskLevel low, medium, or high. Return concise user-facing text in message. ",
                "Available tools: {tools}"
            ),
            tools = tools
        )),
        ChatMessage::user(format!("App context:\n{context}\n\nUser request:\n{}", input.message)),
    ];
    let client = OpenRouterClient::new(&api_key, &model).with_generation(0.2, 1400);
    let mut signatures = Vec::new();
    let mut repaired_invalid_args = false;
    let mut final_draft = None;
    let mut pending_tool_batches = Vec::new();
    let mut tool_completion_evidence = false;
    for step_number in 1..=10 {
        let step = AssistantEvent {
            session_id: session_id.clone(),
            event_type: "step",
            message: format!("Step {step_number}/10"),
            data: None,
        };
        let _ = app.emit("assistant:event", step);
        let response = client
            .complete_json(
                messages.clone(),
                "AssistantResponse",
                assistant_response_schema(),
                &cancelled,
            )
            .await;
        if cancelled.load(Ordering::Acquire) {
            let event = AssistantEvent {
                session_id,
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
                return assistant_error_event(&app, Some(session_id), &error.to_string());
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
                    Some(session_id),
                    &format!("Invalid assistant response: {error}"),
                );
            }
        };
        if draft.tool_call.is_some() && draft.action_batch.is_some() {
            return assistant_error_event(
                &app,
                Some(session_id),
                "Invalid assistant response: toolCall and actionBatch are mutually exclusive",
            );
        }
        let Some(tool_call) = draft.tool_call else {
            final_draft = Some(draft);
            break;
        };
        if would_repeat_tool_call(&signatures, &tool_call.tool_name, &tool_call.args) {
            return assistant_error_event(
                &app,
                Some(session_id),
                &format!(
                    "The assistant repeated \"{}\" with the same arguments 3 times, so I stopped instead of claiming the task was complete.",
                    tool_call.tool_name
                ),
            );
        }
        signatures.push(tool_call_signature(&tool_call.tool_name, &tool_call.args));
        let running = AssistantEvent {
            session_id: session_id.clone(),
            event_type: "tool_running",
            message: format!("Running tool: {}", tool_call.tool_name),
            data: Some(serde_json::json!({
                "toolName": tool_call.tool_name,
                "toolArgs": tool_call.args
            })),
        };
        let _ = app.emit("assistant:event", &running);
        conversation.record(
            "tool_call",
            &serde_json::json!({
                "toolName": tool_call.tool_name,
                "toolArgs": tool_call.args
            })
            .to_string(),
            Some(&model),
            0,
            0,
            0,
        );
        let native_services = NativeAssistantToolServices {
            input: &input,
            providers: &providers,
            config: &raw_config,
            assistant: &snapshot,
        };
        let execution = if tool_call.tool_name == "create_plan" {
            execute_create_plan(&tool_call.args, &input, &session_id, native_services).await
        } else if registered_tool_is_read_only(&tool_call.tool_name) == Some(false) {
            execute_mutating_assistant_tool(
                &tool_call.tool_name,
                &tool_call.args,
                &input,
                &session_id,
            )
        } else {
            MutatingToolExecution {
                result: execute_native_assistant_tool(
                    &tool_call.tool_name,
                    &tool_call.args,
                    native_services,
                )
                .await,
                batches: Vec::new(),
                completion_evidence: false,
            }
        };
        tool_completion_evidence |= execution.completion_evidence;
        let result = execution.result;
        let created_batches = execution.batches;
        for batch in &created_batches {
            if !runtime.add_batch(batch.clone()) {
                return Err(ApiError::Message(
                    "Failed to store assistant action preview".into(),
                ));
            }
            pending_tool_batches.push(batch.clone());
        }
        conversation.record("tool_result", &result.summary, None, 0, 0, 0);
        let tool_result = AssistantEvent {
            session_id: session_id.clone(),
            event_type: "tool_result",
            message: result.summary.clone(),
            data: Some(serde_json::json!({
                "ok": result.ok,
                "summary": result.summary,
                "data": result.data,
                "error": result.error
            })),
        };
        let _ = app.emit("assistant:event", &tool_result);
        if !result.ok {
            let validation_error = result
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("Invalid arguments"));
            if validation_error && !repaired_invalid_args {
                repaired_invalid_args = true;
                messages.push(ChatMessage::system(format!(
                    "Tool argument validation failed for \"{}\": {}. Retry once using only fields allowed by that tool schema.",
                    tool_call.tool_name,
                    result.error.as_deref().unwrap_or_default()
                )));
                continue;
            }
            return assistant_error_event(&app, Some(session_id), &result.summary);
        }
        if !input.autonomous && !created_batches.is_empty() {
            let event = AssistantEvent {
                session_id,
                event_type: "action_batch_created",
                message: result.summary,
                data: Some(serde_json::json!({
                    "actionBatchId": created_batches[0].id,
                    "actionBatch": created_batches[0],
                    "actionBatches": created_batches
                })),
            };
            conversation.record("assistant_message", &event.message, Some(&model), 0, 0, 0);
            let _ = app.emit("assistant:event", &event);
            return Ok(event);
        }
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: serde_json::json!({
                "toolCall": {"toolName": tool_call.tool_name, "args": tool_call.args}
            })
            .to_string(),
        });
        messages.push(ChatMessage::user(tool_result_prompt(&result)));
    }
    let Some(draft) = final_draft else {
        return assistant_error_event(
            &app,
            Some(session_id),
            "I reached the maximum step limit (10) without a final response.",
        );
    };
    if draft.action_batch.is_some() && !pending_tool_batches.is_empty() {
        return assistant_error_event(
            &app,
            Some(session_id),
            "The assistant returned both a native tool preview and a model-authored preview",
        );
    }
    if let Err(error) = validate_completion_evidence(
        &contract,
        draft.action_batch.is_some()
            || !pending_tool_batches.is_empty()
            || tool_completion_evidence,
        &draft.message,
    ) {
        return assistant_error_event(&app, Some(session_id), &error.to_string());
    }
    let event = if let Some(batch) = pending_tool_batches.first().cloned() {
        AssistantEvent {
            session_id,
            event_type: "action_batch_created",
            message: draft.message,
            data: Some(serde_json::json!({
                "actionBatchId": batch.id,
                "actionBatch": batch,
                "actionBatches": pending_tool_batches
            })),
        }
    } else if let Some(batch) = draft.action_batch {
        let batch = match validated_assistant_batch(&session_id, &input, batch) {
            Ok(batch) => batch,
            Err(error) => {
                return assistant_error_event(&app, Some(session_id), &error.to_string());
            }
        };
        if !runtime.add_batch(batch.clone()) {
            return Err(ApiError::Message(
                "Failed to store assistant action preview".into(),
            ));
        }
        AssistantEvent {
            session_id,
            event_type: "action_batch_created",
            message: draft.message,
            data: Some(serde_json::json!({
                "actionBatchId": batch.id,
                "actionBatch": batch
            })),
        }
    } else {
        AssistantEvent {
            session_id,
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
    let tool_names = context_tool_catalog()
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.get("name").cloned())
        .collect::<Vec<_>>();
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
            },
            "toolCall": {
                "type": ["object", "null"],
                "properties": {
                    "toolName": {
                        "type": "string",
                        "enum": tool_names
                    },
                    "args": {"type": "object"}
                },
                "required": ["toolName", "args"]
            }
        },
        "required": ["message", "actionBatch", "toolCall"]
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
    AutoNumberTracks,
    StripTrackTitlePrefixes,
    StripFilenamePrefixes,
    InferTagsFromFilenames,
    ChineseToTraditional,
    ChineseToSimplified,
    GroupByAlbum,
}

impl AssistantTaskRoute {
    fn no_changes_message(self) -> &'static str {
        match self {
            Self::AutoNumberTracks => "Track numbering is already correct. No changes are needed.",
            Self::StripTrackTitlePrefixes => {
                "No track-title number prefixes were found. No changes are needed."
            }
            Self::StripFilenamePrefixes => {
                "No filename number prefixes were found. No changes are needed."
            }
            Self::InferTagsFromFilenames => {
                "No filenames had a clear artist-title shape. No changes are needed."
            }
            Self::ChineseToTraditional | Self::ChineseToSimplified => {
                "The selected metadata is already in the requested Chinese script. No changes are needed."
            }
            Self::GroupByAlbum => "No files need to move into album folders.",
            Self::AutoTag | Self::Audit => "No changes are needed.",
        }
    }
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
    if (text.contains("strip") || text.contains("remove"))
        && text.contains("filename")
        && (text.contains("prefix") || text.contains("number"))
    {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ActionPreviewRequired,
            route: Some(AssistantTaskRoute::StripFilenamePrefixes),
            reason: "strip_filename_prefixes_intent",
            requires_completion_evidence: true,
        };
    }
    if text.contains("filename")
        && (text.contains("infer") || text.contains("parse") || text.contains("derive"))
        && (text.contains("tag") || text.contains("title") || text.contains("artist"))
    {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ActionPreviewRequired,
            route: Some(AssistantTaskRoute::InferTagsFromFilenames),
            reason: "infer_tags_from_filenames_intent",
            requires_completion_evidence: true,
        };
    }
    let chinese_conversion = text.contains("chinese")
        || text.contains("中文")
        || text.contains("traditional")
        || text.contains("simplified")
        || text.contains("繁體")
        || text.contains("繁体")
        || text.contains("简体")
        || text.contains("簡體");
    if chinese_conversion
        && (text.contains("convert") || text.contains("转换") || text.contains("轉換"))
    {
        let route =
            if text.contains("traditional") || text.contains("繁體") || text.contains("繁体") {
                Some(AssistantTaskRoute::ChineseToTraditional)
            } else if text.contains("simplified") || text.contains("简体") || text.contains("簡體")
            {
                Some(AssistantTaskRoute::ChineseToSimplified)
            } else {
                None
            };
        return AssistantTaskContract {
            kind: route.map_or(AssistantTaskContractKind::ClarificationRequired, |_| {
                AssistantTaskContractKind::ActionPreviewRequired
            }),
            route,
            reason: "chinese_convert_intent",
            requires_completion_evidence: route.is_some(),
        };
    }
    if text.contains("album")
        && (text.contains("group") || text.contains("organize") || text.contains("organise"))
        && (text.contains("folder") || text.contains("file"))
    {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ActionPreviewRequired,
            route: Some(AssistantTaskRoute::GroupByAlbum),
            reason: "group_by_album_intent",
            requires_completion_evidence: true,
        };
    }
    if (text.contains("strip") || text.contains("remove"))
        && text.contains("title")
        && (text.contains("prefix") || text.contains("number"))
    {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ActionPreviewRequired,
            route: Some(AssistantTaskRoute::StripTrackTitlePrefixes),
            reason: "strip_track_title_prefixes_intent",
            requires_completion_evidence: true,
        };
    }
    if (text.contains("track") && text.contains("number")) || text.contains("renumber") {
        return AssistantTaskContract {
            kind: AssistantTaskContractKind::ActionPreviewRequired,
            route: Some(AssistantTaskRoute::AutoNumberTracks),
            reason: "auto_number_tracks_intent",
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
) -> Result<Option<AssistantActionBatch>, ApiError> {
    let paths = active_scope_paths(input);
    if paths.is_empty() {
        return Err(ApiError::Message(
            "No tracks are available in the current assistant scope".into(),
        ));
    }
    let (kind, title, summary, risk_level, actions) = match route {
        AssistantTaskRoute::AutoTag => (
            "auto-tag-run",
            "Run auto-tag",
            format!("Preview auto-tag for {} track(s)", paths.len()),
            "low",
            paths
                .into_iter()
                .map(|track_path| AssistantAction {
                    track_path: Some(track_path),
                    ..Default::default()
                })
                .collect(),
        ),
        AssistantTaskRoute::Audit => (
            "audit-run",
            "Run metadata audit",
            format!("Preview metadata audit for {} track(s)", paths.len()),
            "low",
            paths
                .into_iter()
                .map(|track_path| AssistantAction {
                    track_path: Some(track_path),
                    ..Default::default()
                })
                .collect(),
        ),
        AssistantTaskRoute::AutoNumberTracks => {
            let actions = plan_track_numbering(input, &paths);
            if actions.is_empty() {
                return Ok(None);
            }
            (
                "metadata-update",
                "Number tracks",
                format!("Preview {} track-numbering update(s)", actions.len()),
                "low",
                actions,
            )
        }
        AssistantTaskRoute::StripTrackTitlePrefixes => {
            let actions = plan_strip_track_title_prefixes(input, &paths);
            if actions.is_empty() {
                return Ok(None);
            }
            (
                "metadata-update",
                "Strip track-title prefixes",
                format!("Preview {} title update(s)", actions.len()),
                "low",
                actions,
            )
        }
        AssistantTaskRoute::StripFilenamePrefixes => {
            let actions = plan_strip_filename_prefixes(&paths);
            if actions.is_empty() {
                return Ok(None);
            }
            (
                "folder-move",
                "Strip filename prefixes",
                format!("Preview {} file rename(s)", actions.len()),
                "medium",
                actions,
            )
        }
        AssistantTaskRoute::InferTagsFromFilenames => {
            let actions = plan_infer_tags_from_filenames(
                input,
                &paths,
                input.message.to_lowercase().contains("prett"),
            );
            if actions.is_empty() {
                return Ok(None);
            }
            (
                "metadata-update",
                "Infer tags from filenames",
                format!("Preview {} inferred tag update(s)", actions.len()),
                "low",
                actions,
            )
        }
        AssistantTaskRoute::ChineseToTraditional | AssistantTaskRoute::ChineseToSimplified => {
            let target = if route == AssistantTaskRoute::ChineseToTraditional {
                "traditional"
            } else {
                "simplified"
            };
            let actions = plan_chinese_conversion(input, &paths, target);
            if actions.is_empty() {
                return Ok(None);
            }
            (
                "metadata-update",
                "Convert Chinese metadata",
                format!("Preview {} Chinese-script update(s)", actions.len()),
                "low",
                actions,
            )
        }
        AssistantTaskRoute::GroupByAlbum => {
            let actions = plan_group_by_album(input, &paths)?;
            if actions.is_empty() {
                return Ok(None);
            }
            (
                "folder-move",
                "Group files by album",
                format!("Preview {} album-folder move(s)", actions.len()),
                "medium",
                actions,
            )
        }
    };
    Ok(Some(AssistantActionBatch {
        id: format!("batch-{}", uuid::Uuid::new_v4()),
        created_at: time::OffsetDateTime::now_utc().to_string(),
        session_id: session_id.to_string(),
        kind: kind.into(),
        title: title.into(),
        summary,
        risk_level: risk_level.into(),
        actions,
        reversible: true,
        status: "pending".into(),
    }))
}

struct MutatingToolExecution {
    result: AssistantToolResult,
    batches: Vec<AssistantActionBatch>,
    completion_evidence: bool,
}

fn mutating_tool_execution(
    summary: String,
    data: Option<Value>,
    batch: Option<AssistantActionBatch>,
) -> MutatingToolExecution {
    let completion_evidence = batch.is_some();
    let batches = batch.clone().into_iter().collect::<Vec<_>>();
    MutatingToolExecution {
        result: AssistantToolResult {
            ok: true,
            summary,
            data: Some(serde_json::json!({"data": data, "batch": batch})),
            error: None,
        },
        batches,
        completion_evidence,
    }
}

fn mutating_tool_no_changes(summary: impl Into<String>) -> MutatingToolExecution {
    let summary = summary.into();
    MutatingToolExecution {
        result: AssistantToolResult {
            ok: true,
            summary,
            data: Some(serde_json::json!({"outcome": "no_changes"})),
            error: None,
        },
        batches: Vec::new(),
        completion_evidence: true,
    }
}

fn mutating_tool_error(message: impl Into<String>) -> MutatingToolExecution {
    let message = message.into();
    MutatingToolExecution {
        result: AssistantToolResult {
            ok: false,
            summary: message.clone(),
            data: None,
            error: Some(message),
        },
        batches: Vec::new(),
        completion_evidence: false,
    }
}

fn assistant_batch(
    session_id: &str,
    kind: &str,
    title: impl Into<String>,
    summary: impl Into<String>,
    risk_level: &str,
    actions: Vec<AssistantAction>,
    reversible: bool,
) -> AssistantActionBatch {
    AssistantActionBatch {
        id: format!("batch-{}", uuid::Uuid::new_v4()),
        created_at: time::OffsetDateTime::now_utc().to_string(),
        session_id: session_id.into(),
        kind: kind.into(),
        title: title.into(),
        summary: summary.into(),
        risk_level: risk_level.into(),
        actions,
        reversible,
        status: "pending".into(),
    }
}

fn tool_scope_paths(input: &AssistantSendInput, args: &Value) -> Result<Vec<String>, String> {
    let scope = args
        .get("target_scope")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing required field: target_scope".to_string())?;
    let loaded_paths = input
        .tracks
        .iter()
        .filter_map(|track| track.get("path").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let loaded = loaded_paths.iter().copied().collect::<HashSet<_>>();
    let paths = match scope {
        "selected" => input.selected_track_paths.clone(),
        "active_album" => input
            .tracks
            .iter()
            .filter_map(|track| track.get("path").and_then(Value::as_str))
            .filter(|path| {
                input
                    .active_album_path
                    .as_deref()
                    .is_some_and(|album| path_is_inside(Path::new(path), Path::new(album)))
            })
            .map(str::to_string)
            .collect(),
        "library" => loaded_paths
            .iter()
            .map(|path| (*path).to_string())
            .collect(),
        "explicit_paths" => args
            .get("paths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|path| loaded.contains(path))
            .map(str::to_string)
            .collect(),
        _ => return Err(format!("Unsupported target_scope: {scope}")),
    };
    Ok(paths)
}

fn execute_mutating_assistant_tool(
    name: &str,
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    if let Err(error) = validate_registered_tool_args(name, args) {
        return mutating_tool_error(format!("Invalid arguments for {name}: {error}"));
    }
    match name {
        "edit_metadata" => execute_edit_metadata(args, input, session_id),
        "extract_tag_value" => execute_extract_tag_value(args, input, session_id),
        "organize_files" => execute_organize_files(args, input, session_id),
        "run_library_task" => execute_run_library_task(args, input, session_id),
        "auto_numbering_tracks"
        | "strip_track_title_prefixes"
        | "chinese_convert"
        | "strip_filename_prefixes"
        | "infer_tags_from_filenames"
        | "group_by_album" => execute_existing_assistant_macro(name, args, input, session_id),
        _ => mutating_tool_error(format!("Mutating tool {name} is not implemented")),
    }
}

async fn execute_create_plan(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
    services: NativeAssistantToolServices<'_>,
) -> MutatingToolExecution {
    if let Err(error) = validate_registered_tool_args("create_plan", args) {
        return mutating_tool_error(format!("Invalid arguments for create_plan: {error}"));
    }
    let Some(steps) = args.get("steps").and_then(Value::as_array) else {
        return mutating_tool_error("Plan steps must be an array");
    };
    let order = match plan_dependency_order(steps) {
        Ok(order) => order,
        Err(error) => return mutating_tool_error(error),
    };
    let step_by_id = steps
        .iter()
        .filter_map(|step| Some((step.get("id")?.as_str()?.to_string(), step)))
        .collect::<BTreeMap<_, _>>();
    let mut scratchpad = BTreeMap::<String, Value>::new();
    let mut outputs = Vec::new();
    let mut batches = Vec::new();
    let mut completion_evidence = false;
    for step_id in order {
        let Some(step) = step_by_id.get(&step_id).copied() else {
            return mutating_tool_error(format!("Plan step not found: {step_id}"));
        };
        let tool = step.get("tool").and_then(Value::as_str).unwrap_or_default();
        if tool == "create_plan" {
            return mutating_tool_error("Nested create_plan calls are not supported");
        }
        let resolved_args = resolve_plan_args(
            step.get("args")
                .unwrap_or(&Value::Object(Default::default())),
            &scratchpad,
        );
        let execution = if registered_tool_is_read_only(tool) == Some(true) {
            MutatingToolExecution {
                result: execute_native_assistant_tool(tool, &resolved_args, services).await,
                batches: Vec::new(),
                completion_evidence: false,
            }
        } else if registered_tool_is_read_only(tool) == Some(false) {
            execute_mutating_assistant_tool(tool, &resolved_args, input, session_id)
        } else {
            return mutating_tool_error(format!("Unknown plan tool: {tool}"));
        };
        if !execution.result.ok {
            return mutating_tool_error(format!(
                "Plan step {step_id} failed: {}",
                execution.result.summary
            ));
        }
        completion_evidence |= execution.completion_evidence;
        let scratch = execution
            .result
            .data
            .clone()
            .unwrap_or_else(|| Value::String(execution.result.summary.clone()));
        scratchpad.insert(step_id.clone(), scratch.clone());
        outputs.push(serde_json::json!({
            "stepId": step_id,
            "label": step.get("label").and_then(Value::as_str).unwrap_or(&step_id),
            "ok": true,
            "summary": execution.result.summary,
            "data": scratch
        }));
        batches.extend(execution.batches);
    }
    let summary = format!(
        "Plan executed ({} steps, {} pending batch(es)).",
        outputs.len(),
        batches.len()
    );
    MutatingToolExecution {
        result: AssistantToolResult {
            ok: true,
            summary,
            data: Some(serde_json::json!({"stepOutputs": outputs, "batchCount": batches.len()})),
            error: None,
        },
        batches,
        completion_evidence,
    }
}

fn plan_dependency_order(steps: &[Value]) -> Result<Vec<String>, String> {
    let mut step_by_id = BTreeMap::<String, &Value>::new();
    for step in steps {
        let id = step
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Every plan step requires a string id".to_string())?;
        if step_by_id.insert(id.into(), step).is_some() {
            return Err(format!("Duplicate plan step id: {id}"));
        }
    }
    fn visit(
        id: &str,
        steps: &BTreeMap<String, &Value>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.into()) {
            return Err(format!("Circular dependency detected: {id}"));
        }
        let step = steps
            .get(id)
            .ok_or_else(|| format!("Plan step not found: {id}"))?;
        for dependency in step
            .get("depends_on")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !steps.contains_key(dependency) {
                return Err(format!(
                    "Step \"{id}\" depends on unknown step \"{dependency}\""
                ));
            }
            visit(dependency, steps, visiting, visited, order)?;
        }
        visiting.remove(id);
        visited.insert(id.into());
        order.push(id.into());
        Ok(())
    }
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut order = Vec::new();
    for id in steps
        .iter()
        .filter_map(|step| step.get("id").and_then(Value::as_str))
    {
        visit(id, &step_by_id, &mut visiting, &mut visited, &mut order)?;
    }
    Ok(order)
}

fn resolve_plan_args(args: &Value, scratchpad: &BTreeMap<String, Value>) -> Value {
    match args {
        Value::String(value) if value.starts_with('$') => {
            let reference = &value[1..];
            let (step, field) = reference
                .split_once('.')
                .map_or((reference, None), |(step, field)| (step, Some(field)));
            let Some(value) = scratchpad.get(step) else {
                return Value::Null;
            };
            field
                .and_then(|field| value.get(field))
                .cloned()
                .unwrap_or_else(|| value.clone())
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| resolve_plan_args(value, scratchpad))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), resolve_plan_args(value, scratchpad)))
                .collect(),
        ),
        _ => args.clone(),
    }
}

fn execute_existing_assistant_macro(
    name: &str,
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve macro target scope");
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }
    let (kind, title, risk, mut actions) = match name {
        "auto_numbering_tracks" => (
            "metadata-update",
            "Auto-number tracks",
            "low",
            plan_track_numbering(input, &paths),
        ),
        "strip_track_title_prefixes" => (
            "metadata-update",
            "Strip track-title prefixes",
            "low",
            plan_strip_track_title_prefixes(input, &paths),
        ),
        "strip_filename_prefixes" => (
            "folder-move",
            "Strip filename prefixes",
            "medium",
            plan_strip_filename_prefixes(&paths),
        ),
        "infer_tags_from_filenames" => {
            let fields = args.get("fields").and_then(Value::as_array).map(|fields| {
                fields
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<HashSet<_>>()
            });
            let mut actions = plan_infer_tags_from_filenames(
                input,
                &paths,
                args.get("prettify").and_then(Value::as_bool) == Some(true),
            );
            if let Some(fields) = fields {
                actions.retain(|action| {
                    action
                        .field
                        .as_deref()
                        .is_some_and(|field| fields.contains(field))
                });
            }
            (
                "metadata-update",
                "Infer tags from filenames",
                "low",
                actions,
            )
        }
        "chinese_convert" => {
            let target = if args.get("direction").and_then(Value::as_str) == Some("s2t") {
                "traditional"
            } else {
                "simplified"
            };
            let fields = args.get("fields").and_then(Value::as_array).map(|fields| {
                fields
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<HashSet<_>>()
            });
            let mut actions = plan_chinese_conversion(input, &paths, target);
            if let Some(fields) = fields {
                actions.retain(|action| {
                    action
                        .field
                        .as_deref()
                        .is_some_and(|field| fields.contains(field))
                });
            }
            (
                "metadata-update",
                "Convert Chinese metadata",
                "low",
                actions,
            )
        }
        "group_by_album" => {
            let actions = match plan_group_by_album(input, &paths) {
                Ok(actions) => actions,
                Err(error) => return mutating_tool_error(error.to_string()),
            };
            ("folder-move", "Group files by album", "medium", actions)
        }
        _ => return mutating_tool_error(format!("Unknown macro: {name}")),
    };
    if actions.is_empty() {
        return mutating_tool_no_changes("No changes are needed.");
    }
    let summary = format!("Preview {} action(s) from {name}", actions.len());
    let batch = assistant_batch(
        session_id,
        kind,
        title,
        &summary,
        risk,
        std::mem::take(&mut actions),
        true,
    );
    mutating_tool_execution(
        format!("Preview created ({}): {summary}", batch.id),
        None,
        Some(batch),
    )
}

fn execute_edit_metadata(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve metadata target scope");
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }
    let updates = args
        .get("standard_updates")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let removes = args
        .get("standard_removes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let unique_fields = [
        "title",
        "artist",
        "artists",
        "trackNumber",
        "trackTotal",
        "discNumber",
        "discTotal",
    ];
    if updates.iter().any(|(field, value)| {
        unique_fields.contains(&field.as_str())
            && match value {
                Value::String(value) => value.trim().is_empty(),
                Value::Array(values) => values.is_empty(),
                _ => false,
            }
    }) {
        return mutating_tool_execution(
            "Blank title, artist, and track/disc values are not valid metadata fixes.".into(),
            None,
            None,
        );
    }
    if paths.len() > 1
        && updates
            .keys()
            .any(|field| unique_fields.contains(&field.as_str()))
    {
        return mutating_tool_execution(
            "Per-track title, artist, and numbering values cannot be applied identically to multiple tracks. Use filename inference or auto-numbering instead.".into(),
            None,
            None,
        );
    }
    let tracks = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect::<BTreeMap<_, _>>();
    let mut actions = Vec::new();
    for path in &paths {
        let track = tracks.get(path.as_str()).copied();
        let current_extras = read_extra_tags(Path::new(path));
        for (field, value) in &updates {
            let Some(desired) = action_value_string(value) else {
                continue;
            };
            push_string_action(&mut actions, track, path, field, &desired);
        }
        for field in &removes {
            let old_value = track.and_then(|track| track_field_string(track, field));
            if old_value.is_some() {
                actions.push(AssistantAction {
                    tag_kind: Some("standard".into()),
                    track_path: Some(path.clone()),
                    field: Some((*field).into()),
                    old_value,
                    new_value: None,
                    operation: Some("remove".into()),
                    description: Some(format!("Remove {field}")),
                    ..Default::default()
                });
            }
        }
        for upsert in args
            .get("extra_upserts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(key), Some(value)) = (
                upsert.get("key").and_then(Value::as_str),
                upsert.get("value").and_then(Value::as_str),
            ) else {
                continue;
            };
            let matching = current_extras
                .iter()
                .filter(|tag| tag.key.trim().eq_ignore_ascii_case(key.trim()))
                .collect::<Vec<_>>();
            if matching.len() != 1 || matching[0].value != value {
                actions.push(extra_action(path, key, Some(value), "upsert"));
            }
        }
        for key in args
            .get("extra_removes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if current_extras
                .iter()
                .any(|tag| tag.key.trim().eq_ignore_ascii_case(key.trim()))
            {
                actions.push(extra_action(path, key, None, "remove"));
            }
        }
    }
    if actions.is_empty() {
        return mutating_tool_no_changes("No metadata changes are needed.");
    }
    let summary = format!(
        "Update {} metadata field(s) across {} track(s)",
        actions.len(),
        paths.len()
    );
    let batch = assistant_batch(
        session_id,
        "metadata-update",
        "Edit metadata",
        &summary,
        "low",
        actions,
        true,
    );
    mutating_tool_execution(
        format!("Preview created ({}): {summary}", batch.id),
        None,
        Some(batch),
    )
}

fn action_value_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Array(values) => Some(
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("; "),
        ),
        Value::Null | Value::Object(_) => None,
    }
}

fn track_field_string(track: &Value, field: &str) -> Option<String> {
    track.get(field).and_then(action_value_string)
}

fn extra_action(path: &str, key: &str, value: Option<&str>, operation: &str) -> AssistantAction {
    AssistantAction {
        tag_kind: Some("extra".into()),
        track_path: Some(path.into()),
        field: Some(key.into()),
        new_value: value.map(str::to_string),
        operation: Some(operation.into()),
        description: Some(format!("{operation} extra tag {key}")),
        ..Default::default()
    }
}

fn execute_extract_tag_value(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve regex target scope");
    };
    let field = args
        .get("field")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let group_index = args
        .get("group_index")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .unwrap_or(1);
    let regex = match regex::Regex::new(pattern) {
        Ok(regex) => regex,
        Err(error) => return mutating_tool_error(format!("Invalid regex pattern: {error}")),
    };
    let tracks = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect::<BTreeMap<_, _>>();
    let mut actions = Vec::new();
    for path in &paths {
        let Some(track) = tracks.get(path.as_str()).copied() else {
            continue;
        };
        let Some(current) = track_field_string(track, field) else {
            continue;
        };
        let Some(captures) = regex.captures(&current) else {
            continue;
        };
        let Some(extracted) = captures.get(group_index).map(|capture| capture.as_str()) else {
            continue;
        };
        if extracted != current {
            push_string_action(&mut actions, Some(track), path, field, extracted);
        }
    }
    if actions.is_empty() {
        return mutating_tool_no_changes(format!(
            "No {field} values matched the pattern; no changes are needed."
        ));
    }
    let summary = format!("Extract {field} for {} track(s)", actions.len());
    let batch = assistant_batch(
        session_id,
        "metadata-update",
        format!("Extract tag value ({field})"),
        &summary,
        "low",
        actions,
        true,
    );
    mutating_tool_execution(
        format!("Preview created ({}): {summary}", batch.id),
        None,
        Some(batch),
    )
}

fn execute_run_library_task(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve library-task target scope");
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }
    let task = args.get("task").and_then(Value::as_str).unwrap_or_default();
    let auto_tag = task == "auto_tag";
    let title = if auto_tag {
        "Auto-tag tracks"
    } else {
        "Audit tracks"
    };
    let summary = format!(
        "{} {} track(s)",
        if auto_tag { "Auto-tag" } else { "Audit" },
        paths.len()
    );
    let actions = paths
        .iter()
        .map(|path| AssistantAction {
            track_path: Some(path.clone()),
            description: Some(format!(
                "{title}: {}",
                Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
            )),
            ..Default::default()
        })
        .collect();
    let batch = assistant_batch(
        session_id,
        if auto_tag {
            "auto-tag-run"
        } else {
            "audit-run"
        },
        title,
        &summary,
        "medium",
        actions,
        auto_tag,
    );
    mutating_tool_execution(
        format!("Preview created ({}): {summary}", batch.id),
        None,
        Some(batch),
    )
}

fn execute_organize_files(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Some(library) = input.library_path.as_deref().map(Path::new) else {
        return mutating_tool_error("Library path is required to organize files");
    };
    let source = Path::new(
        args.get("source_dir")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    if !path_is_inside(source, library) {
        return mutating_tool_error("Source directory is outside the library root");
    }
    if !source.is_dir() {
        return mutating_tool_error("Source directory does not exist or is not a directory");
    }
    let criterion = args
        .get("criterion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let pattern = args
        .get("pattern_string")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if criterion == "pattern" && pattern.trim().is_empty() {
        return mutating_tool_error("pattern_string is required when criterion is pattern");
    }
    let pattern_regex = if criterion == "pattern" {
        match glob_regex(pattern) {
            Ok(regex) => Some(regex),
            Err(error) => return mutating_tool_error(error),
        }
    } else {
        None
    };
    let extension_filters = if criterion == "extension" && !pattern.trim().is_empty() {
        Some(
            pattern
                .split(|character: char| character == ',' || character.is_whitespace())
                .map(|value| value.trim().trim_start_matches('.').to_lowercase())
                .filter(|value| !value.is_empty())
                .collect::<HashSet<_>>(),
        )
    } else {
        None
    };
    let target_root = source.join(sanitize_dir_name(
        args.get("target_dir_name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    ));
    let entries = match fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) => {
            return mutating_tool_error(format!("Failed to scan source directory: {error}"))
        }
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    let mut reserved = HashSet::new();
    let mut actions = Vec::new();
    let mut skipped = 0usize;
    for entry in entries {
        let filename = entry.file_name();
        let Some(filename_text) = filename.to_str() else {
            skipped += 1;
            continue;
        };
        let path = entry.path();
        if filename_text.starts_with('.') || !path.is_file() {
            skipped += 1;
            continue;
        }
        let destination_dir = match organize_destination(
            criterion,
            &path,
            &target_root,
            pattern_regex.as_ref(),
            extension_filters.as_ref(),
        ) {
            Ok(Some(destination)) => destination,
            Ok(None) => {
                skipped += 1;
                continue;
            }
            Err(error) => return mutating_tool_error(error),
        };
        let destination =
            unique_planned_destination(&path, destination_dir.join(&filename), &mut reserved);
        actions.push(AssistantAction {
            source_path: Some(path.to_string_lossy().into_owned()),
            destination_path: Some(destination.to_string_lossy().into_owned()),
            description: Some(format!("Organize by {criterion}")),
            ..Default::default()
        });
    }
    if actions.is_empty() {
        return mutating_tool_no_changes(format!(
            "No files matched the {criterion} criterion; {skipped} skipped."
        ));
    }
    let summary = format!(
        "Move {} file(s) by {criterion}; {skipped} skipped",
        actions.len()
    );
    let batch = assistant_batch(
        session_id,
        "folder-move",
        format!("Organize files by {criterion}"),
        &summary,
        "medium",
        actions,
        true,
    );
    mutating_tool_execution(
        format!("Preview created ({}): {summary}", batch.id),
        None,
        Some(batch),
    )
}

fn glob_regex(pattern: &str) -> Result<regex::Regex, String> {
    let escaped = regex::escape(pattern.trim())
        .replace(r"\*", ".*")
        .replace(r"\?", ".");
    regex::RegexBuilder::new(&format!("^{escaped}$"))
        .case_insensitive(true)
        .build()
        .map_err(|error| format!("Invalid filename pattern: {error}"))
}

fn organize_destination(
    criterion: &str,
    path: &Path,
    target_root: &Path,
    pattern: Option<&regex::Regex>,
    extension_filters: Option<&HashSet<String>>,
) -> Result<Option<PathBuf>, String> {
    match criterion {
        "extension" => {
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("no-extension")
                .to_lowercase();
            if extension_filters.is_some_and(|filters| !filters.contains(&extension)) {
                return Ok(None);
            }
            Ok(Some(target_root.join(extension)))
        }
        "pattern" => {
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            Ok(pattern
                .filter(|pattern| pattern.is_match(filename))
                .map(|_| target_root.to_path_buf()))
        }
        "date_created" => {
            let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
            let timestamp = metadata
                .created()
                .or_else(|_| metadata.modified())
                .map_err(|error| error.to_string())?;
            let datetime = time::OffsetDateTime::from(timestamp);
            Ok(Some(target_root.join(format!(
                "{:04}-{:02}",
                datetime.year(),
                u8::from(datetime.month())
            ))))
        }
        "size" => {
            let size = fs::metadata(path).map_err(|error| error.to_string())?.len();
            let mib = 1024 * 1024;
            let bucket = if size < 10 * mib {
                "small"
            } else if size < 100 * mib {
                "medium"
            } else if size < 1024 * mib {
                "large"
            } else {
                "huge"
            };
            Ok(Some(target_root.join(bucket)))
        }
        _ => Err(format!("Unsupported organize criterion: {criterion}")),
    }
}

fn plan_track_numbering(
    input: &AssistantSendInput,
    scoped_paths: &[String],
) -> Vec<AssistantAction> {
    let scoped = scoped_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut albums: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for track in &input.tracks {
        let Some(path) = track.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !scoped.contains(path) {
            continue;
        }
        albums
            .entry(numbering_album_key(track, path))
            .or_default()
            .push(track);
    }

    let mut actions = Vec::new();
    for tracks in albums.into_values() {
        let disc_total = tracks
            .iter()
            .filter_map(|track| numeric_field(track, "discNumber"))
            .max();
        let mut discs: BTreeMap<Option<u32>, Vec<&Value>> = BTreeMap::new();
        for track in tracks {
            discs
                .entry(numeric_field(track, "discNumber"))
                .or_default()
                .push(track);
        }
        for (disc_number, mut tracks) in discs {
            tracks.sort_by(|left, right| {
                numeric_field(left, "trackNumber")
                    .unwrap_or(u32::MAX)
                    .cmp(&numeric_field(right, "trackNumber").unwrap_or(u32::MAX))
                    .then_with(|| track_path(left).cmp(track_path(right)))
            });
            let track_total = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
            for (index, track) in tracks.into_iter().enumerate() {
                let desired_track = u32::try_from(index + 1).unwrap_or(u32::MAX);
                push_numeric_action(&mut actions, track, "trackNumber", Some(desired_track));
                push_numeric_action(&mut actions, track, "trackTotal", Some(track_total));
                push_numeric_action(&mut actions, track, "discNumber", disc_number);
                push_numeric_action(&mut actions, track, "discTotal", disc_total);
            }
        }
    }
    actions
}

fn numbering_album_key(track: &Value, path: &str) -> String {
    let artist = track
        .get("albumArtist")
        .and_then(Value::as_str)
        .or_else(|| {
            track
                .get("albumArtists")
                .and_then(Value::as_array)
                .and_then(|artists| artists.first())
                .and_then(Value::as_str)
        });
    let album = track.get("album").and_then(Value::as_str);
    match (artist, album) {
        (Some(artist), Some(album)) if !artist.trim().is_empty() && !album.trim().is_empty() => {
            format!(
                "{}\u{0}{}",
                artist.trim().to_lowercase(),
                album.trim().to_lowercase()
            )
        }
        _ => Path::new(path)
            .parent()
            .unwrap_or_else(|| Path::new(path))
            .to_string_lossy()
            .to_lowercase(),
    }
}

fn numeric_field(track: &Value, field: &str) -> Option<u32> {
    track.get(field).and_then(|value| {
        value
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .or_else(|| value.as_str().and_then(|number| number.parse().ok()))
    })
}

fn track_path(track: &Value) -> &str {
    track
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn push_numeric_action(
    actions: &mut Vec<AssistantAction>,
    track: &Value,
    field: &str,
    desired: Option<u32>,
) {
    let Some(desired) = desired else { return };
    let current = numeric_field(track, field);
    if current == Some(desired) {
        return;
    }
    actions.push(AssistantAction {
        tag_kind: Some("standard".into()),
        track_path: Some(track_path(track).into()),
        field: Some(field.into()),
        old_value: current.map(|value| value.to_string()),
        new_value: Some(desired.to_string()),
        description: Some(format!("Set {field} to {desired}")),
        ..Default::default()
    });
}

fn plan_strip_track_title_prefixes(
    input: &AssistantSendInput,
    scoped_paths: &[String],
) -> Vec<AssistantAction> {
    let scoped = scoped_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    input
        .tracks
        .iter()
        .filter_map(|track| {
            let path = track_path(track);
            let title = track.get("title").and_then(Value::as_str)?;
            if !scoped.contains(path) {
                return None;
            }
            let stripped = strip_track_title_prefix(title);
            (stripped != title).then(|| AssistantAction {
                tag_kind: Some("standard".into()),
                track_path: Some(path.into()),
                field: Some("title".into()),
                old_value: Some(title.into()),
                new_value: Some(stripped),
                description: Some("Strip leading track number from title".into()),
                ..Default::default()
            })
        })
        .collect()
}

fn strip_track_title_prefix(title: &str) -> String {
    use std::sync::OnceLock;
    static PREFIX: OnceLock<regex::Regex> = OnceLock::new();
    PREFIX
        .get_or_init(|| {
            regex::Regex::new(r"^(?:\d+[.)]\s+|\d+\s*[-–]\s+|\d{1,3}\s+)")
                .expect("valid title-prefix regex")
        })
        .replace(title, "")
        .into_owned()
}

fn plan_strip_filename_prefixes(paths: &[String]) -> Vec<AssistantAction> {
    use std::sync::OnceLock;
    static PREFIX: OnceLock<regex::Regex> = OnceLock::new();
    let prefix = PREFIX
        .get_or_init(|| regex::Regex::new(r"^\d+[\s.\\)-]+").expect("valid filename-prefix regex"));
    paths
        .iter()
        .filter_map(|source| {
            let path = Path::new(source);
            let filename = path.file_name()?.to_str()?;
            let stripped = prefix.replace(filename, "");
            if stripped == filename || stripped.is_empty() {
                return None;
            }
            let destination = path.with_file_name(stripped.as_ref());
            Some(AssistantAction {
                source_path: Some(source.clone()),
                destination_path: Some(destination.to_string_lossy().into_owned()),
                description: Some(format!("Rename {filename} to {stripped}")),
                ..Default::default()
            })
        })
        .collect()
}

fn plan_infer_tags_from_filenames(
    input: &AssistantSendInput,
    paths: &[String],
    prettify: bool,
) -> Vec<AssistantAction> {
    let tracks = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect::<BTreeMap<_, _>>();
    let mut actions = Vec::new();
    for path in paths {
        let Some((artist, title)) = infer_artist_title_from_filename(path) else {
            continue;
        };
        let artist = if prettify {
            prettify_tag(&artist)
        } else {
            artist
        };
        let title = if prettify {
            prettify_tag(&title)
        } else {
            title
        };
        let track = tracks.get(path.as_str()).copied();
        push_string_action(&mut actions, track, path, "title", &title);
        push_string_action(&mut actions, track, path, "artist", &artist);
        let artists = split_artist_names(&artist).join("; ");
        push_string_action(&mut actions, track, path, "artists", &artists);
    }
    actions
}

fn infer_artist_title_from_filename(path: &str) -> Option<(String, String)> {
    use std::sync::OnceLock;
    static LEADING_NUMBER: OnceLock<regex::Regex> = OnceLock::new();
    static SPACED_DASH: OnceLock<regex::Regex> = OnceLock::new();
    let leading_number = LEADING_NUMBER.get_or_init(|| {
        regex::Regex::new(r"(?i)^\s*(?:disc\s*)?\d{1,3}(?:[._ -]+|\s+)")
            .expect("valid filename track-number regex")
    });
    let spaced_dash = SPACED_DASH
        .get_or_init(|| regex::Regex::new(r"\s[-–—]\s").expect("valid artist-title regex"));
    let stem = Path::new(path).file_stem()?.to_str()?;
    let had_number = leading_number.is_match(stem);
    let clean = leading_number.replace(stem, "");
    let (artist, title) = if let Some(separator) = spaced_dash.find(&clean) {
        (&clean[..separator.start()], &clean[separator.end()..])
    } else if had_number {
        clean.split_once('-')?
    } else {
        return None;
    };
    let artist = artist.trim();
    let title = title.trim();
    (!artist.is_empty() && !title.is_empty()).then(|| (artist.into(), title.into()))
}

fn split_artist_names(artist: &str) -> Vec<String> {
    use std::sync::OnceLock;
    static DELIMITER: OnceLock<regex::Regex> = OnceLock::new();
    let delimiter = DELIMITER.get_or_init(|| {
        regex::Regex::new(r"(?i)\s+(?:feat\.?|ft\.?|featuring)\s+|\s*[&/;,＋+、，；·‧]\s*")
            .expect("valid multi-artist delimiter regex")
    });
    let normalized = artist.replace(" _ ", " / ");
    let mut seen = HashSet::new();
    delimiter
        .split(&normalized)
        .filter_map(|name| {
            let name = name.trim();
            let key = name.to_lowercase();
            (!name.is_empty() && seen.insert(key)).then(|| name.to_string())
        })
        .collect()
}

fn push_string_action(
    actions: &mut Vec<AssistantAction>,
    track: Option<&Value>,
    path: &str,
    field: &str,
    desired: &str,
) {
    let current = track.and_then(|track| track_field_string(track, field));
    if current.as_deref() == Some(desired) {
        return;
    }
    actions.push(AssistantAction {
        tag_kind: Some("standard".into()),
        track_path: Some(path.into()),
        field: Some(field.into()),
        old_value: current,
        new_value: Some(desired.into()),
        description: Some(format!("Infer {field} from filename")),
        ..Default::default()
    });
}

fn plan_chinese_conversion(
    input: &AssistantSendInput,
    paths: &[String],
    target: &str,
) -> Vec<AssistantAction> {
    const FIELDS: &[&str] = &[
        "title",
        "artist",
        "artists",
        "album",
        "albumArtist",
        "albumArtists",
        "genre",
        "composer",
        "comment",
        "description",
        "lyrics",
    ];
    let scoped = paths.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut actions = Vec::new();
    for track in &input.tracks {
        let path = track_path(track);
        if !scoped.contains(path) {
            continue;
        }
        for field in FIELDS {
            let original = match track.get(*field) {
                Some(Value::String(value)) => value.clone(),
                Some(Value::Array(values)) => values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; "),
                _ => continue,
            };
            if original.is_empty() {
                continue;
            }
            let converted = if matches!(*field, "artists" | "albumArtists") {
                original
                    .split(';')
                    .map(|value| convert_chinese_text(value.trim(), target))
                    .collect::<Vec<_>>()
                    .join("; ")
            } else {
                convert_chinese_text(&original, target)
            };
            if converted != original {
                push_string_action(&mut actions, Some(track), path, field, &converted);
            }
        }
    }
    actions
}

fn plan_group_by_album(
    input: &AssistantSendInput,
    paths: &[String],
) -> Result<Vec<AssistantAction>, ApiError> {
    let library = input
        .library_path
        .as_deref()
        .map(Path::new)
        .ok_or_else(|| {
            ApiError::Message("Library path is required to group files by album".into())
        })?;
    let tracks = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect::<BTreeMap<_, _>>();
    let mut destinations = HashSet::new();
    let mut actions = Vec::new();
    for source in paths {
        let Some(track) = tracks.get(source.as_str()).copied() else {
            continue;
        };
        let Some(album) = track
            .get("album")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|album| !album.is_empty())
        else {
            continue;
        };
        let source_path = Path::new(source);
        if !path_is_inside(source_path, library) {
            continue;
        }
        let destination_dir = library.join(sanitize_dir_name(album));
        if source_path.parent() == Some(destination_dir.as_path()) {
            continue;
        }
        let Some(filename) = source_path.file_name() else {
            continue;
        };
        let destination = unique_planned_destination(
            source_path,
            destination_dir.join(filename),
            &mut destinations,
        );
        actions.push(AssistantAction {
            source_path: Some(source.clone()),
            destination_path: Some(destination.to_string_lossy().into_owned()),
            description: Some(format!("Move into album folder: {}", album.trim())),
            ..Default::default()
        });
    }
    Ok(actions)
}

fn path_is_inside(path: &Path, root: &Path) -> bool {
    fn normalized(path: &Path) -> Option<PathBuf> {
        let mut result = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    if !result.pop() {
                        return None;
                    }
                }
                std::path::Component::CurDir => {}
                _ => result.push(component.as_os_str()),
            }
        }
        Some(result)
    }
    match (normalized(path), normalized(root)) {
        (Some(path), Some(root)) => path.starts_with(root),
        _ => false,
    }
}

fn unique_planned_destination(
    source: &Path,
    destination: PathBuf,
    reserved: &mut HashSet<PathBuf>,
) -> PathBuf {
    if (!destination.exists() || source == destination) && reserved.insert(destination.clone()) {
        return destination;
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new(""));
    let stem = destination
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("file");
    let extension = destination
        .extension()
        .and_then(|extension| extension.to_str());
    for index in 1.. {
        let filename = extension.map_or_else(
            || format!("{stem}_{index}"),
            |extension| format!("{stem}_{index}.{extension}"),
        );
        let candidate = parent.join(filename);
        if !candidate.exists() && reserved.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

fn validate_completion_evidence(
    contract: &AssistantTaskContract,
    has_action_batch: bool,
    response_message: &str,
) -> Result<(), ApiError> {
    if contract.kind == AssistantTaskContractKind::ReadOnlyAnswer && has_action_batch {
        return Err(ApiError::Message(
            "The assistant proposed a mutation preview for a read-only request".into(),
        ));
    }
    if contract.requires_completion_evidence && !has_action_batch {
        return Err(ApiError::Message(format!(
            "No action was performed. This request requires a preview batch, but the assistant only replied: {response_message}"
        )));
    }
    Ok(())
}

fn tool_call_signature(name: &str, args: &Value) -> String {
    format!("{name}|{}", canonical_json(args))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let body = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}

fn would_repeat_tool_call(signatures: &[String], name: &str, args: &Value) -> bool {
    let signature = tool_call_signature(name, args);
    signatures.len() >= 2
        && signatures[signatures.len() - 2..]
            .iter()
            .all(|seen| seen == &signature)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssistantDraft {
    message: String,
    #[serde(default)]
    action_batch: Option<AssistantDraftBatch>,
    #[serde(default)]
    tool_call: Option<AssistantDraftToolCall>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssistantDraftToolCall {
    tool_name: String,
    #[serde(default)]
    args: Value,
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
        ("artists" | "albumArtists", Some(value)) => Value::Array(
            value
                .split(';')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.into()))
                .collect(),
        ),
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

        let numbering = derive_assistant_task_contract("fix track numbers within each album");
        assert_eq!(numbering.route, Some(AssistantTaskRoute::AutoNumberTracks));

        let titles = derive_assistant_task_contract("strip number prefixes from track titles");
        assert_eq!(
            titles.route,
            Some(AssistantTaskRoute::StripTrackTitlePrefixes)
        );

        let filenames = derive_assistant_task_contract("remove number prefixes from filenames");
        assert_eq!(
            filenames.route,
            Some(AssistantTaskRoute::StripFilenamePrefixes)
        );

        let inference =
            derive_assistant_task_contract("infer title and artist tags from filenames");
        assert_eq!(
            inference.route,
            Some(AssistantTaskRoute::InferTagsFromFilenames)
        );

        let chinese = derive_assistant_task_contract("convert Chinese tags to Traditional");
        assert_eq!(
            chinese.route,
            Some(AssistantTaskRoute::ChineseToTraditional)
        );
        let ambiguous_chinese = derive_assistant_task_contract("convert Chinese tags");
        assert_eq!(
            ambiguous_chinese.kind,
            AssistantTaskContractKind::ClarificationRequired
        );

        let grouping = derive_assistant_task_contract("group files into album folders");
        assert_eq!(grouping.route, Some(AssistantTaskRoute::GroupByAlbum));
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

        let batch = deterministic_task_batch("session", &input, AssistantTaskRoute::AutoTag)
            .unwrap()
            .unwrap();

        assert_eq!(batch.kind, "auto-tag-run");
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(
            batch.actions[0].track_path.as_deref(),
            Some("/music/selected.mp3")
        );
    }

    #[test]
    fn deterministic_numbering_groups_by_album_and_emits_only_real_diffs() {
        let input = AssistantSendInput {
            message: "fix track numbers".into(),
            tracks: vec![
                serde_json::json!({
                    "path": "/music/A/Album/02.mp3", "album": "Album", "albumArtist": "A",
                    "trackNumber": 9, "trackTotal": 9, "discNumber": 1, "discTotal": 1
                }),
                serde_json::json!({
                    "path": "/music/A/Album/01.mp3", "album": "Album", "albumArtist": "A",
                    "trackNumber": 1, "trackTotal": 9, "discNumber": 1, "discTotal": 1
                }),
                serde_json::json!({
                    "path": "/music/B/Other/01.mp3", "album": "Other", "albumArtist": "B",
                    "trackNumber": 1, "trackTotal": 1
                }),
            ],
            ..Default::default()
        };

        let batch =
            deterministic_task_batch("session", &input, AssistantTaskRoute::AutoNumberTracks)
                .unwrap()
                .unwrap();

        assert_eq!(batch.kind, "metadata-update");
        assert!(batch.actions.iter().any(|action| {
            action.track_path.as_deref() == Some("/music/A/Album/02.mp3")
                && action.field.as_deref() == Some("trackNumber")
                && action.new_value.as_deref() == Some("2")
        }));
        assert!(!batch
            .actions
            .iter()
            .any(|action| { action.track_path.as_deref() == Some("/music/B/Other/01.mp3") }));
    }

    #[test]
    fn deterministic_numbering_returns_no_change_for_correct_tracks() {
        let input = AssistantSendInput {
            message: "fix track numbers".into(),
            tracks: vec![
                serde_json::json!({
                    "path": "/music/A/Album/01.mp3", "album": "Album", "albumArtist": "A",
                    "trackNumber": 1, "trackTotal": 2, "discNumber": 1, "discTotal": 1
                }),
                serde_json::json!({
                    "path": "/music/A/Album/02.mp3", "album": "Album", "albumArtist": "A",
                    "trackNumber": 2, "trackTotal": 2, "discNumber": 1, "discTotal": 1
                }),
            ],
            ..Default::default()
        };

        let batch =
            deterministic_task_batch("session", &input, AssistantTaskRoute::AutoNumberTracks)
                .unwrap();

        assert!(batch.is_none());
    }

    #[test]
    fn deterministic_title_prefix_cleanup_emits_only_changed_titles() {
        let input = AssistantSendInput {
            message: "strip title prefixes".into(),
            tracks: vec![
                serde_json::json!({"path": "/music/01.mp3", "title": "01. First"}),
                serde_json::json!({"path": "/music/02.mp3", "title": "02 - Second"}),
                serde_json::json!({"path": "/music/03.mp3", "title": "Already Clean"}),
            ],
            ..Default::default()
        };

        let batch = deterministic_task_batch(
            "session",
            &input,
            AssistantTaskRoute::StripTrackTitlePrefixes,
        )
        .unwrap()
        .unwrap();

        assert_eq!(batch.kind, "metadata-update");
        assert_eq!(batch.actions.len(), 2);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("First"));
        assert_eq!(batch.actions[1].new_value.as_deref(), Some("Second"));
        assert!(batch
            .actions
            .iter()
            .all(|action| action.tag_kind.as_deref() == Some("standard")));
    }

    #[test]
    fn deterministic_filename_prefix_cleanup_previews_renames_without_writing() {
        let input = AssistantSendInput {
            message: "strip filename prefixes".into(),
            tracks: vec![
                serde_json::json!({"path": "/music/01. First.mp3"}),
                serde_json::json!({"path": "/music/02 - Second.flac"}),
                serde_json::json!({"path": "/music/Already Clean.ogg"}),
            ],
            ..Default::default()
        };

        let batch =
            deterministic_task_batch("session", &input, AssistantTaskRoute::StripFilenamePrefixes)
                .unwrap()
                .unwrap();

        assert_eq!(batch.kind, "folder-move");
        assert_eq!(batch.risk_level, "medium");
        assert_eq!(batch.actions.len(), 2);
        assert_eq!(
            batch.actions[0].destination_path.as_deref(),
            Some("/music/First.mp3")
        );
        assert_eq!(
            batch.actions[1].destination_path.as_deref(),
            Some("/music/Second.flac")
        );
    }

    #[test]
    fn deterministic_filename_inference_handles_spaced_and_structured_compact_names() {
        let input = AssistantSendInput {
            message: "infer and prettify tags from filenames".into(),
            tracks: vec![
                serde_json::json!({"path": "/music/01 Artist A & Artist B - first_song.flac"}),
                serde_json::json!({"path": "/music/110-hedgehog-you_are_so_famous.flac"}),
                serde_json::json!({"path": "/music/standalone-title.flac"}),
            ],
            ..Default::default()
        };

        let batch = deterministic_task_batch(
            "session",
            &input,
            AssistantTaskRoute::InferTagsFromFilenames,
        )
        .unwrap()
        .unwrap();

        assert_eq!(batch.actions.len(), 6);
        assert!(batch.actions.iter().any(|action| {
            action.track_path.as_deref() == Some("/music/110-hedgehog-you_are_so_famous.flac")
                && action.field.as_deref() == Some("title")
                && action.new_value.as_deref() == Some("You Are So Famous")
        }));
        assert!(batch.actions.iter().any(|action| {
            action.field.as_deref() == Some("artists")
                && action.new_value.as_deref() == Some("Artist A; Artist B")
        }));
        assert!(!batch.actions.iter().any(|action| {
            action.track_path.as_deref() == Some("/music/standalone-title.flac")
        }));
    }

    #[test]
    fn standard_array_actions_are_deserialized_as_separate_values() {
        let patch = action_patch("artists", Some("Artist A; Artist B")).unwrap();
        assert_eq!(
            patch.artists.value(),
            Some(&crate::commands::mutations::StringList::Many(vec![
                "Artist A".into(),
                "Artist B".into()
            ]))
        );
    }

    #[test]
    fn deterministic_chinese_conversion_updates_only_changed_text_fields() {
        let input = AssistantSendInput {
            message: "convert Chinese tags to Traditional".into(),
            tracks: vec![serde_json::json!({
                "path": "/music/one.flac",
                "title": "音乐与未来",
                "artist": "Artist",
                "artists": ["音乐人", "Artist"],
                "album": "专辑"
            })],
            ..Default::default()
        };

        let batch =
            deterministic_task_batch("session", &input, AssistantTaskRoute::ChineseToTraditional)
                .unwrap()
                .unwrap();

        assert!(batch.actions.iter().any(|action| {
            action.field.as_deref() == Some("title")
                && action.new_value.as_deref() == Some("音樂與未來")
        }));
        assert!(batch.actions.iter().any(|action| {
            action.field.as_deref() == Some("artists")
                && action.new_value.as_deref() == Some("音樂人; Artist")
        }));
        assert!(!batch
            .actions
            .iter()
            .any(|action| action.field.as_deref() == Some("artist")));
    }

    #[test]
    fn deterministic_album_grouping_stays_inside_library_and_avoids_collisions() {
        let root = temp_dir();
        let existing_dir = root.join("A B");
        fs::create_dir_all(&existing_dir).unwrap();
        fs::write(existing_dir.join("song.mp3"), b"existing").unwrap();
        let source = root.join("loose/song.mp3");
        let already_grouped = existing_dir.join("other.mp3");
        let outside = root.parent().unwrap().join("outside.mp3");
        let input = AssistantSendInput {
            message: "group files into album folders".into(),
            library_path: Some(root.to_string_lossy().into_owned()),
            tracks: vec![
                serde_json::json!({"path": source, "album": "A/B"}),
                serde_json::json!({"path": already_grouped, "album": "A/B"}),
                serde_json::json!({"path": outside, "album": "Outside"}),
            ],
            ..Default::default()
        };

        let batch = deterministic_task_batch("session", &input, AssistantTaskRoute::GroupByAlbum)
            .unwrap()
            .unwrap();

        assert_eq!(batch.actions.len(), 1);
        assert_eq!(
            batch.actions[0].destination_path.as_deref(),
            Some(existing_dir.join("song_1.mp3").to_string_lossy().as_ref())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn assistant_response_schema_advertises_the_complete_registry() {
        let schema = assistant_response_schema();
        let names = schema["properties"]["toolCall"]["properties"]["toolName"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(names.len(), 21);
        assert!(names.contains(&serde_json::json!("edit_metadata")));
        assert!(names.contains(&serde_json::json!("create_plan")));
    }

    #[test]
    fn edit_metadata_tool_builds_standard_and_extra_preview_actions() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/one.flac".into()],
            tracks: vec![serde_json::json!({
                "path": "/music/one.flac", "album": "Old", "genre": "Rock"
            })],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "edit_metadata",
            &serde_json::json!({
                "target_scope": "selected",
                "standard_updates": {"album": "New"},
                "standard_removes": ["genre"],
                "extra_upserts": [{"key": "MOOD", "value": "Calm"}]
            }),
            &input,
            "session",
        );

        assert!(execution.result.ok);
        assert_eq!(execution.batches.len(), 1);
        let actions = &execution.batches[0].actions;
        assert_eq!(actions.len(), 3);
        assert!(actions.iter().any(|action| {
            action.field.as_deref() == Some("album")
                && action.old_value.as_deref() == Some("Old")
                && action.new_value.as_deref() == Some("New")
        }));
        assert!(actions.iter().any(|action| {
            action.field.as_deref() == Some("genre") && action.new_value.is_none()
        }));
        assert!(actions.iter().any(|action| {
            action.tag_kind.as_deref() == Some("extra") && action.field.as_deref() == Some("MOOD")
        }));
    }

    #[test]
    fn edit_metadata_reports_explicit_no_changes_for_equal_list_values() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/one.flac".into()],
            tracks: vec![serde_json::json!({
                "path": "/music/one.flac", "albumArtists": ["Artist A", "Artist B"]
            })],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "edit_metadata",
            &serde_json::json!({
                "target_scope": "selected",
                "standard_updates": {"albumArtists": ["Artist A", "Artist B"]}
            }),
            &input,
            "session",
        );

        assert!(execution.result.ok);
        assert!(execution.batches.is_empty());
        assert!(execution.completion_evidence);
        assert_eq!(
            execution.result.data.as_ref().unwrap()["outcome"],
            "no_changes"
        );
    }

    #[test]
    fn library_scope_preserves_loaded_track_order() {
        let input = AssistantSendInput {
            tracks: vec![
                serde_json::json!({"path": "/music/z.flac"}),
                serde_json::json!({"path": "/music/a.flac"}),
            ],
            ..Default::default()
        };

        assert_eq!(
            tool_scope_paths(&input, &serde_json::json!({"target_scope": "library"})).unwrap(),
            vec!["/music/z.flac", "/music/a.flac"]
        );
    }

    #[test]
    fn regex_extract_tool_uses_requested_capture_group_and_real_diffs_only() {
        let input = AssistantSendInput {
            tracks: vec![
                serde_json::json!({"path": "/music/one.flac", "album": "01 - Album"}),
                serde_json::json!({"path": "/music/two.flac", "album": "Clean"}),
            ],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "extract_tag_value",
            &serde_json::json!({
                "target_scope": "library", "field": "album",
                "pattern": "^\\d+[\\s.-]+(.+)$", "group_index": 1
            }),
            &input,
            "session",
        );

        assert!(execution.result.ok);
        let actions = &execution.batches[0].actions;
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].new_value.as_deref(), Some("Album"));
    }

    #[test]
    fn organize_files_tool_scans_direct_files_and_previews_extension_folders() {
        let root = temp_dir();
        let source = root.join("loose");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("one.flac"), b"one").unwrap();
        fs::write(source.join("two.mp3"), b"two").unwrap();
        fs::write(source.join(".hidden.mp3"), b"hidden").unwrap();
        let input = AssistantSendInput {
            library_path: Some(root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "organize_files",
            &serde_json::json!({
                "source_dir": source, "criterion": "extension",
                "pattern_string": "flac", "target_dir_name": "By Type"
            }),
            &input,
            "session",
        );

        assert!(execution.result.ok);
        assert_eq!(execution.batches[0].actions.len(), 1);
        assert_eq!(
            execution.batches[0].actions[0].destination_path.as_deref(),
            Some(
                source
                    .join("By Type/flac/one.flac")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(source.join("one.flac").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn library_task_tool_creates_scoped_handoff_preview() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/one.flac".into()],
            tracks: vec![serde_json::json!({"path": "/music/one.flac"})],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "run_library_task",
            &serde_json::json!({"task": "audit", "target_scope": "selected"}),
            &input,
            "session",
        );

        assert_eq!(execution.batches[0].kind, "audit-run");
        assert_eq!(execution.batches[0].actions.len(), 1);
        assert!(!execution.batches[0].reversible);
    }

    #[tokio::test]
    async fn create_plan_resolves_prior_paths_and_collects_preview_batches() {
        let input = AssistantSendInput {
            tracks: vec![serde_json::json!({
                "path": "/music/one.flac", "genre": null
            })],
            ..Default::default()
        };
        let providers = ProviderState::default();
        let config = crate::state::config::AutoTagConfig::default();
        let assistant = AssistantServicesSnapshot::default();
        let execution = execute_create_plan(
            &serde_json::json!({
                "steps": [
                    {"id": "find", "tool": "tracks.search", "args": {"missingGenre": true}},
                    {"id": "edit", "tool": "edit_metadata", "depends_on": ["find"], "args": {
                        "target_scope": "explicit_paths", "paths": "$find.paths",
                        "standard_updates": {"genre": "Rock"}
                    }}
                ]
            }),
            &input,
            "session",
            NativeAssistantToolServices {
                input: &input,
                providers: &providers,
                config: &config,
                assistant: &assistant,
            },
        )
        .await;

        assert!(execution.result.ok);
        assert_eq!(execution.batches.len(), 1);
        assert_eq!(
            execution.batches[0].actions[0].field.as_deref(),
            Some("genre")
        );
        assert_eq!(
            execution.batches[0].actions[0].new_value.as_deref(),
            Some("Rock")
        );
    }

    #[test]
    fn plan_dependency_order_preserves_declaration_order_for_independent_steps() {
        let steps = serde_json::json!([
            {"id": "z-last-alphabetically", "tool": "library.summarize", "args": {}},
            {"id": "a-first-alphabetically", "tool": "library.summarize", "args": {}}
        ]);

        assert_eq!(
            plan_dependency_order(steps.as_array().unwrap()).unwrap(),
            vec!["z-last-alphabetically", "a-first-alphabetically"]
        );
    }

    #[tokio::test]
    async fn read_only_plan_does_not_count_as_mutation_completion_evidence() {
        let input = AssistantSendInput::default();
        let providers = ProviderState::default();
        let config = crate::state::config::AutoTagConfig::default();
        let assistant = AssistantServicesSnapshot::default();
        let execution = execute_create_plan(
            &serde_json::json!({
                "steps": [{"id": "inspect", "tool": "library.summarize", "args": {}}]
            }),
            &input,
            "session",
            NativeAssistantToolServices {
                input: &input,
                providers: &providers,
                config: &config,
                assistant: &assistant,
            },
        )
        .await;

        assert!(execution.result.ok);
        assert!(!execution.completion_evidence);
    }

    #[test]
    fn mutating_contract_rejects_model_reply_without_preview_evidence() {
        let contract = derive_assistant_task_contract("change the album title");
        let error = validate_completion_evidence(&contract, false, "I can do that").unwrap_err();

        assert!(error.to_string().contains("requires a preview batch"));
    }

    #[test]
    fn read_only_contract_rejects_a_model_authored_mutation_preview() {
        let contract = derive_assistant_task_contract("show tracks missing a title");
        let error = validate_completion_evidence(&contract, true, "Apply these fixes").unwrap_err();

        assert!(error.to_string().contains("read-only request"));
    }

    #[test]
    fn repeated_tool_guard_stops_third_identical_call_but_not_distinct_args() {
        let mut signatures = vec![
            tool_call_signature("tracks.search", &serde_json::json!({"artist": "A"})),
            tool_call_signature("tracks.search", &serde_json::json!({"artist": "A"})),
        ];
        assert!(would_repeat_tool_call(
            &signatures,
            "tracks.search",
            &serde_json::json!({"artist": "A"})
        ));
        assert!(!would_repeat_tool_call(
            &signatures,
            "tracks.search",
            &serde_json::json!({"artist": "B"})
        ));
        signatures.push(tool_call_signature(
            "tracks.search",
            &serde_json::json!({"artist": "B"}),
        ));
        assert!(!would_repeat_tool_call(
            &signatures,
            "tracks.search",
            &serde_json::json!({"artist": "A"})
        ));
    }

    #[test]
    fn tool_call_signature_is_stable_across_object_key_order() {
        assert_eq!(
            tool_call_signature(
                "tracks.search",
                &serde_json::json!({"artist": "A", "album": "B"})
            ),
            tool_call_signature(
                "tracks.search",
                &serde_json::json!({"album": "B", "artist": "A"})
            )
        );
    }

    #[test]
    fn musicbrainz_tool_query_extracts_quoted_and_unquoted_fields() {
        assert_eq!(
            parse_musicbrainz_tool_query("artist:\"Radiohead\" album:\"OK Computer\""),
            ("Radiohead".into(), "OK Computer".into())
        );
        assert_eq!(
            parse_musicbrainz_tool_query("album:Blue Train artist:John Coltrane"),
            ("John Coltrane".into(), "Blue Train".into())
        );
    }

    #[test]
    fn tool_result_prompt_includes_bounded_structured_evidence() {
        let result = AssistantToolResult {
            ok: true,
            summary: "Found one track".into(),
            data: Some(serde_json::json!({"paths": ["/music/one.mp3"]})),
            error: None,
        };
        let prompt = tool_result_prompt(&result);
        assert!(prompt.contains("Found one track"));
        assert!(prompt.contains("/music/one.mp3"));

        let large = AssistantToolResult {
            data: Some(Value::String("x".repeat(20_000))),
            ..result
        };
        assert!(tool_result_prompt(&large).len() < 13_000);
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
            "soundrobe-assistant-apply-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
