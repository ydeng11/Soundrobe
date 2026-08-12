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
    track_path, unique_planned_destination, MutatingToolExecution,
};
use crate::commands::organizer::sanitize_dir_name;
use crate::state::assistant::{
    AssistantAction, AssistantActionBatch, AssistantCompletionContract,
    AssistantCompletionExpectation, AssistantCompletionPostcondition,
    AssistantCompletionScopeSnapshot,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

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
    // The extracted value is always a meaningful write target even when it
    // equals the source text (e.g. a paren-less title with a `^([^(]+)`
    // pattern). Whether the destination field actually changes is decided by
    // the caller comparing against the destination's current value.
    Some(extracted)
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
    SplitArtists,
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
            "split_artists" => PipelineOp::SplitArtists,
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
            PipelineOp::SplitArtists => op_split_artists(&current),
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

pub(crate) fn op_split_artists(text: &str) -> Option<String> {
    use std::sync::OnceLock;
    static DELIMITER: OnceLock<regex::Regex> = OnceLock::new();
    let delimiter = DELIMITER.get_or_init(|| {
        regex::Regex::new(r"\s*[&,;，；]\s*").expect("valid plural Artists delimiter regex")
    });
    let mut seen = HashSet::new();
    let artists = delimiter
        .split(text)
        .filter_map(|artist| {
            let artist = artist.trim();
            let key = artist.to_lowercase();
            (!artist.is_empty() && seen.insert(key)).then(|| artist.to_string())
        })
        .collect::<Vec<_>>();
    if artists.len() < 2 {
        return None;
    }
    let split = artists.join("; ");
    (split != text).then_some(split)
}

fn unique_action_paths(actions: &[AssistantAction]) -> Vec<String> {
    let mut seen = HashSet::new();
    actions
        .iter()
        .filter_map(|action| action.track_path.as_ref())
        .filter(|path| seen.insert((*path).clone()))
        .cloned()
        .collect()
}

fn typed_action_value(action: &AssistantAction) -> Value {
    let Some(value) = action.new_value.as_deref() else {
        return Value::Null;
    };
    match action.field.as_deref() {
        Some("artists" | "albumArtists") => Value::Array(
            value
                .split(';')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_string()))
                .collect(),
        ),
        Some("trackNumber" | "trackTotal" | "discNumber" | "discTotal") => value
            .parse::<u64>()
            .map(|value| Value::Number(value.into()))
            .unwrap_or_else(|_| Value::String(value.to_string())),
        Some("compilation") => value
            .parse::<bool>()
            .map(Value::Bool)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        _ => Value::String(value.to_string()),
    }
}

pub(crate) fn completion_expectations(
    actions: &[AssistantAction],
) -> Vec<AssistantCompletionExpectation> {
    actions
        .iter()
        .filter_map(|action| {
            Some(AssistantCompletionExpectation {
                track_path: action.track_path.clone()?,
                tag_kind: action.tag_kind.clone().unwrap_or_else(|| "standard".into()),
                field: action.field.clone()?,
                operation: action.operation.clone().unwrap_or_else(|| {
                    if action.new_value.is_some() {
                        "set".into()
                    } else {
                        "remove".into()
                    }
                }),
                expected_value: typed_action_value(action),
            })
        })
        .collect()
}

fn split_artists_expected_paths(input: &AssistantSendInput, scope_paths: &[String]) -> Vec<String> {
    let tracks = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect::<BTreeMap<_, _>>();
    scope_paths
        .iter()
        .filter(|path| {
            let Some(track) = tracks.get(path.as_str()) else {
                return false;
            };
            let value = match track.get("artists") {
                Some(Value::Array(values)) if values.len() == 1 => values[0].as_str(),
                Some(Value::String(value)) => Some(value.as_str()),
                _ => None,
            };
            value.is_some_and(|value| op_split_artists(value).is_some())
        })
        .cloned()
        .collect()
}

fn extra_field_string(path: &str, field: &str) -> Result<Option<String>, String> {
    crate::commands::tracks::try_read_extra_tags(Path::new(path))
        .map_err(|error| format!("Could not read extra tags for {path}: {error}"))
        .map(|tags| {
            tags.into_iter()
                .find(|tag| tag.key.trim().eq_ignore_ascii_case(field))
                .map(|tag| tag.value)
        })
}

fn completion_scope_snapshot(
    input: &AssistantSendInput,
    scope_paths: &[String],
    standard_fields: &BTreeSet<String>,
    extra_fields: &BTreeSet<String>,
) -> Result<Vec<AssistantCompletionScopeSnapshot>, String> {
    let tracks = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect::<BTreeMap<_, _>>();
    scope_paths
        .iter()
        .map(|path| {
            let track = tracks.get(path.as_str()).copied();
            let standard_values = standard_fields
                .iter()
                .map(|field| {
                    let value = track
                        .and_then(|track| track.get(field))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let value = if matches!(field.as_str(), "artists" | "albumArtists")
                        && value.as_array().is_some_and(Vec::is_empty)
                    {
                        Value::Null
                    } else {
                        value
                    };
                    (field.clone(), value)
                })
                .collect();
            let current_extra = if extra_fields.is_empty() {
                None
            } else {
                Some(
                    crate::commands::tracks::try_read_extra_tags(Path::new(path)).map_err(
                        |error| format!("Could not read extra tags for {path}: {error}"),
                    )?,
                )
            };
            let extra_values = extra_fields
                .iter()
                .map(|field| {
                    let value = current_extra
                        .as_ref()
                        .and_then(|tags| {
                            tags.iter()
                                .find(|tag| tag.key.trim().eq_ignore_ascii_case(field))
                        })
                        .map(|tag| Value::String(tag.value.clone()))
                        .unwrap_or(Value::Null);
                    (field.clone(), value)
                })
                .collect();
            Ok(AssistantCompletionScopeSnapshot {
                path: path.clone(),
                standard_values,
                extra_values,
            })
        })
        .collect()
}

fn metadata_patch_contract_fields(args: &Value) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut standard = BTreeSet::new();
    let mut extra = BTreeSet::new();
    for change in args
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(field) = change.get("field").and_then(Value::as_str) else {
            continue;
        };
        if change.get("tag_kind").and_then(Value::as_str) == Some("extra") {
            extra.insert(field.to_string());
        } else {
            standard.insert(field.to_string());
        }
    }
    for entry in args
        .get("per_track_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for change in entry
            .get("changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(field) = change.get("field").and_then(Value::as_str) {
                standard.insert(field.to_string());
            }
        }
    }
    (standard, extra)
}

fn metadata_patch_expected_paths(
    args: &Value,
    input: &AssistantSendInput,
    scope_paths: &[String],
) -> Result<Vec<String>, String> {
    let tracks = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect::<BTreeMap<_, _>>();
    let per_track = args
        .get("per_track_changes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut expected = Vec::new();
    for path in scope_paths {
        let track = tracks.get(path.as_str()).copied();
        let mut uniform_affects = false;
        for change in args
            .get("changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let field = change
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let tag_kind = change
                .get("tag_kind")
                .and_then(Value::as_str)
                .unwrap_or("standard");
            let old_value = if tag_kind == "extra" {
                extra_field_string(path, field)?
            } else {
                track.and_then(|track| track_field_string(track, field))
            };
            let affects = match change
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("set")
            {
                "set" => {
                    let desired = if change
                        .get("valueFrom")
                        .and_then(Value::as_str)
                        == Some("folder_name")
                    {
                        folder_name_for_path(path)
                    } else {
                        change.get("value").and_then(action_value_string)
                    };
                    desired.as_ref().is_some_and(|desired| {
                        old_value.as_deref() != Some(desired)
                            && (!change
                                .get("only_if_missing")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                                || old_value
                                    .as_deref()
                                    .is_none_or(|value| value.trim().is_empty()))
                    })
                }
                "remove" => old_value.is_some(),
                "upsert" => true,
                _ => false,
            };
            uniform_affects |= affects;
        }
        let per_track_affects = per_track.iter().any(|entry| {
            entry.get("path").and_then(Value::as_str) == Some(path.as_str())
                && entry
                    .get("changes")
                    .and_then(Value::as_array)
                    .is_some_and(|changes| {
                        changes.iter().any(|change| {
                            let field = change
                                .get("field")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            match change
                                .get("action")
                                .and_then(Value::as_str)
                                .unwrap_or("set")
                            {
                                "set" => change
                                    .get("value")
                                    .and_then(action_value_string)
                                    .is_some_and(|desired| {
                                        track
                                            .and_then(|track| track_field_string(track, field))
                                            .as_deref()
                                            != Some(desired.as_str())
                                    }),
                                "remove" => track
                                    .and_then(|track| track_field_string(track, field))
                                    .is_some(),
                                _ => false,
                            }
                        })
                    })
        });
        if uniform_affects || per_track_affects {
            expected.push(path.clone());
        }
    }
    Ok(expected)
}

fn metadata_transform_expected_paths(
    input: &AssistantSendInput,
    scope_paths: &[String],
    source_kind: &str,
    source_field: &str,
    destination_field: &str,
    pipeline: &[PipelineOp],
) -> Vec<String> {
    let tracks = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect::<BTreeMap<_, _>>();
    scope_paths
        .iter()
        .filter(|path| {
            let source = match source_kind {
                "tag" => tracks
                    .get(path.as_str())
                    .and_then(|track| track_field_string(track, source_field))
                    .filter(|value| !value.is_empty()),
                "filename" => Path::new(path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
                    .filter(|value| !value.is_empty()),
                _ => None,
            };
            source
                .and_then(|source| execute_pipeline(&source, pipeline))
                .is_some_and(|desired| {
                    tracks
                        .get(path.as_str())
                        .and_then(|track| track_field_string(track, destination_field))
                        .as_deref()
                        != Some(desired.as_str())
                })
        })
        .cloned()
        .collect()
}

pub(crate) fn validate_native_completion_contract(
    batch: &AssistantActionBatch,
    input: &AssistantSendInput,
) -> Result<(), String> {
    let Some(contract) = batch.completion_contract.as_ref() else {
        return Ok(());
    };
    let actual_paths = unique_action_paths(&batch.actions);
    let actual_set = actual_paths.iter().collect::<HashSet<_>>();
    let expected_set = contract
        .expected_action_paths
        .iter()
        .collect::<HashSet<_>>();
    if actual_set != expected_set {
        let missing = contract
            .expected_action_paths
            .iter()
            .filter(|path| !actual_paths.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = actual_paths
            .iter()
            .filter(|path| !contract.expected_action_paths.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "Completion contract action coverage mismatch; missing: {}; unexpected: {}",
            missing.join(", "),
            unexpected.join(", ")
        ));
    }
    if !contract.expected_actions.is_empty()
        && completion_expectations(&batch.actions) != contract.expected_actions
    {
        return Err(
            "Completion contract action values do not exactly match the native expectations"
                .to_string(),
        );
    }
    if contract.postcondition == AssistantCompletionPostcondition::SplitArtistsNormalized {
        let expected = split_artists_expected_paths(input, &contract.scope_paths);
        let derived_set = expected.iter().collect::<HashSet<_>>();
        if derived_set != expected_set {
            return Err(format!(
                "Completion contract affected set changed; expected {}, derived {}",
                contract.expected_action_paths.join(", "),
                expected.join(", ")
            ));
        }
    }
    Ok(())
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

/// The containing folder name for a track path, used by
/// `valueFrom: "folder_name"`. Returns `None` when the path has no named
/// parent (e.g. a bare filename or a filesystem root).
pub(crate) fn folder_name_for_path(path: &str) -> Option<String> {
    Path::new(path)
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .filter(|name| !name.is_empty())
}

/// Execute `metadata.patch`: apply uniform and/or per-track changes to tag fields.
pub(crate) fn execute_metadata_patch(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let paths = match tool_scope_paths(input, args) {
        Ok(paths) => paths,
        Err(error) => return mutating_tool_error(error),
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
            if matches!(action_type, "set" | "upsert") {
                let has_value = change.get("value").is_some_and(|v| !v.is_null());
                let has_value_from = change.get("valueFrom").is_some_and(|v| !v.is_null());
                if has_value == has_value_from {
                    return mutating_tool_error(format!(
                        "Action '{action_type}' for field '{field}' must specify exactly one of 'value' or 'valueFrom'"
                    ));
                }
                if let Some(source) = change.get("valueFrom").and_then(Value::as_str) {
                    if source != "folder_name" {
                        return mutating_tool_error(format!(
                            "Unsupported valueFrom source: '{source}'"
                        ));
                    }
                    if action_type != "set" {
                        return mutating_tool_error(format!(
                            "valueFrom is supported only for set actions (field '{field}')"
                        ));
                    }
                    if tag_kind != "standard" {
                        return mutating_tool_error(format!(
                            "valueFrom is supported only for standard fields (field '{field}')"
                        ));
                    }
                }
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

    // Pre-resolve folder-name sources before planning any action so an
    // unresolvable path aborts the whole plan with no partial batch.
    let mut folder_name_sources: BTreeMap<String, String> = BTreeMap::new();
    let uses_folder_name_source = args
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|change| change.get("valueFrom").and_then(Value::as_str) == Some("folder_name"));
    if uses_folder_name_source {
        for path in &paths {
            match folder_name_for_path(path) {
                Some(name) => {
                    folder_name_sources.insert(path.clone(), name);
                }
                None => {
                    return mutating_tool_error(format!(
                        "Cannot derive values from a folder name for '{path}': no containing folder could be resolved; no actions were planned."
                    ));
                }
            }
        }
    }

    let mut actions: Vec<AssistantAction> = Vec::new();
    let mut derived_fields: Vec<String> = Vec::new();

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
            let value_from = change.get("valueFrom").and_then(Value::as_str);
            if value_from.is_some() && !derived_fields.iter().any(|f| f == field) {
                derived_fields.push(field.to_string());
            }

            match action_type {
                "set" => {
                    for path in &paths {
                        let track = tracks_map.get(path.as_str()).copied();
                        if tag_kind == "extra" {
                            let Some(v) = value else { continue };
                            let desired = action_value_string(v).unwrap_or_default();
                            let current = match extra_field_string(path, field) {
                                Ok(current) => current,
                                Err(error) => return mutating_tool_error(error),
                            };
                            if current.as_deref() != Some(desired.as_str()) {
                                let mut action = extra_action(path, field, Some(&desired), "set");
                                action.old_value = current;
                                actions.push(action);
                            }
                            continue;
                        }
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
                        } else if value_from == Some("folder_name") {
                            let desired = match folder_name_sources.get(path.as_str()) {
                                Some(name) => name.clone(),
                                None => {
                                    return mutating_tool_error(format!(
                                        "Cannot derive '{field}' from a folder name for '{path}': no containing folder could be resolved; no actions were planned."
                                    ));
                                }
                            };
                            push_string_action(
                                &mut actions,
                                track,
                                path,
                                field,
                                &desired,
                                &format!("Set {field} from containing folder"),
                            );
                        }
                    }
                }
                "remove" => {
                    for path in &paths {
                        let track = tracks_map.get(path.as_str()).copied();
                        let old_value = if tag_kind == "extra" {
                            match extra_field_string(path, field) {
                                Ok(value) => value,
                                Err(error) => return mutating_tool_error(error),
                            }
                        } else {
                            track.and_then(|t| track_field_string(t, field))
                        };
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
    let summary = if !derived_fields.is_empty() {
        let distinct_values = actions
            .iter()
            .filter_map(|action| action.new_value.as_deref())
            .collect::<BTreeSet<_>>();
        let examples = distinct_values.iter().take(5).cloned().collect::<Vec<_>>();
        let mut summary = format!(
            "{} will be derived from each track's containing folder: {} track(s), {} distinct value(s)",
            derived_fields.join(", "),
            affected_tracks,
            distinct_values.len()
        );
        if !examples.is_empty() {
            summary.push_str(&format!("; examples: {}", examples.join(", ")));
            let remaining = distinct_values.len().saturating_sub(examples.len());
            if remaining > 0 {
                summary.push_str(&format!(" (+{remaining} more)"));
            }
        }
        summary
    } else {
        let mut summary = format!(
            "Update {} metadata field(s) across {} track(s)",
            actions.len(),
            affected_tracks
        );
        // Uniform-literal guard: one identical literal across tracks in many
        // distinct folders is exactly the shape of the folder-derivation bug,
        // so the preview must surface it before the user approves.
        let distinct_values = actions
            .iter()
            .filter_map(|action| action.new_value.as_deref())
            .collect::<BTreeSet<_>>();
        if distinct_values.len() == 1 && actions.len() > 1 {
            // Count folders only among the actions actually planned, so the
            // message cannot claim more folders than changed tracks.
            let distinct_folders = actions
                .iter()
                .filter_map(|action| action.track_path.as_deref())
                .filter_map(folder_name_for_path)
                .collect::<BTreeSet<String>>();
            if distinct_folders.len() > 1 {
                let value = distinct_values.iter().next().unwrap_or(&"");
                summary.push_str(&format!(
                    " — Warning: writing the same value '{value}' to {} track(s) across {} different folders. If you meant to derive per-track values from folder names, cancel and rephrase (e.g. \"based on the folder name\").",
                    actions.len(),
                    distinct_folders.len()
                ));
            }
        }
        summary
    };
    let mut batch = assistant_batch(
        session_id,
        "metadata-update",
        "Patch metadata",
        &summary,
        "low",
        actions,
        true,
    );
    let (standard_fields, extra_fields) = metadata_patch_contract_fields(args);
    let expected_actions = completion_expectations(&batch.actions);
    let scope_snapshot =
        match completion_scope_snapshot(input, &paths, &standard_fields, &extra_fields) {
            Ok(snapshot) => snapshot,
            Err(error) => return mutating_tool_error(error),
        };
    let expected_action_paths = match metadata_patch_expected_paths(args, input, &paths) {
        Ok(paths) => paths,
        Err(error) => return mutating_tool_error(error),
    };
    let postcondition = if derived_fields.is_empty() {
        AssistantCompletionPostcondition::ExactMetadataActions
    } else {
        AssistantCompletionPostcondition::DerivedFolderName
    };
    batch.completion_contract = Some(AssistantCompletionContract {
        scope_paths: paths.clone(),
        scope_snapshot,
        expected_action_paths,
        expected_actions,
        postcondition,
    });
    if let Err(error) = validate_native_completion_contract(&batch, input) {
        return mutating_tool_error(error);
    }
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
    let paths = match tool_scope_paths(input, args) {
        Ok(paths) => paths,
        Err(error) => return mutating_tool_error(error),
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
    let split_artists = pipeline
        .iter()
        .any(|operation| matches!(operation, PipelineOp::SplitArtists));
    if split_artists
        && (pipeline.len() != 1
            || source_kind != "tag"
            || source_field != "artists"
            || dest_kind != "tag"
            || dest_field != "artists")
    {
        return mutating_tool_error(
            "split_artists must be the only operation and must read and write the plural artists field"
                .to_string(),
        );
    }

    let tracks_map: BTreeMap<&str, &Value> = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect();

    let mut actions: Vec<AssistantAction> = Vec::new();
    let mut skipped_no_source = 0usize;
    let mut skipped_no_value = 0usize;
    let mut already_equal = 0usize;
    let mut renamed_to_same = 0usize;

    for path in &paths {
        let source_value = match source_kind {
            "tag" => {
                let track = tracks_map.get(path.as_str()).copied();
                if split_artists
                    && track
                        .and_then(|track| track.get(source_field))
                        .and_then(Value::as_array)
                        .is_some_and(|artists| artists.len() != 1)
                {
                    None
                } else {
                    track
                        .and_then(|t| track_field_string(t, source_field))
                        .filter(|v| !v.is_empty())
                }
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
            skipped_no_source += 1;
            continue;
        };

        let result = execute_pipeline(&current, &pipeline);

        let Some(new_value) = result else {
            // Pipeline produced no change
            skipped_no_value += 1;
            continue;
        };

        match dest_kind {
            "tag" => {
                let track = tracks_map.get(path.as_str()).copied();
                let dest_current = track.and_then(|t| track_field_string(t, dest_field));
                if dest_current.as_deref() == Some(new_value.as_str()) {
                    already_equal += 1;
                    continue;
                }
                let description = match dest_current {
                    Some(old) => format!("Transform {dest_field} from '{old}' to '{new_value}'"),
                    None => format!("Transform {dest_field} to '{new_value}'"),
                };
                push_string_action(
                    &mut actions,
                    track,
                    path,
                    dest_field,
                    &new_value,
                    &description,
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
                if dest_path == Path::new(path) {
                    renamed_to_same += 1;
                    continue;
                }
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
        return mutating_tool_no_changes(format!(
            "No transformations produced changes. Scanned {} track(s): {} had no source value, \
             {} produced no extracted value (e.g. the pattern did not match), {} already had \
             the target value, {} resolved to the same filename.",
            paths.len(),
            skipped_no_source,
            skipped_no_value,
            already_equal,
            renamed_to_same
        ));
    }

    let summary = format!(
        "Transform {} field(s) across {} track(s)",
        actions.len(),
        actions.len()
    );
    let risk = if dest_kind == "filename" {
        "medium"
    } else {
        "low"
    };
    let mut batch = assistant_batch(
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
    if dest_kind == "tag" {
        let postcondition = if split_artists {
            AssistantCompletionPostcondition::SplitArtistsNormalized
        } else {
            AssistantCompletionPostcondition::ExactMetadataActions
        };
        let expected_action_paths = if split_artists {
            split_artists_expected_paths(input, &paths)
        } else {
            metadata_transform_expected_paths(
                input,
                &paths,
                source_kind,
                source_field,
                dest_field,
                &pipeline,
            )
        };
        let standard_fields = [source_field.to_string(), dest_field.to_string()]
            .into_iter()
            .collect();
        let expected_actions = completion_expectations(&batch.actions);
        let scope_snapshot =
            match completion_scope_snapshot(input, &paths, &standard_fields, &BTreeSet::new()) {
                Ok(snapshot) => snapshot,
                Err(error) => return mutating_tool_error(error),
            };
        batch.completion_contract = Some(AssistantCompletionContract {
            scope_paths: paths.clone(),
            scope_snapshot,
            expected_action_paths,
            expected_actions,
            postcondition,
        });
        if let Err(error) = validate_native_completion_contract(&batch, input) {
            return mutating_tool_error(error);
        }
    }
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
    let paths = match tool_scope_paths(input, args) {
        Ok(paths) => paths,
        Err(error) => return mutating_tool_error(error),
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
    let mut batch = assistant_batch(
        session_id,
        "folder-move",
        "Transform files",
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

/// Move tracks into sub-folders under the library root, with destination names
/// derived from a tag field or the filename stem via a transformation pipeline.
///
/// `destination.template` is a relative path; `{value}` is replaced with the
/// transformed source value (the template may also be a literal path shared by
/// every track). Every destination is validated to stay inside the library root
/// (lexically and through symlink canonicalization) before a preview is created.
pub(crate) fn execute_files_relocate(
    args: &Value,
    input: &AssistantSendInput,
    session_id: &str,
) -> MutatingToolExecution {
    let paths = match tool_scope_paths(input, args) {
        Ok(paths) => paths,
        Err(error) => return mutating_tool_error(error),
    };
    if paths.is_empty() {
        return mutating_tool_no_changes("No tracks found for the requested scope.");
    }

    let Some(library) = input.library_path.as_deref().map(Path::new) else {
        return mutating_tool_error("Library root is required for file operations");
    };
    if !library.exists() || !library.is_dir() {
        return mutating_tool_error(format!(
            "Library path '{}' does not exist or is not a directory",
            library.display()
        ));
    }

    let destination = args.get("destination").and_then(Value::as_object).cloned();
    let Some(destination) = destination else {
        return mutating_tool_error("files.relocate requires a 'destination' object");
    };
    let template = destination
        .get("template")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if template.is_empty() {
        return mutating_tool_error("destination.template is required and must not be empty");
    }
    if template.starts_with('/') || template.starts_with('\\') {
        return mutating_tool_error(
            "destination.template must be a relative path under the library root",
        );
    }

    let source = destination
        .get("source")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let source_kind = source
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("filename");
    let source_field = source
        .get("field")
        .and_then(Value::as_str)
        .unwrap_or("title");

    let collision = destination
        .get("collision")
        .and_then(Value::as_str)
        .unwrap_or("suffix");
    if !matches!(collision, "suffix" | "skip" | "error") {
        return mutating_tool_error(format!("Unsupported collision policy: {collision}"));
    }

    let pipeline = match destination.get("operations").and_then(Value::as_array) {
        None => Vec::new(),
        Some(operations) => match compile_pipeline(operations) {
            Ok(pipeline) => pipeline,
            Err(error) => return mutating_tool_error(error),
        },
    };

    let tracks_map: BTreeMap<&str, &Value> = input
        .tracks
        .iter()
        .map(|track| (track_path(track), track))
        .collect();
    let mut reserved = HashSet::new();
    let mut actions = Vec::new();
    let mut skipped = 0usize;

    for path in &paths {
        let source_path = Path::new(path);
        if !path_is_inside(source_path, library) {
            return mutating_tool_error(format!("Path '{}' is outside the library root", path));
        }
        if !path_is_inside_canonical(source_path, library) {
            return mutating_tool_error(format!(
                "Path '{}' resolves outside the library root (symlink escape)",
                path
            ));
        }
        let Some(filename) = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            skipped += 1;
            continue;
        };

        let source_value = match source_kind {
            "tag" => tracks_map
                .get(path.as_str())
                .copied()
                .and_then(|track| track_field_string(track, source_field))
                .filter(|value| !value.is_empty()),
            "filename" => source_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
                .filter(|value| !value.is_empty()),
            other => {
                return mutating_tool_error(format!("Unsupported relocate source kind: {other}"))
            }
        };
        let Some(current) = source_value else {
            skipped += 1;
            continue;
        };

        let new_value = if pipeline.is_empty() {
            current.clone()
        } else {
            // A pipeline op that leaves the value unchanged returns None; the
            // unchanged value is still the destination folder name.
            execute_pipeline(&current, &pipeline).unwrap_or_else(|| current.clone())
        };
        if new_value.trim().is_empty() {
            skipped += 1;
            continue;
        }

        let relative_dir = match resolve_destination_dir(template, &new_value) {
            Ok(dir) => dir,
            Err(error) => return mutating_tool_error(error),
        };
        let destination_dir = library.join(&relative_dir);
        if !path_is_inside(&destination_dir, library) {
            return mutating_tool_error(format!(
                "Destination '{}' escapes the library root",
                destination_dir.display()
            ));
        }
        if !canonical_ancestor_inside(&destination_dir, library) {
            return mutating_tool_error(format!(
                "Destination '{}' resolves outside the library root",
                destination_dir.display()
            ));
        }
        if source_path.parent() == Some(destination_dir.as_path()) {
            skipped += 1;
            continue;
        }

        let planned = destination_dir.join(&filename);
        let collides = planned.exists() || reserved.contains(&planned);
        let destination = match (collision, collides) {
            ("skip", true) => {
                skipped += 1;
                continue;
            }
            ("error", true) => {
                return mutating_tool_error(format!(
                    "Destination collision at '{}'; no file was moved",
                    planned.display()
                ));
            }
            _ => unique_planned_destination(source_path, planned, &mut reserved),
        };
        if destination == source_path {
            skipped += 1;
            continue;
        }
        actions.push(AssistantAction {
            operation: Some("relocate".into()),
            source_path: Some(path.clone()),
            destination_path: Some(destination.to_string_lossy().into_owned()),
            description: Some(format!("Move into folder: {}", relative_dir.display())),
            ..Default::default()
        });
    }

    if actions.is_empty() {
        return mutating_tool_no_changes(format!(
            "No file relocations produced changes; {skipped} skipped."
        ));
    }

    let summary = format!("Relocate {} file(s) across {} track(s)", actions.len(), paths.len());
    let mut batch = assistant_batch(
        session_id,
        "folder-move",
        "Relocate files",
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

/// Resolve a relative destination directory from a template plus a substituted
/// value. Path separators in the TEMPLATE create hierarchy; `{value}` is
/// substituted inside its own segment and the whole segment is sanitized as a
/// single directory name — so a value like `AC/DC` becomes `AC_DC`, never
/// nested folders. Literal traversal segments in the template are rejected.
fn resolve_destination_dir(template: &str, value: &str) -> Result<PathBuf, String> {
    if template.starts_with('/') || template.starts_with('\\') {
        return Err("destination.template must be relative to the library root".to_string());
    }
    let mut result = PathBuf::new();
    for raw_segment in template.split(['/', '\\']) {
        if raw_segment == "." || raw_segment == ".." {
            return Err("destination.template escapes the library root".to_string());
        }
        let segment = sanitize_dir_name(&raw_segment.replace("{value}", value));
        if segment.is_empty() {
            continue;
        }
        result.push(segment);
    }
    if result.as_os_str().is_empty() {
        return Err("destination.template produced an empty folder name".to_string());
    }
    Ok(result)
}

/// Containment check that resolves symlinks on both sides; falls back to the
/// lexical check when canonicalization is unavailable (e.g. non-existent paths).
fn path_is_inside_canonical(path: &Path, root: &Path) -> bool {
    match (path.canonicalize(), root.canonicalize()) {
        (Ok(path), Ok(root)) => path.starts_with(root),
        _ => path_is_inside(path, root),
    }
}

/// For a not-yet-existing destination directory, canonicalize the nearest
/// existing ancestor and verify it stays inside the (canonicalized) library root.
fn canonical_ancestor_inside(destination_dir: &Path, library: &Path) -> bool {
    let mut probe = destination_dir.to_path_buf();
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent.to_path_buf(),
            None => return path_is_inside(destination_dir, library),
        }
    }
    path_is_inside_canonical(&probe, library)
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
    fn metadata_patch_value_from_folder_name_produces_per_track_actions() {
        let mut input = test_input();
        input.selected_track_paths = vec!["/music/album1/a.mp3".into(), "/music/album2/b.mp3".into()];
        input.tracks = vec![
            serde_json::json!({"path": "/music/album1/a.mp3", "album": "Old"}),
            serde_json::json!({"path": "/music/album2/b.mp3", "album": "Old"}),
        ];
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "valueFrom": "folder_name"}
                ]
            }),
            &input,
            "session-1",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        let batch = result.batches.first().expect("derived preview batch");
        assert_eq!(batch.actions.len(), 2);
        let values = batch
            .actions
            .iter()
            .map(|action| (action.track_path.as_deref().unwrap(), action.new_value.as_deref().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![("/music/album1/a.mp3", "album1"), ("/music/album2/b.mp3", "album2")]
        );
        // Preview must surface derivation semantics, not just a count.
        assert!(
            result.result.summary.contains("derived from each track's containing folder"),
            "{}",
            result.result.summary
        );
        assert!(result.result.summary.contains("2 distinct value(s)"), "{}", result.result.summary);
        // Completion contract must be marked as folder-derived and cover both paths.
        let contract = batch.completion_contract.as_ref().unwrap();
        assert_eq!(
            contract.postcondition,
            AssistantCompletionPostcondition::DerivedFolderName
        );
        assert_eq!(contract.expected_action_paths.len(), 2);
    }

    #[test]
    fn metadata_patch_value_from_skips_tracks_already_matching_their_folder() {
        let mut input = test_input();
        input.selected_track_paths = vec!["/music/album1/a.mp3".into(), "/music/album1/b.mp3".into()];
        input.tracks = vec![
            serde_json::json!({"path": "/music/album1/a.mp3", "album": "album1"}),
            serde_json::json!({"path": "/music/album1/b.mp3", "album": "Old"}),
        ];
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "valueFrom": "folder_name"}
                ]
            }),
            &input,
            "session-1",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        let batch = result.batches.first().expect("derived preview batch");
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.actions[0].track_path.as_deref(), Some("/music/album1/b.mp3"));
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("album1"));
    }

    #[test]
    fn metadata_patch_value_and_value_from_both_rejected() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "value": "Loose", "valueFrom": "folder_name"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("exactly one of 'value' or 'valueFrom'"));
    }

    #[test]
    fn metadata_patch_unknown_value_from_source_rejected() {
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "valueFrom": "filename"}
                ]
            }),
            &test_input(),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("Unsupported valueFrom source: 'filename'"));
    }

    #[test]
    fn metadata_patch_value_from_unresolvable_path_aborts_planning() {
        // A path with no named parent folder must abort the whole plan with
        // no partial batch, never guess a value.
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "valueFrom": "folder_name"}
                ]
            }),
            &AssistantSendInput {
                selected_track_paths: vec!["a.mp3".into()],
                tracks: vec![serde_json::json!({"path": "a.mp3"})],
                ..Default::default()
            },
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("no actions were planned"));
        assert!(result.batches.is_empty());
    }

    #[test]
    fn metadata_patch_value_from_only_if_missing_preserves_existing_values() {
        // "set album based on their folder name where missing" must derive
        // only into tracks without an existing album; present values survive.
        let mut input = test_input();
        input.selected_track_paths = vec!["/music/album1/a.mp3".into(), "/music/album2/b.mp3".into()];
        input.tracks = vec![
            serde_json::json!({"path": "/music/album1/a.mp3", "album": "Existing"}),
            serde_json::json!({"path": "/music/album2/b.mp3"}), // missing album
        ];
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "valueFrom": "folder_name", "only_if_missing": true}
                ]
            }),
            &input,
            "session-1",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        let batch = result.batches.first().expect("derived preview batch");
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(
            batch.actions[0].track_path.as_deref(),
            Some("/music/album2/b.mp3")
        );
        assert_eq!(batch.actions[0].new_value.as_deref(), Some("album2"));
    }

    #[test]
    fn metadata_patch_uniform_literal_warning_counts_only_affected_folders() {
        // A track that already matches the literal must not inflate the
        // folder count in the uniform-literal preview warning.
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a/x.mp3".into(), "/music/b/y.mp3".into()],
            tracks: vec![
                serde_json::json!({"path": "/music/a/x.mp3", "album": "Loose"}),
                serde_json::json!({"path": "/music/b/y.mp3", "album": "Old"}),
            ],
            ..Default::default()
        };
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "value": "Loose"}
                ]
            }),
            &input,
            "session-1",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        assert!(
            !result.result.summary.contains("Warning:"),
            "only one track changes in one folder; no cross-folder warning expected: {}",
            result.result.summary
        );
    }

    #[test]
    fn metadata_patch_uniform_literal_across_folders_warns_in_preview() {
        // The exact shape of the folder-derivation bug: one identical literal
        // planned across tracks in many distinct folders. The preview must
        // surface it before the user approves.
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/a/a.mp3".into(), "/music/b/b.mp3".into()],
            tracks: vec![
                serde_json::json!({"path": "/music/a/a.mp3", "album": "Old"}),
                serde_json::json!({"path": "/music/b/b.mp3", "album": "Old"}),
            ],
            ..Default::default()
        };
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "value": "Loose"}
                ]
            }),
            &input,
            "session-1",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        assert!(
            result.result.summary.contains("Warning: writing the same value 'Loose'"),
            "{}",
            result.result.summary
        );
    }

    #[test]
    fn metadata_patch_uniform_literal_within_one_folder_does_not_warn() {
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/album/x.mp3".into(), "/music/album/y.mp3".into()],
            tracks: vec![
                serde_json::json!({"path": "/music/album/x.mp3", "album": "Old"}),
                serde_json::json!({"path": "/music/album/y.mp3", "album": "Old"}),
            ],
            ..Default::default()
        };
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "album", "action": "set", "value": "Loose"}
                ]
            }),
            &input,
            "session-1",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        assert!(
            !result.result.summary.contains("Warning:"),
            "{}",
            result.result.summary
        );
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
    fn metadata_patch_splits_plural_artists_without_changing_display_artist() {
        let path = "/music/duet.flac";
        let input = AssistantSendInput {
            selected_track_paths: vec![path.into()],
            tracks: vec![serde_json::json!({
                "path": path,
                "artist": "谭咏麟 & 丁菲飞",
                "artists": ["谭咏麟 & 丁菲飞"]
            })],
            ..Default::default()
        };
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "per_track_changes": [{
                    "path": path,
                    "changes": [{
                        "field": "artists",
                        "action": "set",
                        "value": ["谭咏麟", "丁菲飞"]
                    }]
                }]
            }),
            &input,
            "session-1",
        );

        assert!(result.result.ok);
        let batch = result.batches.first().expect("preview batch");
        assert_eq!(batch.actions.len(), 1);
        let action = &batch.actions[0];
        assert_eq!(action.field.as_deref(), Some("artists"));
        assert_eq!(action.old_value.as_deref(), Some("谭咏麟 & 丁菲飞"));
        assert_eq!(action.new_value.as_deref(), Some("谭咏麟; 丁菲飞"));
        assert_ne!(action.field.as_deref(), Some("artist"));
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
        // A set action without either a literal value or a derivation source
        // must be rejected instead of guessing.
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
        assert!(
            result.result.summary.contains("exactly one of 'value' or 'valueFrom'"),
            "{}",
            result.result.summary
        );
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
        let contract = batch
            .completion_contract
            .as_ref()
            .expect("native patch completion contract");
        assert_eq!(
            contract.expected_action_paths,
            vec!["/music/a.mp3", "/music/c.mp3"]
        );
        assert_eq!(contract.scope_snapshot.len(), 3);
        let mut incomplete = batch.clone();
        incomplete.actions.pop();
        let error = validate_native_completion_contract(&incomplete, &input)
            .expect_err("omitting a matching track must invalidate the preview");
        assert!(error.contains("/music/c.mp3"), "{error}");
        assert!(
            result.result.summary.contains("across 2 track(s)"),
            "summary must report affected tracks, not the three-track input scope"
        );
    }

    #[test]
    fn completion_contract_rejects_omitted_field_on_an_expected_path() {
        let mut input = test_input();
        input.selected_track_paths.truncate(1);
        input.tracks.truncate(1);
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"field": "genre", "action": "set", "value": "Pop"},
                    {"field": "year", "action": "set", "value": "2026"}
                ]
            }),
            &input,
            "session-1",
        );
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        let mut incomplete = batch.clone();
        incomplete.actions.pop();
        let error = validate_native_completion_contract(&incomplete, &input)
            .expect_err("path-only coverage must not hide a missing field action");
        assert!(error.contains("action values"), "{error}");
    }

    #[test]
    fn selected_scope_rejects_missing_track_metadata_instead_of_reporting_no_change() {
        let mut input = test_input();
        input
            .selected_track_paths
            .push("/music/missing-at-end.flac".into());
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [{"field": "genre", "action": "set", "value": "Pop"}]
            }),
            &input,
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("missing-at-end.flac"));
    }

    #[test]
    fn metadata_patch_extra_upsert_creates_extra_actions() {
        let root =
            std::env::temp_dir().join(format!("soundrobe-extra-contract-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../test/fixtures/tauri/media-corpus/minimal.flac");
        let first = root.join("a.flac");
        let second = root.join("b.flac");
        std::fs::copy(&fixture, &first).unwrap();
        std::fs::copy(&fixture, &second).unwrap();
        let mut input = test_input();
        input.selected_track_paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        input.tracks[0]["path"] = Value::String(input.selected_track_paths[0].clone());
        input.tracks[1]["path"] = Value::String(input.selected_track_paths[1].clone());
        let result = execute_metadata_patch(
            &serde_json::json!({
                "target_scope": "selected",
                "changes": [
                    {"tag_kind": "extra", "field": "MOOD", "action": "upsert", "value": "Calm"}
                ]
            }),
            &input,
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
        std::fs::remove_dir_all(root).unwrap();
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
    fn metadata_transform_splits_every_malformed_plural_artists_value_in_scope() {
        let selected_track_paths = (1..=46)
            .map(|index| format!("/music/{index:02}.flac"))
            .collect::<Vec<_>>();
        let tracks = selected_track_paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                if index == 44 {
                    serde_json::json!({
                        "path": path,
                        "artist": "黎明 & 谭咏麟",
                        "artists": ["黎明&谭咏麟"]
                    })
                } else if index == 45 {
                    serde_json::json!({
                        "path": path,
                        "artist": "谭咏麟 & 김완선",
                        "artists": ["谭咏麟&김완선"]
                    })
                } else {
                    serde_json::json!({
                        "path": path,
                        "artist": format!("谭咏麟 & Collaborator {index}"),
                        "artists": ["谭咏麟", format!("Collaborator {index}")]
                    })
                }
            })
            .collect();
        let input = AssistantSendInput {
            selected_track_paths,
            tracks,
            ..Default::default()
        };

        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "artists"},
                "operations": [{"op": "split_artists"}]
            }),
            &input,
            "session-1",
        );

        assert!(result.result.ok, "{}", result.result.summary);
        let batch = result.batches.first().expect("preview batch");
        assert_eq!(
            batch.actions.len(),
            2,
            "a scope-wide transform must not omit a malformed track at the end"
        );
        let contract = batch
            .completion_contract
            .as_ref()
            .expect("native transform completion contract");
        assert_eq!(contract.scope_paths.len(), 46);
        assert_eq!(
            contract.expected_action_paths,
            vec!["/music/45.flac", "/music/46.flac"]
        );
        assert_eq!(
            serde_json::to_value(&contract.postcondition).unwrap(),
            serde_json::json!("splitArtistsNormalized")
        );
        assert_eq!(
            batch
                .actions
                .iter()
                .map(|action| (
                    action.track_path.as_deref(),
                    action.field.as_deref(),
                    action.new_value.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    Some("/music/45.flac"),
                    Some("artists"),
                    Some("黎明; 谭咏麟")
                ),
                (
                    Some("/music/46.flac"),
                    Some("artists"),
                    Some("谭咏麟; 김완선")
                ),
            ]
        );
        assert!(
            batch
                .actions
                .iter()
                .all(|action| action.field.as_deref() != Some("artist")),
            "the singular display Artist must remain unchanged"
        );

        let mut incomplete = batch.clone();
        incomplete.actions.pop();
        let error = validate_native_completion_contract(&incomplete, &input)
            .expect_err("missing the final affected track must invalidate the preview");
        assert!(error.contains("/music/46.flac"), "{error}");
    }

    #[test]
    fn split_artists_contract_excludes_non_target_artist_fields_and_values() {
        let input = AssistantSendInput {
            selected_track_paths: vec![
                "/music/singular.flac".into(),
                "/music/album-artists.flac".into(),
                "/music/correct.flac".into(),
                "/music/atomic.flac".into(),
            ],
            tracks: vec![
                serde_json::json!({
                    "path": "/music/singular.flac",
                    "artist": "Artist A & Artist B",
                    "artists": ["Artist A", "Artist B"]
                }),
                serde_json::json!({
                    "path": "/music/album-artists.flac",
                    "artist": "Artist A",
                    "artists": ["Artist A"],
                    "albumArtists": ["Artist A & Artist B"]
                }),
                serde_json::json!({
                    "path": "/music/correct.flac",
                    "artist": "Artist A feat. Artist B",
                    "artists": ["Artist A", "Artist B"]
                }),
                serde_json::json!({
                    "path": "/music/atomic.flac",
                    "artist": "AC/DC",
                    "artists": ["AC/DC"]
                }),
            ],
            ..Default::default()
        };

        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "artists"},
                "operations": [{"op": "split_artists"}]
            }),
            &input,
            "session-1",
        );

        assert!(result.result.ok, "{}", result.result.summary);
        assert!(result.batches.is_empty());
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

    // ── files.relocate ────────────────────────────────────────────────────

    fn relocate_input(dir: &Path, paths: &[&str]) -> AssistantSendInput {
        AssistantSendInput {
            selected_track_paths: vec![],
            tracks: paths
                .iter()
                .map(|path| serde_json::json!({"path": path}))
                .collect(),
            library_path: Some(dir.to_string_lossy().into_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn files_relocate_filename_template_groups_into_folder() {
        let dir = std::env::temp_dir().join("soundrobe-test-relocate-basic");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("01_song.mp3");
        let _ = std::fs::write(&file_path, b"test");

        let result = execute_files_relocate(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": [file_path.to_string_lossy().as_ref()],
                "destination": {
                    "template": "{value}",
                    "source": {"kind": "filename"},
                    "operations": [
                        {"op": "strip_prefix", "prefix": "01_"}
                    ]
                }
            }),
            &relocate_input(&dir, &[file_path.to_string_lossy().as_ref()]),
            "session-1",
        );
        assert!(
            result.result.ok,
            "relocate failed: {}",
            result.result.error.as_deref().unwrap_or("unknown")
        );
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 1);
        assert_eq!(batch.kind, "folder-move");
        let expected = dir.join("song").join("01_song.mp3").to_string_lossy().into_owned();
        assert_eq!(batch.actions[0].destination_path.as_deref(), Some(expected.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_relocate_tag_source_groups_instrumental_with_vocal() {
        // The failing-session scenario: titles with a `(伴奏)` suffix must group
        // into the same album folder as their vocal version.
        let dir = std::env::temp_dir().join("soundrobe-test-relocate-instrumental");
        let _ = std::fs::create_dir_all(&dir);
        let vocal = dir.join("vocal.mp3");
        let instrumental = dir.join("instrumental.mp3");
        let _ = std::fs::write(&vocal, b"test");
        let _ = std::fs::write(&instrumental, b"test");
        let input = AssistantSendInput {
            selected_track_paths: vec![],
            tracks: vec![
                serde_json::json!({"path": vocal.to_string_lossy(), "title": "喜剧演员"}),
                serde_json::json!({"path": instrumental.to_string_lossy(), "title": "喜剧演员(伴奏)"}),
            ],
            library_path: Some(dir.to_string_lossy().into_owned()),
            ..Default::default()
        };

        let result = execute_files_relocate(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": [
                    vocal.to_string_lossy().as_ref(),
                    instrumental.to_string_lossy().as_ref()
                ],
                "destination": {
                    "template": "{value}",
                    "source": {"kind": "tag", "field": "title"},
                    "operations": [
                        {"op": "strip_suffix", "suffix": "(伴奏)"}
                    ]
                }
            }),
            &input,
            "session-1",
        );
        assert!(
            result.result.ok,
            "relocate failed: {}",
            result.result.error.as_deref().unwrap_or("unknown")
        );
        let batch = result.batches.first().unwrap();
        let mut destinations = batch
            .actions
            .iter()
            .filter_map(|a| a.destination_path.clone())
            .collect::<Vec<_>>();
        destinations.sort();
        let album_dir = dir.join("喜剧演员");
        assert_eq!(
            destinations,
            vec![
                album_dir.join("instrumental.mp3").to_string_lossy().into_owned(),
                album_dir.join("vocal.mp3").to_string_lossy().into_owned(),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_relocate_literal_template_moves_all_into_one_folder() {
        let dir = std::env::temp_dir().join("soundrobe-test-relocate-literal");
        let _ = std::fs::create_dir_all(&dir);
        let a = dir.join("a.mp3");
        let b = dir.join("b.mp3");
        let _ = std::fs::write(&a, b"test");
        let _ = std::fs::write(&b, b"test");

        let result = execute_files_relocate(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": [a.to_string_lossy().as_ref(), b.to_string_lossy().as_ref()],
                "destination": {"template": "Albums/2024"}
            }),
            &relocate_input(&dir, &[a.to_string_lossy().as_ref(), b.to_string_lossy().as_ref()]),
            "session-1",
        );
        assert!(result.result.ok, "{}", result.result.error.as_deref().unwrap_or(""));
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        let target = dir.join("Albums").join("2024");
        assert!(batch.actions.iter().all(|a| a
            .destination_path
            .as_deref()
            .is_some_and(|d| d.starts_with(target.to_string_lossy().as_ref()))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_relocate_traversal_template_rejected() {
        let dir = std::env::temp_dir().join("soundrobe-test-relocate-traversal");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("song.mp3");
        let _ = std::fs::write(&file_path, b"test");

        let result = execute_files_relocate(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": [file_path.to_string_lossy().as_ref()],
                "destination": {"template": "../escape"}
            }),
            &relocate_input(&dir, &[file_path.to_string_lossy().as_ref()]),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("escapes the library root"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_relocate_absolute_template_rejected() {
        let dir = std::env::temp_dir().join("soundrobe-test-relocate-absolute");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("song.mp3");
        let _ = std::fs::write(&file_path, b"test");

        let result = execute_files_relocate(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": [file_path.to_string_lossy().as_ref()],
                "destination": {"template": "/etc"}
            }),
            &relocate_input(&dir, &[file_path.to_string_lossy().as_ref()]),
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("relative path"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_relocate_symlink_escape_rejected() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let dir = std::env::temp_dir().join("soundrobe-test-relocate-symlink");
            let outside = std::env::temp_dir().join("soundrobe-test-relocate-outside");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::create_dir_all(&outside);
            let link = dir.join("linked");
            let _ = symlink(&outside, &link);
            let file_path = link.join("song.mp3");
            let _ = std::fs::write(&file_path, b"test");

            let result = execute_files_relocate(
                &serde_json::json!({
                    "target_scope": "explicit_paths",
                    "paths": [file_path.to_string_lossy().as_ref()],
                    "destination": {"template": "{value}"}
                }),
                &relocate_input(&dir, &[file_path.to_string_lossy().as_ref()]),
                "session-1",
            );
            assert!(!result.result.ok);
            assert!(result.result.summary.contains("resolves outside the library root"));
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_dir_all(&outside);
        }
    }

    #[test]
    fn files_relocate_collision_renames_second_file() {
        let dir = std::env::temp_dir().join("soundrobe-test-relocate-collision");
        let _ = std::fs::create_dir_all(dir.join("sub1"));
        let _ = std::fs::create_dir_all(dir.join("sub2"));
        let a = dir.join("sub1").join("song.mp3");
        let b = dir.join("sub2").join("song.mp3");
        let _ = std::fs::write(&a, b"test");
        let _ = std::fs::write(&b, b"test");

        let result = execute_files_relocate(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": [a.to_string_lossy().as_ref(), b.to_string_lossy().as_ref()],
                "destination": {"template": "{value}", "source": {"kind": "filename"}}
            }),
            &relocate_input(&dir, &[a.to_string_lossy().as_ref(), b.to_string_lossy().as_ref()]),
            "session-1",
        );
        assert!(result.result.ok, "{}", result.result.error.as_deref().unwrap_or(""));
        let batch = result.batches.first().unwrap();
        assert_eq!(batch.actions.len(), 2);
        let album = dir.join("song");
        let mut destinations = batch
            .actions
            .iter()
            .filter_map(|action| action.destination_path.clone())
            .collect::<Vec<_>>();
        destinations.sort();
        assert_eq!(
            destinations,
            vec![
                album.join("song.mp3").to_string_lossy().into_owned(),
                album.join("song_1.mp3").to_string_lossy().into_owned(),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_relocate_requires_library_root() {
        let result = execute_files_relocate(
            &serde_json::json!({
                "target_scope": "explicit_paths",
                "paths": ["/music/song.mp3"],
                "destination": {"template": "{value}"}
            }),
            &AssistantSendInput {
                selected_track_paths: vec![],
                tracks: vec![serde_json::json!({"path": "/music/song.mp3", "title": "Song"})],
                ..Default::default()
            },
            "session-1",
        );
        assert!(!result.result.ok);
        assert!(result.result.summary.contains("Library root is required"));
    }

    #[test]
    fn resolve_destination_dir_sanitizes_segments() {
        assert_eq!(
            resolve_destination_dir("Albums/{value}", "红昭愿").unwrap(),
            PathBuf::from("Albums").join("红昭愿")
        );
        // Filesystem-hostile characters map to safe separators (collapsed).
        let safe = resolve_destination_dir("{value}", "A:B?* C").unwrap();
        assert_eq!(safe, PathBuf::from("A B C"));
        // Blank template values fall back to the organizer's "Unknown Album"
        // folder name (consistent with sanitize_dir_name) instead of a root move.
        assert_eq!(
            resolve_destination_dir("{value}", "  ").unwrap(),
            PathBuf::from("Unknown Album")
        );
        // Only separators in the TEMPLATE create hierarchy: a value such as
        // `AC/DC` becomes a single `AC_DC` folder, never nested folders.
        assert_eq!(
            resolve_destination_dir("{value}", "AC/DC").unwrap(),
            PathBuf::from("AC DC")
        );
        assert_eq!(
            resolve_destination_dir("{value}", "A / B").unwrap(),
            PathBuf::from("A B")
        );
        // Traversal inside a VALUE is sanitized into a safe single segment.
        assert_eq!(
            resolve_destination_dir("{value}", "a/../b").unwrap(),
            PathBuf::from("a .. b")
        );
        // Traversal as a literal TEMPLATE segment is rejected outright.
        assert!(resolve_destination_dir("..", "x").is_err());
        assert!(resolve_destination_dir("a/../b", "x").is_err());
        assert!(resolve_destination_dir("{value}", "x").is_ok());
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

    #[test]
    fn regex_extract_full_source_match_still_yields_value_for_cross_field_write() {
        // Regression for session #1785647941055-728246: "album must be based on
        // the common string from the title - everything before '('". A paren-less
        // title like 红昭愿 matches `^([^(（]+)` with the extracted group equal to
        // the whole source, and the transform still must be able to write it to
        // the destination field (the destination comparison decides whether an
        // action is actually needed).
        assert_eq!(
            op_regex_extract("红昭愿", r"^([^(（]+)", 1),
            Some("红昭愿".into())
        );
        assert_eq!(
            op_regex_extract("36.5℃", r"^([^(]+)", 1),
            Some("36.5℃".into())
        );
    }

    #[test]
    fn metadata_transform_before_paren_groups_vocal_and_instrumental_versions() {
        // Regression for session #1785647941055-728246: the requested transform
        // "album = title before '('" must produce actions for BOTH the original
        // version (paren-less title, placeholder album) and the instrumental
        // version (title with (伴奏) suffix), so both land in the same album.
        let original = "/music/音阙诗听/36.5℃/音阙诗听_李佳思-36.5℃.flac";
        let instrumental = "/music/音阙诗听/喜剧演员(伴奏)/常柏松_音阙诗听-喜剧演员(伴奏).flac";
        let input = AssistantSendInput {
            selected_track_paths: vec![original.into(), instrumental.into()],
            tracks: vec![
                serde_json::json!({
                    "path": original,
                    "title": "36.5℃",
                    "album": "based on their folder name"
                }),
                serde_json::json!({
                    "path": instrumental,
                    "title": "喜剧演员(伴奏)",
                    "album": "based on their folder name"
                }),
            ],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "destination": {"kind": "tag", "field": "album"},
                "operations": [{"op": "regex_extract", "pattern": "^([^(（]+)"}]
            }),
            &input,
            "session-transform",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        let batch = result.batches.first().expect("preview batch");
        assert_eq!(batch.actions.len(), 2, "both versions must get an album action");
        for action in &batch.actions {
            let path = action.track_path.as_deref().unwrap();
            let expected = if path == original { "36.5℃" } else { "喜剧演员" };
            assert_eq!(action.field.as_deref(), Some("album"));
            assert_eq!(action.old_value.as_deref(), Some("based on their folder name"));
            assert_eq!(action.new_value.as_deref(), Some(expected));
            // The preview description must describe the real destination change
            // (old album value), not the source title value.
            let expected_description =
                format!("Transform album from 'based on their folder name' to '{expected}'");
            assert_eq!(
                action.description.as_deref(),
                Some(expected_description.as_str())
            );
        }
    }

    #[test]
    fn metadata_transform_regex_extract_skips_when_destination_already_equal() {
        // Same-field style guard: when the destination already holds the
        // extracted value, no action is produced even though the pattern matched
        // and the extraction equals the source.
        let input = AssistantSendInput {
            selected_track_paths: vec!["/music/红昭愿.flac".into()],
            tracks: vec![serde_json::json!({
                "path": "/music/红昭愿.flac",
                "title": "红昭愿",
                "album": "红昭愿"
            })],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "title"},
                "destination": {"kind": "tag", "field": "album"},
                "operations": [{"op": "regex_extract", "pattern": "^([^(（]+)"}]
            }),
            &input,
            "session-transform",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        assert!(result.batches.is_empty(), "no preview batch when nothing changes");
        assert!(
            result.result.summary.contains("already had the target value"),
            "no-changes summary should explain why: {}",
            result.result.summary
        );
    }

    #[test]
    fn metadata_transform_before_paren_library_scope_fixes_originals_and_instrumentals() {
        // Regression for session #1785647941055-728246's final failing shape:
        // "album tag must be based on the common string from title tag -
        // everything before '('" with target_scope "library". The pattern
        // `^([^(（]+)` must produce actions for BOTH the paren-less original
        // (extraction equals the whole title) and the instrumental version,
        // so the two versions share one album.
        let original = "/music/音阙诗听/36.5℃/音阙诗听_李佳思-36.5℃.flac";
        let instrumental = "/music/音阙诗听/红昭愿(伴奏)/音阙诗听-红昭愿(伴奏).flac";
        let input = AssistantSendInput {
            tracks: vec![
                serde_json::json!({
                    "path": original,
                    "title": "36.5℃",
                    "album": "based on their folder name"
                }),
                serde_json::json!({
                    "path": instrumental,
                    "title": "红昭愿(伴奏)",
                    "album": "based on their folder name"
                }),
            ],
            ..Default::default()
        };
        let result = execute_metadata_transform(
            &serde_json::json!({
                "target_scope": "library",
                "source": {"kind": "tag", "field": "title"},
                "destination": {"kind": "tag", "field": "album"},
                "operations": [{"op": "regex_extract", "pattern": "^([^(（]+)"}]
            }),
            &input,
            "session-transform",
        );
        assert!(result.result.ok, "{}", result.result.summary);
        let batch = result.batches.first().expect("preview batch");
        assert_eq!(batch.actions.len(), 2, "both versions must get an album action");
        let by_path: std::collections::HashMap<&str, &AssistantAction> = batch
            .actions
            .iter()
            .map(|action| (action.track_path.as_deref().unwrap(), action))
            .collect();
        assert_eq!(by_path[original].new_value.as_deref(), Some("36.5℃"));
        assert_eq!(by_path[instrumental].new_value.as_deref(), Some("红昭愿"));
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
