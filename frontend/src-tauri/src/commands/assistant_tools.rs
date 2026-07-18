use crate::commands::assistant::AssistantSendInput;
use serde_json::{Map, Value};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssistantToolOperationKind {
    ReadOnly,
    MetadataEdit,
    FileMove,
    Lookup,
    Planning,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AssistantToolDefinition {
    pub name: &'static str,
    pub input_schema: Value,
    pub read_only: bool,
    pub operation_kind: AssistantToolOperationKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AssistantToolResult {
    pub ok: bool,
    pub summary: String,
    pub data: Option<Value>,
    pub error: Option<String>,
}

pub(crate) fn assistant_tool_definitions() -> Vec<AssistantToolDefinition> {
    use AssistantToolOperationKind as Kind;

    const TOOLS: &[(&str, bool, Kind)] = &[
        ("library.summarize", true, Kind::ReadOnly),
        ("tracks.search", true, Kind::ReadOnly),
        ("tracks.inspect", true, Kind::ReadOnly),
        ("albums.inspect", true, Kind::ReadOnly),
        ("query.metadata", true, Kind::ReadOnly),
        ("query.datasetStatus", true, Kind::ReadOnly),
        ("api.musicbrainzSearch", true, Kind::Lookup),
        ("api.discogsSearch", true, Kind::Lookup),
        ("api.lyricsSearch", true, Kind::Lookup),
        ("tags.prettify", true, Kind::ReadOnly),
        ("edit_metadata", false, Kind::MetadataEdit),
        ("auto_numbering_tracks", false, Kind::MetadataEdit),
        ("strip_track_title_prefixes", false, Kind::MetadataEdit),
        ("extract_tag_value", false, Kind::MetadataEdit),
        ("chinese_convert", false, Kind::MetadataEdit),
        ("strip_filename_prefixes", false, Kind::FileMove),
        ("infer_tags_from_filenames", false, Kind::MetadataEdit),
        ("organize_files", false, Kind::FileMove),
        ("group_by_album", false, Kind::FileMove),
        ("run_library_task", false, Kind::Planning),
        ("create_plan", false, Kind::Planning),
    ];
    TOOLS
        .iter()
        .map(
            |(name, read_only, operation_kind)| AssistantToolDefinition {
                name,
                input_schema: tool_schema(name),
                read_only: *read_only,
                operation_kind: *operation_kind,
            },
        )
        .collect()
}

pub(crate) fn context_tool_catalog() -> Value {
    Value::Array(
        assistant_tool_definitions()
            .into_iter()
            .filter(|definition| {
                matches!(
                    definition.name,
                    "library.summarize"
                        | "tracks.search"
                        | "tracks.inspect"
                        | "albums.inspect"
                        | "query.metadata"
                        | "query.datasetStatus"
                        | "api.musicbrainzSearch"
                        | "api.discogsSearch"
                        | "api.lyricsSearch"
                        | "tags.prettify"
                )
            })
            .map(|definition| {
                serde_json::json!({
                    "name": definition.name,
                    "inputSchema": definition.input_schema,
                    "readOnly": definition.read_only,
                    "operationKind": operation_kind_name(definition.operation_kind)
                })
            })
            .collect(),
    )
}

fn operation_kind_name(kind: AssistantToolOperationKind) -> &'static str {
    match kind {
        AssistantToolOperationKind::ReadOnly => "read_only",
        AssistantToolOperationKind::MetadataEdit => "metadata_edit",
        AssistantToolOperationKind::FileMove => "file_move",
        AssistantToolOperationKind::Lookup => "lookup",
        AssistantToolOperationKind::Planning => "planning",
    }
}

fn tool_schema(name: &str) -> Value {
    match name {
        "tracks.search" => serde_json::json!({
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "artist": {"type": "string"},
                "album": {"type": "string"},
                "genre": {"type": "string"},
                "year": {"type": "string"},
                "codec": {"type": "string"},
                "missingTitle": {"type": "boolean"},
                "missingArtist": {"type": "boolean"},
                "missingAlbum": {"type": "boolean"},
                "missingYear": {"type": "boolean"},
                "missingGenre": {"type": "boolean"},
                "missingCover": {"type": "boolean"},
                "hasDuplicates": {"type": "boolean"}
            },
            "required": []
        }),
        "tracks.inspect" => serde_json::json!({
            "type": "object",
            "properties": {
                "paths": {"type": "array", "items": {"type": "string"}},
                "limit": {"type": "number"}
            },
            "required": []
        }),
        "albums.inspect" => serde_json::json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": []
        }),
        "query.metadata" => serde_json::json!({
            "type": "object",
            "properties": {
                "aggregate": {"type": "boolean"},
                "missingTags": {"type": "string", "enum": ["title", "artist", "album", "year", "genre"]},
                "duplicates": {"type": "boolean"}
            },
            "required": []
        }),
        "api.musicbrainzSearch" => serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}, "limit": {"type": "number"}},
            "required": ["query"]
        }),
        "api.discogsSearch" => serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "type": {"type": "string", "enum": ["release", "master", "artist", "label"]},
                "limit": {"type": "number"}
            },
            "required": ["query"]
        }),
        "api.lyricsSearch" => serde_json::json!({
            "type": "object",
            "properties": {"artist": {"type": "string"}, "title": {"type": "string"}},
            "required": ["artist", "title"]
        }),
        "tags.prettify" => serde_json::json!({
            "type": "object",
            "properties": {"text": {"type": "string"}, "fields": {"type": "object"}},
            "required": []
        }),
        _ => serde_json::json!({"type": "object", "properties": {}, "required": []}),
    }
}

pub(crate) fn execute_context_tool(
    name: &str,
    args: &Value,
    input: &AssistantSendInput,
) -> AssistantToolResult {
    let Some(definition) = assistant_tool_definitions()
        .into_iter()
        .find(|definition| definition.name == name)
    else {
        return tool_error(format!("Unknown tool: {name}"));
    };
    if let Err(error) = validate_tool_args(&definition.input_schema, args) {
        return tool_error(format!("Invalid arguments for {name}: {error}"));
    }
    match name {
        "library.summarize" => summarize_library(input),
        "tracks.search" => search_tracks(input, args),
        "tracks.inspect" => inspect_tracks(input, args),
        "albums.inspect" => inspect_album(input, args),
        "query.metadata" => query_metadata(input, args),
        "tags.prettify" => prettify_tool(args),
        _ => tool_error(format!(
            "Tool {name} requires native service execution and is not a context-only tool"
        )),
    }
}

fn prettify_tool(args: &Value) -> AssistantToolResult {
    if let Some(text) = args.get("text").and_then(Value::as_str) {
        let prettified = prettify_tag(text);
        return tool_ok(
            format!("Prettified: \"{prettified}\""),
            Some(serde_json::json!({"original": text, "prettified": prettified})),
        );
    }
    if let Some(fields) = args.get("fields").and_then(Value::as_object) {
        let mut prettified = Map::new();
        for (field, value) in fields {
            let Some(value) = value.as_str() else {
                return tool_error(format!("Field \"fields.{field}\" should be a string"));
            };
            prettified.insert(field.clone(), Value::String(prettify_tag(value)));
        }
        return tool_ok(
            format!("Prettified {} field(s).", prettified.len()),
            Some(serde_json::json!({"original": fields, "prettified": prettified})),
        );
    }
    tool_error("Provide either text or fields to prettify".into())
}

fn prettify_tag(text: &str) -> String {
    use std::sync::OnceLock;
    static LEADING_NUMBER: OnceLock<regex::Regex> = OnceLock::new();
    static LETTER_DIGIT: OnceLock<regex::Regex> = OnceLock::new();
    static DIGIT_LETTER: OnceLock<regex::Regex> = OnceLock::new();
    let leading_number = LEADING_NUMBER.get_or_init(|| {
        regex::Regex::new(r"(?i)^\s*(?:disc\s*)?\d{1,3}(?:[._ -]+|\s+)")
            .expect("valid leading track number regex")
    });
    let letter_digit = LETTER_DIGIT
        .get_or_init(|| regex::Regex::new(r"([A-Za-z])(\d)").expect("valid letter-digit regex"));
    let digit_letter = DIGIT_LETTER
        .get_or_init(|| regex::Regex::new(r"(\d)([A-Za-z])").expect("valid digit-letter regex"));
    let stripped = leading_number.replace(text.trim(), "");
    let separators = stripped
        .chars()
        .map(|character| {
            if matches!(character, '_' | '-') {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let separated = letter_digit.replace_all(&separators, "$1 $2");
    let separated = digit_letter.replace_all(&separated, "$1 $2");
    separated
        .split_whitespace()
        .map(title_case_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_case_word(word: &str) -> String {
    use std::sync::OnceLock;
    static DOTTED_ACRONYM: OnceLock<regex::Regex> = OnceLock::new();
    let dotted = DOTTED_ACRONYM.get_or_init(|| {
        regex::Regex::new(r"^(?:[A-Za-z]\.)+[A-Za-z]?\.?$").expect("valid dotted acronym regex")
    });
    if word.is_empty() || dotted.is_match(word) {
        return word.to_string();
    }
    let Some(first_ascii) = word.find(|character: char| character.is_ascii_alphanumeric()) else {
        return word.to_string();
    };
    let Some(last_ascii) = word.rfind(|character: char| character.is_ascii_alphanumeric()) else {
        return word.to_string();
    };
    let core_end = last_ascii
        + word[last_ascii..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(0);
    let core = &word[first_ascii..core_end];
    let mut characters = core.chars();
    let Some(first) = characters.next() else {
        return word.to_string();
    };
    format!(
        "{}{}{}{}",
        &word[..first_ascii],
        first.to_ascii_uppercase(),
        characters.as_str().to_ascii_lowercase(),
        &word[core_end..]
    )
}

fn inspect_tracks(input: &AssistantSendInput, args: &Value) -> AssistantToolResult {
    let args = args
        .as_object()
        .expect("validated tool arguments are an object");
    let explicit_paths = args.get("paths").and_then(Value::as_array).map(|paths| {
        paths
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    let paths = explicit_paths
        .filter(|paths| !paths.is_empty())
        .or_else(|| {
            (!input.selected_track_paths.is_empty()).then(|| input.selected_track_paths.clone())
        })
        .unwrap_or_else(|| {
            input
                .tracks
                .iter()
                .filter_map(|track| track.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        });
    if paths.is_empty() {
        return tool_ok("No tracks loaded in the library.".into(), None);
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_f64)
        .unwrap_or(20.0)
        .round()
        .clamp(1.0, 500.0) as usize;
    let paths = paths.into_iter().take(limit).collect::<Vec<_>>();
    let tracks = paths
        .iter()
        .filter_map(|path| {
            input
                .tracks
                .iter()
                .find(|track| track.get("path").and_then(Value::as_str) == Some(path.as_str()))
        })
        .cloned()
        .collect::<Vec<_>>();
    tool_ok(
        format!("Inspecting {} track(s).", tracks.len()),
        Some(serde_json::json!({"paths": paths, "tracks": tracks})),
    )
}

fn inspect_album(input: &AssistantSendInput, args: &Value) -> AssistantToolResult {
    let album_path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| input.active_album_path.clone());
    let Some(album_path) = album_path else {
        return tool_ok("No album path specified and no active album.".into(), None);
    };
    let album = input
        .albums
        .iter()
        .find(|album| album.get("path").and_then(Value::as_str) == Some(album_path.as_str()))
        .cloned();
    let album_root = std::path::Path::new(&album_path);
    let tracks = input
        .tracks
        .iter()
        .filter(|track| {
            track
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| std::path::Path::new(path).starts_with(album_root))
        })
        .cloned()
        .collect::<Vec<_>>();
    tool_ok(
        format!("Album has {} track(s).", tracks.len()),
        Some(serde_json::json!({
            "path": album_path,
            "albumInfo": album,
            "tracks": tracks
        })),
    )
}

fn query_metadata(input: &AssistantSendInput, args: &Value) -> AssistantToolResult {
    if args.get("aggregate").and_then(Value::as_bool) == Some(true) {
        let tracks = &input.tracks;
        let total = tracks.len();
        let completeness = |field| {
            if total == 0 {
                100.0
            } else {
                ((total - missing_count(tracks, field)) as f64 / total as f64 * 100.0).round()
            }
        };
        let data = serde_json::json!({
            "totalTracks": total,
            "totalAlbums": input.albums.len(),
            "totalArtists": distinct_strings(tracks, "artist").len(),
            "totalGenres": distinct_strings(tracks, "genre").len(),
            "tagCompleteness": {
                "title": completeness("title"),
                "artist": completeness("artist"),
                "album": completeness("album"),
                "year": completeness("year"),
                "genre": completeness("genre")
            }
        });
        return tool_ok(format!("Total tracks: {total}"), Some(data));
    }
    if let Some(field) = args.get("missingTags").and_then(Value::as_str) {
        let argument = match field {
            "title" => "missingTitle",
            "artist" => "missingArtist",
            "album" => "missingAlbum",
            "year" => "missingYear",
            "genre" => "missingGenre",
            _ => return tool_error(format!("Unknown missing-tag field: {field}")),
        };
        return search_tracks(input, &serde_json::json!({argument: true}));
    }
    if args.get("duplicates").and_then(Value::as_bool) == Some(true) {
        return search_tracks(input, &serde_json::json!({"hasDuplicates": true}));
    }
    tool_ok(
        "Specify at least one query option: aggregate, missingTags, or duplicates.".into(),
        None,
    )
}

fn summarize_library(input: &AssistantSendInput) -> AssistantToolResult {
    let tracks = &input.tracks;
    let artists = distinct_strings(tracks, "artist");
    let genres = distinct_strings(tracks, "genre");
    let missing_title = missing_count(tracks, "title");
    let missing_artist = missing_count(tracks, "artist");
    let missing_album = missing_count(tracks, "album");
    let missing_year = missing_count(tracks, "year");
    let missing_genre = missing_count(tracks, "genre");
    let total_size = tracks
        .iter()
        .filter_map(|track| track.get("sizeBytes").and_then(Value::as_u64))
        .sum::<u64>();
    let total_duration = tracks
        .iter()
        .filter_map(|track| track.get("duration").and_then(Value::as_f64))
        .sum::<f64>();
    let mut lines = vec![
        format!(
            "Library: {}",
            input.library_path.as_deref().unwrap_or("No library loaded")
        ),
        format!("Albums: {}, Tracks: {}", input.albums.len(), tracks.len()),
        format!("Artists: {}, Genres: {}", artists.len(), genres.len()),
        format!("Total size: {:.1} MB", total_size as f64 / 1_048_576.0),
        format!("Total duration: {} min", (total_duration / 60.0).round()),
    ];
    for (label, count) in [
        ("Missing titles", missing_title),
        ("Missing artists", missing_artist),
        ("Missing albums", missing_album),
        ("Missing years", missing_year),
        ("Missing genres", missing_genre),
    ] {
        if count > 0 {
            lines.push(format!("{label}: {count}"));
        }
    }
    tool_ok(
        lines.join("\n"),
        Some(serde_json::json!({
            "summary": {
                "albumCount": input.albums.len(),
                "trackCount": tracks.len(),
                "artistCount": artists.len(),
                "genreCount": genres.len(),
                "missingTitle": missing_title,
                "missingArtist": missing_artist,
                "missingAlbum": missing_album,
                "missingYear": missing_year,
                "missingGenre": missing_genre,
                "totalSizeBytes": total_size,
                "totalDurationSeconds": total_duration
            }
        })),
    )
}

fn search_tracks(input: &AssistantSendInput, args: &Value) -> AssistantToolResult {
    let args = args
        .as_object()
        .expect("validated tool arguments are an object");
    let duplicate_keys = duplicate_track_keys(&input.tracks);
    let matches = input
        .tracks
        .iter()
        .filter(|track| track_matches(track, args, &duplicate_keys))
        .collect::<Vec<_>>();
    let limited = matches
        .iter()
        .take(20)
        .copied()
        .cloned()
        .collect::<Vec<_>>();
    let summary = if matches.is_empty() {
        "No tracks match the query.".to_string()
    } else {
        format!("Found {} track(s).", matches.len())
    };
    let paths = limited
        .iter()
        .filter_map(|track| track.get("path").and_then(Value::as_str))
        .collect::<Vec<_>>();
    tool_ok(
        summary,
        Some(serde_json::json!({
            "total": matches.len(),
            "tracks": limited,
            "paths": paths
        })),
    )
}

fn track_matches(track: &Value, args: &Map<String, Value>, duplicates: &HashSet<String>) -> bool {
    for field in ["title", "artist", "album", "genre", "codec"] {
        if let Some(query) = args.get(field).and_then(Value::as_str) {
            let actual = track.get(field).and_then(Value::as_str).unwrap_or_default();
            if !actual.to_lowercase().contains(&query.to_lowercase()) {
                return false;
            }
        }
    }
    if let Some(year) = args.get("year").and_then(Value::as_str) {
        if track.get("year").and_then(Value::as_str) != Some(year) {
            return false;
        }
    }
    for (argument, field) in [
        ("missingTitle", "title"),
        ("missingArtist", "artist"),
        ("missingAlbum", "album"),
        ("missingYear", "year"),
        ("missingGenre", "genre"),
    ] {
        if args.get(argument).and_then(Value::as_bool) == Some(true) && !missing_value(track, field)
        {
            return false;
        }
    }
    if args.get("missingCover").and_then(Value::as_bool) == Some(true)
        && track.get("hasCover").and_then(Value::as_bool) != Some(false)
    {
        return false;
    }
    if args.get("hasDuplicates").and_then(Value::as_bool) == Some(true)
        && !duplicates.contains(&track_identity(track))
    {
        return false;
    }
    true
}

fn duplicate_track_keys(tracks: &[Value]) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();
    for track in tracks {
        let key = track_identity(track);
        if !seen.insert(key.clone()) {
            duplicates.insert(key);
        }
    }
    duplicates
}

fn track_identity(track: &Value) -> String {
    ["title", "artist", "album"]
        .iter()
        .map(|field| {
            track
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_lowercase()
        })
        .collect::<Vec<_>>()
        .join("\u{0}")
}

fn distinct_strings(values: &[Value], field: &str) -> HashSet<String> {
    values
        .iter()
        .filter_map(|value| value.get(field).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn missing_count(values: &[Value], field: &str) -> usize {
    values
        .iter()
        .filter(|value| missing_value(value, field))
        .count()
}

fn missing_value(value: &Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
}

fn tool_ok(summary: String, data: Option<Value>) -> AssistantToolResult {
    AssistantToolResult {
        ok: true,
        summary,
        data,
        error: None,
    }
}

fn tool_error(error: String) -> AssistantToolResult {
    AssistantToolResult {
        ok: false,
        summary: error.clone(),
        data: None,
        error: Some(error),
    }
}

pub(crate) fn validate_tool_args(schema: &Value, args: &Value) -> Result<(), String> {
    let args = args
        .as_object()
        .ok_or_else(|| "Tool arguments should be an object".to_string())?;
    validate_object(schema, args, "")
}

fn validate_object(schema: &Value, args: &Map<String, Value>, prefix: &str) -> Result<(), String> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    for field in required {
        if args.get(field).is_none_or(Value::is_null) {
            return Err(format!("Missing required field: {prefix}{field}"));
        }
    }
    for (field, value) in args {
        let Some(field_schema) = properties.get(field) else {
            return Err(format!("Unknown field: {prefix}{field}"));
        };
        if value.is_null() {
            continue;
        }
        validate_value(&format!("{prefix}{field}"), field_schema, value)?;
    }
    Ok(())
}

fn validate_value(field: &str, schema: &Value, value: &Value) -> Result<(), String> {
    let expected = schema.get("type").and_then(Value::as_str);
    if let Some(expected) = expected {
        let matches = match expected {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            _ => true,
        };
        if !matches {
            return Err(format!(
                "Field \"{field}\" should be a {expected}, got {}",
                value_type(value)
            ));
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            let values = allowed
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string())
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("Field \"{field}\" should be one of: {values}"));
        }
    }
    if let (Some(values), Some(item_schema)) = (
        value.as_array(),
        schema.get("items").filter(|schema| schema.is_object()),
    ) {
        for (index, item) in values.iter().enumerate() {
            validate_value(&format!("{field}[{index}]"), item_schema, item)?;
        }
    }
    if let Some(object) = value.as_object() {
        if schema.get("properties").is_some() {
            validate_object(schema, object, &format!("{field}."))?;
        }
    }
    Ok(())
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::assistant::AssistantSendInput;

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "number"},
                "scope": {"type": "string", "enum": ["selected", "library"]},
                "paths": {"type": "array", "items": {"type": "string"}},
                "filter": {
                    "type": "object",
                    "properties": {"missing": {"type": "boolean"}},
                    "required": ["missing"]
                }
            },
            "required": ["query"]
        })
    }

    #[test]
    fn validates_required_unknown_type_enum_array_and_nested_fields() {
        assert_eq!(
            validate_tool_args(&schema(), &serde_json::json!({})).unwrap_err(),
            "Missing required field: query"
        );
        assert_eq!(
            validate_tool_args(
                &schema(),
                &serde_json::json!({"query": "album", "invented": true})
            )
            .unwrap_err(),
            "Unknown field: invented"
        );
        assert!(validate_tool_args(
            &schema(),
            &serde_json::json!({"query": "album", "limit": "five"})
        )
        .unwrap_err()
        .contains("should be a number"));
        assert!(validate_tool_args(
            &schema(),
            &serde_json::json!({"query": "album", "scope": "album"})
        )
        .unwrap_err()
        .contains("should be one of"));
        assert!(validate_tool_args(
            &schema(),
            &serde_json::json!({"query": "album", "paths": ["ok", 2]})
        )
        .unwrap_err()
        .contains("paths[1]"));
        assert_eq!(
            validate_tool_args(
                &schema(),
                &serde_json::json!({"query": "album", "filter": {}})
            )
            .unwrap_err(),
            "Missing required field: filter.missing"
        );
    }

    #[test]
    fn accepts_valid_typed_arguments() {
        validate_tool_args(
            &schema(),
            &serde_json::json!({
                "query": "album",
                "limit": 5,
                "scope": "selected",
                "paths": ["/music/a.mp3"],
                "filter": {"missing": true}
            }),
        )
        .unwrap();
    }

    #[test]
    fn tool_catalog_has_exact_unique_baseline_names() {
        let definitions = assistant_tool_definitions();
        let names = definitions
            .iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>();
        let unique = names
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(names.len(), 21);
        assert_eq!(unique.len(), names.len());
        assert_eq!(
            names,
            vec![
                "library.summarize",
                "tracks.search",
                "tracks.inspect",
                "albums.inspect",
                "query.metadata",
                "query.datasetStatus",
                "api.musicbrainzSearch",
                "api.discogsSearch",
                "api.lyricsSearch",
                "tags.prettify",
                "edit_metadata",
                "auto_numbering_tracks",
                "strip_track_title_prefixes",
                "extract_tag_value",
                "chinese_convert",
                "strip_filename_prefixes",
                "infer_tags_from_filenames",
                "organize_files",
                "group_by_album",
                "run_library_task",
                "create_plan",
            ]
        );
        assert!(definitions[..10]
            .iter()
            .all(|definition| definition.read_only));
        assert!(definitions[10..]
            .iter()
            .all(|definition| !definition.read_only));
    }

    fn input() -> AssistantSendInput {
        AssistantSendInput {
            library_path: Some("/music".into()),
            active_album_path: Some("/music/Artist/Album".into()),
            selected_track_paths: vec!["/music/Artist/Album/01.mp3".into()],
            tracks: vec![
                serde_json::json!({
                    "path": "/music/Artist/Album/01.mp3",
                    "title": "First",
                    "artist": "Artist",
                    "album": "Album",
                    "genre": "Rock",
                    "year": "2020",
                    "codec": "MP3",
                    "duration": 120,
                    "sizeBytes": 1000,
                    "hasCover": true
                }),
                serde_json::json!({
                    "path": "/music/Artist/Album/02.mp3",
                    "title": null,
                    "artist": "Artist",
                    "album": "Album",
                    "genre": null,
                    "codec": "MP3",
                    "duration": 180,
                    "sizeBytes": 2000,
                    "hasCover": false
                }),
            ],
            albums: vec![serde_json::json!({
                "path": "/music/Artist/Album",
                "name": "Album",
                "artistHint": "Artist",
                "trackCount": 2,
                "hasCover": true
            })],
            ..Default::default()
        }
    }

    #[test]
    fn deterministic_library_summary_reports_completeness_and_totals() {
        let result = execute_context_tool("library.summarize", &serde_json::json!({}), &input());

        assert!(result.ok);
        assert!(result.summary.contains("Albums: 1, Tracks: 2"));
        assert!(result.summary.contains("Missing titles: 1"));
        assert!(result.summary.contains("Missing genres: 1"));
        assert_eq!(
            result.data.unwrap()["summary"]["totalDurationSeconds"].as_f64(),
            Some(300.0)
        );
    }

    #[test]
    fn deterministic_track_search_returns_exact_paths_and_validates_args() {
        let result = execute_context_tool(
            "tracks.search",
            &serde_json::json!({"missingTitle": true}),
            &input(),
        );
        assert!(result.ok);
        assert_eq!(
            result.data.unwrap()["paths"],
            serde_json::json!(["/music/Artist/Album/02.mp3"])
        );

        let invalid = execute_context_tool(
            "tracks.search",
            &serde_json::json!({"invented": true}),
            &input(),
        );
        assert!(!invalid.ok);
        assert!(invalid.error.unwrap().contains("Unknown field"));
    }

    #[test]
    fn unknown_or_non_context_tool_fails_explicitly() {
        let unknown = execute_context_tool("not.real", &serde_json::json!({}), &input());
        assert!(!unknown.ok);
        assert!(unknown.summary.contains("Unknown tool"));

        let network = execute_context_tool(
            "api.musicbrainzSearch",
            &serde_json::json!({"query": "artist:Artist album:Album"}),
            &input(),
        );
        assert!(!network.ok);
        assert!(network
            .error
            .unwrap()
            .contains("requires native service execution"));
    }

    #[test]
    fn inspect_tools_honor_selection_and_active_album_defaults() {
        let tracks = execute_context_tool("tracks.inspect", &serde_json::json!({}), &input());
        assert!(tracks.ok);
        assert_eq!(
            tracks.data.unwrap()["paths"],
            serde_json::json!(["/music/Artist/Album/01.mp3"])
        );

        let album = execute_context_tool("albums.inspect", &serde_json::json!({}), &input());
        assert!(album.ok);
        assert_eq!(album.data.unwrap()["tracks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn metadata_query_aggregates_and_reuses_missing_tag_semantics() {
        let aggregate = execute_context_tool(
            "query.metadata",
            &serde_json::json!({"aggregate": true}),
            &input(),
        );
        assert!(aggregate.ok);
        assert_eq!(aggregate.data.unwrap()["totalTracks"], 2);

        let missing = execute_context_tool(
            "query.metadata",
            &serde_json::json!({"missingTags": "genre"}),
            &input(),
        );
        assert!(missing.ok);
        assert_eq!(
            missing.data.unwrap()["paths"],
            serde_json::json!(["/music/Artist/Album/02.mp3"])
        );
    }

    #[test]
    fn prettify_tool_preserves_acronyms_and_normalizes_track_separators() {
        let single = execute_context_tool(
            "tags.prettify",
            &serde_json::json!({"text": "110-hedgehog-you_are_so_famous"}),
            &input(),
        );
        assert!(single.ok);
        assert_eq!(
            single.data.unwrap()["prettified"],
            "Hedgehog You Are So Famous"
        );

        let fields = execute_context_tool(
            "tags.prettify",
            &serde_json::json!({"fields": {"artist": "F.I.R.", "title": "track2_live"}}),
            &input(),
        );
        assert!(fields.ok);
        assert_eq!(fields.data.unwrap()["prettified"]["artist"], "F.I.R.");
    }
}
