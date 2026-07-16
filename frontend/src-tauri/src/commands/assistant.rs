//! Assistant runtime and tool-service commands.

use crate::commands::{
    mutations::{write_track_queued, TrackPatch},
    tracks::read_track_metadata,
};
use crate::error::ApiError;
use crate::state::assistant::{
    AssistantActionBatch, AssistantRuntimeState, AssistantServicesConfig, AssistantServicesState,
};
use crate::state::conversation::ConversationState;
use crate::state::write_queue::WriteQueue;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssistantEvent {
    session_id: String,
    #[serde(rename = "type")]
    event_type: &'static str,
    message: String,
    data: Option<Value>,
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
) -> Value {
    let actions = batch.actions.iter().filter(|action| {
        action.track_path.is_some()
            && action.field.is_some()
            && (!metadata_only || action.tag_kind.as_deref() == Some("standard"))
    });
    let mut updates: Vec<(String, TrackPatch)> = Vec::new();
    for action in actions {
        let path = action.track_path.as_ref().expect("filtered").clone();
        let field = action.field.as_deref().expect("filtered");
        let patch = match action_patch(field, action.new_value.as_deref()) {
            Ok(patch) => patch,
            Err(error) => {
                runtime.mark_batch_failed(batch_id, &error.to_string());
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
            Ok(track) => undo.push(serde_json::json!({ "path": path, "metadata": track })),
            Err(error) => {
                runtime.mark_batch_failed(batch_id, &error.to_string());
                return serde_json::json!({ "success": false, "error": error.to_string(), "undoSnapshots": undo });
            }
        }
    }
    let mut results = Vec::new();
    for (path, patch) in updates {
        match write_track_queued(queue, PathBuf::from(&path), patch).await {
            Ok(()) => results.push(serde_json::json!({ "trackPath": path, "success": true, "updatedTrack": read_track_metadata(Path::new(&path)).ok() })),
            Err(error) => results.push(serde_json::json!({ "trackPath": path, "success": false, "error": error.to_string() })),
        }
    }
    let failed = results
        .iter()
        .filter(|result| result["success"] == false)
        .count();
    if failed > 0 {
        let error = format!("Failed to update {failed} track(s)");
        runtime.mark_batch_failed(batch_id, &error);
        serde_json::json!({ "success": false, "error": error, "results": results.into_iter().filter(|result| result["success"] == false).collect::<Vec<_>>(), "undoSnapshots": undo })
    } else {
        runtime.mark_batch_applied(batch_id);
        serde_json::json!({ "success": true, "results": results, "undoSnapshots": undo })
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
        "tag-update" => apply_standard_actions(runtime, queue, &batch, batch_id, false).await,
        "metadata-update" => {
            let has_extra = batch
                .actions
                .iter()
                .any(|action| action.tag_kind.as_deref() == Some("extra"));
            if has_extra {
                let error = "Metadata batch includes extra tags; combined apply is not implemented";
                runtime.mark_batch_failed(batch_id, error);
                serde_json::json!({ "success": false, "error": error })
            } else {
                apply_standard_actions(runtime, queue, &batch, batch_id, true).await
            }
        }
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
