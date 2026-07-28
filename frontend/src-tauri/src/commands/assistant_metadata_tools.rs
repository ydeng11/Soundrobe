//! Metadata and file transformation tools for the AI assistant.
//!
//! Provides pure pipeline operations and tool executors for:
//! - `metadata.patch` — uniform or per-track set / remove / upsert of tag fields
//! - `metadata.transform` — deterministic transformation pipeline on a tag field
//! - `files.transform` — transformation pipeline for filenames and paths
//!
//! These replace the narrow macro tools while keeping the same action-batch model.

pub(crate) use crate::commands::assistant::AssistantSendInput;
use crate::commands::assistant::{
    action_value_string, assistant_batch, extra_action, mutating_tool_error,
    mutating_tool_execution, mutating_tool_no_changes, push_string_action, track_field_string,
    track_path, MutatingToolExecution,
};
use crate::state::assistant::AssistantAction;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

// ── Re-exported helpers from assistant.rs ────────────────────────────

pub(crate) use crate::commands::assistant::path_is_inside;
pub(crate) use crate::commands::assistant::tool_scope_paths;

// ── Operation pipeline functions ────────────────────────────────────
// Each op is a pure function: `fn(&str) -> Option<String>`.
// None means "no change" — the track is skipped.

pub(crate) fn op_regex_replace(text: &str, pattern: &str, replacement: &str) -> Option<String> {
    let re = regex::Regex::new(pattern).ok()?;
    let result = re.replace_all(text, replacement).into_owned();
    (result != text).then_some(result)
}

pub(crate) fn op_regex_extract(text: &str, pattern: &str, group_index: usize) -> Option<String> {
    let re = regex::Regex::new(pattern).ok()?;
    let extracted = re.captures(text)?.get(group_index)?.as_str().to_string();
    (extracted != text).then_some(extracted)
}

pub(crate) fn op_strip_prefix(text: &str, prefix: &str) -> Option<String> {
    text.strip_prefix(prefix).map(|s| s.to_string())
}

pub(crate) fn op_strip_suffix(text: &str, suffix: &str) -> Option<String> {
    text.strip_suffix(suffix).map(|s| s.to_string())
}

pub(crate) fn op_literal_replace(text: &str, find: &str, replacement: &str) -> Option<String> {
    let result = text.replace(find, replacement);
    (result != text).then_some(result)
}

pub(crate) fn op_trim(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (trimmed != text).then_some(trimmed.to_string())
}

pub(crate) fn op_lowercase(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    (lower != text).then_some(lower)
}

pub(crate) fn op_uppercase(text: &str) -> Option<String> {
    let upper = text.to_uppercase();
    (upper != text).then_some(upper)
}

pub(crate) fn op_title_case(text: &str) -> Option<String> {
    let result = text
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().to_string() + chars.as_str().to_lowercase().as_str()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (result != text).then_some(result)
}

/// A compiled pipeline step ready for execution.
#[derive(Clone, Debug)]
pub(crate) enum PipelineOp {
    RegexReplace {
        pattern: regex::Regex,
        replacement: String,
    },
    RegexExtract {
        pattern: regex::Regex,
        group_index: usize,
    },
    StripPrefix {
        prefix: String,
    },
    StripSuffix {
        suffix: String,
    },
    LiteralReplace {
        find: String,
        replacement: String,
    },
    Trim,
    Lowercase,
    Uppercase,
    TitleCase,
    Prettify,
    ChineseToSimplified,
    ChineseToTraditional,
}

/// Compile and validate a pipeline of operation descriptors.
/// Returns the compiled pipeline or a descriptive error.
pub(crate) fn compile_pipeline(operations: &[Value]) -> Result<Vec<PipelineOp>, String> {
    let mut pipeline = Vec::new();
    for (i, op_def) in operations.iter().enumerate() {
        let op_name = op_def
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Pipeline operation {}: missing 'op' field", i))?;
        let op = match op_name {
            "regex_replace" => {
                let pattern_str = op_def
                    .get("pattern")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{}: missing 'pattern' field", op_name))?;
                let pattern = regex::Regex::new(pattern_str)
                    .map_err(|e| format!("{}: invalid regex '{}': {}", op_name, pattern_str, e))?;
                let replacement = op_def
                    .get("replacement")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                PipelineOp::RegexReplace {
                    pattern,
                    replacement,
                }
            }
            "regex_extract" => {
                let pattern_str = op_def
                    .get("pattern")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{}: missing 'pattern' field", op_name))?;
                let pattern = regex::Regex::new(pattern_str)
                    .map_err(|e| format!("{}: invalid regex '{}': {}", op_name, pattern_str, e))?;
                let group_index = op_def
                    .get("group_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(1) as usize;
                PipelineOp::RegexExtract {
                    pattern,
                    group_index,
                }
            }
            "strip_prefix" => {
                let prefix = op_def
                    .get("prefix")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{}: missing 'prefix' field", op_name))?;
                if prefix.is_empty() {
                    return Err(format!("{}: 'prefix' cannot be empty", op_name));
                }
                PipelineOp::StripPrefix {
                    prefix: prefix.to_string(),
                }
            }
            "strip_suffix" => {
                let suffix = op_def
                    .get("suffix")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{}: missing 'suffix' field", op_name))?;
                if suffix.is_empty() {
                    return Err(format!("{}: 'suffix' cannot be empty", op_name));
                }
                PipelineOp::StripSuffix {
                    suffix: suffix.to_string(),
                }
            }
            "literal_replace" => {
                let find = op_def
                    .get("find")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{}: missing 'find' field", op_name))?;
                if find.is_empty() {
                    return Err(format!("{}: 'find' cannot be empty", op_name));
                }
                let replacement = op_def
                    .get("replacement")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                PipelineOp::LiteralReplace {
                    find: find.to_string(),
                    replacement,
                }
            }
            "trim" => PipelineOp::Trim,
            "lowercase" => PipelineOp::Lowercase,
            "uppercase" => PipelineOp::Uppercase,
            "title_case" => PipelineOp::TitleCase,
            "prettify" => PipelineOp::Prettify,
            "chinese_to_simplified" => PipelineOp::ChineseToSimplified,
            "chinese_to_traditional" => PipelineOp::ChineseToTraditional,
            _ => return Err(format!("Unknown pipeline operation: {op_name}")),
        };
        pipeline.push(op);
    }
    Ok(pipeline)
}

/// Execute a compiled pipeline on a text value.
fn execute_pipeline(text: &str, pipeline: &[PipelineOp]) -> Option<String> {
    let mut current = text.to_string();
    let mut changed = false;
    for op in pipeline {
        let result = match op {
            PipelineOp::RegexReplace {
                pattern,
                replacement,
            } => op_regex_replace(&current, pattern.as_str(), replacement),
            PipelineOp::RegexExtract {
                pattern,
                group_index,
            } => op_regex_extract(&current, pattern.as_str(), *group_index),
            PipelineOp::StripPrefix { prefix } => op_strip_prefix(&current, prefix),
            PipelineOp::StripSuffix { suffix } => op_strip_suffix(&current, suffix),
            PipelineOp::LiteralReplace { find, replacement } => {
                op_literal_replace(&current, find, replacement)
            }
            PipelineOp::Trim => op_trim(&current),
            PipelineOp::Lowercase => op_lowercase(&current),
            PipelineOp::Uppercase => op_uppercase(&current),
            PipelineOp::TitleCase => op_title_case(&current),
            PipelineOp::Prettify => op_prettify(&current),
            PipelineOp::ChineseToSimplified => op_chinese_to_simplified(&current),
            PipelineOp::ChineseToTraditional => op_chinese_to_traditional(&current),
        };
        match result {
            Some(new_value) => {
                changed = true;
                current = new_value;
            }
            None => {}
        }
    }
    changed.then_some(current)
}

pub(crate) fn op_prettify(text: &str) -> Option<String> {
    let prettified = crate::commands::assistant_tools::prettify_tag(text);
    (prettified != text).then_some(prettified)
}

pub(crate) fn op_chinese_to_simplified(text: &str) -> Option<String> {
    let converted = crate::state::providers::convert_chinese_text(text, "simplified");
    (converted != text).then_some(converted)
}

pub(crate) fn op_chinese_to_traditional(text: &str) -> Option<String> {
    let converted = crate::state::providers::convert_chinese_text(text, "traditional");
    (converted != text).then_some(converted)
}

/// Apply a pipeline of operation descriptors to a text value.
/// Each descriptor is a JSON object with an "op" field.
/// Returns None if no operation changed the text.

// ── Tool executors ──────────────────────────────────────────────────

/// Execute `metadata.patch`: apply uniform and/or per-track changes to tag fields.
pub(crate) fn execute_metadata_patch(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve metadata patch target scope");
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }

    // Validate field names upfront
    if let Some(changes) = args.get("changes").and_then(Value::as_array) {
        for change in changes {
            let field = change
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let tag_kind = change
                .get("tag_kind")
                .and_then(Value::as_str)
                .unwrap_or("standard");
            if !is_valid_field(field, tag_kind) {
                return mutating_tool_error(format!("Unknown standard field: '{field}'"));
            }
            let action_type = change
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("set");
            let only_if_missing = change
                .get("only_if_missing")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if only_if_missing && action_type != "set" {
                return mutating_tool_error(
                    "only_if_missing is supported only for set actions".to_string(),
                );
            }
            if only_if_missing && tag_kind != "standard" {
                return mutating_tool_error(
                    "only_if_missing is supported only for standard fields".to_string(),
                );
            }
            if matches!(action_type, "set" | "upsert")
                && change.get("value").is_none_or(Value::is_null)
            {
                return mutating_tool_error(format!(
                    "Action '{action_type}' requires a 'value' for field '{field}'"
                ));
            }
        }
    }
    if let Some(per_track) = args.get("per_track_changes").and_then(Value::as_array) {
        for entry in per_track {
            if let Some(changes) = entry.get("changes").and_then(Value::as_array) {
                for change in changes {
                    let field = change
                        .get("field")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let action_type = change
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("set");
                    if !is_valid_field(field, "standard") {
                        return mutating_tool_error(format!("Unknown standard field: '{field}'"));
                    }
                    if matches!(action_type, "set")
                        && change.get("value").is_none_or(Value::is_null)
                    {
                        return mutating_tool_error(format!(
                            "Action '{action_type}' requires a 'value' for field '{field}'"
                        ));
                    }
                }
            }
        }
    }

    let tracks_map: BTreeMap<&str, &Value> = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect();

    let mut actions: Vec<AssistantAction> = Vec::new();

    // Uniform changes
    if let Some(changes) = args.get("changes").and_then(Value::as_array) {
        for change in changes {
            let field = change
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let action_type = change
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("set");
            let tag_kind = change
                .get("tag_kind")
                .and_then(Value::as_str)
                .unwrap_or("standard");
            let only_if_missing = change
                .get("only_if_missing")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let value = change.get("value");

            match action_type {
                "set" => {
                    for path in &paths {
                        let track = tracks_map.get(path.as_str()).copied();
                        if only_if_missing
                            && !track.is_some_and(|track| {
                                track_field_string(track, field)
                                    .is_none_or(|value| value.trim().is_empty())
                            })
                        {
                            continue;
                        }
                        if let Some(v) = value {
                            if let Some(str_val) = v.as_str() {
                                if str_val.trim().is_empty() && is_unique_field(field) {
                                    return mutating_tool_error(format!(
                                        "Blank value for field '{field}' is not allowed"
                                    ));
                                }
                            }
                            let desired = action_value_string(v).unwrap_or_default();
                            push_string_action(
                                &mut actions,
                                track,
                                path,
                                field,
                                &desired,
                                &format!("Set {field} to {desired}"),
                            );
                        }
                    }
                }
                "remove" => {
                    for path in &paths {
                        let track = tracks_map.get(path.as_str()).copied();
                        let old_value = track.and_then(|t| track_field_string(t, field));
                        if old_value.is_some() {
                            actions.push(AssistantAction {
                                tag_kind: Some(tag_kind.into()),
                                track_path: Some(path.clone()),
                                field: Some(field.into()),
                                old_value,
                                new_value: None,
                                operation: Some("remove".into()),
                                description: Some(format!("Remove {field}")),
                                ..Default::default()
                            });
                        }
                    }
                }
                "upsert" => {
                    // Only valid for extra tags
                    if tag_kind != "extra" {
                        return mutating_tool_error(String::from(
                            "Upsert is only supported for extra tags",
                        ));
                    }
                    let key = field.to_string();
                    let val = value
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    for path in &paths {
                        actions.push(extra_action(path, &key, Some(&val), "upsert"));
                    }
                }
                other => {
                    return mutating_tool_error(format!("Unknown action type: {other}"));
                }
            }
        }
    }

    // Per-track changes
    if let Some(per_track) = args.get("per_track_changes").and_then(Value::as_array) {
        let allowed: HashSet<&str> = paths.iter().map(String::as_str).collect();
        for entry in per_track {
            let entry_path = entry
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !allowed.contains(entry_path) {
                return mutating_tool_error(format!(
                    "Per-track change path '{}' is not in the resolved scope",
                    entry_path
                ));
            }
            let track = tracks_map.get(entry_path).copied();
            if let Some(changes) = entry.get("changes").and_then(Value::as_array) {
                for change in changes {
                    let field = change
                        .get("field")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let action_type = change
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("set");
                    let value = change.get("value");
                    match action_type {
                        "set" => {
                            if let Some(v) = value {
                                if let Some(str_val) = v.as_str() {
                                    if str_val.trim().is_empty() && is_unique_field(field) {
                                        return mutating_tool_error(format!(
                                            "Blank value for field '{field}' is not allowed"
                                        ));
                                    }
                                }
                                let desired = action_value_string(v).unwrap_or_default();
                                push_string_action(
                                    &mut actions,
                                    track,
                                    entry_path,
                                    field,
                                    &desired,
                                    &format!("Set {field} to {desired}"),
                                );
                            }
                        }
                        "remove" => {
                            let old_value = track.and_then(|t| track_field_string(t, field));
                            if old_value.is_some() {
                                actions.push(AssistantAction {
                                    tag_kind: Some("standard".into()),
                                    track_path: Some(entry_path.into()),
                                    field: Some(field.into()),
                                    old_value,
                                    new_value: None,
                                    operation: Some("remove".into()),
                                    description: Some(format!("Remove {field}")),
                                    ..Default::default()
                                });
                            }
                        }
                        other => {
                            return mutating_tool_error(format!(
                                "Unknown per-track action type: {other}"
                            ));
                        }
                    }
                }
            }
        }
    }

    if actions.is_empty() {
        return mutating_tool_no_changes("No metadata changes are needed.");
    }

    let affected_tracks = actions
        .iter()
        .filter_map(|action| action.track_path.as_deref())
        .collect::<HashSet<_>>()
        .len();
    let summary = format!(
        "Update {} metadata field(s) across {} track(s)",
        actions.len(),
        affected_tracks
    );
    let batch = assistant_batch(
        session_id,
        "metadata-update",
        "Patch metadata",
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

/// Execute `metadata.transform`: apply a pipeline of operations to a tag field.
pub(crate) fn execute_metadata_transform(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve metadata transform target scope");
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }

    let source = args
        .get("source")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| {
            [("kind".into(), Value::String("tag".into()))]
                .into_iter()
                .collect()
        });
    let source_kind = source.get("kind").and_then(Value::as_str).unwrap_or("tag");
    let source_field = source
        .get("field")
        .and_then(Value::as_str)
        .unwrap_or("title");

    let destination = args
        .get("destination")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| source.clone());
    let dest_kind = destination
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or(source_kind);
    let dest_field = destination
        .get("field")
        .and_then(Value::as_str)
        .unwrap_or(source_field);

    let Some(operations) = args.get("operations").and_then(Value::as_array) else {
        return mutating_tool_error("Transform requires an 'operations' array".to_string());
    };
    if operations.is_empty() {
        return mutating_tool_no_changes("No operations specified.");
    }

    // Compile pipeline once before processing tracks
    let pipeline = match compile_pipeline(operations) {
        Ok(p) => p,
        Err(e) => return mutating_tool_error(e),
    };

    let tracks_map: BTreeMap<&str, &Value> = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect();

    let mut actions: Vec<AssistantAction> = Vec::new();

    for path in &paths {
        let source_value = match source_kind {
            "tag" => {
                let track = tracks_map.get(path.as_str()).copied();
                track
                    .and_then(|t| track_field_string(t, source_field))
                    .filter(|v| !v.is_empty())
            }
            "filename" => {
                // Extract filename stem (without extension)
                let stem = Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string);
                stem.filter(|s| !s.is_empty())
            }
            other => {
                return mutating_tool_error(format!("Unsupported source kind: {other}"));
            }
        };

        let Some(current) = source_value else {
            // No source value available, skip track
            continue;
        };

        let result = execute_pipeline(&current, &pipeline);

        let Some(new_value) = result else {
            // Pipeline produced no change
            continue;
        };

        match dest_kind {
            "tag" => {
                let track = tracks_map.get(path.as_str()).copied();
                push_string_action(
                    &mut actions,
                    track,
                    path,
                    dest_field,
                    &new_value,
                    &format!("Transform {dest_field} from '{current}' to '{new_value}'"),
                );
            }
            "filename" => {
                // File rename action
                let old_dir = Path::new(path).parent().unwrap_or(Path::new(""));
                let extension = Path::new(path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{e}"))
                    .unwrap_or_default();
                let new_filename = format!("{new_value}{extension}");
                let dest_path = old_dir.join(&new_filename);
                actions.push(AssistantAction {
                    tag_kind: None,
                    track_path: Some(path.clone()),
                    field: None,
                    old_value: Some(current),
                    new_value: Some(new_value),
                    operation: Some("rename".into()),
                    source_path: Some(path.clone()),
                    destination_path: Some(dest_path.to_string_lossy().into_owned()),
                    description: Some(format!("Rename to '{new_filename}'")),
                    ..Default::default()
                });
            }
            other => {
                return mutating_tool_error(format!("Unsupported destination kind: {other}"));
            }
        }
    }

    if actions.is_empty() {
        return mutating_tool_no_changes("No transformations produced changes.");
    }

    let summary = format!(
        "Transform {} field(s) across {} track(s)",
        actions.len(),
        paths.len()
    );
    let risk = if dest_kind == "filename" {
        "medium"
    } else {
        "low"
    };
    let batch = assistant_batch(
        session_id,
        if dest_kind == "filename" {
            "folder-move"
        } else {
            "metadata-update"
        },
        "Transform metadata",
        &summary,
        risk,
        actions,
        true,
    );
    mutating_tool_execution(
        format!("Preview created ({}): {summary}", batch.id),
        None,
        Some(batch),
    )
}

/// Execute `files.transform`: apply operations to filenames/paths with path containment.
pub(crate) fn execute_files_transform(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let Ok(paths) = tool_scope_paths(input, args) else {
        return mutating_tool_error("Could not resolve files transform target scope");
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }

    let source = args
        .get("source")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| {
            [("kind".into(), Value::String("filename".into()))]
                .into_iter()
                .collect()
        });
    let source_kind = source
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("filename");
    let source_field = source
        .get("field")
        .and_then(Value::as_str)
        .unwrap_or("title");

    let Some(operations) = args.get("operations").and_then(Value::as_array) else {
        return mutating_tool_error("Transform requires an 'operations' array".to_string());
    };
    if operations.is_empty() {
        return mutating_tool_no_changes("No operations specified.");
    }

    // Compile pipeline once before processing tracks
    let pipeline = match compile_pipeline(operations) {
        Ok(p) => p,
        Err(e) => return mutating_tool_error(e),
    };

    // Require library root for file operations
    let Some(library) = input.library_path.as_deref().map(Path::new) else {
        return mutating_tool_error("Library root is required for file operations");
    };
    if !library.exists() || !library.is_dir() {
        return mutating_tool_error(format!(
            "Library path '{}' does not exist or is not a directory",
            library.display()
        ));
    }

    let tracks_map: BTreeMap<&str, &Value> = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect();

    let mut actions: Vec<AssistantAction> = Vec::new();

    for path in &paths {
        // Check path containment against library root
        if !path_is_inside(Path::new(path), library) {
            return mutating_tool_error(format!("Path '{}' is outside the library root", path));
        }

        let source_value = match source_kind {
            "tag" => {
                let track = tracks_map.get(path.as_str()).copied();
                track
                    .and_then(|t| track_field_string(t, source_field))
                    .filter(|v| !v.is_empty())
            }
            "filename" => {
                let stem = Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string);
                stem.filter(|s| !s.is_empty())
            }
            other => {
                return mutating_tool_error(format!("Unsupported files source kind: {other}"));
            }
        };

        let Some(current) = source_value else {
            continue;
        };

        let Some(new_value) = execute_pipeline(&current, &pipeline) else {
            continue;
        };

        if source_kind == "filename" && new_value == current {
            continue;
        }

        // Sanitize the new filename (reject path separators, `.`, `..`, and traversal)
        let sanitized = sanitize_filename(&new_value);
        if sanitized.is_empty() {
            return mutating_tool_error(format!(
                "Resulting filename is empty after sanitization for '{}'",
                path
            ));
        }

        // Build rename action
        let old_path = Path::new(path);
        let parent = old_path.parent().unwrap_or(Path::new(""));
        if !path_is_inside(parent, library) {
            return mutating_tool_error(format!(
                "Parent directory '{}' is outside the library root",
                parent.display()
            ));
        }
        let extension = old_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let new_filename = format!("{sanitized}{extension}");
        let dest_path = parent.join(&new_filename);

        // Prevent overwrite by adding suffix if destination exists
        let dest_path = if dest_path.exists() && dest_path != old_path {
            let mut counter = 1;
            let stem = Path::new(&new_filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&new_filename);
            loop {
                let candidate = parent.join(format!("{stem}_{counter}{extension}"));
                if !candidate.exists() || candidate == old_path {
                    break candidate;
                }
                counter += 1;
            }
        } else {
            dest_path
        };

        actions.push(AssistantAction {
            tag_kind: None,
            track_path: Some(path.clone()),
            field: None,
            old_value: Some(current),
            new_value: Some(new_value),
            operation: Some("rename".into()),
            source_path: Some(path.clone()),
            destination_path: Some(dest_path.to_string_lossy().into_owned()),
            description: Some(format!(
                "Rename to '{}'",
                dest_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&new_filename)
            )),
            ..Default::default()
        });
    }

    if actions.is_empty() {
        return mutating_tool_no_changes("No file transformations produced changes.");
    }

    let summary = format!(
        "Transform {} file(s) across {} track(s)",
        actions.len(),
        paths.len()
    );
    let batch = assistant_batch(
        session_id,
        "folder-move",
        "Transform files",
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

/// Standard tag fields that can be set/removed via metadata.patch.
const STANDARD_FIELDS: &[&str] = &[
    "title",
    "artist",
    "artists",
    "album",
    "albumArtist",
    "albumArtists",
    "year",
    "trackNumber",
    "trackTotal",
    "discNumber",
    "discTotal",
    "genre",
    "composer",
    "comment",
    "description",
    "lyrics",
    "compilation",
    "musicbrainzTrackId",
    "musicbrainzAlbumId",
    "musicbrainzArtistId",
    "discogsArtistId",
    "discogsReleaseId",
];

/// Sanitize a filename by removing path separators and traversal sequences.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .filter(|&c| !matches!(c, '/' | '\\' | '\0'))
        .collect::<String>()
        .trim()
        .to_string()
}

fn is_valid_field(field: &str, tag_kind: &str) -> bool {
    if tag_kind == "extra" {
        true // Extra tags have no fixed allowlist
    } else {
        STANDARD_FIELDS.contains(&field)
    }
}

/// Returns true for fields where a blank value is invalid.
fn is_unique_field(field: &str) -> bool {
    matches!(
        field,
        "title" | "artist" | "artists" | "trackNumber" | "trackTotal" | "discNumber" | "discTotal"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::assistant::AssistantSendInput;

    fn apply_pipeline_helper(
        text: &str,
        operations: &[serde_json::Value],
    ) -> Result<Option<String>, String> {
        let pipeline = compile_pipeline(operations)?;
        Ok(execute_pipeline(text, &pipeline))
    }

    fn test_input() -> AssistantSendInput {
        AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into(), "/music/b.mp3".into()],
            tracks: vec![
                serde_json::json!({
                    "path": "/music/a.mp3",
                    "title": "First Song",
                    "artist": "Artist A",
                    "album": "Album",
                    "genre": "Rock",
                    "year": "2020"
                }),
                serde_json::json!({
                    "path": "/music/b.mp3",
                    "title": "Second Song",
                    "artist": "Artist B",
                    "album": "Album",
                    "genre": "Jazz",
                    "year": "2021"
                }),
            ],
            ..Default::default()
        }
    }

    // ── execute_metadata_patch ────────────────────────────────────────────

    #[test]
    fn metadata_patch_uniform_set_produces_actions() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "genre", "action": "set", "value": "Pop"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(result.result.ok);
        assert!(result.result.summary.contains("Preview created"));
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        // Both tracks get genre: Pop
        assert!(batch
            .actions
            .iter()
            .all(|a| a.field.as_deref() == Some("genre")));
        assert!(batch
            .actions
            .iter()
            .all(|a| a.new_value.as_deref() == Some("Pop")));
    }

    #[test]
    fn metadata_patch_uniform_remove_produces_actions() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "genre", "action": "remove"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        assert!(batch
            .actions
            .iter()
            .all(|a| a.operation.as_deref() == Some("remove")));
    }

    #[test]
    fn metadata_patch_remove_missing_field_is_no_change() {
        let mut input = test_input();
        // Remove tracks' genres by clearing them from track data
        for track in &mut input.tracks {
            let obj = track.as_object_mut().unwrap();
            obj.remove("genre");
        }
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "genre", "action": "remove"}
                ]
            }),
            &input,
            "session-1",
        );
        assert!(result.result.ok);
        assert!(result.result.summary.contains("No metadata changes"));
        assert!(result.batches.is_empty());
    }

    #[test]
    fn metadata_patch_per_track_overrides_uniform() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "genre", "action": "set", "value": "Pop"}
                ],
                "per_track_changes": [
                    {
                        "path": "/music/a.mp3",
                        "changes": [
                            {"field": "title", "action": "set", "value": "Custom Title"}
                        ]
                    }
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        // 2 genre changes + 1 title change = 3 actions
        assert_eq!(batch.actions.len(), 3);
        let title_actions: Vec<_> = batch
            .actions
            .iter()
            .filter(|a| a.field.as_deref() == Some("title"))
            .collect();
        assert_eq!(title_actions.len(), 1);
        assert_eq!(title_actions[0].new_value.as_deref(), Some("Custom Title"));
    }

    #[test]
    fn metadata_patch_per_track_path_must_be_in_scope() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "per_track_changes": [
                    {
                        "path": "/music/outside.mp3",
                        "changes": [{"field": "title", "action": "set", "value": "X"}]
                    }
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("not in the resolved scope"));
    }

    #[test]
    fn metadata_patch_blank_unique_field_rejected() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "title", "action": "set", "value": ""}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("Blank value"));
    }

    #[test]
    fn metadata_patch_unknown_field_rejected() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "made_up_field_xyz", "action": "set", "value": "test"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("Unknown standard field"));
    }

    #[test]
    fn metadata_patch_set_missing_value_rejected() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "genre", "action": "set"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("requires a 'value'"));
    }

    #[test]
    fn metadata_patch_unknown_action_rejected() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "title", "action": "unknown", "value": "x"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("Unknown action"));
    }

    #[test]
    fn metadata_patch_empty_scope_is_no_change() {
        let input = AssistantSendInput {
            selected_track_paths: vec![],
            tracks: vec![],
            ..Default::default()
        };
        let result = execute_metadata_patch(
            &serde_json::json!({"target_scope": "selected"}),
            &input,
            "session-1",
        );
        assert!(result.result.ok);
        assert!(result.result.summary.contains("No tracks"));
    }

    #[test]
    fn metadata_patch_no_changes_when_same_value() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "genre", "action": "set", "value": "Rock"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        // Track a.mp3 already has genre "Rock" -> no change for that track
        // Track b.mp3 has genre "Jazz" -> change
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].track_path.as_deref(), Some("/music/b.mp3"));
    }

    #[test]
    fn metadata_patch_only_if_missing_preserves_existing_values() {
        let mut input = test_input();
        input.selected_track_paths.push("/music/c.mp3".into());
        input.tracks[0].as_object_mut().unwrap().remove("genre");
        input.tracks.push(serde_json::json!({
            "path": "/music/c.mp3",
            "title": "Third Song",
            "genre": "   "
        }));

        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [{
                    "field": "genre",
                    "action": "set",
                    "value": "Pop, Cantopop",
                    "only_if_missing": true
                }]
            }),
            &input,
            "session-1",
        );

        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        assert_eq!(
            batch
                .actions
                .iter()
                .filter_map(|action| action.track_path.as_deref())
                .collect::<Vec<_>>(),
            vec!["/music/a.mp3", "/music/c.mp3"]
        );
        assert!(batch
            .actions
            .iter()
            .all(|action| action.new_value.as_deref() == Some("Pop, Cantopop")));
        assert!(
            result.result.summary.contains("across 2 track(s)"),
            "summary must report affected tracks, not the three-track input scope"
        );
    }

    #[test]
    fn metadata_patch_extra_upsert_creates_extra_actions() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"tag_kind": "extra", "field": "MOOD", "action": "upsert", "value": "Calm"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        assert!(batch
            .actions
            .iter()
            .all(|a| a.tag_kind.as_deref() == Some("extra")));
        assert!(batch
            .actions
            .iter()
            .all(|a| a.operation.as_deref() == Some("upsert")));
    }

    #[test]
    fn metadata_patch_upsert_on_standard_rejected() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"tag_kind": "standard", "field": "title", "action": "upsert", "value": "X"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
    }

    // ── execute_metadata_transform ────────────────────────────────────────

    #[test]
    fn metadata_transform_tag_source_with_regex_replace() {
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "regex_replace", "pattern": "^First\\s*", "replacement": ""}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].track_path.as_deref(), Some("/music/a.mp3"));
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("Song"));
    }

    #[test]
    fn metadata_transform_tag_source_pipeline() {
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "lowercase"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("first song"));
        assert_eq!(batch.actions[1].new_value.as_deref(), Some("second song"));
    }

    #[test]
    fn metadata_transform_filename_source() {
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "filename"},
                "destination": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "title_case"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        // a.mp3 -> "a" -> title_case "A"
        // b.mp3 -> "b" -> title_case "B"
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("A"));
        assert_eq!(batch.actions[1].new_value.as_deref(), Some("B"));
    }

    #[test]
    fn metadata_transform_no_change_produces_no_actions() {
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "lowercase"}
                ]
            }),
            // Input with already-lowercase titles
            &AssistantSendInput {
                selected_track_paths: vec!["/music/c.mp3".into()],
                tracks: vec![serde_json::json!({"path": "/music/c.mp3", "title": "song"})],
                ..Default::default()
            },
            "session-1",
        );
        assert!(result.result.ok);
        assert!(result.batches.is_empty());
        assert!(result.result.summary.contains("No transformations"));
    }

    #[test]
    fn metadata_transform_empty_operations_is_no_change() {
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": []
            }),
            &test_input(),
            "session-1",
        );
        assert!(result.result.ok);
        assert!(result.result.summary.contains("No operations"));
    }

    #[test]
    fn metadata_transform_strip_track_numbers_via_pipeline() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/01_song.mp3".into()],
            tracks: vec![serde_json::json!({
                "path": "/music/01_song.mp3",
                "title": "01. Song Title"
            })],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "regex_replace", "pattern": "^\\d+\\.\\s*", "replacement": ""}
                ]
            }),
            &input,
            "session-1",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("Song Title"));
    }

    #[test]
    fn metadata_transform_unsupported_source_kind_rejected() {
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "invalid"},
                "operations": [{"op": "lowercase"}]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
    }

    #[test]
    fn metadata_transform_unknown_op_rejected() {
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [{"op": "nonexistent"}]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
    }

    // ── execute_files_transform ───────────────────────────────────────────

    #[test]
    fn files_transform_filename_strip_prefix() {
        // Create a temporary directory as library root with a test file
        let dir = std::env::temp_dir().join("soundrobe-test-files-transform");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("01_song.mp3");
        let _ = std::fs::write(&file_path, b"test");

        let result = execute_files_transform(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": [file_path.to_string_lossy().as_ref()],
                "source": {"kind": "filename"},
                "operations": [
                    {"op": "regex_replace", "pattern": "^\\d+_", "replacement": ""}
                ]
            }),
            &AssistantSendInput {
                selected_track_paths: vec![],
                tracks: vec![
                    serde_json::json!({"path": file_path.to_string_lossy().as_ref(), "title": "Song"}),
                ],
                library_path: Some(dir.to_string_lossy().into_owned()),
                ..Default::default()
            },
            "session-1",
        );
        assert!(
            result.result.ok,
            "files_transform failed: {}",
            result.result.error.as_deref().unwrap_or("unknown")
        );
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        let expected_dest = dir.join("song.mp3").to_string_lossy().into_owned();
        assert_eq!(
            batch.actions[0].destination_path.as_deref(),
            Some(expected_dest.as_str())
        );
        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_transform_no_change_produces_no_actions() {
        let dir = std::env::temp_dir().join("soundrobe-test-files-nochange");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("song.mp3");
        let _ = std::fs::write(&file_path, b"test");

        let result = execute_files_transform(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": [file_path.to_string_lossy().as_ref()],
                "source": {"kind": "filename"},
                "operations": [
                    {"op": "lowercase"}
                ]
            }),
            &AssistantSendInput {
                selected_track_paths: vec![],
                tracks: vec![
                    serde_json::json!({"path": file_path.to_string_lossy().as_ref(), "title": "Song"}),
                ],
                library_path: Some(dir.to_string_lossy().into_owned()),
                ..Default::default()
            },
            "session-1",
        );
        assert!(result.result.ok);
        assert!(result.result.summary.contains("No file transformations"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_transform_path_outside_library_rejected() {
        let dir = std::env::temp_dir().join("soundrobe-test-files-outside");
        let _ = std::fs::create_dir_all(&dir);

        let result = execute_files_transform(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": ["/outside/file.mp3"],
                "source": {"kind": "filename"},
                "operations": [
                    {"op": "lowercase"}
                ]
            }),
            &AssistantSendInput {
                selected_track_paths: vec![],
                tracks: vec![serde_json::json!({"path": "/outside/file.mp3", "title": "Song"})],
                library_path: Some(dir.to_string_lossy().into_owned()),
                ..Default::default()
            },
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("outside the library"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Compatibility tests: old macro behavior via new tools ─────────────

    #[test]
    fn compat_strip_track_title_prefixes_via_metadata_transform() {
        // Old macro: strip_track_title_prefixes("01. Song Title") -> "Song Title"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/01_song.mp3".into()],
            tracks: vec![serde_json::json!({
                "path": "/music/01_song.mp3",
                "title": "01. Song Title"
            })],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "regex_replace", "pattern": "^(?:\\d+[.)]\\s+|\\d+\\s*[-–]\\s+|\\d{1,3}\\s+)", "replacement": ""}
                ]
            }),
            &input,
            "session-compat",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("Song Title"));
        assert_eq!(batch.actions[0].field.as_deref(), Some("title"));
    }

    #[test]
    fn compat_chinese_convert_via_metadata_transform() {
        // Old macro: chinese_convert + "s2t" direction
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![serde_json::json!({
                "path": "/music/a.mp3",
                "title": "简体",
                "artist": "简体歌手"
            })],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [{"op": "chinese_to_traditional"}]
            }),
            &input,
            "session-compat",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("簡體"));
    }

    #[test]
    fn compat_regex_extract_via_metadata_transform() {
        // Old macro: extract_tag_value with regex capture group
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![serde_json::json!({
                "path": "/music/a.mp3",
                "title": "01 - Song Title (feat. Artist)"
            })],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [{"op": "regex_extract", "pattern": "^\\d+\\s*-\\s*(.+?)(?:\\s*\\(.*\\))?$", "group_index": 1}]
            }),
            &input,
            "session-compat",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("Song Title"));
    }

    // ── Catalog and schema validation tests ──────────────────────────────

    #[test]
    fn new_tool_schemas_validate_correctly() {
        // metadata.patch: valid uniform set
        crate::commands::assistant_tools::validate_registered_tool_args(
            "metadata.patch",
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [{"field": "genre", "action": "set", "value": "Pop"}]
            }),
        )
        .unwrap();

        // metadata.patch: valid per-track
        crate::commands::assistant_tools::validate_registered_tool_args(
            "metadata.patch",
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": ["/a.mp3"],
                "per_track_changes": [{
                    "path": "/a.mp3",
                    "changes": [{"field": "title", "action": "set", "value": "New"}]
                }]
            }),
        )
        .unwrap();

        // metadata.transform: valid operations
        crate::commands::assistant_tools::validate_registered_tool_args(
            "metadata.transform",
            &serde_json::json!({
                "target_scope": "selected",
                "operations": [{"op": "lowercase"}]
            }),
        )
        .unwrap();

        // metadata.transform: full pipeline
        crate::commands::assistant_tools::validate_registered_tool_args(
            "metadata.transform",
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "regex_replace", "pattern": "^\\d+", "replacement": ""},
                    {"op": "trim"}
                ]
            }),
        )
        .unwrap();

        // files.transform: valid
        crate::commands::assistant_tools::validate_registered_tool_args(
            "files.transform",
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "filename"},
                "operations": [{"op": "lowercase"}]
            }),
        )
        .unwrap();
    }

    #[test]
    fn new_tool_schemas_reject_invalid_args() {
        // metadata.patch: missing target_scope
        assert!(
            crate::commands::assistant_tools::validate_registered_tool_args(
                "metadata.patch",
                &serde_json::json!({"changes": []}),
            )
            .is_err()
        );

        // metadata.transform: missing operations
        assert!(
            crate::commands::assistant_tools::validate_registered_tool_args(
                "metadata.transform",
                &serde_json::json!({"target_scope": "selected"}),
            )
            .is_err()
        );

        // metadata.transform: unknown operation
        assert!(
            crate::commands::assistant_tools::validate_registered_tool_args(
                "metadata.transform",
                &serde_json::json!({
                    "target_scope": "selected",
                    "operations": [{"op": "nonexistent"}]
                }),
            )
            .is_err()
        );

        // metadata.patch: per_track_change missing path
        assert!(
            crate::commands::assistant_tools::validate_registered_tool_args(
                "metadata.patch",
                &serde_json::json!({
                    "target_scope": "selected",
                    "per_track_changes": [{"changes": [{"field": "title", "action": "set"}]}]
                }),
            )
            .is_err()
        );

        // files.transform: missing operations
        assert!(
            crate::commands::assistant_tools::validate_registered_tool_args(
                "files.transform",
                &serde_json::json!({"target_scope": "selected"}),
            )
            .is_err()
        );
    }

    #[test]
    fn new_tools_appear_in_catalog() {
        let catalog = crate::commands::assistant_tools::context_tool_catalog();
        let names: Vec<&str> = catalog
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert!(
            names.contains(&"metadata.patch"),
            "metadata.patch should be in catalog"
        );
        assert!(
            names.contains(&"metadata.transform"),
            "metadata.transform should be in catalog"
        );
        assert!(
            names.contains(&"files.transform"),
            "files.transform should be in catalog"
        );
        // Descriptions are present
        for entry in catalog.as_array().unwrap() {
            let desc = entry
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            assert!(
                !desc.is_empty(),
                "tool {} has no description",
                entry["name"]
            );
        }
    }

    #[test]
    fn new_tool_descriptions_are_meaningful() {
        let catalog = crate::commands::assistant_tools::context_tool_catalog();
        for entry in catalog.as_array().unwrap() {
            let name = entry["name"].as_str().unwrap();
            let desc = entry["description"].as_str().unwrap_or("");
            assert!(!desc.is_empty(), "Tool {} missing description", name);
            // Description should be longer than just a label
            assert!(
                desc.len() > 20,
                "Tool {} description too short: '{}'",
                name,
                desc
            );
        }
    }

    // ── Mocked user-example tests (no LLM) ────────────────────────────────

    #[test]
    fn example_remove_title_from_tracks() {
        // User: "remove title from these tracks"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![serde_json::json!({"path": "/music/a.mp3", "title": "Song"})],
            ..Default::default()
        };
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [{"field": "title", "action": "remove"}]
            }),
            &input,
            "session-eg",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].field.as_deref(), Some("title"));
        assert_eq!(batch.actions[0].operation.as_deref(), Some("remove"));
    }

    #[test]
    fn example_strip_track_numbers_from_titles() {
        // User: "strip leading track numbers from all titles"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/01_song.mp3".into()],
            tracks: vec![
                serde_json::json!({"path": "/music/01_song.mp3", "title": "01. Song Title"}),
            ],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "regex_replace", "pattern": "^\\d+\\.\\s*", "replacement": ""},
                    {"op": "trim"}
                ]
            }),
            &input,
            "session-eg",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("Song Title"));
    }

    #[test]
    fn example_convert_titles_to_lowercase() {
        // User: "convert all titles to lowercase"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![serde_json::json!({"path": "/music/a.mp3", "title": "Hello World"})],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [{"op": "lowercase"}]
            }),
            &input,
            "session-eg",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("hello world"));
    }

    #[test]
    fn example_extract_first_word_as_title() {
        // User: "extract the first word as the new title"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![serde_json::json!({"path": "/music/a.mp3", "title": "Hello World"})],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [{"op": "regex_extract", "pattern": "^(\\w+)", "group_index": 1}]
            }),
            &input,
            "session-eg",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("Hello"));
    }

    #[test]
    fn example_prettify_all_titles() {
        // User: "prettify all titles"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![serde_json::json!({"path": "/music/a.mp3", "title": "hello_world-song"})],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [{"op": "prettify"}]
            }),
            &input,
            "session-eg",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(
            batch.actions[0].new_value.as_deref(),
            Some("Hello World Song")
        );
    }

    #[test]
    fn example_set_album_to_greatest_hits() {
        // User: "set album to 'Greatest Hits'"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![
                serde_json::json!({"path": "/music/a.mp3", "title": "Song", "album": "Old Album"}),
            ],
            ..Default::default()
        };
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [{"field": "album", "action": "set", "value": "Greatest Hits"}]
            }),
            &input,
            "session-eg",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].field.as_deref(), Some("album"));
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("Greatest Hits"));
    }

    #[test]
    fn example_convert_chinese_to_traditional() {
        // User: "convert Chinese titles to Traditional"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![serde_json::json!({"path": "/music/a.mp3", "title": "简体中文"})],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [{"op": "chinese_to_traditional"}]
            }),
            &input,
            "session-eg",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("簡體中文"));
    }

    #[test]
    fn example_remove_parts_of_title() {
        // User: "remove (feat. ...) parts from titles"
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a.mp3".into()],
            tracks: vec![
                serde_json::json!({"path": "/music/a.mp3", "title": "Song (feat. Artist)"}),
            ],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "operations": [
                    {"op": "regex_replace", "pattern": "\\s*\\(.*\\)", "replacement": ""},
                    {"op": "trim"}
                ]
            }),
            &input,
            "session-eg",
        );
        assert!(result.result.ok);
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("Song"));
    }

    // ── op_regex_replace ────────────────────────────────────────────────

    #[test]
    fn regex_replace_matches_and_replaces() {
        assert_eq!(
            op_regex_replace("01. Song Title", r"^\d+\s*[-.]?\s*", ""),
            Some("Song Title".into())
        );
    }

    #[test]
    fn regex_replace_no_match_returns_none() {
        assert_eq!(op_regex_replace("Hello", r"^\d+", ""), None);
    }

    #[test]
    fn regex_replace_global_flag_replaces_all() {
        assert_eq!(op_regex_replace("a1b2c3", r"\d", ""), Some("abc".into()));
    }

    #[test]
    fn regex_replace_invalid_pattern_returns_none() {
        assert_eq!(op_regex_replace("test", r"(unclosed", ""), None);
    }

    // ── op_regex_extract ────────────────────────────────────────────────

    #[test]
    fn regex_extract_capture_group() {
        assert_eq!(
            op_regex_extract("01 - Song Title", r"^\d+\s*[-.]?\s*(.+)", 1),
            Some("Song Title".into())
        );
    }

    #[test]
    fn regex_extract_group_zero_returns_full_match() {
        assert_eq!(
            op_regex_extract("hello world", r"\w+", 0),
            Some("hello".into())
        );
    }

    #[test]
    fn regex_extract_no_match_returns_none() {
        assert_eq!(op_regex_extract("abc", r"\d+", 1), None);
    }

    #[test]
    fn regex_extract_missing_group_returns_none() {
        assert_eq!(op_regex_extract("abc", r"(a)(b)", 5), None);
    }

    #[test]
    fn regex_extract_invalid_pattern_returns_none() {
        assert_eq!(op_regex_extract("test", r"(unclosed", 1), None);
    }

    // ── op_strip_prefix ─────────────────────────────────────────────────

    #[test]
    fn strip_prefix_removes_leading_text() {
        assert_eq!(op_strip_prefix("01. Song", "01. "), Some("Song".into()));
    }

    #[test]
    fn strip_prefix_no_match_returns_none() {
        assert_eq!(op_strip_prefix("Song", "01. "), None);
    }

    #[test]
    fn strip_prefix_empty_prefix_returns_some() {
        assert_eq!(op_strip_prefix("hello", ""), Some("hello".into()));
    }

    // ── op_strip_suffix ─────────────────────────────────────────────────

    #[test]
    fn strip_suffix_removes_trailing_text() {
        assert_eq!(
            op_strip_suffix("Song (live)", " (live)"),
            Some("Song".into())
        );
    }

    #[test]
    fn strip_suffix_no_match_returns_none() {
        assert_eq!(op_strip_suffix("Song", " (live)"), None);
    }

    // ── op_literal_replace ──────────────────────────────────────────────

    #[test]
    fn literal_replaces_first_occurrence() {
        assert_eq!(
            op_literal_replace("Hello World", "World", "There"),
            Some("Hello There".into())
        );
    }

    #[test]
    fn literal_replace_no_match_returns_none() {
        assert_eq!(op_literal_replace("Hello", "World", "There"), None);
    }

    #[test]
    fn literal_replace_empty_find_returns_some() {
        // Rust's str::replace with empty find inserts between each char
        let result = op_literal_replace("Hello", "", "X");
        assert!(result.is_some());
    }

    // ── op_trim ─────────────────────────────────────────────────────────

    #[test]
    fn trim_whitespace() {
        assert_eq!(op_trim("  hello  "), Some("hello".into()));
    }

    #[test]
    fn trim_already_trimmed_returns_none() {
        assert_eq!(op_trim("hello"), None);
    }

    #[test]
    fn trim_empty_returns_none() {
        assert_eq!(op_trim(""), None);
    }

    // ── op_lowercase ────────────────────────────────────────────────────

    #[test]
    fn lowercase_converts_case() {
        assert_eq!(op_lowercase("Hello World"), Some("hello world".into()));
    }

    #[test]
    fn lowercase_already_lower_returns_none() {
        assert_eq!(op_lowercase("hello"), None);
    }

    // ── op_uppercase ────────────────────────────────────────────────────

    #[test]
    fn uppercase_converts_case() {
        assert_eq!(op_uppercase("Hello World"), Some("HELLO WORLD".into()));
    }

    #[test]
    fn uppercase_already_upper_returns_none() {
        assert_eq!(op_uppercase("HELLO"), None);
    }

    // ── op_title_case ───────────────────────────────────────────────────

    #[test]
    fn title_case_capitalizes_each_word() {
        assert_eq!(op_title_case("hello world"), Some("Hello World".into()));
    }

    #[test]
    fn title_case_already_title_case_returns_none() {
        assert_eq!(op_title_case("Hello World"), None);
    }

    #[test]
    fn title_case_mixed_case_normalizes() {
        assert_eq!(op_title_case("hELLO wORLD"), Some("Hello World".into()));
    }

    // ── op_prettify ─────────────────────────────────────────────────────

    #[test]
    fn prettify_normalizes_separators_and_case() {
        let result = op_prettify("110-hedgehog-you_are_so_famous");
        assert_eq!(result, Some("Hedgehog You Are So Famous".into()));
    }

    #[test]
    fn prettify_already_pretty_returns_none() {
        assert_eq!(op_prettify("Hello"), None);
    }

    // ── Chinese conversion ops ──────────────────────────────────────────

    #[test]
    fn chinese_to_simplified_converts() {
        let result = op_chinese_to_simplified("繁體");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "繁体");
    }

    #[test]
    fn chinese_to_simplified_already_simplified_returns_none() {
        assert_eq!(op_chinese_to_simplified("简体"), None);
    }

    #[test]
    fn chinese_to_traditional_converts() {
        let result = op_chinese_to_traditional("简体");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "簡體");
    }

    #[test]
    fn chinese_to_traditional_already_traditional_returns_none() {
        assert_eq!(op_chinese_to_traditional("繁體"), None);
    }

    // ── apply_pipeline ──────────────────────────────────────────────────

    #[test]
    fn pipeline_chains_multiple_ops() {
        let ops = serde_json::json!([
            {"op": "regex_replace", "pattern": "^\\d+\\s*[-.]?\\s*", "replacement": ""},
            {"op": "trim"},
            {"op": "title_case"}
        ]);
        let result = apply_pipeline_helper("01. hello world", ops.as_array().unwrap()).unwrap();
        assert_eq!(result, Some("Hello World".into()));
    }

    #[test]
    fn pipeline_no_changes_returns_none() {
        let ops = serde_json::json!([
            {"op": "lowercase"}
        ]);
        let result = apply_pipeline_helper("hello", ops.as_array().unwrap()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn pipeline_round_trip_reports_change() {
        // lower+upper on already-upper text: intermediate changes still count
        let ops = serde_json::json!([
            {"op": "lowercase"},
            {"op": "uppercase"}
        ]);
        let result = apply_pipeline_helper("HELLO", ops.as_array().unwrap()).unwrap();
        // The pipeline saw a change (lowercase changed it), so it reports Some
        assert_eq!(result, Some("HELLO".into()));
    }

    #[test]
    fn pipeline_unknown_op_returns_error() {
        let ops = serde_json::json!([
            {"op": "nonexistent"}
        ]);
        let result = apply_pipeline_helper("hello", ops.as_array().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("nonexistent"));
    }

    #[test]
    fn pipeline_missing_op_field_returns_error() {
        let ops = serde_json::json!([
            {"pattern": "test"}
        ]);
        let result = apply_pipeline_helper("hello", ops.as_array().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("'op'"));
    }

    #[test]
    fn pipeline_empty_operations_returns_none() {
        let result = apply_pipeline_helper("hello", &[]).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn pipeline_strip_track_title_prefixes_behavior_exact() {
        // Replicates the old strip_track_title_prefix behavior
        let ops = serde_json::json!([
            {"op": "regex_replace", "pattern": "^(?:\\d+[.)]\\s+|\\d+\\s*[-–]\\s+|\\d{1,3}\\s+)", "replacement": ""}
        ]);
        assert_eq!(
            apply_pipeline_helper("01. Song", ops.as_array().unwrap()).unwrap(),
            Some("Song".into())
        );
        assert_eq!(
            apply_pipeline_helper("01 - Song", ops.as_array().unwrap()).unwrap(),
            Some("Song".into())
        );
        assert_eq!(
            apply_pipeline_helper("1  Song", ops.as_array().unwrap()).unwrap(),
            Some("Song".into())
        );
        assert_eq!(
            apply_pipeline_helper("Song", ops.as_array().unwrap()).unwrap(),
            None
        );
    }

    // ── apply_pipeline on list-like values ──────────────────────────────

    #[test]
    fn pipeline_with_strip_prefix() {
        let ops = serde_json::json!([
            {"op": "strip_prefix", "prefix": "Mr. "}
        ]);
        assert_eq!(
            apply_pipeline_helper("Mr. Smith", ops.as_array().unwrap()).unwrap(),
            Some("Smith".into())
        );
        assert_eq!(
            apply_pipeline_helper("Smith", ops.as_array().unwrap()).unwrap(),
            None
        );
    }

    #[test]
    fn pipeline_with_literal_replace() {
        let ops = serde_json::json!([
            {"op": "literal_replace", "find": " - ", "replacement": ": "}
        ]);
        assert_eq!(
            apply_pipeline_helper("Song - Remix", ops.as_array().unwrap()).unwrap(),
            Some("Song: Remix".into())
        );
    }
}
