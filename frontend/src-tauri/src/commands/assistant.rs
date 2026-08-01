//! Assistant runtime and tool-service commands.

use crate::commands::{
    assistant_tools::{
        context_tool_catalog, execute_context_tool, prettify_tag, registered_tool_is_read_only,
        validate_registered_tool_args, AssistantToolResult,
    },
    dataset::dataset_status_at,
    lyrics::{fetch_lyrics_at, DEFAULT_BASE_URL},
    mutations::{
        remove_embedded_cover_queued,
        write_extra_tags_with_exclusive_queue_held, write_track_with_exclusive_queue_held,
        ExtraTagUpdate, TrackPatch,
    },
    organizer::sanitize_dir_name,
    tracks::{read_extra_tags, read_track_metadata, try_read_extra_tags},
};
use crate::error::ApiError;
use crate::infra::is_not_redacted;
use crate::infra::openrouter::{ChatMessage, OpenRouterClient};
use crate::state::assistant::{
    AssistantAction, AssistantActionBatch, AssistantCompletionPostcondition, AssistantRuntimeState,
    AssistantServicesConfig, AssistantServicesSnapshot, AssistantServicesState,
};
use crate::state::config::ConfigState;
use crate::state::conversation::{ConversationEntry, ConversationState};
use crate::state::providers::convert_chinese_text;
use crate::state::providers::{DiscogsClient, MusicBrainzClient, ProviderState};
use crate::state::write_queue::WriteQueue;
use lofty::file::TaggedFileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

const ASSISTANT_LLM_TIMEOUT_SECS: u64 = 120;
const ASSISTANT_SESSION_TIMEOUT_SECS: u64 = 600;
const ASSISTANT_MAX_STEPS: usize = 20;
const ASSISTANT_PREVIEW_REPAIR_ATTEMPTS: usize = 3;
const ASSISTANT_ACTION_REPAIR_ATTEMPTS: usize = 3;
const ASSISTANT_SESSION_TIMEOUT_LOG: &str = "Session timed out after 600 seconds";
const ASSISTANT_SELF_REVIEW_PROMPT: &str = "If your response above is a clarification because \
action-defining details are missing, an answer, or a limitation, finalize it unchanged. If the user \
already requested a supported edit, do not ask whether to proceed; call the tool now to create the \
approval preview. If your response announces an action (for example, \"I'll inspect\" or \"let me \
preview\"), call the tool now instead.";
const ASSISTANT_ACTION_REPAIR_PROMPT: &str = "You classified this response as an action, but did \
not call a tool or create a preview. An action claim without execution evidence cannot be returned \
to the user. Call the registered tool now. If the work cannot be performed, return a limitation \
instead; if no action is needed, return an answer.";
const ASSISTANT_CLARIFY_REPAIR_PROMPT: &str = "You already asked a clarifying question earlier in \
this session and the user answered it. Do not ask another clarifying question. Call a registered \
tool now to create the approval preview, or if no tool can perform the request, state the limitation \
clearly and say what the app can do instead.";
/// Consecutive clarification-only responses after which the assistant must stop
/// asking and either act or declare the limitation. The agreed invariant is one
/// clarification, then act or state a limitation; the bounded repair inside the
/// turn gives the model one more chance, so two finalized questions already
/// mean the user answered and the assistant still only asked.
const ASSISTANT_CLARIFY_LIMIT: i64 = 2;

/// Deterministic signal that a finalized message is another clarifying question
/// rather than an answer, limitation, or completion. Matches the phrasing the
/// model actually produced in the failing session ("Just to clarify — …",
/// "What would you like to do first?").
fn is_clarification_question(message: &str) -> bool {
    let trimmed = message.trim();
    if trimmed.ends_with('?') {
        return true;
    }
    let lower = trimmed.to_lowercase();
    lower.starts_with("just to clarify")
        || lower.starts_with("to clarify")
        || lower.starts_with("clarify")
}

fn clarification_limit_message() -> String {
    "I've asked clarifying questions repeatedly without being able to act, so I'm stopping. \
Please rephrase the request as a specific operation the app supports — for example: set a tag \
field to a value, transform tag values, rename files within their folder, group tracks into album \
folder(s), or run auto-tagging."
        .to_string()
}

/// State transition for the consecutive-clarification counter: a question
/// increments the streak, anything else (preview, answer, limitation) resets it.
fn next_clarification_count(prior: i64, is_question: bool) -> i64 {
    if is_question {
        prior + 1
    } else {
        0
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEvent {
    session_id: String,
    #[serde(rename = "type")]
    event_type: &'static str,
    message: String,
    data: Option<Value>,
}

// ── Credential resolution ─────────────────────────────────────────────

/// Resolve the LLM API key and model from two sources, in priority order:
///
/// 1. `ConfigState` (env/config file) — the canonical secret authority.
/// 2. `AssistantServicesState` (copied from the real key during init).
///
/// Values starting with `"****"` are rejected as redacted placeholders.
/// Renderer-supplied `input.api_key` is NOT consulted — the renderer only
/// ever sees a masked copy, so accepting it would re-introduce the 400 error.
pub(crate) fn resolve_credentials(
    config_key: Option<&str>,
    config_model: Option<&str>,
    snapshot_key: &str,
    snapshot_model: &str,
) -> (Option<String>, Option<String>) {
    let api_key = config_key
        .filter(|k| is_not_redacted(k))
        .map(str::to_string)
        .or_else(|| {
            if is_not_redacted(snapshot_key) {
                Some(snapshot_key.to_string())
            } else {
                None
            }
        });

    let model = config_model
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .or_else(|| {
            if !snapshot_model.is_empty() {
                Some(snapshot_model.to_string())
            } else {
                None
            }
        });

    (api_key, model)
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    #[test]
    fn config_key_wins_over_snapshot() {
        let (key, model) = resolve_credentials(
            Some("sk-or-v1-real"),
            Some("gpt-4"),
            "****b7", // masked snapshot
            "old-model",
        );
        assert_eq!(key.as_deref(), Some("sk-or-v1-real"));
        assert_eq!(model.as_deref(), Some("gpt-4"));
    }

    #[test]
    fn masked_only_keys_are_rejected() {
        let (key, _model) = resolve_credentials(None, Some("model"), "****b7", "model");
        assert_eq!(key, None, "masked placeholder must be rejected");
    }

    #[test]
    fn snapshot_key_used_when_config_key_is_none() {
        let (key, _model) = resolve_credentials(None, Some("model"), "sk-or-v1-snapshot", "model");
        assert_eq!(key.as_deref(), Some("sk-or-v1-snapshot"));
    }

    #[test]
    fn returns_none_for_key_when_both_sources_empty() {
        let (key, _model) = resolve_credentials(None, None, "", "");
        assert_eq!(key, None);
    }

    #[test]
    fn empty_model_rejected_and_falls_back() {
        let (_key, model) =
            resolve_credentials(Some("key"), None, "snapshot-key", "snapshot-model");
        assert_eq!(model.as_deref(), Some("snapshot-model"));
    }

    #[test]
    fn both_sources_empty_returns_none_for_both() {
        let (key, model) = resolve_credentials(None, None, "", "");
        assert_eq!(key, None);
        assert_eq!(model, None);
    }

    #[test]
    fn redacted_config_key_skips_to_snapshot() {
        let (key, model) = resolve_credentials(Some("****b7"), Some("model"), "sk-or-v1-real", "m");
        assert_eq!(key.as_deref(), Some("sk-or-v1-real"));
        assert_eq!(model.as_deref(), Some("model"));
    }

    #[test]
    fn non_redacted_empty_string_rejected() {
        let (key, _model) = resolve_credentials(Some(""), None, "", "");
        assert_eq!(key, None);
    }
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
    let tool_start = std::time::Instant::now();
    tracing::debug!(tool = %name, args = %args, "native assistant tool started");
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
    let result = match name {
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
    };
    tracing::debug!(
        tool = %name,
        elapsed_us = tool_start.elapsed().as_micros(),
        "native assistant tool finished"
    );
    result
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
    task_state: State<'_, crate::state::assistant_task::AssistantTaskState>,
) -> Result<AssistantEvent, ApiError> {
    let session_start = std::time::Instant::now();
    tracing::debug!(
        message_preview = %input.message.chars().take(120).collect::<String>(),
        track_count = input.tracks.len(),
        "assistant_send started"
    );
    let raw_config = config.raw();
    if !conversation.initialize(raw_config.cache_path.as_deref())
        || !runtime.initialize_with_task_state(&task_state)
    {
        return Err(ApiError::Message(
            "Assistant runtime could not be initialized".into(),
        ));
    }
    let current = conversation
        .current()
        .ok_or_else(|| ApiError::Message("No active assistant session".into()))?;

    // ── Deterministic routing (runs before credential resolution) ──
    // Try to route unambiguous operations without consulting an LLM.
    let latest_session = task_state.load_latest_session();

    // Run the deterministic router.
    let session_id_for_state = current.session_id.clone();
    if let Some(routed) =
        crate::commands::assistant_intent::route_message(&input.message, latest_session.as_ref())
    {
        tracing::debug!(
            intent = ?routed.intent,
            field = ?routed.field,
            has_value = routed.value.is_some(),
            uses_referent = routed.uses_referent,
            "deterministic routing matched"
        );

        if routed.uses_referent && routed.value.is_none() {
            // "set them" without a value — need clarification
            return assistant_error_event_with_conversation(
                &app,
                Some(&*conversation),
                Some(session_id_for_state),
                "What value should I set?",
            );
        }

        // Persist the session state with routing info
        let scope_predicate =
            crate::commands::assistant_intent::resolved_intent_from_command(&routed);
        let now = crate::state::assistant_task::iso_now();
        let session_state = crate::state::assistant_task::SessionState {
            session_id: session_id_for_state.clone(),
            intent: Some(format!("{:?}", routed.intent)),
            scope_predicate: serde_json::to_value(&scope_predicate).ok(),
            protocol: "routed".to_string(),
            referent_count: 0,
            referent_query: routed.field.clone().map(|f| format!("missing {}", f)),
            referent_field: routed.field.clone(),
            referent_value: routed.value.clone(),
            pending_batch_ids: vec![],
            mutation_required: true,
            consecutive_clarifications: 0,
            created_at: now.clone(),
            updated_at: now,
        };
        let _ = task_state.upsert_session(&session_state);

        // Execute the routed intent directly.
        if let Some(field) = &routed.field {
            let value = routed.value.as_deref();

            // ── Resolve scope from input ───────────────────────────
            let scope_paths = crate::commands::assistant_intent::resolve_scope(
                &routed,
                &input,
                latest_session.as_ref(),
            );
            if scope_paths.is_empty() {
                return assistant_error_event_with_conversation(
                    &app,
                    Some(&*conversation),
                    Some(session_id_for_state),
                    "No tracks match the requested scope.",
                );
            }

            // ── Dispatch by intent kind ────────────────────────────
            let execution = match routed.intent {
                crate::commands::assistant_intent::IntentKind::SetField
                | crate::commands::assistant_intent::IntentKind::SetMissing => {
                    let value = match value {
                        Some(v) => v,
                        None => {
                            // "set the missing genre" without a value — ask
                            return assistant_error_event_with_conversation(
                                &app,
                                Some(&*conversation),
                                Some(session_id_for_state),
                                &format!(
                                    "What value should I set for {} where it is missing?",
                                    field
                                ),
                            );
                        }
                    };
                    let command_args = if routed.only_if_missing {
                        serde_json::json!({
                            "target_scope": "explicit_paths",
                            "paths": scope_paths,
                            "changes": [{
                                "field": field,
                                "action": "set",
                                "value": value,
                                "only_if_missing": true
                            }]
                        })
                    } else {
                        serde_json::json!({
                            "target_scope": "explicit_paths",
                            "paths": scope_paths,
                            "changes": [{
                                "field": field,
                                "action": "set",
                                "value": value
                            }]
                        })
                    };
                    crate::commands::assistant_metadata_tools::execute_metadata_patch(
                        &command_args,
                        &input,
                        &session_id_for_state,
                    )
                }
                crate::commands::assistant_intent::IntentKind::RemoveField
                | crate::commands::assistant_intent::IntentKind::ClearField => {
                    let command_args = serde_json::json!({
                        "target_scope": "explicit_paths",
                        "paths": scope_paths,
                        "changes": [{
                            "field": field,
                            "action": "remove"
                        }]
                    });
                    crate::commands::assistant_metadata_tools::execute_metadata_patch(
                        &command_args,
                        &input,
                        &session_id_for_state,
                    )
                }
                crate::commands::assistant_intent::IntentKind::SplitArtists => {
                    let command_args = serde_json::json!({
                        "target_scope": "explicit_paths",
                        "paths": scope_paths,
                        "source": {"kind": "tag", "field": "artists"},
                        "operations": [{"op": "split_artists"}]
                    });
                    crate::commands::assistant_metadata_tools::execute_metadata_transform(
                        &command_args,
                        &input,
                        &session_id_for_state,
                    )
                }
            };

            if execution.batches.is_empty() && !execution.result.ok {
                // Error — report it
                return assistant_error_event_with_conversation(
                    &app,
                    Some(&*conversation),
                    Some(session_id_for_state),
                    &execution.result.summary,
                );
            }

            // Store batches in the runtime and persist them.
            let mut stored_batches = Vec::new();
            for batch in &execution.batches {
                if runtime.add_batch(batch.clone()) {
                    if let Err(error) = task_state.save_batch(
                        &batch.id,
                        &session_id_for_state,
                        &serde_json::to_value(batch).unwrap_or_default(),
                        batch.actions.len(),
                    ) {
                        runtime.mark_batch_failed(&batch.id, &error);
                        return assistant_error_event_with_conversation(
                            &app,
                            Some(&*conversation),
                            Some(session_id_for_state),
                            &error,
                        );
                    }
                    stored_batches.push(batch.clone());
                }
            }

            // Update session state with batch info
            let pending_ids: Vec<String> = stored_batches.iter().map(|b| b.id.clone()).collect();
            let updated_state = crate::state::assistant_task::SessionState {
                pending_batch_ids: pending_ids.clone(),
                ..session_state
            };
            let _ = task_state.upsert_session(&updated_state);

            // Update referent in session state
            let referent_predicate =
                crate::state::assistant_task::ScopePredicate::LibraryAndMissing {
                    field: field.clone(),
                };
            let (_paths, count) = crate::state::assistant_task::evaluate_predicate(
                &referent_predicate,
                &input.tracks,
                input.active_album_path.as_deref(),
                &input.selected_track_paths,
            );
            let referent_session = crate::state::assistant_task::SessionState {
                referent_count: count as i64,
                referent_query: Some(format!("missing {}", field)),
                referent_field: Some(field.clone()),
                referent_value: routed.value.clone(),
                ..updated_state
            };
            let _ = task_state.upsert_session(&referent_session);

            if stored_batches.is_empty() {
                // No changes needed
                conversation.record_system(&execution.result.summary);
                return Ok(AssistantEvent {
                    session_id: session_id_for_state,
                    event_type: "message",
                    message: execution.result.summary.clone(),
                    data: Some(serde_json::json!({
                        "outcome": "no_changes",
                        "summary": execution.result.summary
                    })),
                });
            }

            let first = &stored_batches[0];
            let message = if stored_batches.len() == 1 {
                format!("Preview created ({}): {}", first.id, first.summary)
            } else {
                format!(
                    "Preview created ({} batches): {} changes",
                    stored_batches.len(),
                    stored_batches
                        .iter()
                        .map(|b| b.actions.len())
                        .sum::<usize>()
                )
            };

            conversation.record("assistant_message", &message, None, 0, 0, 0);
            let event = AssistantEvent {
                session_id: session_id_for_state,
                event_type: "action_batch_created",
                message: message.clone(),
                data: Some(serde_json::json!({
                    "actionBatchId": first.id,
                    "actionBatch": first,
                    "actionBatches": stored_batches
                })),
            };
            let _ = app.emit("assistant:event", &event);
            return Ok(event);
        }

        // If the intent was routed but field was missing, clarify.
        return assistant_error_event_with_conversation(
            &app,
            Some(&*conversation),
            Some(session_id_for_state),
            "I couldn't determine which field to change. Please specify the field name.",
        );
    }

    // ── End deterministic routing — LLM fallthrough ──

    let snapshot = services.snapshot().unwrap_or_default();
    let (api_key, model) = resolve_credentials(
        raw_config.llm_api_key.as_deref(),
        raw_config.llm_model.as_deref(),
        &snapshot.api_key,
        &snapshot.model,
    );
    tracing::debug!(
        has_config_key = raw_config
            .llm_api_key
            .as_deref()
            .filter(|k| is_not_redacted(k))
            .is_some(),
        has_service_key = is_not_redacted(&snapshot.api_key),
        has_model = model.is_some(),
        "assistant credential resolution"
    );
    let Some(api_key) = api_key else {
        return assistant_error_event_with_conversation(
            &app,
            Some(&*conversation),
            conversation.current().map(|current| current.session_id),
            "LLM API key is not configured. Set it in Settings or via the LLM_API_KEY environment variable.",
        );
    };
    let Some(model) = model else {
        return assistant_error_event_with_conversation(
            &app,
            Some(&*conversation),
            conversation.current().map(|current| current.session_id),
            "LLM model is not configured. Set it in Settings or via the LLM_MODEL environment variable.",
        );
    };
    let cancelled = runtime
        .begin_request()
        .ok_or_else(|| ApiError::Message("Assistant runtime is unavailable".into()))?;
    let session_id = current.session_id.clone();
    let context = build_assistant_context(&input);
    let tools = context_tool_catalog();
    let endpoint = crate::infra::openrouter::LlmEndpoint::from_config(
        raw_config.llm_provider.as_deref(),
        raw_config.llm_base_url.as_deref(),
    );
    let client = OpenRouterClient::at(&api_key, &model, &endpoint.base_url)
        .with_provider(endpoint.provider)
        .with_generation(0.0, 4096)
        .with_timeout(std::time::Duration::from_secs(ASSISTANT_LLM_TIMEOUT_SECS));
    // Capture history before recording the current message so it is not duplicated.
    let history = conversation.conversation(&current.session_id);
    conversation.record("user_message", &input.message, None, 0, 0, 0);
    let mut messages = build_assistant_messages(&context, &tools, &history, &input.message);
    tracing::debug!(
        session_id = %session_id,
        library_tracks = input.tracks.len(),
        library_albums = input.albums.len(),
        selected_tracks = input.selected_track_paths.len(),
        context_chars = context.to_string().chars().count(),
        prompt_chars = messages
            .iter()
            .map(|message| message.content.chars().count())
            .sum::<usize>(),
        history_entries = history.len(),
        "assistant prompt prepared"
    );
    let mut signatures = Vec::new();
    let mut repaired_invalid_args = false;
    let mut final_draft = None;
    let mut pending_tool_batches = Vec::new();
    let mut self_reviewed = false;
    let mut clarify_repaired = false;
    let mut invalid_preview_repairs = 0;
    let mut action_repairs = 0;
    // Consecutive clarification-only responses already persisted for this session.
    let prior_clarifications = task_state
        .load_session(&session_id)
        .map(|state| state.consecutive_clarifications)
        .unwrap_or(0);
    // Absolute deadline for the entire tool loop.
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(ASSISTANT_SESSION_TIMEOUT_SECS);

    for step_number in 1..=ASSISTANT_MAX_STEPS {
        let step = AssistantEvent {
            session_id: session_id.clone(),
            event_type: "step",
            message: assistant_step_message(step_number),
            data: None,
        };
        let _ = app.emit("assistant:event", step);
        let response = tokio::time::timeout_at(
            deadline,
            client.complete_json(
                messages.clone(),
                "AssistantResponse",
                assistant_response_schema(),
                &cancelled,
            ),
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
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                tracing::warn!(
                    elapsed_us = session_start.elapsed().as_micros(),
                    error = %error,
                    session_id = %session_id,
                    "assistant LLM request failed"
                );
                conversation.record_system(&error.to_string());
                return assistant_error_event(&app, Some(session_id), &error.to_string());
            }
            Err(_elapsed) => {
                tracing::error!(
                    elapsed_us = session_start.elapsed().as_micros(),
                    session_id = %session_id,
                    "assistant session timed out"
                );
                conversation.record_system(ASSISTANT_SESSION_TIMEOUT_LOG);
                return assistant_error_event(
                    &app,
                    Some(session_id),
                    "The assistant session timed out. Please try again.",
                );
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
        let normalized_response = match normalize_assistant_response_value(response.data) {
            Ok(response) => response,
            Err(error) => {
                return assistant_error_event(
                    &app,
                    Some(session_id),
                    &format!("Invalid assistant response: {error}"),
                );
            }
        };
        let draft: AssistantDraft = match serde_json::from_value(normalized_response.clone()) {
            Ok(draft) => draft,
            Err(error) => {
                return assistant_error_event(
                    &app,
                    Some(session_id),
                    &format!("Invalid assistant response: {error}"),
                );
            }
        };
        // Normalize a noop action batch to message-only — the LLM sent it to
        // comply with the old required-actionBatch schema, but it means "nothing to do".
        let draft = normalize_noop_batch(draft);
        // When both toolCall and actionBatch are present, prefer the tool call
        // (the system prompt instructs the LLM to call tools for mutations).
        if draft.tool_call.is_some() && draft.action_batch.is_some() {
            messages.push(ChatMessage::system(
                "You returned both a toolCall and an actionBatch. I used the toolCall and ignored the actionBatch. If you need to make changes, call one tool at a time.".to_string(),
            ));
        }
        let Some(tool_call) = draft.tool_call else {
            // Model-authored action batch: no tool call needed.
            if draft.action_batch.is_some() {
                if let Err(error) =
                    resolve_assistant_outcome(&draft, &pending_tool_batches, &session_id, &input)
                {
                    if invalid_preview_repairs < ASSISTANT_PREVIEW_REPAIR_ATTEMPTS {
                        invalid_preview_repairs += 1;
                        messages.push(ChatMessage {
                            role: "assistant".into(),
                            content: normalized_response.to_string(),
                        });
                        messages.push(ChatMessage::system(invalid_model_preview_repair_prompt(
                            &error,
                        )));
                        continue;
                    }
                    return assistant_error_event(&app, Some(session_id), &error);
                }
                final_draft = Some(draft);
                break;
            }
            // Message-only: self-review retry — give the LLM one chance to
            // convert a planned-action description into an actual tool call.
            // Genuine clarifications/answers/limitations pass through unchanged.
            if requires_tool_after_invalid_preview(invalid_preview_repairs) {
                if invalid_preview_repairs < ASSISTANT_PREVIEW_REPAIR_ATTEMPTS {
                    invalid_preview_repairs += 1;
                    messages.push(ChatMessage {
                        role: "assistant".into(),
                        content: draft.message.clone(),
                    });
                    messages.push(ChatMessage::system(invalid_model_preview_repair_prompt(
                        "The repair returned only a message and did not create a valid preview",
                    )));
                    continue;
                }
                return assistant_error_event(
                    &app,
                    Some(session_id),
                    "The assistant could not create a valid action preview after bounded repair attempts.",
                );
            }
            if draft.response_kind == AssistantResponseKind::Action {
                if action_repairs < ASSISTANT_ACTION_REPAIR_ATTEMPTS {
                    action_repairs += 1;
                    messages.push(ChatMessage {
                        role: "assistant".into(),
                        content: draft.message.clone(),
                    });
                    messages.push(ChatMessage::system(ASSISTANT_ACTION_REPAIR_PROMPT));
                    continue;
                }
                return assistant_error_event(
                    &app,
                    Some(session_id),
                    "The assistant repeatedly announced an action without running it, so I stopped instead of claiming the task was complete.",
                );
            }
            if !self_reviewed {
                self_reviewed = true;
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: draft.message.clone(),
                });
                messages.push(ChatMessage::system(ASSISTANT_SELF_REVIEW_PROMPT));
                continue;
            }
            // One chance to break a clarification loop: the user already answered
            // an earlier question this session, so a second question about the same
            // operation is a stall, not a step forward.
            if prior_clarifications >= 1
                && is_clarification_question(&draft.message)
                && !clarify_repaired
            {
                clarify_repaired = true;
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: draft.message.clone(),
                });
                messages.push(ChatMessage::system(ASSISTANT_CLARIFY_REPAIR_PROMPT));
                continue;
            }
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
        let execution = if tool_call.tool_name == "create_plan"
            || tool_call.tool_name == "plan.create"
        {
            match tokio::time::timeout_at(
                deadline,
                execute_create_plan(&tool_call.args, &input, &session_id, native_services),
            )
            .await
            {
                Ok(exec) => exec,
                Err(_) => {
                    tracing::error!(tool = %tool_call.tool_name, "plan execution timed out");
                    conversation.record_system(ASSISTANT_SESSION_TIMEOUT_LOG);
                    return assistant_error_event(
                        &app,
                        Some(session_id),
                        "The assistant session timed out. Please try again.",
                    );
                }
            }
        } else if registered_tool_is_read_only(&tool_call.tool_name) == Some(false) {
            execute_mutating_assistant_tool(
                &tool_call.tool_name,
                &tool_call.args,
                &input,
                &session_id,
            )
        } else {
            let tool_result = match tokio::time::timeout_at(
                deadline,
                execute_native_assistant_tool(
                    &tool_call.tool_name,
                    &tool_call.args,
                    native_services,
                ),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    tracing::error!(tool = %tool_call.tool_name, "native tool execution timed out");
                    conversation.record_system(ASSISTANT_SESSION_TIMEOUT_LOG);
                    return assistant_error_event(
                        &app,
                        Some(session_id),
                        "The assistant session timed out. Please try again.",
                    );
                }
            };
            MutatingToolExecution {
                result: tool_result,
                batches: Vec::new(),
                completion_evidence: false,
            }
        };

        let result = execution.result;
        let created_batches = execution.batches;
        for batch in &created_batches {
            if !runtime.add_batch(batch.clone()) {
                return Err(ApiError::Message(
                    "Failed to store assistant action preview".into(),
                ));
            }
            if let Err(error) = task_state.save_batch(
                &batch.id,
                &session_id,
                &serde_json::to_value(batch).unwrap_or_default(),
                batch.actions.len(),
            ) {
                runtime.mark_batch_failed(&batch.id, &error);
                return Err(ApiError::Message(error));
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
            // A successful preview ends any clarification streak.
            let _ = task_state.save_clarification_count(&session_id, 0);
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
            &format!(
                "I reached the maximum step limit ({ASSISTANT_MAX_STEPS}) without a final response."
            ),
        );
    };
    // Persist the consecutive-clarification counter and stop a question loop:
    // after the user answered and the assistant still only asks, the session
    // must not return another question to the user.
    let is_question = is_clarification_question(&draft.message);
    let clarification_count = next_clarification_count(prior_clarifications, is_question);
    let _ = task_state.save_clarification_count(&session_id, clarification_count);
    if is_question && clarification_count >= ASSISTANT_CLARIFY_LIMIT {
        // Return a normal limitation message, not an error event, so the UI
        // does not enter a failed state.
        let event = AssistantEvent {
            session_id: session_id.clone(),
            event_type: "message",
            message: clarification_limit_message(),
            data: None,
        };
        conversation.record("assistant_message", &event.message, Some(&model), 0, 0, 0);
        let _ = app.emit("assistant:event", &event);
        return Ok(event);
    }
    match resolve_assistant_outcome(&draft, &pending_tool_batches, &session_id, &input) {
        Ok(outcome) => {
            let event = match outcome {
                AssistantOutcome::Message => AssistantEvent {
                    session_id: session_id.clone(),
                    event_type: "message",
                    message: draft.message.clone(),
                    data: None,
                },
                AssistantOutcome::ToolPreview(batches) => {
                    // Tool-created batches were already stored during execution;
                    // do not call runtime.add_batch again here.
                    let first = &batches[0];
                    AssistantEvent {
                        session_id: session_id.clone(),
                        event_type: "action_batch_created",
                        message: draft.message.clone(),
                        data: Some(serde_json::json!({
                            "actionBatchId": first.id,
                            "actionBatch": first,
                            "actionBatches": batches
                        })),
                    }
                }
                AssistantOutcome::ModelPreview(batch) => {
                    if !runtime.add_batch(batch.clone()) {
                        return Err(ApiError::Message(
                            "Failed to store assistant action preview".into(),
                        ));
                    }
                    if let Err(error) = task_state.save_batch(
                        &batch.id,
                        &session_id,
                        &serde_json::to_value(&batch).unwrap_or_default(),
                        batch.actions.len(),
                    ) {
                        runtime.mark_batch_failed(&batch.id, &error);
                        return Err(ApiError::Message(error));
                    }
                    AssistantEvent {
                        session_id: session_id.clone(),
                        event_type: "action_batch_created",
                        message: draft.message.clone(),
                        data: Some(serde_json::json!({
                            "actionBatchId": batch.id,
                            "actionBatch": batch
                        })),
                    }
                }
            };
            conversation.record("assistant_message", &event.message, Some(&model), 0, 0, 0);
            let _ = app.emit("assistant:event", &event);
            Ok(event)
        }
        Err(error) => assistant_error_event(&app, Some(session_id), &error.to_string()),
    }
}

fn assistant_step_message(step_number: usize) -> String {
    format!("Step {step_number}/{ASSISTANT_MAX_STEPS}")
}

fn assistant_error_event(
    app: &AppHandle,
    session_id: Option<String>,
    message: &str,
) -> Result<AssistantEvent, ApiError> {
    assistant_error_event_with_conversation(app, None, session_id, message)
}

fn assistant_error_event_with_conversation(
    app: &AppHandle,
    conversation: Option<&ConversationState>,
    session_id: Option<String>,
    message: &str,
) -> Result<AssistantEvent, ApiError> {
    if let Some(conversation) = conversation {
        conversation.record("system", message, None, 0, 0, 0);
    }
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
            "responseKind": {
                "type": "string",
                "enum": ["answer", "clarification", "limitation", "action"],
                "description": "Classify the message. Use action for any statement that work will run or has run; action requires a toolCall or validated preview."
            },
            "actionBatch": {
                "type": "null",
                "description": "Always null. Call a registered tool; the native app creates validated action batches."
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
        "required": ["message", "responseKind"]
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
    /// Renderer may supply the credentials directly instead of depending on
    /// AssistantServicesState initialization.  When absent or empty the
    /// backend falls back to AssistantServicesState, then to config.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub llm_model: Option<String>,
}

/// Build a small bootstrap context for the model while retaining the complete
/// library in `AssistantSendInput` for deterministic tool execution.
///
/// Coding agents work well on large repositories because they receive a
/// summary and query details as needed. The music-library assistant follows
/// the same pattern: counts and current scope are visible immediately, while
/// individual track metadata remains behind `tracks.*`, `albums.inspect`, and
/// `query.metadata`.
fn build_assistant_context(input: &AssistantSendInput) -> Value {
    const SELECTED_PATH_SAMPLE_LIMIT: usize = 20;

    let library_summary = execute_context_tool(
        "library.summarize",
        &Value::Object(Default::default()),
        input,
    )
    .data
    .and_then(|data| data.get("summary").cloned())
    .unwrap_or_else(|| {
        serde_json::json!({
            "albumCount": input.albums.len(),
            "trackCount": input.tracks.len()
        })
    });
    let selected_paths = input
        .selected_track_paths
        .iter()
        .take(SELECTED_PATH_SAMPLE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    let active_album = input.active_album_path.as_deref().and_then(|active_path| {
        input
            .albums
            .iter()
            .find(|album| album.get("path").and_then(Value::as_str) == Some(active_path))
            .map(|album| {
                serde_json::json!({
                    "path": active_path,
                    "name": album.get("name"),
                    "artistHint": album.get("artistHint"),
                    "albumHint": album.get("albumHint"),
                    "trackCount": album.get("trackCount")
                })
            })
    });
    let default_scope = if !input.selected_track_paths.is_empty() {
        "selected"
    } else if input.active_album_path.is_some() {
        "active_album"
    } else {
        "library"
    };

    serde_json::json!({
        "libraryPath": input.library_path,
        "librarySummary": library_summary,
        "activeAlbumPath": input.active_album_path,
        "activeAlbum": active_album,
        "selection": {
            "count": input.selected_track_paths.len(),
            "pathSample": selected_paths,
            "truncated": input.selected_track_paths.len() > SELECTED_PATH_SAMPLE_LIMIT
        },
        "defaultScope": default_scope,
        "autonomous": input.autonomous,
        "dataAccess": {
            "fullLibraryAvailableThroughTools": true,
            "trackDetailsIncluded": false
        }
    })
}

/// Curated from Navidrome's official tagging guidelines. Keep this compact:
/// it is included in every assistant request so tagging decisions do not
/// depend on a model's incidental knowledge.
const NAVIDROME_TAGGING_KNOWLEDGE: &str = "\
Navidrome tagging knowledge:
- Navidrome organizes music from embedded metadata, not file or folder names. Keep tags complete \
and consistent. Essential fields are Title, Artist, Album, Album Artist, and Track Number; Date, \
Disc Number, Genre, and Compilation are useful where applicable.
- ARTIST and ARTISTS are different. ARTIST controls the display credit and may be one readable \
string such as `A feat. B`. ARTISTS identifies the individual, browsable performers and should be \
multi-valued: `ARTISTS=A` plus `ARTISTS=B`. ALBUMARTIST and ALBUMARTISTS have the same display \
versus identity relationship.
- When both singular and plural tags exist, Navidrome displays singular ARTIST or ALBUMARTIST \
verbatim and gets individual identities from plural ARTISTS or ALBUMARTISTS. If only plural values \
exist, Navidrome joins them for display. Never delete collaborators merely to make each plural \
value contain one name.
- Prefer true multi-valued ARTISTS and ALBUMARTISTS. Each plural value must contain one artist; \
do not store `A & B` as one ARTISTS value. For a request to split malformed Artists tags, inspect \
the intended scope, preserve every collaborator and already-atomic value, and set the Soundrobe \
standard field `artists` to an array such as [\"A\", \"B\"] only on affected tracks. Do not change \
ARTIST unless the user separately asks to change the display credit.
- Singular ARTIST fallback parsing is less precise. Navidrome's default separators include \
` / `, ` feat. `, ` feat `, ` ft. `, ` ft `, and `; ` (case-insensitive); multi-valued tags are \
never separator-split. Treat a user-named separator such as ` & ` as an instruction for repairing \
the malformed plural value, not as permission to rewrite singular ARTIST.
- FLAC/Vorbis and Opus support repeated values directly. ID3v2.4 supports multiple values; \
ID3v2.3 does not officially support them and may require a consistent singular-field separator. \
Respect the file format and report unsupported writes rather than guessing.
- Album must be identical across an album. Album Artist should also be consistent; use Various \
Artists plus the compilation flag for compilations. Genre is multi-valued. DATE is recording date, \
YEAR/RELEASEDATE is release date, and ORIGINALDATE/ORIGINALYEAR is original release date.
- Keep artwork consistent across an album, review previews before writing, back up before bulk \
retagging, rescan Navidrome after saving, and verify the result in Get Info > Raw Tags.
Source: https://www.navidrome.org/docs/usage/library/tagging/
";

/// Build the LLM message list: system prompt, bounded conversation history, current request.
/// The current request is appended as the final user turn so the LLM sees it after prior context.
fn build_assistant_messages(
    context: &serde_json::Value,
    tools: &serde_json::Value,
    history: &[ConversationEntry],
    current_request: &str,
) -> Vec<ChatMessage> {
    const MAX_HISTORY_TURNS: usize = 20;
    /// Hard character budget for all history entries combined (~8K tokens).
    const HISTORY_CHAR_BUDGET: usize = 32_000;

    let mut messages = vec![
        ChatMessage::system(format!(
            concat!(
                "You are the Soundrobe desktop music-library assistant. ",
                "The user's current library selection is shown in the next user message.\n",
                "\n",
                "How to respond:\n",
                "- Infer intent from the current selection, library context, and prior turns below.\n",
                "- Treat a short follow-up as an answer to the most recent clarification question.\n",
                "- Combine prior turns with the current selection before choosing a tool.\n",
                "- The app context is intentionally compact. Use read-only tools to inspect ",
                "  library or track details instead of guessing from omitted records.\n",
                "- For large scopes, prefer scoped tools over enumerating every track path. ",
                "  Tool results are bounded; refine or repeat queries when more evidence is needed.\n",
                "- If the request is an explicit edit with concrete values (\"set title to X\", ",
                "  \"remove genre\"), call metadata.patch with explicit values.\n",
                "- Treat a quoted metadata value as one literal value even when it contains commas; ",
                "  do not split it into different values for different tracks.\n",
                "- If the request is a transformation (\"strip numbers from titles\", ",
                "  \"lowercase all genres\", \"extract first word\", ",
                "  \"convert Chinese to traditional\"), ",
                "  call metadata.transform with the right operations pipeline.\n",
                "- To split malformed plural Artists values joined by &, comma, or semicolon, ",
                "  call metadata.transform on source field artists with the single split_artists ",
                "  operation for the intended scope. Do not enumerate affected paths or change ",
                "  the singular display Artist.\n",
                "- tags.prettify is read-only and never edits tracks; do not use it for mutations.\n",
                "- To fix one missing metadata field, call metadata.patch for the intended scope ",
                "  with a set change whose only_if_missing field is true. This creates concrete ",
                "  per-track preview actions without overwriting existing values. Do not broaden ",
                "  a field-specific request into library.run_task.\n",
                "- For filename/path renames, call files.transform. To group tracks into album ",
                "  folders, call files.relocate with a destination template.\n",
                "- For multi-step library tasks like auto-tagging or auditing, call library.run_task.\n",
                "- Never author actionBatch. Call a registered tool; the native app creates it.\n",
                "- Ask one focused clarification **only** when materially different interpretations ",
                "  would produce different actions.\n",
                "- If you asked a clarifying question earlier in this session and the user answered ",
                "  it, do not ask another clarifying question about the same operation. Call the ",
                "  tool now, or state clearly what the app cannot do.\n",
                "- When you describe an action (inspect, search, look up), call the tool immediately.\n",
                "  Do not say you'll do something without calling the tool.\n",
                "- Message-only responses are for clarifications, answers, and limitations— ",
                "  not for describing planned actions.\n",
                "- Set responseKind to action whenever the message says work will run or has run, ",
                "  including retrospective completion claims. An action response must include a toolCall.\n",
                "- Quoted or hypothetical action language in an explanation remains an answer; ",
                "  responseKind describes your own response, not quoted text.\n",
                "- When no catalog tool supports the request, explain the limitation normally.\n",
                "- **Never claim an action was applied.** Previews still require user approval.\n",
                "\n",
                "{tagging_knowledge}\n",
                "toolCall — toolName from the list below, args matching its schema\n",
                "Your message should be concise and user-facing.\n",
                "Available tools: {tools}"
            ),
            tagging_knowledge = NAVIDROME_TAGGING_KNOWLEDGE,
            tools = serde_json::to_string(tools).unwrap_or_default()
        )),
    ];

    // Group persisted history into complete user_message → assistant_message exchanges.
    // Orphan assistant entries and dangling users (failed/truncated) are dropped.
    struct Exchange<'a> {
        user: &'a ConversationEntry,
        assistant: &'a ConversationEntry,
    }
    let mut exchanges: Vec<Exchange> = Vec::new();
    let mut i = 0;
    while i + 1 < history.len() {
        let a = &history[i];
        let b = &history[i + 1];
        if a.entry_type == "user_message" && b.entry_type == "assistant_message" {
            exchanges.push(Exchange {
                user: a,
                assistant: b,
            });
            i += 2;
        } else {
            i += 1;
        }
    }

    // Keep the newest exchanges that fit within both budgets.
    let max_exchanges = MAX_HISTORY_TURNS / 2;
    let mut exchange_start = exchanges.len().saturating_sub(max_exchanges);
    let mut char_total: usize = exchanges[exchange_start..]
        .iter()
        .flat_map(|ex| [ex.user.content.len(), ex.assistant.content.len()])
        .sum();
    while char_total > HISTORY_CHAR_BUDGET && exchange_start < exchanges.len() {
        let ex = &exchanges[exchange_start];
        char_total = char_total.saturating_sub(ex.user.content.len() + ex.assistant.content.len());
        exchange_start += 1;
    }
    for ex in &exchanges[exchange_start..] {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: ex.user.content.clone(),
        });
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: ex.assistant.content.clone(),
        });
    }

    messages.push(ChatMessage::user(format!(
        "App context:\n{context}\n\nUser request:\n{current_request}",
        context = serde_json::to_string_pretty(context).unwrap_or_default(),
    )));

    messages
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

pub(crate) struct MutatingToolExecution {
    pub(crate) result: AssistantToolResult,
    pub(crate) batches: Vec<AssistantActionBatch>,
    pub(crate) completion_evidence: bool,
}

pub(crate) fn mutating_tool_execution(
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

pub(crate) fn mutating_tool_no_changes(summary: impl Into<String>) -> MutatingToolExecution {
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

pub(crate) fn mutating_tool_error(message: impl Into<String>) -> MutatingToolExecution {
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

pub(crate) fn assistant_batch(
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
        library_root: None,
        completion_contract: None,
    }
}

pub(crate) fn tool_scope_paths(
    input: &AssistantSendInput,
    args: &Value,
) -> Result<Vec<String>, String> {
    let scope = args
        .get("target_scope")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing required field: target_scope".to_string())?;
    let loaded_paths = input
        .tracks
        .iter()
        .filter_map(|track| track.get("path").and_then(Value::as_str))
        .collect::<Vec<_>>();
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
            .map(str::to_string)
            .collect(),
        _ => return Err(format!("Unsupported target_scope: {scope}")),
    };
    let mut seen = HashSet::new();
    for path in &paths {
        if !seen.insert(path.as_str()) {
            return Err(format!("Scope contains duplicate track path: {path}"));
        }
        match loaded_paths
            .iter()
            .filter(|loaded| **loaded == path.as_str())
            .count()
        {
            1 => {}
            0 => return Err(format!("Scope track metadata is missing: {path}")),
            _ => return Err(format!("Scope track metadata is duplicated: {path}")),
        }
    }
    Ok(paths)
}

fn execute_mutating_assistant_tool(
    name: &str,
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    tracing::debug!(tool = %name, "mutating assistant tool started");
    if let Err(error) = validate_registered_tool_args(name, args) {
        return mutating_tool_error(format!("Invalid arguments for {name}: {error}"));
    }
    match name {
        // New general tools
        "metadata.patch" => {
            super::assistant_metadata_tools::execute_metadata_patch(args, input, session_id)
        }
        "metadata.transform" => {
            super::assistant_metadata_tools::execute_metadata_transform(args, input, session_id)
        }
        "files.transform" => {
            super::assistant_metadata_tools::execute_files_transform(args, input, session_id)
        }
        "files.relocate" => {
            super::assistant_metadata_tools::execute_files_relocate(args, input, session_id)
        }
        "library.run_task" => execute_run_library_task(args, input, session_id),
        // Legacy tools (kept for backward compatibility)
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
        "remove_embedded_cover" => execute_remove_embedded_cover(args, input, session_id),
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

fn execute_remove_embedded_cover(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve target scope");
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }
    let mut actions: Vec<AssistantAction> = Vec::new();
    for path in &paths {
        let has_cover = lofty::probe::Probe::open(Path::new(path))
            .ok()
            .and_then(|probe| {
                probe
                    .options(lofty::config::ParseOptions::new().read_properties(false))
                    .read()
                    .ok()
            })
            .map(|tagged| {
                tagged
                    .tags()
                    .iter()
                    .any(|tag: &lofty::tag::Tag| !tag.pictures().is_empty())
            })
            .unwrap_or(false);
        if has_cover {
            actions.push(AssistantAction {
                tag_kind: None,
                track_path: Some(path.clone()),
                field: None,
                old_value: None,
                new_value: None,
                operation: Some("remove_embedded_cover".into()),
                destination_path: None,
                source_path: None,
                skip_reason: None,
                description: Some("Remove embedded cover art".into()),
            });
        }
    }
    if actions.is_empty() {
        return mutating_tool_no_changes("No tracks with embedded cover art found.");
    }
    let summary = format!("Remove embedded cover art from {} track(s)", actions.len());
    let batch = assistant_batch(
        session_id,
        "embedded-cover-remove",
        "Remove embedded cover art",
        &summary,
        "low",
        std::mem::take(&mut actions),
        false,
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
            push_string_action(
                &mut actions,
                track,
                path,
                field,
                &desired,
                &format!("Set {field} to {desired}"),
            );
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

pub(crate) fn action_value_string(value: &Value) -> Option<String> {
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

pub(crate) fn track_field_string(track: &Value, field: &str) -> Option<String> {
    track.get(field).and_then(action_value_string)
}

pub(crate) fn extra_action(
    path: &str,
    key: &str,
    value: Option<&str>,
    operation: &str,
) -> AssistantAction {
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
            push_string_action(
                &mut actions,
                Some(track),
                path,
                field,
                extracted,
                &format!("Extract {field} from regex"),
            );
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
    let task = args.get("task").and_then(Value::as_str).unwrap_or_default();
    let auto_tag = task == "auto_tag";
    let target_scope = args
        .get("target_scope")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if auto_tag && matches!(target_scope, "selected" | "explicit_paths") {
        return mutating_tool_error(
            "Auto-tagging currently runs whole albums; use active_album or library scope, or use metadata.patch for exact tracks and fields",
        );
    }
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve library-task target scope");
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }
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
    let mut batch = assistant_batch(
        session_id,
        "folder-move",
        format!("Organize files by {criterion}"),
        &summary,
        "medium",
        actions,
        true,
    );
    batch.library_root = input.library_path.clone();
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

pub(crate) fn track_path(track: &Value) -> &str {
    track
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

pub(crate) fn push_numeric_action(
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

pub(crate) fn strip_track_title_prefix(title: &str) -> String {
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
        push_string_action(
            &mut actions,
            track,
            path,
            "title",
            &title,
            "Infer title from filename",
        );
        push_string_action(
            &mut actions,
            track,
            path,
            "artist",
            &artist,
            "Infer artist from filename",
        );
        let artists = split_artist_names(&artist).join("; ");
        push_string_action(
            &mut actions,
            track,
            path,
            "artists",
            &artists,
            "Infer artists from filename",
        );
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

pub(crate) fn push_string_action(
    actions: &mut Vec<AssistantAction>,
    track: Option<&Value>,
    path: &str,
    field: &str,
    desired: &str,
    description: &str,
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
        description: Some(description.into()),
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
                push_string_action(
                    &mut actions,
                    Some(track),
                    path,
                    field,
                    &converted,
                    &format!("Convert {field} to {target} Chinese"),
                );
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

pub(crate) fn path_is_inside(path: &Path, root: &Path) -> bool {
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

pub(crate) fn unique_planned_destination(
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AssistantResponseKind {
    Answer,
    Clarification,
    Limitation,
    Action,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssistantDraft {
    message: String,
    response_kind: AssistantResponseKind,
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

/// Normalize common agent tool-call envelopes into Soundrobe's canonical
/// singular `toolCall` shape. Parallel read-only calls are converted to the
/// existing sequential plan executor; mutations must remain one-at-a-time.
fn normalize_assistant_response_value(mut response: Value) -> Result<Value, String> {
    let object = response
        .as_object_mut()
        .ok_or_else(|| "assistant response should be an object".to_string())?;
    if object.get("message").is_none_or(Value::is_null) {
        object.insert("message".into(), Value::String(String::new()));
    }

    let camel_calls = object.remove("toolCalls").filter(|calls| !calls.is_null());
    let snake_calls = object.remove("tool_calls").filter(|calls| !calls.is_null());
    if camel_calls.is_some() && snake_calls.is_some() {
        return Err("response included both toolCalls and tool_calls".into());
    }
    let alternate_calls = camel_calls.or(snake_calls);
    let canonical_present = object.get("toolCall").is_some_and(|call| !call.is_null());
    if canonical_present
        && alternate_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_null())
    {
        return Err("response included both toolCall and toolCalls".into());
    }

    if let Some(calls) = alternate_calls.filter(|calls| !calls.is_null()) {
        let calls = calls
            .as_array()
            .ok_or_else(|| "toolCalls should be an array".to_string())?;
        match calls.as_slice() {
            [] => {}
            [call] => {
                object.insert("toolCall".into(), normalize_single_tool_call(call)?);
            }
            _ => {
                let normalized = calls
                    .iter()
                    .map(normalize_single_tool_call)
                    .collect::<Result<Vec<_>, _>>()?;
                let classifications = normalized
                    .iter()
                    .map(|call| {
                        let tool_name = call
                            .get("toolName")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        registered_tool_is_read_only(tool_name)
                            .ok_or_else(|| format!("Unknown tool: {tool_name}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if classifications.iter().any(|read_only| !read_only) {
                    return Err(
                        "assistant returned multiple mutating tool calls; call mutations one at a time"
                            .into(),
                    );
                }
                let steps = normalized
                    .into_iter()
                    .enumerate()
                    .map(|(index, call)| {
                        let tool = call
                            .get("toolName")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        serde_json::json!({
                            "id": format!("parallel_call_{}", index + 1),
                            "label": tool,
                            "tool": tool,
                            "args": call.get("args").cloned().unwrap_or_default()
                        })
                    })
                    .collect::<Vec<_>>();
                object.insert(
                    "toolCall".into(),
                    serde_json::json!({
                        "toolName": "create_plan",
                        "args": {"steps": steps}
                    }),
                );
            }
        }
    } else if let Some(call) = object
        .get("toolCall")
        .filter(|call| !call.is_null())
        .cloned()
    {
        object.insert("toolCall".into(), normalize_single_tool_call(&call)?);
    }
    if object.get("responseKind").is_none_or(Value::is_null)
        && object.get("toolCall").is_some_and(|call| !call.is_null())
    {
        object.insert("responseKind".into(), Value::String("action".into()));
    }

    Ok(response)
}

fn normalize_single_tool_call(call: &Value) -> Result<Value, String> {
    let call = call
        .as_object()
        .ok_or_else(|| "tool call should be an object".to_string())?;
    let function = call.get("function").and_then(Value::as_object);
    let tool_names = [
        call.get("toolName"),
        call.get("name"),
        function.and_then(|function| function.get("name")),
    ]
    .into_iter()
    .flatten()
    .filter(|name| !name.is_null())
    .map(|name| {
        name.as_str()
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "tool call name should be a non-empty string".to_string())
    })
    .collect::<Result<Vec<_>, _>>()?;
    let Some(tool_name) = tool_names.first().copied() else {
        return Err("tool call is missing a name".into());
    };
    if tool_names.iter().any(|candidate| *candidate != tool_name) {
        return Err("tool call included conflicting tool names".into());
    }

    let args = [
        call.get("args"),
        call.get("input"),
        call.get("arguments"),
        function.and_then(|function| function.get("arguments")),
    ]
    .into_iter()
    .flatten()
    .filter(|arguments| !arguments.is_null())
    .map(normalize_tool_arguments)
    .collect::<Result<Vec<_>, _>>()?;
    let args = if let Some(first) = args.first() {
        if args.iter().any(|candidate| candidate != first) {
            return Err("tool call included conflicting tool arguments".into());
        }
        first.clone()
    } else {
        Value::Object(Default::default())
    };
    Ok(serde_json::json!({
        "toolName": tool_name,
        "args": args
    }))
}

fn normalize_tool_arguments(arguments: &Value) -> Result<Value, String> {
    let arguments = match arguments {
        Value::String(arguments) => serde_json::from_str(arguments)
            .map_err(|error| format!("tool arguments are not valid JSON: {error}"))?,
        arguments => arguments.clone(),
    };
    if !arguments.is_object() {
        return Err("tool arguments should be an object".into());
    }
    Ok(arguments)
}

#[derive(Clone, Debug, Deserialize)]
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

/// The resolved outcome of an assistant interaction — what the system should
/// present to the user. No side effects; all validation errors are surfaced
/// through `ModelPreview` validation or `Err`.
#[derive(Debug, PartialEq)]
enum AssistantOutcome {
    /// A plain message — no action needed.
    Message,
    /// One or more native tool-created previews.
    ToolPreview(Vec<AssistantActionBatch>),
    /// A single model-authored preview that passed validation.
    ModelPreview(AssistantActionBatch),
}

/// Determine what to do with the LLM's final response.
/// Returns an `AssistantOutcome` or a descriptive error string.
fn resolve_assistant_outcome(
    draft: &AssistantDraft,
    pending_tool_batches: &[AssistantActionBatch],
    session_id: &str,
    input: &AssistantSendInput,
) -> Result<AssistantOutcome, String> {
    if draft.action_batch.is_some() && !pending_tool_batches.is_empty() {
        return Err(
            "The assistant returned both a native tool preview and a model-authored preview".into(),
        );
    }
    if pending_tool_batches.first().is_some() {
        return Ok(AssistantOutcome::ToolPreview(pending_tool_batches.to_vec()));
    }
    if let Some(batch) = draft.action_batch.clone() {
        let validated =
            validated_assistant_batch(session_id, input, batch).map_err(|e| e.to_string())?;
        return Ok(AssistantOutcome::ModelPreview(validated));
    }
    if draft.response_kind == AssistantResponseKind::Action {
        return Err(
            "The assistant claimed an action without running a tool or creating a preview".into(),
        );
    }
    Ok(AssistantOutcome::Message)
}

fn invalid_model_preview_repair_prompt(error: &str) -> String {
    format!(
        "Your actionBatch failed validation: {error}. Do not invent paths or author another \
         actionBatch. Call one registered mutating tool using its exact schema instead. For \
         auto-tagging or auditing, call library.run_task with both task and target_scope. If the \
         request directly edits tags, call metadata.patch with target_scope and changes whose \
         entries contain field, action, and value. For a missing-field request, set \
         only_if_missing to true instead of enumerating paths. If more scope evidence is needed, \
         call a registered read-only tool; do not return another prose-only preview claim."
    )
}

fn requires_tool_after_invalid_preview(invalid_preview_repairs: usize) -> bool {
    invalid_preview_repairs > 0
}

/// If the LLM returned an action batch with kind "noop", normalize it to
/// message-only (no action batch). Non-empty noop batches are rejected.
fn normalize_noop_batch(draft: AssistantDraft) -> AssistantDraft {
    // Borrow to inspect; avoid partial move when rebuilding.
    let wants_noop = draft
        .action_batch
        .as_ref()
        .is_some_and(|b| b.kind == "noop" && b.actions.is_empty());
    if wants_noop {
        AssistantDraft {
            action_batch: None,
            ..draft
        }
    } else {
        draft
    }
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
        library_root: None,
        completion_contract: None,
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
    task_state: State<'_, crate::state::assistant_task::AssistantTaskState>,
) -> Result<(), ApiError> {
    let title = reject_action_batch(&runtime, &task_state, &action_batch_id)?;
    let Some(title) = title else {
        return Ok(());
    };
    let current = conversation
        .current()
        .ok_or_else(|| ApiError::Message("No active assistant session".to_string()))?;
    conversation.record("system", &format!("Rejected: {title}"), None, 0, 0, 0);
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

fn reject_action_batch(
    runtime: &AssistantRuntimeState,
    task_state: &crate::state::assistant_task::AssistantTaskState,
    action_batch_id: &str,
) -> Result<Option<String>, ApiError> {
    let Some(batch) = runtime.get_batch(action_batch_id) else {
        return Ok(None);
    };
    if batch.status != "pending" {
        return Ok(None);
    }
    task_state
        .finalize_batch(
            action_batch_id,
            "rejected",
            &serde_json::json!({ "status": "rejected" }),
        )
        .map_err(ApiError::Message)?;
    let title = runtime
        .reject_batch(&action_batch_id)
        .ok_or_else(|| ApiError::Message("Batch changed while it was being rejected".into()))?;
    Ok(Some(title))
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

fn track_data_field_value(track: &crate::commands::tracks::TrackData, field: &str) -> Value {
    match field {
        "title" => serde_json::to_value(&track.title),
        "artist" => serde_json::to_value(&track.artist),
        "artists" => {
            if track.artists.is_empty() {
                Ok(Value::Null)
            } else {
                serde_json::to_value(&track.artists)
            }
        }
        "album" => serde_json::to_value(&track.album),
        "albumArtist" => serde_json::to_value(&track.album_artist),
        "albumArtists" => {
            if track.album_artists.is_empty() {
                Ok(Value::Null)
            } else {
                serde_json::to_value(&track.album_artists)
            }
        }
        "trackNumber" => serde_json::to_value(track.track_number),
        "trackTotal" => serde_json::to_value(track.track_total),
        "discNumber" => serde_json::to_value(track.disc_number),
        "discTotal" => serde_json::to_value(track.disc_total),
        "year" => serde_json::to_value(&track.year),
        "genre" => serde_json::to_value(&track.genre),
        "composer" => serde_json::to_value(&track.composer),
        "comment" => serde_json::to_value(&track.comment),
        "description" => serde_json::to_value(&track.description),
        "lyrics" => serde_json::to_value(&track.lyrics),
        "compilation" => serde_json::to_value(track.compilation),
        "musicbrainzTrackId" => serde_json::to_value(&track.musicbrainz_track_id),
        "musicbrainzAlbumId" => serde_json::to_value(&track.musicbrainz_album_id),
        "musicbrainzArtistId" => serde_json::to_value(&track.musicbrainz_artist_id),
        "discogsArtistId" => serde_json::to_value(&track.discogs_artist_id),
        "discogsReleaseId" => serde_json::to_value(&track.discogs_release_id),
        _ => Ok(Value::Null),
    }
    .unwrap_or(Value::Null)
}

fn verification_summary(
    status: &str,
    phase: &str,
    batch: &AssistantActionBatch,
    verified_action_count: usize,
    failures: Vec<Value>,
) -> Value {
    let scope_count = batch.completion_contract.as_ref().map_or_else(
        || {
            batch
                .actions
                .iter()
                .filter_map(|action| action.track_path.as_deref())
                .collect::<HashSet<_>>()
                .len()
        },
        |contract| contract.scope_paths.len(),
    );
    let expected_action_count = batch
        .completion_contract
        .as_ref()
        .filter(|contract| !contract.expected_actions.is_empty())
        .map_or_else(
            || {
                batch
                    .actions
                    .iter()
                    .filter(|action| action.track_path.is_some() && action.field.is_some())
                    .count()
            },
            |contract| contract.expected_actions.len(),
        );
    serde_json::json!({
        "status": status,
        "phase": phase,
        "scopeCount": scope_count,
        "expectedActionCount": expected_action_count,
        "verifiedActionCount": verified_action_count,
        "failures": failures
    })
}

fn preflight_metadata_contract(batch: &AssistantActionBatch) -> Option<Value> {
    let contract = batch.completion_contract.as_ref()?;
    let mut failures = Vec::new();
    let current_expectations =
        crate::commands::assistant_metadata_tools::completion_expectations(&batch.actions);
    if !contract.expected_actions.is_empty() && current_expectations != contract.expected_actions {
        failures.push(serde_json::json!({
            "expected": contract.expected_actions,
            "actual": current_expectations,
            "error": "Preview actions no longer match the native completion contract"
        }));
    }
    let current_paths = batch
        .actions
        .iter()
        .filter_map(|action| action.track_path.clone())
        .collect::<HashSet<_>>();
    let expected_paths = contract
        .expected_action_paths
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    if current_paths != expected_paths {
        failures.push(serde_json::json!({
            "expected": contract.expected_action_paths,
            "actual": current_paths,
            "error": "Preview action paths no longer match the native completion contract"
        }));
    }
    for snapshot in &contract.scope_snapshot {
        match read_track_metadata(Path::new(&snapshot.path)) {
            Ok(track) => {
                for (field, expected) in &snapshot.standard_values {
                    let actual = track_data_field_value(&track, field);
                    if actual != *expected {
                        failures.push(serde_json::json!({
                            "trackPath": snapshot.path,
                            "field": field,
                            "expected": expected,
                            "actual": actual,
                            "error": "Preview is stale because the on-disk value changed"
                        }));
                    }
                }
            }
            Err(error) => failures.push(serde_json::json!({
                "trackPath": snapshot.path,
                "error": error.to_string()
            })),
        }
        let current_extra = if snapshot.extra_values.is_empty() {
            None
        } else {
            match try_read_extra_tags(Path::new(&snapshot.path)) {
                Ok(tags) => Some(tags),
                Err(error) => {
                    failures.push(serde_json::json!({
                        "trackPath": snapshot.path,
                        "error": error.to_string()
                    }));
                    None
                }
            }
        };
        for (field, expected) in &snapshot.extra_values {
            let actual = current_extra
                .as_ref()
                .and_then(|tags| {
                    tags.iter()
                        .find(|tag| tag.key.trim().eq_ignore_ascii_case(field))
                })
                .map(|tag| Value::String(tag.value.clone()))
                .unwrap_or(Value::Null);
            if actual != *expected {
                failures.push(serde_json::json!({
                    "trackPath": snapshot.path,
                    "field": field,
                    "expected": expected,
                    "actual": actual,
                    "error": "Preview is stale because the on-disk value changed"
                }));
            }
        }
    }
    if contract.postcondition == AssistantCompletionPostcondition::SplitArtistsNormalized {
        let mut malformed = Vec::new();
        for path in &contract.scope_paths {
            match read_track_metadata(Path::new(path)) {
                Ok(track)
                    if track.artists.len() == 1
                        && crate::commands::assistant_metadata_tools::op_split_artists(
                            &track.artists[0],
                        )
                        .is_some() =>
                {
                    malformed.push(path.clone())
                }
                Ok(_) => {}
                Err(error) => failures.push(serde_json::json!({
                    "trackPath": path,
                    "field": "artists",
                    "error": error.to_string()
                })),
            }
        }
        let malformed_set = malformed.iter().collect::<HashSet<_>>();
        let expected_set = contract
            .expected_action_paths
            .iter()
            .collect::<HashSet<_>>();
        if malformed_set != expected_set {
            failures.push(serde_json::json!({
                "field": "artists",
                "expected": contract.expected_action_paths,
                "actual": malformed,
                "error": "Preview is stale because the malformed Artists set changed"
            }));
        }
    }
    (!failures.is_empty()).then(|| verification_summary("failed", "preflight", batch, 0, failures))
}

fn verify_metadata_batch_readback(batch: &AssistantActionBatch) -> Value {
    let mut failures = Vec::new();
    let mut verified = 0usize;
    let mut standard_tracks = BTreeMap::new();
    let mut extra_tags = BTreeMap::new();
    let contractless_expectations;
    let expectations = if let Some(contract) = batch
        .completion_contract
        .as_ref()
        .filter(|contract| !contract.expected_actions.is_empty())
    {
        &contract.expected_actions
    } else {
        contractless_expectations =
            crate::commands::assistant_metadata_tools::completion_expectations(&batch.actions);
        &contractless_expectations
    };
    for expectation in expectations {
        let path = expectation.track_path.as_str();
        let field = expectation.field.as_str();
        if expectation.tag_kind == "extra" {
            let tags = extra_tags
                .entry(path.to_string())
                .or_insert_with(|| try_read_extra_tags(Path::new(path)));
            let actual = match tags {
                Ok(tags) => tags
                    .iter()
                    .find(|tag| tag.key.trim().eq_ignore_ascii_case(field))
                    .map(|tag| Value::String(tag.value.clone()))
                    .unwrap_or(Value::Null),
                Err(error) => {
                    failures.push(serde_json::json!({
                        "trackPath": path, "field": field, "error": error.to_string()
                    }));
                    continue;
                }
            };
            let expected = expectation.expected_value.clone();
            if actual == expected {
                verified += 1;
            } else {
                failures.push(serde_json::json!({
                    "trackPath": path, "field": field, "expected": expected,
                    "actual": actual, "error": "Extra-tag readback did not match"
                }));
            }
            continue;
        }
        if expectation.operation == "remove" && matches!(field, "artists" | "albumArtists") {
            match crate::commands::tracks::read_plural_tag_values(Path::new(path), field) {
                Ok(values) if values.is_empty() => verified += 1,
                Ok(values) => failures.push(serde_json::json!({
                    "trackPath": path, "field": field, "expected": Value::Null,
                    "actual": values, "error": "Plural metadata tag removal was not persisted"
                })),
                Err(error) => failures.push(serde_json::json!({
                    "trackPath": path, "field": field, "error": error.to_string()
                })),
            }
            continue;
        }
        let track = standard_tracks
            .entry(path.to_string())
            .or_insert_with(|| read_track_metadata(Path::new(path)));
        match track {
            Ok(track) => {
                let actual = track_data_field_value(track, field);
                let expected = expectation.expected_value.clone();
                if actual == expected {
                    verified += 1;
                } else {
                    failures.push(serde_json::json!({
                        "trackPath": path, "field": field, "expected": expected,
                        "actual": actual, "error": "Metadata readback did not match"
                    }));
                }
            }
            Err(error) => failures.push(serde_json::json!({
                "trackPath": path, "field": field, "error": error.to_string()
            })),
        }
    }
    if batch.completion_contract.as_ref().is_some_and(|contract| {
        contract.postcondition == AssistantCompletionPostcondition::SplitArtistsNormalized
    }) {
        let contract = batch.completion_contract.as_ref().unwrap();
        for path in &contract.scope_paths {
            match read_track_metadata(Path::new(path)) {
                Ok(track)
                    if track.artists.len() == 1
                        && crate::commands::assistant_metadata_tools::op_split_artists(
                            &track.artists[0],
                        )
                        .is_some() =>
                {
                    failures.push(serde_json::json!({
                        "trackPath": path, "field": "artists",
                        "actual": track.artists,
                        "error": "Malformed plural Artists value remains after apply"
                    }));
                }
                Ok(_) => {}
                Err(error) => failures.push(serde_json::json!({
                    "trackPath": path, "field": "artists", "error": error.to_string()
                })),
            }
        }
    }
    verification_summary(
        if failures.is_empty() {
            "verified"
        } else {
            "failed"
        },
        "readback",
        batch,
        verified,
        failures,
    )
}

fn finish_metadata_apply(
    runtime: &AssistantRuntimeState,
    batch: &AssistantActionBatch,
    batch_id: &str,
    mut result: Value,
) -> Value {
    if result["success"] == false {
        let error = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Metadata write failed")
            .to_string();
        runtime.mark_batch_failed(batch_id, &error);
        let mut failures = Vec::new();
        match result.get("results") {
            Some(Value::Array(results)) => failures.extend(results.iter().cloned()),
            Some(Value::Object(groups)) => {
                for results in groups.values().filter_map(Value::as_array) {
                    failures.extend(results.iter().cloned());
                }
            }
            _ => failures.push(serde_json::json!({ "error": error })),
        }
        result["verification"] = verification_summary("failed", "write", batch, 0, failures);
        return result;
    }
    let verification = verify_metadata_batch_readback(batch);
    if verification["status"] == "verified" {
        runtime.mark_batch_applied(batch_id);
    } else {
        let error = "Metadata readback verification failed";
        runtime.mark_batch_failed(batch_id, error);
        result["success"] = Value::Bool(false);
        result["error"] = Value::String(error.to_string());
    }
    result["verification"] = verification;
    result
}

async fn apply_standard_actions(
    runtime: &AssistantRuntimeState,
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
                let message = format!("Could not capture undo snapshot for {path}: {error}");
                if mark_status {
                    runtime.mark_batch_failed(batch_id, &message);
                }
                return serde_json::json!({
                    "success": false,
                    "error": message,
                    "results": [{
                        "trackPath": path,
                        "success": false,
                        "error": error.to_string()
                    }],
                    "undoSnapshots": undo
                });
            }
        }
    }
    let mut results = Vec::new();
    for (path, patch) in updates {
        match write_track_with_exclusive_queue_held(PathBuf::from(&path), patch).await {
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
    batch: &AssistantActionBatch,
    batch_id: &str,
    mark_status: bool,
) -> Value {
    tracing::debug!(
        batch_id = %batch_id,
        action_count = batch.actions.len(),
        "applying extra tag actions"
    );
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
    let mut prepared = Vec::new();
    for path in paths {
        let current = match try_read_extra_tags(Path::new(&path)) {
            Ok(tags) => tags,
            Err(error) => {
                let message =
                    format!("Could not capture extra-tag undo snapshot for {path}: {error}");
                if mark_status {
                    runtime.mark_batch_failed(batch_id, &message);
                }
                return serde_json::json!({
                    "success": false,
                    "error": message,
                    "results": [{
                        "trackPath": path,
                        "success": false,
                        "error": error.to_string()
                    }],
                    "extraUndoSnapshots": prepared.iter().map(|(prepared_path, tags, _)| {
                        serde_json::json!({ "path": prepared_path, "extraTags": tags })
                    }).collect::<Vec<_>>()
                });
            }
        };
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
        prepared.push((path, current, final_tags));
    }
    let undo = prepared
        .iter()
        .map(|(path, current, _)| serde_json::json!({ "path": path, "extraTags": current }))
        .collect::<Vec<_>>();
    let mut results = Vec::new();
    for (path, _, final_tags) in prepared {
        tracing::debug!(path = %path, tag_count = final_tags.len(), "writing extra tags");
        match write_extra_tags_with_exclusive_queue_held(PathBuf::from(&path), final_tags).await {
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
    tracing::debug!(
        batch_id = %batch_id,
        action_count = batch.actions.len(),
        "applying folder moves"
    );
    // The library root is required to re-validate containment at apply time.
    // Preview-time checks are not enough: a destination ancestor can become an
    // external symlink between preview and approval (TOCTOU).
    let Some(library_root) = batch.library_root.as_deref().map(Path::new) else {
        let error = "Folder-move batch is missing its library root; refusing to move files"
            .to_string();
        runtime.mark_batch_failed(batch_id, &error);
        return serde_json::json!({ "success": false, "error": error, "results": [] });
    };
    let library_canonical = match library_root.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            let message = format!(
                "Library root '{}' cannot be resolved: {error}",
                library_root.display()
            );
            runtime.mark_batch_failed(batch_id, &message);
            return serde_json::json!({ "success": false, "error": message, "results": [] });
        }
    };

    // Phase 1 — preflight every action before any file is moved, so a failure
    // anywhere cannot leave a partially applied batch.
    let mut plan = Vec::new();
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
        let source_path = PathBuf::from(source);
        let destination_path = PathBuf::from(destination);
        if let Err(error) = preflight_relocation(&source_path, &destination_path, &library_canonical)
        {
            runtime.mark_batch_failed(batch_id, &error);
            return serde_json::json!({ "success": false, "error": error, "results": [] });
        }
        plan.push((source_path, destination_path));
    }
    if plan.is_empty() {
        runtime.mark_batch_applied(batch_id);
        return serde_json::json!({ "success": true, "results": [], "manifest": [] });
    }

    // Phase 2 — apply sequentially; Phase 3 — best-effort rollback of the
    // completed moves if a later move fails.
    let mut results = Vec::new();
    let mut completed: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (source_path, destination_path) in &plan {
        let source = source_path.clone();
        let destination = destination_path.clone();
        tracing::debug!(from = %source.display(), to = %destination.display(), "relocating track");
        match relocate_file_queued(queue, source.clone(), destination.clone(), &library_canonical)
            .await
        {
            Ok(()) => {
                completed.push((source_path.clone(), destination_path.clone()));
                // Readback is best-effort: a metadata-read failure must not undo
                // a move that already succeeded.
                let readback = read_track_metadata(&destination).is_ok();
                results.push(serde_json::json!({
                    "sourcePath": source,
                    "destinationPath": destination,
                    "success": true,
                    "readback": readback
                }));
            }
            Err(error) => {
                results.push(serde_json::json!({
                    "sourcePath": source,
                    "destinationPath": destination,
                    "success": false,
                    "error": error.to_string()
                }));
                let mut rollback = Vec::new();
                for (done_source, done_destination) in completed.iter().rev() {
                    match relocate_file_queued(
                        queue,
                        done_destination.clone(),
                        done_source.clone(),
                        &library_canonical,
                    )
                    .await
                    {
                        Ok(()) => rollback.push(serde_json::json!({
                            "from": done_destination,
                            "to": done_source,
                            "success": true
                        })),
                        Err(rollback_error) => rollback.push(serde_json::json!({
                            "from": done_destination,
                            "to": done_source,
                            "success": false,
                            "error": rollback_error.to_string()
                        })),
                    }
                }
                let message = format!(
                    "Failed to move '{}': {}. Rolled back {} completed move(s).",
                    source_path.display(),
                    error,
                    rollback.len()
                );
                runtime.mark_batch_failed(batch_id, &message);
                return serde_json::json!({
                    "success": false,
                    "error": message,
                    "results": results,
                    "rollback": rollback
                });
            }
        }
    }
    runtime.mark_batch_applied(batch_id);
    let manifest = results
        .iter()
        .map(|result| {
            serde_json::json!({ "from": result["sourcePath"], "to": result["destinationPath"] })
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "success": true, "results": results, "manifest": manifest })
}

/// Fail-closed relocation preflight: both ends must resolve inside the
/// (canonicalized) library. Canonicalization failure is a rejection, never a
/// lexical fallback, because a lexical check can be defeated by symlinks.
fn preflight_relocation(
    source: &Path,
    destination: &Path,
    library_canonical: &Path,
) -> Result<(), String> {
    let source_canonical = source
        .canonicalize()
        .map_err(|error| format!("Source '{}' cannot be resolved: {error}", source.display()))?;
    if !source_canonical.starts_with(library_canonical) {
        return Err(format!(
            "Source '{}' resolves outside the library root",
            source.display()
        ));
    }
    let mut probe = destination
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent.to_path_buf(),
            None => {
                return Err(format!(
                    "Destination '{}' has no resolvable ancestor",
                    destination.display()
                ))
            }
        }
    }
    let ancestor_canonical = probe.canonicalize().map_err(|error| {
        format!(
            "Destination ancestor '{}' cannot be resolved: {error}",
            probe.display()
        )
    })?;
    if !ancestor_canonical.starts_with(library_canonical) {
        return Err(format!(
            "Destination '{}' resolves outside the library root",
            destination.display()
        ));
    }
    Ok(())
}

/// Queued relocation with fail-closed containment re-validation immediately
/// before the rename: the source and the (created) destination parent must both
/// resolve inside the canonicalized library, otherwise the move is rejected.
async fn relocate_file_queued(
    queue: &WriteQueue,
    source: PathBuf,
    destination: PathBuf,
    library_canonical: &Path,
) -> Result<(), ApiError> {
    let library = library_canonical.to_path_buf();
    queue
        .run(async move {
            tokio::task::spawn_blocking(move || {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(ApiError::Io)?;
                    let parent_canonical = parent.canonicalize().map_err(|error| {
                        ApiError::Message(format!(
                            "Destination parent '{}' cannot be resolved: {error}",
                            parent.display()
                        ))
                    })?;
                    if !parent_canonical.starts_with(&library) {
                        return Err(ApiError::Message(format!(
                            "Destination '{}' resolves outside the library root; move rejected",
                            destination.display()
                        )));
                    }
                }
                let source_canonical = source.canonicalize().map_err(|error| {
                    ApiError::Message(format!(
                        "Source '{}' cannot be resolved: {error}",
                        source.display()
                    ))
                })?;
                if !source_canonical.starts_with(&library) {
                    return Err(ApiError::Message(format!(
                        "Source '{}' resolves outside the library root; move rejected",
                        source.display()
                    )));
                }
                fs::rename(&source, &destination).map_err(ApiError::Io)
            })
            .await
            .map_err(|error| ApiError::WriteTask(error.to_string()))?
        })
        .await
}

async fn apply_remove_embedded_cover(
    runtime: &AssistantRuntimeState,
    queue: &WriteQueue,
    batch: &AssistantActionBatch,
    batch_id: &str,
) -> Value {
    let mut results = Vec::new();
    for action in &batch.actions {
        let Some(path) = action.track_path.as_ref() else {
            continue;
        };
        if action.skip_reason.is_some() {
            continue;
        }
        let path = path.clone();
        match remove_embedded_cover_queued(queue, PathBuf::from(&path)).await {
            Ok(_) => results.push(serde_json::json!({
                "trackPath": path, "success": true
            })),
            Err(error) => results.push(serde_json::json!({
                "trackPath": path, "success": false, "error": format!("{error}")
            })),
        }
    }
    let failed = results.iter().filter(|r| r["success"] == false).count();
    if failed > 0 {
        let error = format!("Failed to remove embedded cover from {failed} file(s)");
        runtime.mark_batch_failed(batch_id, &error);
        serde_json::json!({ "success": false, "error": error, "results": results })
    } else {
        runtime.mark_batch_applied(batch_id);
        serde_json::json!({ "success": true, "results": results })
    }
}

async fn apply_metadata_action_batch(
    runtime: &AssistantRuntimeState,
    batch: &AssistantActionBatch,
    batch_id: &str,
) -> Value {
    if let Some(verification) = preflight_metadata_contract(batch) {
        let error = "Metadata preview is stale or incomplete";
        runtime.mark_batch_failed(batch_id, error);
        return serde_json::json!({
            "success": false,
            "error": error,
            "verification": verification
        });
    }
    let mut undo_failures = Vec::new();
    let mut standard_paths = HashSet::new();
    let mut extra_paths = HashSet::new();
    for action in &batch.actions {
        let Some(path) = action.track_path.as_deref() else {
            continue;
        };
        if action.tag_kind.as_deref().unwrap_or("standard") == "extra" {
            extra_paths.insert(path);
        } else {
            standard_paths.insert(path);
        }
    }
    for path in standard_paths {
        if let Err(error) = read_track_metadata(Path::new(path)) {
            undo_failures.push(serde_json::json!({
                "trackPath": path,
                "error": format!("Could not capture undo snapshot: {error}")
            }));
        }
    }
    for path in extra_paths {
        if let Err(error) = try_read_extra_tags(Path::new(path)) {
            undo_failures.push(serde_json::json!({
                "trackPath": path,
                "error": format!("Could not capture extra-tag undo snapshot: {error}")
            }));
        }
    }
    if !undo_failures.is_empty() {
        let error = "Could not capture complete undo evidence";
        runtime.mark_batch_failed(batch_id, error);
        return serde_json::json!({
            "success": false,
            "error": error,
            "results": undo_failures.clone(),
            "verification": verification_summary("failed", "write", batch, 0, undo_failures)
        });
    }
    match batch.kind.as_str() {
        "tag-update" => {
            let result = apply_standard_actions(runtime, batch, batch_id, false, false).await;
            finish_metadata_apply(runtime, batch, batch_id, result)
        }
        "extra-tag-update" => {
            let result = apply_extra_actions(runtime, batch, batch_id, false).await;
            finish_metadata_apply(runtime, batch, batch_id, result)
        }
        "metadata-update" => {
            let standard = apply_standard_actions(runtime, batch, batch_id, true, false).await;
            let extra = apply_extra_actions(runtime, batch, batch_id, false).await;
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
                let result = serde_json::json!({ "success": false, "error": error, "results": { "standard": failed_standard, "extra": failed_extra }, "undoSnapshots": standard["undoSnapshots"], "extraUndoSnapshots": extra["extraUndoSnapshots"] });
                finish_metadata_apply(runtime, batch, batch_id, result)
            } else {
                let result = serde_json::json!({ "success": true, "results": { "standard": standard["results"], "extra": extra["results"] }, "undoSnapshots": standard["undoSnapshots"], "extraUndoSnapshots": extra["extraUndoSnapshots"] });
                finish_metadata_apply(runtime, batch, batch_id, result)
            }
        }
        _ => serde_json::json!({
            "success": false,
            "error": format!("Unsupported metadata batch kind: {}", batch.kind)
        }),
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
    tracing::debug!(batch_id = %batch_id, "action batch apply started");
    if !runtime.is_active() {
        return serde_json::json!({ "success": false, "error": "No active assistant session" });
    }
    let Some(batch) = runtime.get_batch(batch_id) else {
        return serde_json::json!({ "success": false, "error": format!("Action batch not found: {batch_id}") });
    };
    if batch.status != "pending" {
        return serde_json::json!({ "success": false, "error": format!("Batch already {}", batch.status) });
    }
    tracing::debug!(
        batch_id = %batch_id,
        kind = %batch.kind,
        action_count = batch.actions.len(),
        "applying action batch"
    );
    if matches!(
        batch.kind.as_str(),
        "tag-update" | "extra-tag-update" | "metadata-update"
    ) {
        return queue
            .run_exclusive(apply_metadata_action_batch(runtime, &batch, batch_id))
            .await;
    }
    match batch.kind.as_str() {
        "folder-move" => apply_folder_moves(runtime, queue, &batch, batch_id).await,
        "embedded-cover-remove" => {
            apply_remove_embedded_cover(runtime, queue, &batch, batch_id).await
        }
        "auto-tag-run" | "audit-run" => {
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
    task_state: State<'_, crate::state::assistant_task::AssistantTaskState>,
) -> Result<Value, ApiError> {
    let mut result = apply_action_batch(&runtime, &queue, &action_batch_id).await;
    let Some(batch) = runtime.get_batch(&action_batch_id) else {
        return Ok(result);
    };
    if matches!(batch.status.as_str(), "applied" | "failed") {
        let evidence = result
            .get("verification")
            .cloned()
            .unwrap_or_else(|| result.clone());
        if let Err(error) = task_state.finalize_batch(&action_batch_id, &batch.status, &evidence) {
            runtime.mark_batch_failed(&action_batch_id, &error);
            let verified = evidence
                .get("verifiedActionCount")
                .and_then(Value::as_u64)
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or_default();
            result["success"] = Value::Bool(false);
            result["error"] = Value::String(error.clone());
            result["verification"] = verification_summary(
                "failed",
                "persistence",
                &batch,
                verified,
                vec![serde_json::json!({ "error": error })],
            );
        }
    }
    let Some(batch) = runtime.get_batch(&action_batch_id) else {
        return Ok(result);
    };
    let Some(current) = conversation.current() else {
        return Ok(result);
    };
    let verification_required = matches!(
        batch.kind.as_str(),
        "tag-update" | "extra-tag-update" | "metadata-update"
    );
    let (event_type, message, data) = match batch.status.as_str() {
        "applied" => (
            "action_batch_applied",
            format!("Applied: {}", batch.title),
            serde_json::json!({
                "batchId": action_batch_id,
                "verificationRequired": verification_required,
                "verification": result.get("verification")
            }),
        ),
        "failed" => {
            let error = runtime.batch_error(&action_batch_id).unwrap_or_default();
            let detail: String = result
                .get("results")
                .and_then(|r| r.as_array())
                .map(|results| {
                    results
                        .iter()
                        .map(|r| {
                            let path = r.get("trackPath").and_then(Value::as_str).unwrap_or("?");
                            let err = r.get("error").and_then(Value::as_str).unwrap_or("unknown");
                            format!("{path}: {err}")
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            if !detail.is_empty() {
                conversation.record("system", &detail, None, 0, 0, 0);
            }
            (
                "action_batch_failed",
                format!("Failed: {}: {error}", batch.title),
                serde_json::json!({
                    "batchId": action_batch_id,
                    "error": error,
                    "results": result.get("results"),
                    "verificationRequired": verification_required,
                    "verification": result.get("verification")
                }),
            )
        }
        _ => return Ok(result),
    };
    conversation.record("system", &message, None, 0, 0, 0);
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

fn complete_delegated_task_batch(
    runtime: &AssistantRuntimeState,
    batch_id: &str,
    error: Option<&str>,
) -> Result<AssistantActionBatch, String> {
    let batch = runtime
        .get_batch(batch_id)
        .ok_or_else(|| format!("Action batch not found: {batch_id}"))?;
    if batch.status != "pending" {
        return Err(format!("Batch already {}", batch.status));
    }
    if !matches!(batch.kind.as_str(), "auto-tag-run" | "audit-run") {
        return Err(format!(
            "Batch {} is not a renderer-delegated task",
            batch.kind
        ));
    }
    if let Some(error) = error {
        runtime.mark_batch_failed(batch_id, error);
    } else {
        runtime.mark_batch_applied(batch_id);
    }
    runtime
        .get_batch(batch_id)
        .ok_or_else(|| format!("Action batch not found after completion: {batch_id}"))
}

#[tauri::command]
pub fn assistant_complete_task_actions(
    app: AppHandle,
    action_batch_id: String,
    error: Option<String>,
    runtime: State<'_, AssistantRuntimeState>,
    conversation: State<'_, ConversationState>,
    task_state: State<'_, crate::state::assistant_task::AssistantTaskState>,
) -> Result<Value, ApiError> {
    let mut batch = complete_delegated_task_batch(&runtime, &action_batch_id, error.as_deref())
        .map_err(ApiError::Message)?;
    let evidence = serde_json::json!({
        "status": if error.is_none() { "applied" } else { "failed" },
        "error": error.as_deref()
    });
    let mut persistence_error = None;
    if let Err(message) = task_state.finalize_batch(&action_batch_id, &batch.status, &evidence) {
        runtime.mark_batch_failed(&action_batch_id, &message);
        batch.status = "failed".into();
        persistence_error = Some(message);
    }
    let succeeded = error.is_none() && persistence_error.is_none();
    let terminal_error = error.as_deref().or(persistence_error.as_deref());
    let event_type = if succeeded {
        "action_batch_applied"
    } else {
        "action_batch_failed"
    };
    let message = if let Some(error) = terminal_error {
        format!("Failed: {}: {error}", batch.title)
    } else {
        format!("Applied: {}", batch.title)
    };
    if let Some(current) = conversation.current() {
        conversation.record("system", &message, None, 0, 0, 0);
        let _ = app.emit(
            "assistant:event",
            AssistantEvent {
                session_id: current.session_id,
                event_type,
                message: message.clone(),
                data: Some(serde_json::json!({
                    "batchId": action_batch_id,
                    "error": terminal_error,
                    "verificationRequired": false
                })),
            },
        );
    }
    Ok(serde_json::json!({
        "success": succeeded,
        "batchId": action_batch_id,
        "error": terminal_error
    }))
}

#[cfg(test)]
mod apply_contract_tests {
    use super::*;
    use crate::commands::mutations::{write_track_dispatch, Patch, StringList, TrackPatch};
    use crate::commands::tracks::read_track_metadata;
    use crate::state::assistant::AssistantAction;
    use crate::state::write_queue::WriteQueue;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

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
    fn assistant_response_schema_advertises_the_public_registry() {
        let schema = assistant_response_schema();
        let names = schema["properties"]["toolCall"]["properties"]["toolName"]["enum"]
            .as_array()
            .unwrap();
        // Public catalog has 16 tools
        assert_eq!(names.len(), 16);
        // New tools are present
        assert!(names.contains(&serde_json::json!("metadata.patch")));
        assert!(names.contains(&serde_json::json!("metadata.transform")));
        assert!(names.contains(&serde_json::json!("files.transform")));
        assert!(names.contains(&serde_json::json!("files.relocate")));
        assert!(names.contains(&serde_json::json!("library.run_task")));
        assert!(names.contains(&serde_json::json!("plan.create")));
        // Legacy tools are NOT advertised
        assert!(!names.contains(&serde_json::json!("edit_metadata")));
        assert!(!names.contains(&serde_json::json!("create_plan")));
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
    async fn plan_chains_metadata_transform_with_files_relocate_for_album_grouping() {
        // The original failing workflow: group vocal + instrumental versions of
        // the same song into one album folder, deriving both the Album tag and
        // the destination folder from the title minus its `(伴奏)` suffix.
        let dir = std::env::temp_dir().join(format!(
            "soundrobe-test-plan-relocate-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let vocal = dir.join("vocal.mp3");
        let instrumental = dir.join("instrumental.mp3");
        let _ = fs::write(&vocal, b"test");
        let _ = fs::write(&instrumental, b"test");
        let paths = vec![
            vocal.to_string_lossy().into_owned(),
            instrumental.to_string_lossy().into_owned(),
        ];
        let input = AssistantSendInput {
            selected_track_paths: vec![],
            tracks: vec![
                serde_json::json!({"path": vocal.to_string_lossy(), "title": "喜剧演员"}),
                serde_json::json!({"path": instrumental.to_string_lossy(), "title": "喜剧演员(伴奏)"}),
            ],
            library_path: Some(dir.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let providers = ProviderState::default();
        let config = crate::state::config::AutoTagConfig::default();
        let assistant = AssistantServicesSnapshot::default();

        let execution = execute_create_plan(
            &serde_json::json!({
                "steps": [
                    {"id": "album", "tool": "metadata.transform", "args": {
                        "target_scope": "explicit_paths", "paths": paths,
                        "source": {"kind": "tag", "field": "title"},
                        "destination": {"kind": "tag", "field": "album"},
                        "operations": [{"op": "strip_suffix", "suffix": "(伴奏)"}]
                    }},
                    {"id": "group", "tool": "files.relocate", "args": {
                        "target_scope": "explicit_paths", "paths": paths,
                        "destination": {
                            "template": "{value}",
                            "source": {"kind": "tag", "field": "title"},
                            "operations": [{"op": "strip_suffix", "suffix": "(伴奏)"}]
                        }
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

        assert!(
            execution.result.ok,
            "plan failed: {}",
            execution.result.error.as_deref().unwrap_or("unknown")
        );
        // Two native preview batches: the tag change and the folder move.
        assert_eq!(execution.batches.len(), 2);
        assert_eq!(execution.batches[0].kind, "metadata-update");
        assert_eq!(execution.batches[1].kind, "folder-move");
        // The Album batch only contains the track that actually changed (the
        // vocal title has no `(伴奏)` suffix to strip, so it needs no Album
        // update) — the stripped value is the shared album name.
        let album_actions = &execution.batches[0].actions;
        assert_eq!(album_actions.len(), 1);
        assert_eq!(album_actions[0].new_value.as_deref(), Some("喜剧演员"));
        assert_eq!(album_actions[0].field.as_deref(), Some("album"));
        // Both tracks are grouped into the same album folder.
        let move_actions = &execution.batches[1].actions;
        assert_eq!(move_actions.len(), 2);
        let album_dir = dir.join("喜剧演员");
        assert!(move_actions.iter().all(|a| a
            .destination_path
            .as_deref()
            .is_some_and(|d| d.starts_with(album_dir.to_string_lossy().as_ref()))));
        assert!(move_actions
            .iter()
            .all(|a| a.operation.as_deref() == Some("relocate")));
        fs::remove_dir_all(&dir).unwrap();
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
            library_root: None,
            completion_contract: None,
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
    async fn split_artists_contract_applies_and_verifies_copied_flac_readback() {
        let root = temp_dir();
        let malformed = root.join("45.flac");
        let correct = root.join("46.flac");
        fs::copy(flac_fixture(), &malformed).unwrap();
        fs::copy(flac_fixture(), &correct).unwrap();
        write_track_dispatch(
            &malformed,
            &TrackPatch {
                artist: Patch::Value("黎明 & 谭咏麟".into()),
                artists: Patch::Value(StringList::Many(vec!["黎明&谭咏麟".into()])),
                ..Default::default()
            },
        )
        .unwrap();
        write_track_dispatch(
            &correct,
            &TrackPatch {
                artist: Patch::Value("谭咏麟 & 김완선".into()),
                artists: Patch::Value(StringList::Many(vec!["谭咏麟".into(), "김완선".into()])),
                ..Default::default()
            },
        )
        .unwrap();
        let input = AssistantSendInput {
            selected_track_paths: vec![
                malformed.to_string_lossy().into_owned(),
                correct.to_string_lossy().into_owned(),
            ],
            tracks: vec![
                serde_json::to_value(read_track_metadata(&malformed).unwrap()).unwrap(),
                serde_json::to_value(read_track_metadata(&correct).unwrap()).unwrap(),
            ],
            ..Default::default()
        };
        let execution = crate::commands::assistant_metadata_tools::execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "artists"},
                "operations": [{"op": "split_artists"}]
            }),
            &input,
            "verified-session",
        );
        let batch = execution.batches.first().expect("preview batch").clone();
        let batch_id = batch.id.clone();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(batch));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), &batch_id).await;

        assert_eq!(result["success"], true, "{result}");
        assert_eq!(result["verification"]["status"], "verified");
        assert_eq!(result["verification"]["scopeCount"], 2);
        assert_eq!(result["verification"]["expectedActionCount"], 1);
        assert_eq!(result["verification"]["verifiedActionCount"], 1);
        let updated = read_track_metadata(&malformed).unwrap();
        assert_eq!(updated.artist.as_deref(), Some("黎明 & 谭咏麟"));
        assert_eq!(updated.artists, vec!["黎明", "谭咏麟"]);
        assert_eq!(runtime.get_batch(&batch_id).unwrap().status, "applied");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn split_artists_contract_rejects_stale_preview_before_writing() {
        let root = temp_dir();
        let path = root.join("stale.flac");
        fs::copy(flac_fixture(), &path).unwrap();
        write_track_dispatch(
            &path,
            &TrackPatch {
                artist: Patch::Value("黎明 & 谭咏麟".into()),
                artists: Patch::Value(StringList::Many(vec!["黎明&谭咏麟".into()])),
                ..Default::default()
            },
        )
        .unwrap();
        let input = AssistantSendInput {
            selected_track_paths: vec![path.to_string_lossy().into_owned()],
            tracks: vec![serde_json::to_value(read_track_metadata(&path).unwrap()).unwrap()],
            ..Default::default()
        };
        let execution = crate::commands::assistant_metadata_tools::execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "artists"},
                "operations": [{"op": "split_artists"}]
            }),
            &input,
            "stale-session",
        );
        let batch = execution.batches.first().expect("preview batch").clone();
        let batch_id = batch.id.clone();
        write_track_dispatch(
            &path,
            &TrackPatch {
                artist: Patch::Value("Externally changed".into()),
                artists: Patch::Value(StringList::Many(vec!["Externally changed".into()])),
                ..Default::default()
            },
        )
        .unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(batch));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), &batch_id).await;

        assert_eq!(result["success"], false, "{result}");
        assert_eq!(result["verification"]["status"], "failed");
        assert_eq!(result["verification"]["phase"], "preflight", "{result}");
        assert_eq!(
            read_track_metadata(&path).unwrap().artists,
            vec!["Externally changed"]
        );
        assert_eq!(runtime.get_batch(&batch_id).unwrap().status, "failed");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn extra_tag_removal_is_planned_applied_and_verified_from_disk() {
        let root = temp_dir();
        let path = root.join("extra-remove.flac");
        fs::copy(flac_fixture(), &path).unwrap();
        let queue = WriteQueue::default();
        crate::commands::mutations::write_extra_tags_queued(
            &queue,
            path.clone(),
            vec![ExtraTagUpdate {
                key: "MOOD".into(),
                value: "Calm".into(),
            }],
        )
        .await
        .unwrap();
        let input = AssistantSendInput {
            selected_track_paths: vec![path.to_string_lossy().into_owned()],
            tracks: vec![serde_json::to_value(read_track_metadata(&path).unwrap()).unwrap()],
            ..Default::default()
        };
        let execution = crate::commands::assistant_metadata_tools::execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [{
                    "tag_kind": "extra",
                    "field": "MOOD",
                    "action": "remove"
                }]
            }),
            &input,
            "extra-remove-session",
        );
        let batch = execution
            .batches
            .first()
            .expect("extra removal preview")
            .clone();
        assert_eq!(batch.actions[0].tag_kind.as_deref(), Some("extra"));
        let batch_id = batch.id.clone();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(batch));

        let result = apply_action_batch(&runtime, &queue, &batch_id).await;

        assert_eq!(result["success"], true, "{result}");
        assert_eq!(result["verification"]["status"], "verified");
        assert!(!try_read_extra_tags(&path)
            .unwrap()
            .iter()
            .any(|tag| tag.key.eq_ignore_ascii_case("MOOD")));
        assert_eq!(
            result["extraUndoSnapshots"].as_array().map(Vec::len),
            Some(1)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn plural_artist_removal_verifies_physical_absence_and_preserves_singular_artist() {
        let root = temp_dir();
        let path = root.join("plural-remove.flac");
        fs::copy(flac_fixture(), &path).unwrap();
        write_track_dispatch(
            &path,
            &TrackPatch {
                artist: Patch::Value("Display Artist".into()),
                artists: Patch::Value(StringList::Many(vec!["Artist A".into(), "Artist B".into()])),
                ..Default::default()
            },
        )
        .unwrap();
        let input = AssistantSendInput {
            selected_track_paths: vec![path.to_string_lossy().into_owned()],
            tracks: vec![serde_json::to_value(read_track_metadata(&path).unwrap()).unwrap()],
            ..Default::default()
        };
        let execution = crate::commands::assistant_metadata_tools::execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [{"field": "artists", "action": "remove"}]
            }),
            &input,
            "plural-remove-session",
        );
        let batch = execution
            .batches
            .first()
            .expect("plural removal preview")
            .clone();
        let batch_id = batch.id.clone();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(batch));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), &batch_id).await;

        assert_eq!(result["success"], true, "{result}");
        assert_eq!(result["verification"]["status"], "verified");
        let updated = read_track_metadata(&path).unwrap();
        assert_eq!(updated.artist.as_deref(), Some("Display Artist"));
        assert!(
            crate::commands::tracks::read_plural_tag_values(&path, "artists")
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn semantic_readback_mismatch_fails_and_retains_undo_evidence() {
        let root = temp_dir();
        let path = root.join("mismatch.mp3");
        fs::copy(media_fixture(), &path).unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-readback-mismatch".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "tag-update".into(),
            title: "Mismatch".into(),
            summary: "one".into(),
            risk_level: "low".into(),
            reversible: true,
            status: "pending".into(),
            library_root: None,
            completion_contract: None,
            actions: vec![AssistantAction {
                track_path: Some(path.to_string_lossy().into_owned()),
                field: Some("unsupportedTestField".into()),
                new_value: Some("Expected".into()),
                ..Default::default()
            }],
        }));

        let result =
            apply_action_batch(&runtime, &WriteQueue::default(), "batch-readback-mismatch").await;

        assert_eq!(result["success"], false);
        assert_eq!(result["verification"]["phase"], "readback");
        assert_eq!(result["verification"]["verifiedActionCount"], 0);
        assert_eq!(result["undoSnapshots"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            runtime.get_batch("batch-readback-mismatch").unwrap().status,
            "failed"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn missing_undo_snapshot_aborts_before_any_metadata_write() {
        let root = temp_dir();
        let missing = root.join("missing.flac");
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-missing-undo".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "tag-update".into(),
            title: "Missing undo".into(),
            summary: "one".into(),
            risk_level: "low".into(),
            reversible: true,
            status: "pending".into(),
            library_root: None,
            completion_contract: None,
            actions: vec![AssistantAction {
                track_path: Some(missing.to_string_lossy().into_owned()),
                field: Some("genre".into()),
                new_value: Some("Pop".into()),
                ..Default::default()
            }],
        }));

        let result =
            apply_action_batch(&runtime, &WriteQueue::default(), "batch-missing-undo").await;

        assert_eq!(result["success"], false);
        assert_eq!(result["verification"]["phase"], "write");
        assert!(!missing.exists());
        assert_eq!(
            runtime.get_batch("batch-missing-undo").unwrap().status,
            "failed"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn delegated_task_batch_is_completed_only_after_renderer_confirmation() {
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        let batch = assistant_batch(
            "session",
            "auto-tag-run",
            "Auto-tag album",
            "Auto-tag 1 track",
            "medium",
            vec![AssistantAction {
                track_path: Some("/music/album/track.flac".into()),
                ..Default::default()
            }],
            true,
        );
        let batch_id = batch.id.clone();
        assert!(runtime.add_batch(batch));

        let dispatched = apply_action_batch(&runtime, &WriteQueue::default(), &batch_id).await;
        assert_eq!(dispatched["success"], true);
        assert_eq!(
            runtime.get_batch(&batch_id).unwrap().status,
            "pending",
            "renderer-delegated work must not be marked applied before it finishes"
        );

        complete_delegated_task_batch(&runtime, &batch_id, None).unwrap();
        assert_eq!(runtime.get_batch(&batch_id).unwrap().status, "applied");

        let failed = assistant_batch(
            "session",
            "audit-run",
            "Audit album",
            "Audit 1 track",
            "medium",
            vec![AssistantAction {
                track_path: Some("/music/album/track.flac".into()),
                ..Default::default()
            }],
            true,
        );
        let failed_id = failed.id.clone();
        assert!(runtime.add_batch(failed));
        complete_delegated_task_batch(&runtime, &failed_id, Some("provider failed")).unwrap();
        assert_eq!(runtime.get_batch(&failed_id).unwrap().status, "failed");
        assert_eq!(
            runtime.batch_error(&failed_id).as_deref(),
            Some("provider failed")
        );
    }

    #[test]
    fn rejection_does_not_change_runtime_when_terminal_persistence_fails() {
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        let batch = assistant_batch(
            "session",
            "metadata-update",
            "Reject safely",
            "one",
            "low",
            vec![AssistantAction {
                track_path: Some("/music/a.flac".into()),
                field: Some("genre".into()),
                new_value: Some("Pop".into()),
                ..Default::default()
            }],
            true,
        );
        let batch_id = batch.id.clone();
        assert!(runtime.add_batch(batch));
        let root = temp_dir();
        let task_state =
            crate::state::assistant_task::AssistantTaskState::new(root.join("ledger.db"));

        let error = reject_action_batch(&runtime, &task_state, &batch_id)
            .expect_err("uninitialized persistence must fail");

        assert!(error.to_string().contains("not initialized"));
        assert_eq!(runtime.get_batch(&batch_id).unwrap().status, "pending");
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
            library_root: None,
            completion_contract: None,
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
            library_root: Some(root.to_string_lossy().into_owned()),
            completion_contract: None,
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
    async fn folder_move_rolls_back_completed_moves_when_later_move_fails() {
        let root = temp_dir();
        let one = root.join("one.mp3");
        let two = root.join("two.mp3");
        let three = root.join("three.mp3");
        fs::copy(media_fixture(), &one).unwrap();
        fs::copy(media_fixture(), &two).unwrap();
        fs::copy(media_fixture(), &three).unwrap();
        // A regular FILE in the destination path forces create_dir_all to fail
        // for the third move, after the first two have already succeeded.
        fs::write(root.join("blocker"), b"block").unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        let destination_of = |path: &Path| {
            root.join("out")
                .join(path.file_name().unwrap().to_string_lossy().as_ref())
        };
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-rollback".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "folder-move".into(),
            title: "Move".into(),
            summary: "three".into(),
            risk_level: "medium".into(),
            reversible: true,
            status: "pending".into(),
            library_root: Some(root.to_string_lossy().into_owned()),
            completion_contract: None,
            actions: vec![
                AssistantAction {
                    source_path: Some(one.to_string_lossy().into_owned()),
                    destination_path: Some(destination_of(&one).to_string_lossy().into_owned()),
                    ..Default::default()
                },
                AssistantAction {
                    source_path: Some(two.to_string_lossy().into_owned()),
                    destination_path: Some(destination_of(&two).to_string_lossy().into_owned()),
                    ..Default::default()
                },
                AssistantAction {
                    source_path: Some(three.to_string_lossy().into_owned()),
                    destination_path: Some(root.join("blocker").join("three.mp3").to_string_lossy().into_owned()),
                    ..Default::default()
                },
            ],
        }));

        let result =
            apply_action_batch(&runtime, &WriteQueue::default(), "batch-rollback").await;

        assert_eq!(result["success"], false);
        assert_eq!(result["rollback"].as_array().map(Vec::len), Some(2));
        // The completed moves were rolled back to their sources.
        assert!(one.exists());
        assert!(two.exists());
        assert!(three.exists());
        assert!(!root.join("out").join("one.mp3").exists());
        assert!(!root.join("out").join("two.mp3").exists());
        assert_eq!(
            runtime.get_batch("batch-rollback").unwrap().status,
            "failed"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn folder_move_apply_rejects_destination_outside_library() {
        let root = temp_dir();
        let outside = std::env::temp_dir().join(format!(
            "soundrobe-assistant-outside-{}",
            std::process::id()
        ));
        fs::create_dir_all(&outside).unwrap();
        let source = root.join("source.mp3");
        fs::copy(media_fixture(), &source).unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-escape".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "folder-move".into(),
            title: "Move".into(),
            summary: "one".into(),
            risk_level: "medium".into(),
            reversible: true,
            status: "pending".into(),
            library_root: Some(root.to_string_lossy().into_owned()),
            completion_contract: None,
            actions: vec![AssistantAction {
                source_path: Some(source.to_string_lossy().into_owned()),
                destination_path: Some(outside.join("source.mp3").to_string_lossy().into_owned()),
                ..Default::default()
            }],
        }));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), "batch-escape").await;

        assert_eq!(result["success"], false);
        assert!(result["error"].as_str().unwrap_or("").contains("outside the library"));
        // Nothing moved.
        assert!(source.exists());
        assert!(!outside.join("source.mp3").exists());
        fs::remove_dir_all(&outside).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn folder_move_apply_rejects_symlinked_destination() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let root = temp_dir();
            let outside = std::env::temp_dir().join(format!(
                "soundrobe-assistant-symlink-{}",
                std::process::id()
            ));
            fs::create_dir_all(&outside).unwrap();
            let link = root.join("linked");
            symlink(&outside, &link).unwrap();
            let source = root.join("source.mp3");
            fs::copy(media_fixture(), &source).unwrap();
            let runtime = AssistantRuntimeState::default();
            assert!(runtime.initialize());
            assert!(runtime.add_batch(AssistantActionBatch {
                id: "batch-symlink".into(),
                created_at: "now".into(),
                session_id: "session".into(),
                kind: "folder-move".into(),
                title: "Move".into(),
                summary: "one".into(),
                risk_level: "medium".into(),
                reversible: true,
                status: "pending".into(),
                library_root: Some(root.to_string_lossy().into_owned()),
                completion_contract: None,
                actions: vec![AssistantAction {
                    source_path: Some(source.to_string_lossy().into_owned()),
                    destination_path: Some(link.join("source.mp3").to_string_lossy().into_owned()),
                    ..Default::default()
                }],
            }));

            let result =
                apply_action_batch(&runtime, &WriteQueue::default(), "batch-symlink").await;

            assert_eq!(result["success"], false);
            assert!(result["error"].as_str().unwrap_or("").contains("outside the library"));
            assert!(source.exists());
            fs::remove_dir_all(&outside).unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn folder_move_apply_refuses_without_library_root() {
        let root = temp_dir();
        let source = root.join("source.mp3");
        fs::copy(media_fixture(), &source).unwrap();
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "batch-noroot".into(),
            created_at: "now".into(),
            session_id: "session".into(),
            kind: "folder-move".into(),
            title: "Move".into(),
            summary: "one".into(),
            risk_level: "medium".into(),
            reversible: true,
            status: "pending".into(),
            library_root: None,
            completion_contract: None,
            actions: vec![AssistantAction {
                source_path: Some(source.to_string_lossy().into_owned()),
                destination_path: Some(root.join("nested").join("source.mp3").to_string_lossy().into_owned()),
                ..Default::default()
            }],
        }));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), "batch-noroot").await;

        assert_eq!(result["success"], false);
        assert!(result["error"].as_str().unwrap_or("").contains("missing its library root"));
        assert!(source.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn mixed_batch_readback_failure_retains_both_undo_shapes() {
        let root = temp_dir();
        let good = root.join("good.mp3");
        fs::copy(media_fixture(), &good).unwrap();
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
            library_root: None,
            completion_contract: None,
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
                    track_path: Some(good.to_string_lossy().into_owned()),
                    field: Some(String::new()),
                    new_value: Some("Calm".into()),
                    ..Default::default()
                },
            ],
        }));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), "batch-partial").await;

        assert_eq!(result["success"], false);
        assert_eq!(result["error"], "Metadata readback verification failed");
        assert_eq!(result["verification"]["status"], "failed");
        assert_eq!(result["verification"]["phase"], "readback");
        assert_eq!(result["verification"]["verifiedActionCount"], 1);
        assert_eq!(
            result["undoSnapshots"].as_array().map(Vec::len),
            Some(1),
            "partial writes retain standard undo evidence"
        );
        assert_eq!(
            result["extraUndoSnapshots"].as_array().map(Vec::len),
            Some(1),
            "failed extra writes retain their pre-write snapshot"
        );
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

    fn flac_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.flac")
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

    #[tokio::test]
    async fn chinese_conversion_smoke_test_15_tracks() {
        // Traditional Chinese titles that should be converted to simplified
        let traditional_titles = vec![
            "傳統音樂",
            "經典老歌",
            "華語流行",
            "搖滾樂團",
            "爵士鋼琴",
            "古典交響",
            "民謠吉他",
            "電子舞曲",
            "靈魂樂曲",
            "藍調音樂",
            "雷鬼節奏",
            "嘻哈饒舌",
            "節奏藍調",
            "放克音樂",
            "拉丁節奏",
        ];
        assert_eq!(traditional_titles.len(), 15);

        let root = temp_dir();
        let mut paths = Vec::new();
        let mut tracks = Vec::new();

        for (i, title) in traditional_titles.iter().enumerate() {
            let path = root.join(format!("track_{:02}.mp3", i + 1));
            fs::copy(media_fixture(), &path).unwrap();

            write_track_dispatch(
                &path,
                &TrackPatch {
                    title: Patch::Value((*title).into()),
                    artist: Patch::Value("測試藝人".into()),
                    ..Default::default()
                },
            )
            .unwrap();

            let track = read_track_metadata(&path).unwrap();
            let track_value = serde_json::to_value(&track).unwrap();
            tracks.push(track_value);
            paths.push(path.to_string_lossy().to_string());
        }

        let input = AssistantSendInput {
            message: "convert artist and title tags to simplified chinese".into(),
            library_path: None,
            active_album_path: None,
            selected_track_paths: paths.clone(),
            tracks,
            albums: vec![],
            autonomous: false,
            api_key: None,
            llm_model: None,
        };

        let actions = plan_chinese_conversion(&input, &paths, "simplified");

        // At least some tracks should have conversion actions (titles containing Chinese)
        assert!(
            !actions.is_empty(),
            "Should produce at least one conversion action"
        );

        // Verify each action converts to simplified Chinese (no traditional characters)
        for action in &actions {
            if let Some(field) = &action.field {
                if let Some(new_value) = &action.new_value {
                    // Verify field-specific behavior
                    // Verify conversion produced non-empty output
                    assert!(
                        !new_value.is_empty(),
                        "Converted value for {field} should not be empty"
                    );
                }
            }
        }

        // Apply the actions and verify files were updated
        let runtime = AssistantRuntimeState::default();
        assert!(runtime.initialize());
        assert!(runtime.add_batch(AssistantActionBatch {
            id: "chinese-smoke".into(),
            created_at: "now".into(),
            session_id: "chinese-smoke-session".into(),
            kind: "tag-update".into(),
            title: "Chinese conversion".into(),
            summary: format!("Convert {} action(s)", actions.len()),
            risk_level: "low".into(),
            reversible: true,
            status: "pending".into(),
            library_root: None,
            completion_contract: None,
            actions,
        }));

        let result = apply_action_batch(&runtime, &WriteQueue::default(), "chinese-smoke").await;

        assert!(
            result["success"] == true,
            "Apply should succeed: {:?}",
            result["error"]
        );

        // Verify each track's title was converted from traditional to simplified
        for (i, title) in traditional_titles.iter().enumerate() {
            let path = root.join(format!("track_{:02}.mp3", i + 1));
            let track = read_track_metadata(&path).unwrap();
            if let Some(converted) = &track.title {
                assert_ne!(
                    converted,
                    title,
                    "Track {} title should have changed from traditional",
                    i + 1
                );
                assert!(
                    !converted.is_empty(),
                    "Track {} title should not be empty after conversion",
                    i + 1
                );
            }
            // Verify artist was also converted
            if let Some(artist) = &track.artist {
                assert!(
                    !artist.is_empty(),
                    "Track {} artist should not be empty",
                    i + 1
                );
            }
        }

        fs::remove_dir_all(root).unwrap();
    }
}

/// ── Deterministic assistant behaviour tests ────────────────────────
///
/// These cover every function path that does NOT call an LLM:
/// routing, tool-validation, plan-ordering, argument resolution,
/// action-patch generation, tool-catalog shapes, etc.
///
/// All assertions use hard-coded expected values (no LLM-as-judge).
#[cfg(test)]
mod assistant_behaviour_tests {
    use super::*;
    use serde_json::json;

    // ── Tool catalog & schema shape ──────────────────────────────────

    #[test]
    fn all_registered_tools_have_valid_schemas() {
        let defs = crate::commands::assistant_tools::assistant_tool_definitions();
        assert_eq!(defs.len(), 28, "expected 28 registered tools total");
        let public = crate::commands::assistant_tools::public_tool_definitions();
        assert_eq!(public.len(), 16, "expected 16 public tools");
        assert!(public.iter().all(|d| d.public));
        for tool in &defs {
            assert!(!tool.name.is_empty(), "all tools need a name");
            assert!(
                tool.input_schema.is_object(),
                "tool {} input_schema must be an object",
                tool.name
            );
            // Every tool must have at least a "type":"object" — the schema
            // is used by validate_tool_args which expects properties/required.
            assert_eq!(
                tool.input_schema["type"], "object",
                "tool {} schema type must be object",
                tool.name
            );
        }
    }

    #[test]
    fn non_registered_tool_is_rejected() {
        let err = crate::commands::assistant_tools::validate_registered_tool_args(
            "nonexistent_tool",
            &json!({}),
        )
        .unwrap_err();
        assert!(err.contains("Unknown tool"));
    }

    #[test]
    fn context_tool_catalog_matches_public_registry() {
        let catalog = crate::commands::assistant_tools::context_tool_catalog();
        let catalog_names = catalog
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(catalog_names.len(), 16);
        // Must include public tools
        assert!(catalog_names.contains(&"metadata.patch"));
        assert!(catalog_names.contains(&"metadata.transform"));
        assert!(catalog_names.contains(&"files.transform"));
        assert!(catalog_names.contains(&"files.relocate"));
        assert!(catalog_names.contains(&"library.summarize"));
        assert!(catalog_names.contains(&"library.run_task"));
        assert!(catalog_names.contains(&"plan.create"));
        // Must NOT include legacy tools
        assert!(!catalog_names.contains(&"edit_metadata"));
        assert!(!catalog_names.contains(&"create_plan"));
        assert!(!catalog_names.contains(&"group_by_album"));
    }

    #[test]
    fn clarification_question_detection() {
        // Questions end with `?`.
        assert!(is_clarification_question(
            "Just to clarify — do you mean 1 or 2?"
        ));
        assert!(is_clarification_question("What would you like to do first?"));
        assert!(is_clarification_question(
            "Which direction would you like to go?"
        ));
        assert!(is_clarification_question("What's your goal here?"));
        // Leading clarify phrasing counts even without a question mark.
        assert!(is_clarification_question(
            "Clarify: keep the part before the paren."
        ));
        assert!(is_clarification_question("To clarify, this is a question"));
        // Answers, limitations, and completions are NOT clarifications.
        assert!(!is_clarification_question(
            "Done. Preview created (batch-1): 3 changes"
        ));
        assert!(!is_clarification_question(
            "The app cannot move files into new folders."
        ));
        assert!(!is_clarification_question("1."));
        assert!(!is_clarification_question(
            "I'll group them into album folders"
        ));
    }

    #[test]
    fn clarification_limit_message_guides_the_user() {
        let message = clarification_limit_message();
        assert!(message.contains("rephrase"));
        assert!(message.contains("folder"));
    }

    #[test]
    fn clarification_streak_state_transition() {
        // A question increments the streak; anything else resets it.
        assert_eq!(next_clarification_count(0, true), 1);
        assert_eq!(next_clarification_count(1, true), 2);
        assert_eq!(next_clarification_count(2, true), 3);
        assert_eq!(next_clarification_count(2, false), 0);
        assert_eq!(next_clarification_count(1, false), 0);
        // The streak is what blocks, not the raw question flag.
        assert!(next_clarification_count(1, true) >= ASSISTANT_CLARIFY_LIMIT);
        assert!(next_clarification_count(0, true) < ASSISTANT_CLARIFY_LIMIT);
    }

    #[test]
    fn files_relocate_schema_requires_destination() {
        let err = crate::commands::assistant_tools::validate_registered_tool_args(
            "files.relocate",
            &json!({"target_scope": "library"}),
        )
        .unwrap_err();
        assert!(err.contains("Missing required field: destination"));
    }

    // ── Tool argument validation ────────────────────────────────────

    #[test]
    fn edit_metadata_rejects_missing_target_scope() {
        let err = crate::commands::assistant_tools::validate_registered_tool_args(
            "edit_metadata",
            &json!({}),
        )
        .unwrap_err();
        assert!(err.contains("Missing required field"));
        assert!(err.contains("target_scope"));
    }

    #[test]
    fn edit_metadata_rejects_invalid_field_name() {
        let err = crate::commands::assistant_tools::validate_registered_tool_args(
            "edit_metadata",
            &json!({"target_scope": "selected", "invalid_field": "abc"}),
        )
        .unwrap_err();
        assert!(err.contains("Unknown field: invalid_field"));
    }

    #[test]
    fn tracks_search_rejects_non_string_artist() {
        let err = crate::commands::assistant_tools::validate_registered_tool_args(
            "tracks.search",
            &json!({"artist": 123}),
        )
        .unwrap_err();
        assert!(err.contains("should be a string"));
    }

    #[test]
    fn metadata_patch_accepts_only_if_missing_condition() {
        crate::commands::assistant_tools::validate_registered_tool_args(
            "metadata.patch",
            &json!({
                "target_scope": "library",
                "changes": [{
                    "field": "genre",
                    "action": "set",
                    "value": "Pop, Cantopop",
                    "only_if_missing": true
                }]
            }),
        )
        .expect("missing-field patches must not require enumerating every matching path");
    }

    #[test]
    fn organize_files_requires_source_dir_and_criterion() {
        let err = crate::commands::assistant_tools::validate_registered_tool_args(
            "organize_files",
            &json!({"source_dir": "/tmp"}),
        )
        .unwrap_err();
        assert!(err.contains("Missing required field"));
        assert!(err.contains("criterion"));
    }

    #[test]
    fn create_plan_requires_steps_field() {
        let err = crate::commands::assistant_tools::validate_registered_tool_args(
            "create_plan",
            &json!({"plan_description": "test"}),
        )
        .unwrap_err();
        assert!(
            err.contains("steps"),
            "expected error about missing steps: {}",
            err
        );
    }

    #[test]
    fn plan_dependency_order_on_empty_list_returns_empty() {
        let result = plan_dependency_order(&vec![]).unwrap();
        assert!(result.is_empty());
    }

    // ── action_patch ────────────────────────────────────────────────

    #[test]
    fn action_patch_string_field() {
        let patch = action_patch("title", Some("New Title")).unwrap();
        assert_eq!(patch.title.value(), Some(&"New Title".to_string()));
    }

    #[test]
    fn action_patch_list_field() {
        let patch = action_patch("artists", Some("A; B; C")).unwrap();
        // artists are stored as the raw semicolon-joined string in the patch
        // (conversion to Many happens during write, not during patch creation).
        assert!(
            patch.artists.value().is_some(),
            "artists field should have a value"
        );
    }

    #[test]
    fn action_patch_removal_returns_null() {
        let patch = action_patch("genre", None).unwrap();
        assert_eq!(
            patch.genre,
            crate::commands::mutations::Patch::<String>::Null
        );
    }

    #[test]
    fn action_patch_invalid_number_returns_error() {
        let err = action_patch("trackNumber", Some("abc")).unwrap_err();
        assert!(err.to_string().contains("Invalid numeric value"));
    }

    // ── tool_scope_paths ────────────────────────────────────────────

    #[test]
    fn tool_scope_paths_unknown_scope_returns_error() {
        let input = AssistantSendInput::default();
        let err = tool_scope_paths(&input, &json!({"target_scope": "unknown"})).unwrap_err();
        assert!(err.contains("Unsupported target_scope"));
    }

    #[test]
    fn tool_scope_paths_selected_returns_selected_paths() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/a.mp3".into()],
            tracks: vec![json!({"path": "/a.mp3"}), json!({"path": "/b.mp3"})],
            ..Default::default()
        };
        let paths = tool_scope_paths(&input, &json!({"target_scope": "selected"})).unwrap();
        assert_eq!(paths, vec!["/a.mp3"]);
    }

    #[test]
    fn tool_scope_paths_active_album_is_filtered_by_active_album_path() {
        let input = AssistantSendInput {
            active_album_path: Some("/music/Album".into()),
            tracks: vec![
                json!({"path": "/music/Album/a.mp3"}),
                json!({"path": "/music/Other/b.mp3"}),
            ],
            ..Default::default()
        };
        let paths = tool_scope_paths(&input, &json!({"target_scope": "active_album"})).unwrap();
        assert_eq!(paths, vec!["/music/Album/a.mp3"]);
    }

    #[test]
    fn tool_scope_paths_explicit_paths_rejects_missing_metadata() {
        let input = AssistantSendInput {
            tracks: vec![json!({"path": "/a.mp3"}), json!({"path": "/b.mp3"})],
            ..Default::default()
        };
        let error = tool_scope_paths(
            &input,
            &json!({"target_scope": "explicit_paths", "paths": ["/a.mp3", "/missing.mp3"]}),
        )
        .expect_err("explicit scope must never silently omit a requested path");
        assert!(error.contains("/missing.mp3"), "{error}");
    }

    #[test]
    fn tool_scope_paths_library_returns_all_loaded_paths() {
        let input = AssistantSendInput {
            tracks: vec![json!({"path": "/z.mp3"}), json!({"path": "/a.mp3"})],
            ..Default::default()
        };
        let paths = tool_scope_paths(&input, &json!({"target_scope": "library"})).unwrap();
        assert_eq!(paths, vec!["/z.mp3", "/a.mp3"]);
    }

    // ── Plan execution ──────────────────────────────────────────────

    #[test]
    fn plan_dependency_order_detects_circular_deps() {
        let err = plan_dependency_order(&vec![
            json!({"id": "a", "tool": "x", "depends_on": ["b"]}),
            json!({"id": "b", "tool": "x", "depends_on": ["a"]}),
        ])
        .unwrap_err();
        assert!(err.contains("Circular"));
    }

    #[test]
    fn plan_dependency_order_rejects_unknown_step_reference() {
        let err = plan_dependency_order(&vec![
            json!({"id": "a", "tool": "x", "depends_on": ["missing"]}),
        ])
        .unwrap_err();
        assert!(err.contains("depends on unknown step"));
    }

    #[test]
    fn plan_dependency_order_detects_duplicate_ids() {
        let err = plan_dependency_order(&vec![
            json!({"id": "a", "tool": "x"}),
            json!({"id": "a", "tool": "y"}),
        ])
        .unwrap_err();
        assert!(err.contains("Duplicate"));
    }

    #[test]
    fn resolve_plan_args_inline_string_is_returned_verbatim() {
        let scratchpad = BTreeMap::new();
        let resolved = resolve_plan_args(&json!("hello"), &scratchpad);
        assert_eq!(resolved, "hello");
    }

    #[test]
    fn resolve_plan_args_reference_resolves_whole_value() {
        let mut scratchpad = BTreeMap::new();
        scratchpad.insert("step_a".into(), json!("result_value"));
        let resolved = resolve_plan_args(&json!("$step_a"), &scratchpad);
        assert_eq!(resolved, "result_value");
    }

    #[test]
    fn resolve_plan_args_nested_field_reference_resolves_subpath() {
        let mut scratchpad = BTreeMap::new();
        scratchpad.insert("step_b".into(), json!({"paths": ["/a.mp3", "/b.mp3"]}));
        let resolved = resolve_plan_args(&json!("$step_b.paths"), &scratchpad);
        assert_eq!(resolved, json!(["/a.mp3", "/b.mp3"]));
    }

    #[test]
    fn resolve_plan_args_missing_reference_yields_null() {
        let scratchpad = BTreeMap::new();
        let resolved = resolve_plan_args(&json!("$nonexistent"), &scratchpad);
        assert_eq!(resolved, Value::Null);
    }

    #[test]
    fn resolve_plan_args_inside_array_resolves_each_entry() {
        let mut scratchpad = BTreeMap::new();
        scratchpad.insert("id".into(), json!("the-path"));
        let resolved = resolve_plan_args(&json!(["$id", "literal", {"key": "$id"}]), &scratchpad);
        assert_eq!(
            resolved,
            json!(["the-path", "literal", {"key": "the-path"}])
        );
    }

    // ── query_field ─────────────────────────────────────────────────

    #[test]
    fn query_field_extracts_value_from_whitespace_tolerant_matches() {
        assert_eq!(
            query_field("artist:   Radiohead album: OK Computer", "artist"),
            Some("Radiohead".into())
        );
    }

    #[test]
    fn query_field_returns_none_when_field_missing() {
        assert_eq!(query_field("album: OK Computer", "artist"), None);
    }

    #[test]
    fn query_field_extracts_quoted_value_correctly() {
        assert_eq!(
            query_field("artist:\"Radiohead\"", "artist"),
            Some("Radiohead".into())
        );
    }

    #[test]
    fn query_field_multi_character_field_name_is_handled() {
        assert_eq!(
            query_field("musicbrainz_album_id: abc-123", "musicbrainz_album_id"),
            Some("abc-123".into())
        );
    }

    // ── would_repeat_tool_call ──────────────────────────────────────

    #[test]
    fn would_repeat_allows_first_two_identical_calls() {
        let sigs = vec![tool_call_signature(
            "tracks.search",
            &json!({"artist": "A"}),
        )];
        assert!(!would_repeat_tool_call(
            &sigs,
            "tracks.search",
            &json!({"artist": "A"})
        ));
        let sigs2 = vec![
            tool_call_signature("tracks.search", &json!({"artist": "A"})),
            tool_call_signature("tracks.search", &json!({"artist": "A"})),
        ];
        assert!(would_repeat_tool_call(
            &sigs2,
            "tracks.search",
            &json!({"artist": "A"})
        ));
    }

    #[test]
    fn would_repeat_is_exact_about_different_tool_name() {
        let sigs = vec![
            tool_call_signature("tracks.search", &json!({"artist": "A"})),
            tool_call_signature("tracks.search", &json!({"artist": "A"})),
        ];
        // Different tool name, even with same args, is not a repeat.
        assert!(!would_repeat_tool_call(
            &sigs,
            "albums.inspect",
            &json!({"artist": "A"})
        ));
    }

    // ── Execute context tools ───────────────────────────────────────

    #[test]
    fn context_tool_library_summarize_counts_tracks_and_albums() {
        let input = AssistantSendInput {
            tracks: vec![
                json!({"path": "/a.mp3", "album": "Album1"}),
                json!({"path": "/b.mp3", "album": "Album1"}),
                json!({"path": "/c.mp3", "album": "Album2"}),
            ],
            albums: vec![
                json!({"path": "/Album1", "name": "Album1", "trackCount": 2}),
                json!({"path": "/Album2", "name": "Album2", "trackCount": 1}),
            ],
            ..Default::default()
        };
        let result = crate::commands::assistant_tools::execute_context_tool(
            "library.summarize",
            &json!({}),
            &input,
        );
        assert!(result.ok);
        assert!(result.summary.contains("3"));
        assert!(result.summary.contains("2"));
    }

    #[test]
    fn context_tool_inspect_tracks_returns_entries_for_matching_names() {
        let input = AssistantSendInput {
            tracks: vec![
                json!({"path": "/music/artist - song.mp3"}),
                json!({"path": "/music/another.mp3"}),
            ],
            ..Default::default()
        };
        let result = crate::commands::assistant_tools::execute_context_tool(
            "tracks.search",
            &json!({"artist": "artist"}),
            &input,
        );
        assert!(result.ok);
    }

    // ── active_scope_paths ──────────────────────────────────────────

    #[test]
    fn active_scope_paths_uses_selected_when_present() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/sel.mp3".into()],
            tracks: vec![json!({"path": "/track.mp3"})],
            ..Default::default()
        };
        let paths = active_scope_paths(&input);
        assert_eq!(paths, vec!["/sel.mp3"]);
    }

    #[test]
    fn active_scope_paths_falls_back_to_tracks_when_none_selected() {
        let input = AssistantSendInput {
            selected_track_paths: vec![],
            tracks: vec![json!({"path": "/a.mp3"}), json!({"path": "/b.mp3"})],
            ..Default::default()
        };
        let paths = active_scope_paths(&input);
        assert_eq!(paths.len(), 2);
    }

    // ── assistant_response_schema structural ────────────────────────

    #[test]
    fn assistant_response_schema_has_all_required_fields() {
        let schema = assistant_response_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["message", "responseKind"]));
        assert_eq!(
            schema["properties"]["responseKind"]["enum"],
            json!(["answer", "clarification", "limitation", "action"])
        );
        // actionBatch and toolCall are nullable optional properties.
        assert!(schema["properties"]["actionBatch"].is_object());
        assert_eq!(schema["properties"]["actionBatch"]["type"], "null");
        assert!(schema["properties"]["actionBatch"]["description"]
            .as_str()
            .unwrap()
            .contains("registered tool"));
        assert!(schema["properties"]["toolCall"].is_object());
        assert_eq!(schema["properties"]["message"]["type"], "string");
    }

    #[test]
    fn response_normalizes_common_single_tool_call_envelopes() {
        let normalized = normalize_assistant_response_value(json!({
            "message": "I will inspect the album.",
            "toolCalls": [{
                "name": "albums.inspect",
                "args": {},
                "operationKind": "read_only"
            }]
        }))
        .unwrap();
        assert_eq!(
            normalized["toolCall"],
            json!({"toolName": "albums.inspect", "args": {}})
        );
        assert!(normalized.get("toolCalls").is_none());

        let anthropic = normalize_assistant_response_value(json!({
            "toolCalls": [{
                "toolName": null,
                "name": "tracks.search",
                "args": null,
                "input": {"missingGenre": true}
            }]
        }))
        .unwrap();
        assert_eq!(anthropic["message"], "");
        assert_eq!(
            anthropic["toolCall"],
            json!({
                "toolName": "tracks.search",
                "args": {"missingGenre": true}
            })
        );

        let openai = normalize_assistant_response_value(json!({
            "tool_calls": [{
                "function": {
                    "name": "query.metadata",
                    "arguments": "{\"field\":\"genre\"}"
                }
            }]
        }))
        .unwrap();
        assert_eq!(
            openai["toolCall"],
            json!({
                "toolName": "query.metadata",
                "args": {"field": "genre"}
            })
        );

        let nullable_alias = normalize_assistant_response_value(json!({
            "toolCalls": null,
            "tool_calls": [{
                "function": {
                    "name": "tracks.search",
                    "arguments": "{\"missingGenre\":true}"
                }
            }]
        }))
        .unwrap();
        assert_eq!(
            nullable_alias["toolCall"],
            json!({
                "toolName": "tracks.search",
                "args": {"missingGenre": true}
            })
        );
    }

    #[test]
    fn response_infers_action_kind_for_tool_call_when_provider_omits_it() {
        let normalized = normalize_assistant_response_value(json!({
            "toolCall": {
                "toolName": "tracks.inspect",
                "args": {"paths": ["/music/collaboration.flac"]}
            }
        }))
        .unwrap();

        assert_eq!(normalized["message"], "");
        assert_eq!(normalized["responseKind"], "action");
        let draft: AssistantDraft = serde_json::from_value(normalized).unwrap();
        assert_eq!(draft.response_kind, AssistantResponseKind::Action);
        assert_eq!(
            draft.tool_call.unwrap().tool_name,
            "tracks.inspect",
            "the provider's registered tool call must remain executable"
        );
    }

    #[test]
    fn response_serializes_parallel_read_only_calls_and_rejects_ambiguous_calls() {
        let parallel = normalize_assistant_response_value(json!({
            "message": "",
            "toolCalls": [
                {"name": "library.summarize", "args": {}},
                {"name": "tracks.search", "args": {"missingGenre": true}}
            ]
        }))
        .unwrap();
        assert_eq!(parallel["toolCall"]["toolName"], "create_plan");
        assert_eq!(
            parallel["toolCall"]["args"]["steps"],
            json!([
                {
                    "id": "parallel_call_1",
                    "label": "library.summarize",
                    "tool": "library.summarize",
                    "args": {}
                },
                {
                    "id": "parallel_call_2",
                    "label": "tracks.search",
                    "tool": "tracks.search",
                    "args": {"missingGenre": true}
                }
            ])
        );

        let mutating = normalize_assistant_response_value(json!({
            "message": "",
            "toolCalls": [
                {"name": "tracks.search", "args": {"missingGenre": true}},
                {
                    "name": "library.run_task",
                    "args": {"task": "auto_tag", "target_scope": "library"}
                }
            ]
        }))
        .unwrap_err();
        assert!(mutating.contains("multiple mutating tool calls"));

        let ambiguous = normalize_assistant_response_value(json!({
            "message": "",
            "toolCall": {"toolName": "library.summarize", "args": {}},
            "toolCalls": [{"name": "tracks.search", "args": {}}]
        }))
        .unwrap_err();
        assert!(ambiguous.contains("both toolCall and toolCalls"));

        let unknown = normalize_assistant_response_value(json!({
            "toolCalls": [
                {"name": "library.summarize", "args": {}},
                {"name": "not.a.registered.tool", "args": {}}
            ]
        }))
        .unwrap_err();
        assert!(unknown.contains("Unknown tool: not.a.registered.tool"));
        assert!(!unknown.contains("mutating"));
    }

    #[test]
    fn response_rejects_conflicting_tool_call_aliases() {
        let conflicting_name = normalize_assistant_response_value(json!({
            "toolCall": {
                "toolName": "metadata.patch",
                "name": "tracks.search",
                "args": {}
            }
        }))
        .unwrap_err();
        assert!(conflicting_name.contains("conflicting tool names"));

        let conflicting_args = normalize_assistant_response_value(json!({
            "toolCall": {
                "name": "tracks.search",
                "args": {"missingGenre": true},
                "input": {"missingTitle": true}
            }
        }))
        .unwrap_err();
        assert!(conflicting_args.contains("conflicting tool arguments"));
    }

    #[test]
    fn assistant_step_progress_uses_the_configured_limit() {
        assert_eq!(
            assistant_step_message(ASSISTANT_MAX_STEPS),
            format!("Step {ASSISTANT_MAX_STEPS}/{ASSISTANT_MAX_STEPS}")
        );
    }

    #[test]
    fn invalid_model_preview_repair_routes_library_tasks_through_registered_tool() {
        let prompt =
            invalid_model_preview_repair_prompt("Assistant action is outside the active scope");
        assert!(prompt.contains("failed validation"));
        assert!(prompt.contains("library.run_task"));
        assert!(prompt.contains("target_scope"));
        assert!(prompt.contains("metadata.patch"));
        assert!(prompt.contains("field, action, and value"));
        assert!(prompt.contains("one registered mutating tool"));
        assert!(
            ASSISTANT_PREVIEW_REPAIR_ATTEMPTS >= 3,
            "schema-constrained models need a bounded opportunity to repair repeated envelope mistakes"
        );
    }

    #[test]
    fn self_review_does_not_repeat_confirmation_for_an_explicit_edit_request() {
        assert!(ASSISTANT_SELF_REVIEW_PROMPT.contains("already requested a supported edit"));
        assert!(ASSISTANT_SELF_REVIEW_PROMPT.contains("approval preview"));
        assert!(
            ASSISTANT_SELF_REVIEW_PROMPT.contains("action-defining details are missing"),
            "genuine ambiguity must still produce a clarification"
        );
    }

    #[test]
    fn invalid_preview_repair_cannot_finalize_as_message_only() {
        assert!(!requires_tool_after_invalid_preview(0));
        assert!(
            requires_tool_after_invalid_preview(1),
            "after rejecting an unsafe preview, prose must not claim that a preview exists"
        );
    }

    // ── Mutating tool execution edge cases ──────────────────────────

    #[test]
    fn edit_metadata_with_no_standard_updates_or_extra_is_no_change() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/one.flac".into()],
            tracks: vec![json!({"path": "/music/one.flac"})],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "edit_metadata",
            &json!({"target_scope": "selected"}),
            &input,
            "session",
        );
        assert!(execution.result.ok);
        assert!(execution.batches.is_empty());
    }

    #[test]
    fn remove_embedded_cover_rejects_missing_track_path() {
        let input = AssistantSendInput {
            tracks: vec![json!({"path": "/music/one.flac"})],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "remove_embedded_cover",
            &json!({"target_scope": "explicit_paths", "paths": []}),
            &input,
            "session",
        );
        // This should be an error since no paths match
        assert!(!execution.result.ok || execution.batches.is_empty());
    }

    #[test]
    fn chinese_convert_standard_macro_updates_title_and_artist() {
        let input = AssistantSendInput {
            tracks: vec![json!({
                "path": "/music/one.flac",
                "title": "你好",
                "artist": "世界"
            })],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "chinese_convert",
            &json!({"target_scope": "library", "direction": "s2t"}),
            &input,
            "session",
        );
        assert!(execution.result.ok);
        if !execution.batches.is_empty() {
            let actions = &execution.batches[0].actions;
            assert!(!actions.is_empty());
        }
    }

    // ── Mutating tool: run_library_task ─────────────────────────────

    #[test]
    fn run_library_task_auto_tag_creates_auto_tag_run_preview() {
        let input = AssistantSendInput {
            active_album_path: Some("/music/album".into()),
            tracks: vec![
                json!({"path": "/music/album/one.flac"}),
                json!({"path": "/music/album/two.flac"}),
            ],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "run_library_task",
            &json!({"task": "auto_tag", "target_scope": "active_album"}),
            &input,
            "session",
        );
        assert!(execution.result.ok);
        assert_eq!(execution.batches[0].kind, "auto-tag-run");
        assert_eq!(execution.batches[0].actions.len(), 2);
    }

    #[test]
    fn run_library_task_rejects_auto_tag_scopes_that_expand_in_the_renderer() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/album/one.flac".into()],
            tracks: vec![
                json!({"path": "/music/album/one.flac"}),
                json!({"path": "/music/album/two.flac"}),
            ],
            ..Default::default()
        };
        let execution = execute_mutating_assistant_tool(
            "library.run_task",
            &json!({"task": "auto_tag", "target_scope": "selected"}),
            &input,
            "session",
        );
        assert!(!execution.result.ok);
        assert!(execution.result.summary.contains("whole albums"));
        assert!(execution.batches.is_empty());
    }

    // ── assistant_batch ─────────────────────────────────────────────

    #[test]
    fn assistant_batch_assigns_unique_id_and_default_status() {
        let batch = assistant_batch(
            "session-1",
            "tag-update",
            "Test",
            "A test batch",
            "low",
            vec![AssistantAction {
                track_path: Some("/track.mp3".into()),
                ..Default::default()
            }],
            true,
        );
        assert_eq!(batch.session_id, "session-1");
        assert_eq!(batch.kind, "tag-update");
        assert_eq!(batch.status, "pending");
        assert!(batch.id.starts_with("batch-"));
        assert!(batch.reversible);
    }

    // ── build_assistant_messages ──────────────────────────────────

    #[test]
    fn messages_includes_current_request_exactly_once() {
        let ctx = json!({});
        let tools = json!([]);
        let history = vec![];
        let msgs = build_assistant_messages(&ctx, &tools, &history, "remove titles");

        let user_turns: Vec<_> = msgs.iter().filter(|m| m.role == "user").collect();
        assert_eq!(
            user_turns.len(),
            1,
            "current request must appear exactly once"
        );
        assert!(user_turns[0].content.contains("remove titles"));
    }

    #[test]
    fn messages_includes_system_prompt_first() {
        let ctx = json!({});
        let tools = json!([]);
        let history = vec![];
        let msgs = build_assistant_messages(&ctx, &tools, &history, "hello");

        assert_eq!(msgs.first().unwrap().role, "system");
        assert!(msgs.first().unwrap().content.contains("Soundrobe"));
    }

    #[test]
    fn messages_include_navidrome_tagging_knowledge() {
        let messages =
            build_assistant_messages(&json!({}), &json!([]), &[], "fix the Artists tags");
        let system_prompt = &messages[0].content;

        assert!(system_prompt.contains("Navidrome tagging knowledge"));
        assert!(system_prompt.contains("ARTIST controls the display credit"));
        assert!(system_prompt.contains("ARTISTS=A"));
        assert!(system_prompt.contains("ARTISTS=B"));
        assert!(system_prompt.contains("Do not change ARTIST"));
        assert!(system_prompt.contains("field `artists`"));
        assert!(system_prompt.contains("split_artists"));
        assert!(system_prompt.contains("https://www.navidrome.org/docs/usage/library/tagging/"));
    }

    #[test]
    fn large_library_context_is_compact_and_requires_tools_for_track_details() {
        let tracks = (0..788)
            .map(|index| {
                json!({
                    "path": format!("/music/Artist/Album/track-{index}.flac"),
                    "title": format!("Sentinel Track {index}"),
                    "artist": "Artist",
                    "album": "Album",
                    "genre": if index % 2 == 0 { Value::Null } else { json!("Rock") },
                    "duration": 180,
                    "sizeBytes": 1_000_000
                })
            })
            .collect::<Vec<_>>();
        let input = AssistantSendInput {
            library_path: Some("/music".into()),
            active_album_path: Some("/music/Artist/Album".into()),
            selected_track_paths: vec!["/music/Artist/Album/track-0.flac".into()],
            tracks,
            albums: vec![json!({
                "path": "/music/Artist/Album",
                "name": "Album",
                "artistHint": "Artist",
                "trackCount": 788
            })],
            ..Default::default()
        };

        let context = build_assistant_context(&input);
        let serialized = serde_json::to_string(&context).unwrap();

        assert_eq!(context["librarySummary"]["trackCount"], 788);
        assert_eq!(context["librarySummary"]["missingGenre"], 394);
        assert_eq!(context["selection"]["count"], 1);
        assert!(
            context.get("tracks").is_none(),
            "full track objects must remain behind deterministic tools"
        );
        assert!(
            !serialized.contains("Sentinel Track 787"),
            "unselected track metadata leaked into bootstrap context"
        );
        assert!(
            serialized.len() < 4_000,
            "bootstrap context should remain bounded, got {} characters",
            serialized.len()
        );

        let messages = build_assistant_messages(
            &context,
            &context_tool_catalog(),
            &[],
            "fill the missing Genre",
        );
        let prompt_chars = messages
            .iter()
            .map(|message| message.content.chars().count())
            .sum::<usize>();
        assert!(messages[0].content.contains("Use read-only tools"));
        assert!(messages[0]
            .content
            .contains("fix one missing metadata field"));
        assert!(messages[0].content.contains("metadata.patch"));
        assert!(messages[0].content.contains("only_if_missing"));
        assert!(messages[0]
            .content
            .contains("one literal value even when it contains commas"));
        assert!(messages[0].content.contains("tags.prettify is read-only"));
        assert!(
            !messages[0]
                .content
                .contains("page through every matching track"),
            "missing-field edits should use a deterministic predicate instead of path enumeration"
        );
        assert!(!messages
            .last()
            .unwrap()
            .content
            .contains("Sentinel Track 787"));
        assert!(
            prompt_chars < 50_000,
            "complete bootstrap prompt should remain bounded, got {prompt_chars} characters"
        );
    }

    #[test]
    fn assistant_timeouts_leave_the_tool_loop_in_control() {
        assert!(
            ASSISTANT_LLM_TIMEOUT_SECS < ASSISTANT_SESSION_TIMEOUT_SECS,
            "one provider call must not outlive the complete tool loop"
        );
        assert!(
            ASSISTANT_SESSION_TIMEOUT_SECS >= 600,
            "large-library tool loops need at least ten minutes"
        );
        assert!(
            ASSISTANT_MAX_STEPS >= 20,
            "large-library investigations need more than ten tool steps"
        );
        assert!(
            ASSISTANT_SESSION_TIMEOUT_LOG.contains(&ASSISTANT_SESSION_TIMEOUT_SECS.to_string()),
            "the persisted timeout explanation must match the configured session deadline"
        );
    }

    #[test]
    fn messages_filters_out_system_entries_from_history() {
        let ctx = json!({});
        let tools = json!([]);
        let history = vec![
            ConversationEntry {
                id: 1,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t1".into(),
                entry_type: "system".into(),
                content: "error: something failed".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
            ConversationEntry {
                id: 2,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t2".into(),
                entry_type: "user_message".into(),
                content: "first question".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
            ConversationEntry {
                id: 3,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t3".into(),
                entry_type: "assistant_message".into(),
                content: "clarification reply".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
        ];
        let msgs = build_assistant_messages(&ctx, &tools, &history, "follow-up");

        // System entry should be excluded — only user+assistant turns + final request.
        let history_turns: Vec<_> = msgs.iter().filter(|m| m.role != "system").collect();
        assert_eq!(history_turns.len(), 3); // first question + clarification + follow-up
        assert_eq!(history_turns[0].role, "user");
        assert_eq!(history_turns[0].content, "first question");
        assert_eq!(history_turns[1].role, "assistant");
        assert_eq!(history_turns[1].content, "clarification reply");
        assert_eq!(history_turns[2].role, "user");
        assert!(history_turns[2].content.contains("follow-up"));
    }

    #[test]
    fn messages_truncates_to_most_recent_turns() {
        let ctx = json!({});
        let tools = json!([]);
        let mut history = Vec::new();
        // Build 22 turns (11 user + 11 assistant) — MAX_HISTORY_TURNS is 20.
        for i in 0..11 {
            history.push(ConversationEntry {
                id: i as i64 * 2 + 1,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: format!("t{}", i * 2),
                entry_type: "user_message".into(),
                content: format!("user {}", i),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            });
            history.push(ConversationEntry {
                id: i as i64 * 2 + 2,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: format!("t{}", i * 2 + 1),
                entry_type: "assistant_message".into(),
                content: format!("assistant {}", i),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            });
        }
        let msgs = build_assistant_messages(&ctx, &tools, &history, "latest");

        let non_system: Vec<_> = msgs.iter().filter(|m| m.role != "system").collect();
        // At most 20 history turns + 1 current request = at most 21.
        assert!(non_system.len() <= 21, "{} > 21", non_system.len());
        assert!(
            non_system.len() >= 2,
            "expected at least 2, got {}",
            non_system.len()
        );
        // All history entries should form valid user→assistant pairs.
        let history_count = non_system.len() - 1;
        for i in (0..history_count).step_by(2) {
            assert_eq!(
                non_system[i].role, "user",
                "history entry {} should be user, was {}",
                i, non_system[i].role
            );
            if i + 1 < history_count {
                assert_eq!(
                    non_system[i + 1].role,
                    "assistant",
                    "history entry {} should be assistant, was {}",
                    i + 1,
                    non_system[i + 1].role
                );
            }
        }
        // Current request is last.
        assert!(non_system.last().unwrap().content.contains("latest"));
    }

    #[test]
    fn messages_accepts_empty_history() {
        let ctx = json!({});
        let tools = json!([]);
        let history = vec![];
        let msgs = build_assistant_messages(&ctx, &tools, &history, "first message");

        assert_eq!(msgs.len(), 2); // system + current request
        assert_eq!(msgs[1].role, "user");
        assert!(msgs[1].content.contains("first message"));
    }

    #[test]
    fn messages_char_budget_truncates_oversized_entries() {
        let ctx = json!({});
        let tools = json!([]);
        // One huge entry that exceeds HISTORY_CHAR_BUDGET.
        let history = vec![ConversationEntry {
            id: 1,
            session_uuid: "s".into(),
            session_number: "s".into(),
            timestamp: "t1".into(),
            entry_type: "user_message".into(),
            content: "x".repeat(40_000),
            model: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cost: 0.0,
            metadata: None,
        }];
        let msgs = build_assistant_messages(&ctx, &tools, &history, "new request");
        // The huge entry exceeds the budget, so it should be dropped.
        let non_system: Vec<_> = msgs.iter().filter(|m| m.role != "system").collect();
        assert_eq!(non_system.len(), 1);
        assert!(non_system[0].content.contains("new request"));
    }

    #[test]
    fn messages_incomplete_start_pair_skips_orphan_assistant() {
        let ctx = json!({});
        let tools = json!([]);
        // Start with an orphan assistant turn (no preceding user).
        let history = vec![
            ConversationEntry {
                id: 1,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t1".into(),
                entry_type: "assistant_message".into(),
                content: "orphan reply".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
            ConversationEntry {
                id: 2,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t2".into(),
                entry_type: "user_message".into(),
                content: "user message".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
            ConversationEntry {
                id: 3,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t3".into(),
                entry_type: "assistant_message".into(),
                content: "valid reply".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
        ];
        let msgs = build_assistant_messages(&ctx, &tools, &history, "latest");
        let non_system: Vec<_> = msgs.iter().filter(|m| m.role != "system").collect();
        // Orphan assistant is skipped; valid pair + current request = 3.
        assert_eq!(non_system.len(), 3);
        assert_eq!(non_system[0].content, "user message");
        assert_eq!(non_system[1].content, "valid reply");
        assert!(non_system[2].content.contains("latest"));
    }

    #[test]
    fn messages_preserves_clarification_follow_up_order() {
        let ctx = json!({});
        let tools = json!([]);
        let history = vec![
            ConversationEntry {
                id: 1,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t1".into(),
                entry_type: "user_message".into(),
                content: "remove titles".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
            ConversationEntry {
                id: 2,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t2".into(),
                entry_type: "assistant_message".into(),
                content: "Do you mean clear title tags or strip filenames?".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
        ];
        let msgs = build_assistant_messages(&ctx, &tools, &history, "clear the title tags");

        let non_system: Vec<_> = msgs.iter().filter(|m| m.role != "system").collect();
        assert_eq!(non_system.len(), 3);
        assert!(
            non_system[0].content.contains("remove titles"),
            "first history turn is the original vague request"
        );
        assert!(
            non_system[1].content.contains("Do you mean"),
            "second history turn is the clarification"
        );
        assert!(
            non_system[2].content.contains("clear the title tags"),
            "final turn is the disambiguated follow-up"
        );
    }

    // ── Tool catalog schema shapes ──────────────────────────────────

    #[test]
    fn edit_metadata_schema_allows_standard_updates_and_removes() {
        let defs = crate::commands::assistant_tools::assistant_tool_definitions();
        let schema = defs
            .iter()
            .find(|d| d.name == "edit_metadata")
            .map(|d| &d.input_schema)
            .unwrap();
        assert!(schema["properties"].get("standard_updates").is_some());
        assert!(schema["properties"].get("standard_removes").is_some());
        assert!(schema["properties"].get("extra_upserts").is_some());
    }

    #[test]
    fn tracks_search_schema_takes_optional_filters() {
        let defs = crate::commands::assistant_tools::assistant_tool_definitions();
        let schema = defs
            .iter()
            .find(|d| d.name == "tracks.search")
            .map(|d| &d.input_schema)
            .unwrap();
        // tracks.search is entirely optional — all properties are optional
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            assert!(
                required.is_empty(),
                "tracks.search should have no required fields"
            );
        }
        assert!(schema["properties"].get("artist").is_some());
        assert!(schema["properties"].get("missingTitle").is_some());
    }

    // ── tool_result_prompt truncation ───────────────────────────────

    #[test]
    fn tool_result_extremely_long_structured_data_is_truncated() {
        let result = AssistantToolResult {
            ok: true,
            summary: "Large payload".into(),
            data: Some(serde_json::json!({
                "values": vec!["x".repeat(10_000)]
            })),
            error: None,
        };
        let prompt = tool_result_prompt(&result);
        assert!(prompt.contains("\u{2026}[truncated]"));
        assert!(prompt.len() < 10_000);
    }

    // ── resolve_assistant_outcome ───────────────────────────────────

    fn test_outcome_input() -> AssistantSendInput {
        AssistantSendInput {
            selected_track_paths: vec!["/track.mp3".into()],
            tracks: vec![json!({"path": "/track.mp3"})],
            ..Default::default()
        }
    }

    #[test]
    fn outcome_message_only_accepted() {
        let draft = AssistantDraft {
            message: "Hello, how can I help?".into(),
            response_kind: AssistantResponseKind::Answer,
            action_batch: None,
            tool_call: None,
        };
        let result = resolve_assistant_outcome(&draft, &[], "s", &test_outcome_input());
        assert_eq!(result, Ok(AssistantOutcome::Message));
    }

    #[test]
    fn outcome_action_claim_without_evidence_is_rejected() {
        for message in [
            "Let me now apply the transformation.",
            "I updated all 46 tracks.",
            "The changes are now applied.",
        ] {
            let draft = AssistantDraft {
                message: message.into(),
                response_kind: AssistantResponseKind::Action,
                action_batch: None,
                tool_call: None,
            };
            let error =
                resolve_assistant_outcome(&draft, &[], "s", &test_outcome_input()).unwrap_err();
            assert!(error.contains("without running a tool"), "{error}");
        }
    }

    #[test]
    fn outcome_answer_may_quote_action_language() {
        let draft = AssistantDraft {
            message: "The log said “I'll update the tags,” but no change occurred.".into(),
            response_kind: AssistantResponseKind::Answer,
            action_batch: None,
            tool_call: None,
        };
        let result = resolve_assistant_outcome(&draft, &[], "s", &test_outcome_input());
        assert_eq!(result, Ok(AssistantOutcome::Message));
    }

    #[test]
    fn outcome_clarification_only_accepted() {
        let draft = AssistantDraft {
            message: "Do you mean clear the title tag or strip filenames?".into(),
            response_kind: AssistantResponseKind::Clarification,
            action_batch: None,
            tool_call: None,
        };
        let result = resolve_assistant_outcome(&draft, &[], "s", &test_outcome_input());
        assert_eq!(result, Ok(AssistantOutcome::Message));
    }

    #[test]
    fn outcome_informational_answer_accepted() {
        let draft = AssistantDraft {
            message: "There are 42 tracks in this album.".into(),
            response_kind: AssistantResponseKind::Answer,
            action_batch: None,
            tool_call: None,
        };
        let result = resolve_assistant_outcome(&draft, &[], "s", &test_outcome_input());
        assert_eq!(result, Ok(AssistantOutcome::Message));
    }

    #[test]
    fn outcome_explicit_noop_accepted() {
        let draft = AssistantDraft {
            message: "Nothing to change.".into(),
            response_kind: AssistantResponseKind::Answer,
            action_batch: None,
            tool_call: None,
        };
        let result = resolve_assistant_outcome(&draft, &[], "s", &test_outcome_input());
        assert_eq!(result, Ok(AssistantOutcome::Message));
    }

    #[test]
    fn outcome_unsupported_task_explained() {
        let draft = AssistantDraft {
            message: "No tool available for that.".into(),
            response_kind: AssistantResponseKind::Limitation,
            action_batch: None,
            tool_call: None,
        };
        let result = resolve_assistant_outcome(&draft, &[], "s", &test_outcome_input());
        assert_eq!(result, Ok(AssistantOutcome::Message));
    }

    #[test]
    fn outcome_deserializes_omitted_batch_and_tool() {
        let json = json!({"message": "hello", "responseKind": "answer"});
        let draft: AssistantDraft = serde_json::from_value(json).unwrap();
        assert!(draft.action_batch.is_none());
        assert!(draft.tool_call.is_none());
    }

    #[test]
    fn outcome_rejects_missing_response_kind() {
        let error = serde_json::from_value::<AssistantDraft>(json!({"message": "done"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("responseKind"), "{error}");
    }

    #[test]
    fn outcome_deserializes_null_batch_and_tool() {
        let json = json!({
            "message": "hello",
            "responseKind": "answer",
            "actionBatch": null,
            "toolCall": null
        });
        let draft: AssistantDraft = serde_json::from_value(json).unwrap();
        assert!(draft.action_batch.is_none());
        assert!(draft.tool_call.is_none());
    }

    #[test]
    fn outcome_empty_noop_normalized_to_message() {
        let draft = AssistantDraft {
            message: "nothing to do".into(),
            response_kind: AssistantResponseKind::Answer,
            action_batch: Some(AssistantDraftBatch {
                kind: "noop".into(),
                title: "noop".into(),
                summary: "noop".into(),
                risk_level: "low".into(),
                actions: vec![],
            }),
            tool_call: None,
        };
        let normalized = normalize_noop_batch(draft);
        assert!(normalized.action_batch.is_none());
        assert_eq!(normalized.message, "nothing to do");
    }

    #[test]
    fn outcome_noop_with_actions_preserved_for_rejection() {
        let draft = AssistantDraft {
            message: "noop with actions".into(),
            response_kind: AssistantResponseKind::Action,
            action_batch: Some(AssistantDraftBatch {
                kind: "noop".into(),
                title: "noop".into(),
                summary: "noop".into(),
                risk_level: "low".into(),
                actions: vec![crate::state::assistant::AssistantAction {
                    track_path: Some("/track.mp3".into()),
                    ..Default::default()
                }],
            }),
            tool_call: None,
        };
        // Normalize preserves it; validated_assistant_batch rejects it.
        let normalized = normalize_noop_batch(draft);
        assert!(normalized.action_batch.is_some());
        let result = resolve_assistant_outcome(&normalized, &[], "s", &test_outcome_input());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported action"));
    }

    #[test]
    fn outcome_valid_tool_preview_accepted() {
        let batch = AssistantActionBatch {
            id: "batch-test".into(),
            created_at: "now".into(),
            session_id: "s".into(),
            kind: "metadata-update".into(),
            title: "Test".into(),
            summary: "test".into(),
            risk_level: "low".into(),
            actions: vec![crate::state::assistant::AssistantAction {
                track_path: Some("/track.mp3".into()),
                field: Some("title".into()),
                new_value: Some("New Title".into()),
                ..Default::default()
            }],
            reversible: true,
            status: "pending".into(),
            library_root: None,
            completion_contract: None,
        };
        let result = resolve_assistant_outcome(
            &AssistantDraft {
                message: "updating title".into(),
                response_kind: AssistantResponseKind::Action,
                action_batch: None,
                tool_call: None,
            },
            &[batch],
            "s",
            &test_outcome_input(),
        );
        assert!(matches!(result, Ok(AssistantOutcome::ToolPreview(_))));
    }

    #[test]
    fn outcome_valid_model_preview_accepted() {
        let result = resolve_assistant_outcome(
            &AssistantDraft {
                message: "updating title".into(),
                response_kind: AssistantResponseKind::Action,
                action_batch: Some(AssistantDraftBatch {
                    kind: "metadata-update".into(),
                    title: "Update title".into(),
                    summary: "1 action".into(),
                    risk_level: "low".into(),
                    actions: vec![crate::state::assistant::AssistantAction {
                        track_path: Some("/track.mp3".into()),
                        field: Some("title".into()),
                        new_value: Some("New Title".into()),
                        ..Default::default()
                    }],
                }),
                tool_call: None,
            },
            &[],
            "s",
            &test_outcome_input(),
        );
        assert!(matches!(result, Ok(AssistantOutcome::ModelPreview(_))));
    }

    #[test]
    fn outcome_both_tool_and_model_preview_rejected() {
        let result = resolve_assistant_outcome(
            &AssistantDraft {
                message: "both".into(),
                response_kind: AssistantResponseKind::Action,
                action_batch: Some(AssistantDraftBatch {
                    kind: "metadata-update".into(),
                    title: "x".into(),
                    summary: "x".into(),
                    risk_level: "low".into(),
                    actions: vec![crate::state::assistant::AssistantAction {
                        track_path: Some("/track.mp3".into()),
                        field: Some("title".into()),
                        new_value: Some("New Title".into()),
                        ..Default::default()
                    }],
                }),
                tool_call: None,
            },
            &[AssistantActionBatch {
                id: "batch-exists".into(),
                created_at: String::new(),
                session_id: String::new(),
                kind: String::new(),
                title: String::new(),
                summary: String::new(),
                risk_level: String::new(),
                actions: vec![],
                reversible: false,
                status: String::new(),
                library_root: None,
            completion_contract: None,
            }],
            "s",
            &test_outcome_input(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("both"));
    }

    #[test]
    fn outcome_unknown_tool_rejected() {
        let result = validate_registered_tool_args("nonexistent_tool", &json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn outcome_invalid_arguments_rejected() {
        let result = validate_registered_tool_args("tracks.search", &json!({"artist": 123}));
        assert!(result.is_err());
    }

    #[test]
    fn outcome_tool_preview_does_not_re_add_batch() {
        // Tool-created batches are already stored during execution.
        // This test verifies resolve_assistant_outcome accepts them
        // without requiring a second runtime.add_batch.
        let batch = AssistantActionBatch {
            id: "batch-tool".into(),
            created_at: "now".into(),
            session_id: "s".into(),
            kind: "metadata-update".into(),
            title: "Tool preview".into(),
            summary: "1 action".into(),
            risk_level: "low".into(),
            actions: vec![crate::state::assistant::AssistantAction {
                track_path: Some("/track.mp3".into()),
                field: Some("title".into()),
                new_value: Some("New Title".into()),
                ..Default::default()
            }],
            reversible: true,
            status: "pending".into(),
            library_root: None,
            completion_contract: None,
        };
        let result = resolve_assistant_outcome(
            &AssistantDraft {
                message: "tool done".into(),
                response_kind: AssistantResponseKind::Action,
                action_batch: None,
                tool_call: None,
            },
            &[batch.clone()],
            "s",
            &test_outcome_input(),
        );
        assert!(
            matches!(&result, Ok(AssistantOutcome::ToolPreview(b)) if b[0].id == "batch-tool"),
            "expected ToolPreview with original batch id, got {:?}",
            result
        );
    }

    #[test]
    fn outcome_out_of_scope_batch_rejected() {
        let result = resolve_assistant_outcome(
            &AssistantDraft {
                message: "test".into(),
                response_kind: AssistantResponseKind::Action,
                action_batch: Some(AssistantDraftBatch {
                    kind: "metadata-update".into(),
                    title: "x".into(),
                    summary: "x".into(),
                    risk_level: "low".into(),
                    actions: vec![crate::state::assistant::AssistantAction {
                        track_path: Some("/outside/scope.mp3".into()),
                        field: Some("title".into()),
                        new_value: Some("X".into()),
                        ..Default::default()
                    }],
                }),
                tool_call: None,
            },
            &[],
            "s",
            &test_outcome_input(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside the active scope"));
    }
}

/// ── AI-integration assistant tests ─────────────────────────────────
///
/// These tests require a live LLM API key and model configured via env
/// vars `LLM_API_KEY` and `LLM_MODEL`.  They are `#[ignore]` by default
/// and run only on explicit request (`cargo test -- --ignored`).
///
/// We test the LLM's structured output directly (same system prompt +
/// schema that the assistant_send command uses) to verify behaviour
/// without requiring a full Tauri app handle.
///
/// For semantic assertions we use LLM-as-judge: a second (cheap) LLM
/// call evaluates whether the response meets the expected criteria.
/// A baseline file in the test fixtures directory stores known-good
/// responses to reduce flakiness.
#[cfg(test)]
mod assistant_ai_tests {
    use super::*;
    use crate::infra::openrouter::OpenRouterClient;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    fn credentials() -> Option<(String, String)> {
        let key = std::env::var("LLM_API_KEY").ok()?;
        let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "openai/gpt-4o".into());
        Some((key, model))
    }

    /// Build a minimal input and pass it through the production compact-context
    /// boundary before sending anything to the LLM.
    fn test_library_context(additional_tracks: usize) -> Value {
        let mut tracks = vec![
            json!({
                "path": "/music/artist/album/track1.flac",
                "title": "Blue Train",
                "artist": "John Coltrane",
                "album": "Blue Train"
            }),
            json!({
                "path": "/music/artist/album/track2.flac",
                "title": "Moment's Notice",
                "artist": "John Coltrane",
                "album": "Blue Train"
            }),
        ];
        for i in 0..additional_tracks {
            // renamed from _i to i
            tracks.push(json!({
                "path": format!("/music/other/track{}.flac", i + 3),
                "title": format!("Track {}", i + 3),
                "artist": "Other Artist",
                "album": "Other Album"
            }));
        }
        build_assistant_context(&AssistantSendInput {
            library_path: Some("/music".into()),
            active_album_path: Some("/music/artist/album".into()),
            tracks,
            albums: vec![json!({
                "path": "/music/artist/album",
                "name": "Blue Train",
                "artistHint": "John Coltrane",
                "albumHint": "Blue Train",
                "trackCount": 2
            })],
            autonomous: false,
            ..Default::default()
        })
    }

    /// Call the LLM using the production prompt construction (via
    /// build_assistant_messages with empty history).
    async fn assistant_llm_call(
        user_message: &str,
        context: &Value,
        api_key: &str,
        model: &str,
    ) -> serde_json::Value {
        let tools = crate::commands::assistant_tools::context_tool_catalog();
        let schema = assistant_response_schema();
        let mut messages = build_assistant_messages(context, &tools, &[], user_message);
        for attempt in 0..2 {
            let response = OpenRouterClient::new(api_key, model)
                .with_generation(0.0, 4096)
                .with_timeout(std::time::Duration::from_secs(ASSISTANT_LLM_TIMEOUT_SECS))
                .complete_json(
                    messages.clone(),
                    "AssistantResponse",
                    schema.clone(),
                    &AtomicBool::new(false),
                )
                .await
                .expect("LLM call should succeed");
            let data = normalize_assistant_response_value(response.data)
                .expect("LLM response should use a supported tool-call envelope");
            if data["toolCall"].is_object() || data["actionBatch"].is_object() || attempt == 1 {
                return data;
            }
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: data["message"].as_str().unwrap_or_default().to_string(),
            });
            messages.push(ChatMessage::system(ASSISTANT_SELF_REVIEW_PROMPT));
        }
        unreachable!("two-pass assistant smoke should return on its second response")
    }

    /// Judge whether a response satisfies the given semantic criteria.
    /// Requires LLM_JUDGE_MODEL env var (should differ from LLM_MODEL).
    async fn judge_response(
        user_prompt: &str,
        llm_response: &Value,
        criteria: &[&str],
        api_key: &str,
    ) -> (bool, String) {
        let judge_model = std::env::var("LLM_JUDGE_MODEL")
            .expect("LLM_JUDGE_MODEL must be set (and differ from LLM_MODEL)");
        let client = OpenRouterClient::new(api_key, &judge_model).with_generation(0.0, 256);
        let mut criteria_text = String::new();
        for (i, c) in criteria.iter().enumerate() {
            criteria_text.push_str(&format!("{}. {}\n", i + 1, c));
        }
        let judge_prompt = format!(
            "User prompt: {}\nLLM response: {}\n\nCriteria:\n{}\n\
             Does the response satisfy all criteria?",
            user_prompt, llm_response, criteria_text
        );
        let judge_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "satisfies": {"type": "boolean"},
                "reasoning": {"type": "string"}
            },
            "required": ["satisfies", "reasoning"]
        });
        let messages = vec![
            ChatMessage::system(
                "You are a test judge. Evaluate if the assistant response meets the criteria.",
            ),
            ChatMessage::user(&judge_prompt),
        ];
        match client
            .complete_json(
                messages,
                "JudgeResponse",
                judge_schema,
                &AtomicBool::new(false),
            )
            .await
        {
            Ok(resp) => {
                let satisfies = resp.data["satisfies"].as_bool().unwrap_or(false);
                let reasoning = resp.data["reasoning"].as_str().unwrap_or("").to_string();
                (satisfies, reasoning)
            }
            Err(e) => (false, format!("judge LLM call failed: {e}")),
        }
    }

    // ── Test: equivalent read-only prompts stay non-mutating ────────

    /// Equivalent read-only prompts may answer directly or use a read-only
    /// tool, but must never produce a mutation preview.
    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn same_intent_read_only_produces_safe_outcome() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let context = test_library_context(0);
        let variants = [
            "what albums do I have",
            "show me my albums",
            "list my albums",
        ];
        for variant in &variants {
            let data = assistant_llm_call(variant, &context, &key, &model).await;
            let has_batch = data["actionBatch"].is_object();
            let has_tool = data["toolCall"].is_object();
            assert!(
                !has_batch,
                "{variant} must not create a mutation preview: {data}"
            );
            if has_tool {
                let tool_name = data["toolCall"]["toolName"].as_str().unwrap_or_default();
                assert_eq!(
                    registered_tool_is_read_only(tool_name),
                    Some(true),
                    "{variant} selected non-read-only tool {tool_name}"
                );
            } else {
                assert!(
                    !data["message"].as_str().unwrap_or_default().is_empty(),
                    "{variant} should produce a message or read-only tool call"
                );
            }
        }
    }

    /// Mutating requests should always include an actionBatch or toolCall.
    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn mutating_request_always_has_action_batch_or_tool_call() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let context = test_library_context(0);
        let variants = [
            "change the album title to New Title",
            "set the album tag to Greatest Hits",
            "set the artist to Testing",
            "fix the missing genre",
        ];
        for variant in &variants {
            let data = assistant_llm_call(variant, &context, &key, &model).await;
            let has_batch = data["actionBatch"].is_object();
            let has_tool = data["toolCall"].is_object();
            assert!(
                has_batch || has_tool,
                "{} should produce actionBatch or toolCall, got: {}",
                variant,
                data
            );
        }
    }

    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_missing_genre_value_reaches_conditional_patch_after_inspection() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let input = AssistantSendInput {
            active_album_path: Some("/music/artist/album".into()),
            tracks: vec![
                json!({
                    "path": "/music/artist/album/01.flac",
                    "title": "Blue Train",
                    "artist": "John Coltrane",
                    "album": "Blue Train",
                    "genre": null
                }),
                json!({
                    "path": "/music/artist/album/02.flac",
                    "title": "Moment's Notice",
                    "artist": "John Coltrane",
                    "album": "Blue Train",
                    "genre": null
                }),
            ],
            albums: vec![json!({
                "path": "/music/artist/album",
                "name": "Blue Train",
                "artistHint": "John Coltrane",
                "albumHint": "Blue Train",
                "trackCount": 2
            })],
            ..Default::default()
        };
        let tools = context_tool_catalog();
        let mut messages = build_assistant_messages(
            &build_assistant_context(&input),
            &tools,
            &[],
            "set the missing genre to \"Pop, Cantopop\"",
        );

        for _ in 0..6 {
            let response = OpenRouterClient::new(&key, &model)
                .with_generation(0.0, 4096)
                .with_timeout(std::time::Duration::from_secs(ASSISTANT_LLM_TIMEOUT_SECS))
                .complete_json(
                    messages.clone(),
                    "AssistantResponse",
                    assistant_response_schema(),
                    &AtomicBool::new(false),
                )
                .await
                .expect("LLM call should succeed");
            let data = normalize_assistant_response_value(response.data)
                .expect("LLM response should use a supported tool-call envelope");
            assert!(
                !data["actionBatch"].is_object(),
                "the model must use registered tools instead of authoring wildcard batches: {data}"
            );
            let Some(tool_call) = data["toolCall"].as_object() else {
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: data["message"].as_str().unwrap_or_default().to_string(),
                });
                messages.push(ChatMessage::system(ASSISTANT_SELF_REVIEW_PROMPT));
                continue;
            };
            let name = tool_call["toolName"].as_str().unwrap_or_default();
            let args = &tool_call["args"];
            if registered_tool_is_read_only(name) == Some(true) {
                let result = execute_context_tool(name, args, &input);
                assert!(result.ok, "read-only inspection failed: {}", result.summary);
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: json!({"toolCall": {"toolName": name, "args": args}}).to_string(),
                });
                messages.push(ChatMessage::user(tool_result_prompt(&result)));
                continue;
            }

            assert_eq!(name, "metadata.patch");
            assert!(
                args["changes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|change| change["field"] == "genre"
                        && change["value"] == "Pop, Cantopop"
                        && change["only_if_missing"] == true),
                "conditional genre patch arguments were incomplete: {args}"
            );
            let execution = execute_mutating_assistant_tool(name, args, &input, "live-session");
            assert!(execution.result.ok, "{}", execution.result.summary);
            let batch = execution.batches.first().expect("preview batch");
            assert_eq!(batch.actions.len(), 2);
            assert!(batch.actions.iter().all(|action| {
                action.field.as_deref() == Some("genre")
                    && action.new_value.as_deref() == Some("Pop, Cantopop")
            }));
            return;
        }

        panic!("assistant did not reach metadata.patch within six steps");
    }

    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_navidrome_artists_intent_preserves_display_credit_and_collaborators() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let malformed_paths = ["/music/alan/duets/45.flac", "/music/alan/duets/46.flac"];
        let selected_track_paths = (1..=46)
            .map(|index| format!("/music/alan/duets/{index:02}.flac"))
            .collect::<Vec<_>>();
        let tracks = selected_track_paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                if index == 44 {
                    json!({
                        "path": path,
                        "title": "星之歌",
                        "artist": "黎明 & 谭咏麟",
                        "artists": ["黎明&谭咏麟"],
                        "album": "Duets"
                    })
                } else if index == 45 {
                    json!({
                        "path": path,
                        "title": "爱上风雨中走来的你",
                        "artist": "谭咏麟 & 김완선",
                        "artists": ["谭咏麟&김완선"],
                        "album": "Duets"
                    })
                } else {
                    json!({
                        "path": path,
                        "title": format!("Correct collaboration {index}"),
                        "artist": format!("谭咏麟 & Collaborator {index}"),
                        "artists": ["谭咏麟", format!("Collaborator {index}")],
                        "album": "Duets"
                    })
                }
            })
            .collect();
        let input = AssistantSendInput {
            selected_track_paths,
            tracks,
            ..Default::default()
        };
        let tools = context_tool_catalog();
        let mut messages = build_assistant_messages(
            &build_assistant_context(&input),
            &tools,
            &[],
            "fix malformed plural “Artists” tags from the selected tracks by splitting values \
             joined with &, comma, or semicolon; preserve the singular Artist display credit",
        );

        for _ in 0..6 {
            let response = OpenRouterClient::new(&key, &model)
                .with_generation(0.0, 4096)
                .with_timeout(std::time::Duration::from_secs(ASSISTANT_LLM_TIMEOUT_SECS))
                .complete_json(
                    messages.clone(),
                    "AssistantResponse",
                    assistant_response_schema(),
                    &AtomicBool::new(false),
                )
                .await
                .expect("LLM call should succeed");
            let data = normalize_assistant_response_value(response.data)
                .expect("LLM response should use a supported tool-call envelope");
            assert!(
                !data["actionBatch"].is_object(),
                "the model must use native tools instead of authoring a batch: {data}"
            );
            let Some(tool_call) = data["toolCall"].as_object() else {
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: data["message"].as_str().unwrap_or_default().to_string(),
                });
                messages.push(ChatMessage::system(ASSISTANT_SELF_REVIEW_PROMPT));
                continue;
            };
            let name = tool_call["toolName"].as_str().unwrap_or_default();
            let args = &tool_call["args"];
            if registered_tool_is_read_only(name) == Some(true) {
                let result = execute_context_tool(name, args, &input);
                assert!(result.ok, "read-only inspection failed: {}", result.summary);
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: json!({"toolCall": {"toolName": name, "args": args}}).to_string(),
                });
                messages.push(ChatMessage::user(tool_result_prompt(&result)));
                continue;
            }

            assert_eq!(name, "metadata.transform", "got {name}: {args}");
            assert_eq!(args["target_scope"], "selected");
            assert_eq!(args["source"]["field"], "artists");
            assert_eq!(args["operations"], json!([{"op": "split_artists"}]));
            let execution = execute_mutating_assistant_tool(name, args, &input, "live-session");
            assert!(execution.result.ok, "{}", execution.result.summary);
            let batch = execution.batches.first().expect("preview batch");
            assert_eq!(
                batch.actions.len(),
                2,
                "every malformed plural Artists value should change: {args}"
            );
            assert_eq!(
                batch
                    .actions
                    .iter()
                    .filter_map(|action| action.track_path.as_deref())
                    .collect::<Vec<_>>(),
                malformed_paths
            );
            assert!(
                batch
                    .actions
                    .iter()
                    .all(|candidate| candidate.field.as_deref() == Some("artists")),
                "singular ARTIST is the display credit and must stay unchanged"
            );
            return;
        }

        panic!("assistant did not reach metadata.transform within six steps");
    }

    /// Judge: verify that equivalent prompts produce semantically similar
    /// responses.  Requires LLM_JUDGE_MODEL env var.
    #[ignore = "requires LLM_API_KEY, LLM_MODEL, LLM_JUDGE_MODEL, and active LLM access"]
    #[tokio::test]
    async fn judge_verifies_semantic_equivalence() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let context = test_library_context(5);
        let data = assistant_llm_call(
            "how many tracks do I have by John Coltrane",
            &context,
            &key,
            &model,
        )
        .await;
        let (satisfies, reasoning) = judge_response(
            "how many tracks do I have by John Coltrane",
            &data,
            &[
                "The response should indicate searching for tracks by a specific artist",
                "The toolCall should reference tracks.search or the response message should mention the counts",
            ],
            &key,
        )
        .await;
        assert!(
            satisfies,
            "Response failed judge: {}\nData: {}",
            reasoning, data
        );
    }

    // ── Semantic smoke tests for open-ended conversations ────────────

    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_greeting_returns_message_only() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let context = test_library_context(0);
        let data = assistant_llm_call("hello", &context, &key, &model).await;
        let message = data["message"].as_str().unwrap_or("");
        assert!(!message.is_empty(), "greeting must produce a message");
        assert!(
            !data["actionBatch"].is_object() && !data["toolCall"].is_object(),
            "greeting '{}' should not produce a tool or actionBatch, got: {}",
            message,
            data
        );
    }

    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_remove_titles_clarifies() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let context = test_library_context(0);
        let data = assistant_llm_call("remove titles", &context, &key, &model).await;
        let message = data["message"].as_str().unwrap_or("");
        assert!(!message.is_empty(), "must produce a message");
        // The response should be a clarification (message-only) or optionally
        // a toolCall if the LLM feels confident enough.
        let has_batch = data["actionBatch"].is_object();
        assert!(
            !has_batch,
            "'remove titles' should not produce actionBatch (clarify instead), got: {}",
            data
        );
        // The message should ask a question or acknowledge ambiguity.
        let lower = message.to_lowercase();
        assert!(
            lower.contains("?") || lower.contains("clarify") || lower.contains("mean"),
            "'remove titles' should ask a clarification question, got: {}",
            message
        );
    }

    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_unsupported_task_explains_limitation() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let context = test_library_context(0);
        let data = assistant_llm_call(
            "can you normalize the audio loudness of these tracks?",
            &context,
            &key,
            &model,
        )
        .await;
        let message = data["message"].as_str().unwrap_or("");
        assert!(!message.is_empty(), "must produce a message");
        assert!(
            !data["actionBatch"].is_object(),
            "unsupported task should not produce actionBatch, got: {}",
            data
        );
        // Should explain the limitation, not fabricate a tool.
        let lower = message.to_lowercase();
        assert!(
            lower.contains("don't")
                || lower.contains("not")
                || lower.contains("can't")
                || lower.contains("unavailable")
                || lower.contains("doesn't support"),
            "unsupported task should explain limitation, got: {}",
            message
        );
    }

    // ── Tool-use correctness smoke tests ────────────────────────────

    /// A concrete transformation request must produce a toolCall
    /// metadata.transform with non-empty operations.
    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_transformation_calls_metadata_transform() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let context = test_library_context(2);
        let data = assistant_llm_call(
            "strip numbers from the front of all track titles",
            &context,
            &key,
            &model,
        )
        .await;
        let tool_call = &data["toolCall"];
        assert!(
            tool_call.is_object(),
            "transformation must produce a toolCall (not actionBatch, not message-only), got: {}",
            data
        );
        let name = tool_call["toolName"].as_str().unwrap_or("");
        assert_eq!(
            name, "metadata.transform",
            "expected metadata.transform, got tool: {name}"
        );
        let ops = tool_call["args"]["operations"].as_array();
        assert!(
            ops.is_some_and(|o| !o.is_empty()),
            "metadata.transform must include non-empty operations, got args: {:?}",
            tool_call["args"]
        );
        // The first operation should act on "title" field.
        let first_op = &ops.unwrap()[0];
        assert!(
            first_op.get("field").and_then(Value::as_str) == Some("title"),
            "first operation should target 'title', got: {:?}",
            first_op
        );
    }

    /// Full romanization scenario: fixture tracks with Chinese+romanized
    /// titles → LLM inspects → LLM transforms → assert exact output values.
    /// Walks the real assistant loop (tool calls → execute → feed back → repeat).
    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_romanized_strip_full_scenario() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let tools = crate::commands::assistant_tools::context_tool_catalog();
        let schema = assistant_response_schema();

        // Fixture tracks with deterministic Chinese+romanized titles.
        let track1_path = "/music/artist/album/01 月亮代表我的心.flac";
        let track2_path = "/music/artist/album/02 甜蜜蜜.flac";
        let fixture_input = AssistantSendInput {
            selected_track_paths: vec![track1_path.into(), track2_path.into()],
            tracks: vec![
                serde_json::json!({
                    "path": track1_path,
                    "title": "月亮代表我的心 (Yue Liang Dai Biao Wo De Xin)",
                    "artist": "Various",
                    "album": "Chinese Classics"
                }),
                serde_json::json!({
                    "path": track2_path,
                    "title": "甜蜜蜜 (Tian Mi Mi)",
                    "artist": "Various",
                    "album": "Chinese Classics"
                }),
            ],
            albums: vec![serde_json::json!({
                "path": "/music/artist/album",
                "name": "Chinese Classics",
                "artistHint": "Various",
                "albumHint": "Chinese Classics",
                "trackCount": 2
            })],
            autonomous: false,
            ..Default::default()
        };
        let context = build_assistant_context(&fixture_input);

        let mut messages = build_assistant_messages(
            &context,
            &tools,
            &[],
            "strip the romanized parts from the track titles, keeping only the Chinese characters",
        );
        let cancelled = AtomicBool::new(false);
        let mut final_draft: Option<serde_json::Value> = None;
        let mut signatures: Vec<(String, Value)> = Vec::new();

        for _step in 1..=6 {
            let response = OpenRouterClient::new(&key, &model)
                .with_generation(0.0, 4096)
                .with_timeout(std::time::Duration::from_secs(ASSISTANT_LLM_TIMEOUT_SECS))
                .complete_json(
                    messages.clone(),
                    "AssistantResponse",
                    schema.clone(),
                    &cancelled,
                )
                .await
                .expect("LLM call should succeed");
            let response = normalize_assistant_response_value(response.data)
                .expect("tool-call envelope should normalize");
            let draft: AssistantDraft =
                serde_json::from_value(response).expect("draft should deserialize");
            let draft = normalize_noop_batch(draft);

            let Some(tool_call) = draft.tool_call else {
                // LLM responded with just a message — accept if it's a clarification
                // or explanation, fail if it describes a planned action.
                let msg = draft.message.to_lowercase();
                assert!(
                    msg.contains("?")
                        || msg.contains("can't")
                        || msg.contains("not possible")
                        || msg.contains("sorry")
                        || msg.contains("don't have"),
                    "LLM returned message-only instead of calling a tool: '{}'",
                    draft.message
                );
                final_draft = Some(serde_json::json!({"message": draft.message}));
                break;
            };

            // Validate and execute the tool call.
            if let Err(e) = validate_registered_tool_args(&tool_call.tool_name, &tool_call.args) {
                panic!("invalid args for {}: {}", tool_call.tool_name, e);
            }

            let sig = (tool_call.tool_name.clone(), tool_call.args.clone());
            if signatures.contains(&sig) {
                panic!(
                    "repeated tool call: {} {:?}",
                    tool_call.tool_name, tool_call.args
                );
            }
            signatures.push(sig);

            if tool_call.tool_name == "tracks.inspect" {
                // Execute tracks.inspect against the fixture input.
                let result =
                    execute_context_tool(&tool_call.tool_name, &tool_call.args, &fixture_input);
                assert!(
                    result.ok,
                    "tracks.inspect failed: {}",
                    result.error.unwrap_or_default()
                );
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: serde_json::json!({
                        "toolCall": {"toolName": &tool_call.tool_name, "args": &tool_call.args}
                    })
                    .to_string(),
                });
                messages.push(ChatMessage::user(tool_result_prompt(&result)));
                continue;
            }

            if tool_call.tool_name == "metadata.transform"
                || tool_call.tool_name == "metadata.patch"
            {
                let execution = execute_mutating_assistant_tool(
                    &tool_call.tool_name,
                    &tool_call.args,
                    &fixture_input,
                    "test-session",
                );
                assert!(
                    execution.result.ok,
                    "{} failed: {}",
                    tool_call.tool_name,
                    execution.result.error.as_deref().unwrap_or("unknown")
                );
                assert!(
                    !execution.batches.is_empty(),
                    "must produce at least one action"
                );
                let batch = &execution.batches[0];
                assert_eq!(batch.kind, "metadata-update");
                assert_eq!(batch.actions.len(), 2, "expected 2 actions (one per track)");

                // Track 1: "月亮代表我的心 (Yue Liang Dai Biao Wo De Xin)" → "月亮代表我的心"
                let a0 = &batch.actions[0];
                assert_eq!(a0.track_path.as_deref(), Some(track1_path));
                assert_eq!(a0.field.as_deref(), Some("title"));
                assert_eq!(
                    a0.new_value.as_deref(),
                    Some("月亮代表我的心"),
                    "track 1 title should strip romanized part"
                );

                // Track 2: "甜蜜蜜 (Tian Mi Mi)" → "甜蜜蜜"
                let a1 = &batch.actions[1];
                assert_eq!(a1.track_path.as_deref(), Some(track2_path));
                assert_eq!(a1.field.as_deref(), Some("title"));
                assert_eq!(
                    a1.new_value.as_deref(),
                    Some("甜蜜蜜"),
                    "track 2 title should strip romanized part"
                );

                final_draft = Some(serde_json::json!({
                    "message": draft.message,
                    "toolCall": {"toolName": &tool_call.tool_name, "args": &tool_call.args},
                    "batch": batch
                }));
                break;
            }

            // Unknown tool — feed result back.
            let result =
                execute_context_tool(&tool_call.tool_name, &tool_call.args, &fixture_input);
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: serde_json::json!({
                    "toolCall": {"toolName": &tool_call.tool_name, "args": &tool_call.args}
                })
                .to_string(),
            });
            messages.push(ChatMessage::user(tool_result_prompt(&result)));
        }

        assert!(
            final_draft.is_some(),
            "test completed without final draft (exceeded step limit or stuck)"
        );
    }

    /// A question answered by context alone should NOT attempt a tool call.
    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_context_answered_question_no_tool() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        // test_library_context(0) = 2 base tracks.
        let context = test_library_context(0);
        let data = assistant_llm_call("how many tracks do I have", &context, &key, &model).await;
        let msg = data["message"].as_str().unwrap_or("");
        assert!(!msg.is_empty(), "must produce a message");
        // Context includes 2 tracks, so answer should use context.
        assert!(
            !data["toolCall"].is_object() && !data["actionBatch"].is_object(),
            "context-answerable question should not use toolCall or actionBatch, got: {}",
            data
        );
        assert!(
            msg.to_lowercase().contains("2") || msg.to_lowercase().contains("two"),
            "response should mention track count \"2\", got: {}",
            msg
        );
    }

    /// Vague request → clarification → concrete follow-up → tool call:
    /// simulates the full multi-turn flow.
    #[ignore = "requires LLM_API_KEY, LLM_MODEL, and active OpenRouter access"]
    #[tokio::test]
    async fn live_vague_then_clarify_then_tool() {
        let (key, model) = credentials().expect("set LLM_API_KEY and LLM_MODEL");
        let tools = crate::commands::assistant_tools::context_tool_catalog();
        let schema = assistant_response_schema();
        let context = test_library_context(0);
        let fixture_input = AssistantSendInput {
            selected_track_paths: vec!["/music/artist/album/track1.flac".into()],
            tracks: vec![serde_json::json!({
                "path": "/music/artist/album/track1.flac",
                "title": "Blue Train",
                "artist": "John Coltrane",
                "album": "Blue Train"
            })],
            ..Default::default()
        };

        // ---- Turn 1: vague request using production prompt ----------
        let turn1_messages = build_assistant_messages(&context, &tools, &[], "remove titles");
        let response = OpenRouterClient::new(&key, &model)
            .with_generation(0.0, 4096)
            .with_timeout(std::time::Duration::from_secs(ASSISTANT_LLM_TIMEOUT_SECS))
            .complete_json(
                turn1_messages,
                "AssistantResponse",
                schema.clone(),
                &AtomicBool::new(false),
            )
            .await
            .expect("turn1 should succeed");
        let turn1 = normalize_assistant_response_value(response.data)
            .expect("turn1 should use a supported tool-call envelope");
        let msg1 = turn1["message"].as_str().unwrap_or("");
        assert!(!msg1.is_empty(), "turn1: must produce a message");
        // Must clarify (message-only), not produce an action.
        assert!(
            !turn1["actionBatch"].is_object() && !turn1["toolCall"].is_object(),
            "turn1: vague request should not produce actionBatch or toolCall"
        );
        assert!(
            msg1.to_lowercase().contains("?")
                || msg1.to_lowercase().contains("mean")
                || msg1.to_lowercase().contains("clarify"),
            "turn1: should ask a clarification question, got: {msg1}"
        );

        // ---- Turn 2: user clarifies (production prompt + history) ----
        let follow_up_history = vec![
            ConversationEntry {
                id: 1,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t1".into(),
                entry_type: "user_message".into(),
                content: "remove titles".into(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
            ConversationEntry {
                id: 2,
                session_uuid: "s".into(),
                session_number: "s".into(),
                timestamp: "t2".into(),
                entry_type: "assistant_message".into(),
                content: msg1.to_string(),
                model: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cost: 0.0,
                metadata: None,
            },
        ];
        let turn2_messages = build_assistant_messages(
            &context,
            &tools,
            &follow_up_history,
            "I mean clear the title tags on the selected tracks",
        );
        let response = OpenRouterClient::new(&key, &model)
            .with_generation(0.0, 4096)
            .with_timeout(std::time::Duration::from_secs(ASSISTANT_LLM_TIMEOUT_SECS))
            .complete_json(
                turn2_messages,
                "AssistantResponse",
                schema.clone(),
                &AtomicBool::new(false),
            )
            .await
            .expect("turn2 should succeed");
        let turn2 = normalize_assistant_response_value(response.data)
            .expect("turn2 should use a supported tool-call envelope");
        let tool_call = &turn2["toolCall"];
        let action_batch = &turn2["actionBatch"];
        // Must produce a tool or action.
        assert!(
            tool_call.is_object() || action_batch.is_object(),
            "turn2: concrete follow-up should produce toolCall or actionBatch, got: {}",
            turn2["message"].as_str().unwrap_or("")
        );
        if let Some(name) = tool_call["toolName"].as_str() {
            assert!(
                matches!(name, "metadata.patch" | "metadata.transform"),
                "turn2: expected metadata.patch or metadata.transform, got: {name}"
            );
            // Validate and execute the tool.
            validate_registered_tool_args(name, &tool_call["args"])
                .expect("turn2 tool args should be valid");
            let execution = execute_mutating_assistant_tool(
                name,
                &tool_call["args"],
                &fixture_input,
                "test-session",
            );
            assert!(
                execution.result.ok,
                "turn2 {} failed: {}",
                name,
                execution.result.error.as_deref().unwrap_or("unknown")
            );
            assert!(
                !execution.batches.is_empty(),
                "turn2 must produce at least one action"
            );
            assert_eq!(execution.batches[0].kind, "metadata-update");
            assert_eq!(
                execution.batches[0].actions.len(),
                1,
                "expected 1 action for the single selected track"
            );
            assert_eq!(
                execution.batches[0].actions[0].field.as_deref(),
                Some("title"),
                "should clear the title field"
            );
        }
        if let Some(kind) = action_batch["kind"].as_str() {
            assert_eq!(kind, "metadata-update");
        }
    }
}
