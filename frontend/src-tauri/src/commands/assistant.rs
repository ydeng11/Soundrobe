//! Assistant runtime and tool-service commands.

use crate::commands::{
    mutations::{
        rename_track_queued, write_extra_tags_queued, write_track_queued, ExtraTagUpdate,
        TrackPatch,
    },
    tracks::{read_extra_tags, read_track_metadata},
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
