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
    pub description: &'static str,
    pub input_schema: Value,
    pub read_only: bool,
    pub public: bool,
    pub operation_kind: AssistantToolOperationKind,
}

/// Returns only the public tool definitions (the orthogonal set shown to the LLM).
pub(crate) fn public_tool_definitions() -> Vec<AssistantToolDefinition> {
    assistant_tool_definitions()
        .into_iter()
        .filter(|d| d.public)
        .collect()
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

    #[derive(Clone, Copy)]
    struct ToolSpec {
        name: &'static str,
        description: &'static str,
        read_only: bool,
        public: bool,
        operation_kind: AssistantToolOperationKind,
    }

    const TOOLS: &[ToolSpec] = &[
        // ── Public read-only tools ───────────────────────────────────────
        ToolSpec {
            name: "library.summarize",
            description: "Get a high-level summary of the current library: album count, track count, artists, genres, total size and duration, and counts of missing tags.",
            read_only: true, public: true,
            operation_kind: Kind::ReadOnly,
        },
        ToolSpec {
            name: "tracks.search",
            description: "Search loaded tracks by title, artist, album, genre, year, codec, missing tags, missing cover, or duplicates. Results are paginated with offset, limit, total, and nextOffset; follow nextOffset until it is null.",
            read_only: true, public: true,
            operation_kind: Kind::ReadOnly,
        },
        ToolSpec {
            name: "tracks.inspect",
            description: "Inspect detailed metadata for specific tracks by path or for selected/active tracks. Results are paginated with offset, limit, total, and nextOffset; follow nextOffset until it is null.",
            read_only: true, public: true,
            operation_kind: Kind::ReadOnly,
        },
        ToolSpec {
            name: "albums.inspect",
            description: "Inspect an album and its tracks by path. Defaults to the active album when no path is given.",
            read_only: true, public: true,
            operation_kind: Kind::ReadOnly,
        },
        ToolSpec {
            name: "query.metadata",
            description: "Query aggregate library statistics (tag completeness, total tracks/albums), find tracks missing a specific tag, or find duplicate tracks.",
            read_only: true, public: true,
            operation_kind: Kind::ReadOnly,
        },
        ToolSpec {
            name: "query.datasetStatus",
            description: "Check whether the local MusicBrainz dataset is available and how many records it contains.",
            read_only: true, public: true,
            operation_kind: Kind::ReadOnly,
        },
        ToolSpec {
            name: "api.musicbrainzSearch",
            description: "Search MusicBrainz for releases by artist and album. Query format: artist:<name> album:<name>. Returns up to 5 releases.",
            read_only: true, public: true,
            operation_kind: Kind::Lookup,
        },
        ToolSpec {
            name: "api.discogsSearch",
            description: "Search Discogs for releases by query string. Requires a configured Discogs token. Returns up to 5 releases.",
            read_only: true, public: true,
            operation_kind: Kind::Lookup,
        },
        ToolSpec {
            name: "api.lyricsSearch",
            description: "Fetch lyrics for a track by artist and title from the configured lyrics API.",
            read_only: true, public: true,
            operation_kind: Kind::Lookup,
        },
        ToolSpec {
            name: "tags.prettify",
            description: "Preview prettified (title-cased, separator-normalized) tag values. Provide a single text string or a map of fields.",
            read_only: true, public: true,
            operation_kind: Kind::ReadOnly,
        },
        // ── Public mutating tools (the orthogonal set) ───────────────────
        ToolSpec {
            name: "metadata.patch",
            description: "Apply uniform or per-track changes to tag fields. Supports set, remove, and upsert (extra tags only) actions. Set only_if_missing on a standard-field change to preserve existing values. Use for explicit value changes where you know the new values.",
            read_only: false, public: true,
            operation_kind: Kind::MetadataEdit,
        },
        ToolSpec {
            name: "metadata.transform",
            description: "Apply a pipeline of deterministic operations to a tag field. Supports regex_replace, regex_extract, strip_prefix, strip_suffix, trim, lowercase, uppercase, title_case, prettify, split_artists, chinese_to_simplified, chinese_to_traditional. Use split_artists on the plural artists field to repair every malformed single value in the requested scope without enumerating tracks. Source can be a tag field or filename stem.",
            read_only: false, public: true,
            operation_kind: Kind::MetadataEdit,
        },
        ToolSpec {
            name: "files.transform",
            description: "Apply operations to filenames with path containment. Same operations as metadata.transform but for renaming files. Higher risk than metadata changes.",
            read_only: false, public: true,
            operation_kind: Kind::FileMove,
        },
        ToolSpec {
            name: "library.run_task",
            description: "Run the auto-tagging or audit task on a scope (selected, active_album, library).",
            read_only: false, public: true,
            operation_kind: Kind::Planning,
        },
        ToolSpec {
            name: "plan.create",
            description: "Create a multi-step plan that chains tool calls with dependency ordering and variable passing between steps.",
            read_only: false, public: true,
            operation_kind: Kind::Planning,
        },
        // ── Legacy internal-only aliases (hidden from LLM) ───────────────
        ToolSpec { name: "edit_metadata", description: "Legacy: use metadata.patch instead.", read_only: false, public: false, operation_kind: Kind::MetadataEdit },
        ToolSpec { name: "auto_numbering_tracks", description: "Legacy: use metadata.transform instead.", read_only: false, public: false, operation_kind: Kind::MetadataEdit },
        ToolSpec { name: "strip_track_title_prefixes", description: "Legacy: use metadata.transform instead.", read_only: false, public: false, operation_kind: Kind::MetadataEdit },
        ToolSpec { name: "extract_tag_value", description: "Legacy: use metadata.transform instead.", read_only: false, public: false, operation_kind: Kind::MetadataEdit },
        ToolSpec { name: "chinese_convert", description: "Legacy: use metadata.transform instead.", read_only: false, public: false, operation_kind: Kind::MetadataEdit },
        ToolSpec { name: "strip_filename_prefixes", description: "Legacy: use files.transform instead.", read_only: false, public: false, operation_kind: Kind::FileMove },
        ToolSpec { name: "infer_tags_from_filenames", description: "Legacy: use metadata.transform instead.", read_only: false, public: false, operation_kind: Kind::MetadataEdit },
        ToolSpec { name: "organize_files", description: "Legacy: use files.transform instead.", read_only: false, public: false, operation_kind: Kind::FileMove },
        ToolSpec { name: "group_by_album", description: "Legacy: use files.transform instead.", read_only: false, public: false, operation_kind: Kind::FileMove },
        ToolSpec { name: "remove_embedded_cover", description: "Legacy: use metadata.patch instead.", read_only: false, public: false, operation_kind: Kind::MetadataEdit },
        ToolSpec { name: "run_library_task", description: "Legacy: superseded by library.run_task.", read_only: false, public: false, operation_kind: Kind::Planning },
        ToolSpec { name: "create_plan", description: "Legacy: superseded by plan.create.", read_only: false, public: false, operation_kind: Kind::Planning },
    ];
    TOOLS
        .iter()
        .map(|spec| AssistantToolDefinition {
            name: spec.name,
            description: spec.description,
            input_schema: tool_schema(spec.name),
            read_only: spec.read_only,
            public: spec.public,
            operation_kind: spec.operation_kind,
        })
        .collect()
}

pub(crate) fn context_tool_catalog() -> Value {
    Value::Array(
        public_tool_definitions()
            .into_iter()
            .map(|definition| {
                serde_json::json!({
                    "name": definition.name,
                    "description": definition.description,
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
    let target_scope = || {
        serde_json::json!({
            "target_scope": {
                "type": "string",
                "enum": ["selected", "active_album", "library", "explicit_paths"]
            },
            "paths": {"type": "array", "items": {"type": "string"}}
        })
    };
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
                "hasDuplicates": {"type": "boolean"},
                "offset": {"type": "number"},
                "limit": {"type": "number"}
            },
            "required": []
        }),
        "tracks.inspect" => serde_json::json!({
            "type": "object",
            "properties": {
                "paths": {"type": "array", "items": {"type": "string"}},
                "offset": {"type": "number"},
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
        "edit_metadata" => serde_json::json!({
            "type": "object",
            "properties": {
                "target_scope": {"type": "string", "enum": ["selected", "active_album", "library", "explicit_paths"]},
                "paths": {"type": "array", "items": {"type": "string"}},
                "standard_updates": {"type": "object", "properties": standard_update_schema(), "required": []},
                "standard_removes": {"type": "array", "items": {"type": "string", "enum": standard_field_values()}},
                "extra_upserts": {"type": "array", "items": {
                    "type": "object",
                    "properties": {"key": {"type": "string"}, "value": {"type": "string"}},
                    "required": ["key", "value"]
                }},
                "extra_removes": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["target_scope"]
        }),
        "auto_numbering_tracks"
        | "strip_track_title_prefixes"
        | "strip_filename_prefixes"
        | "group_by_album"
        | "remove_embedded_cover" => serde_json::json!({
            "type": "object", "properties": target_scope(), "required": ["target_scope"]
        }),
        "extract_tag_value" => serde_json::json!({
            "type": "object",
            "properties": {
                "target_scope": {"type": "string", "enum": ["selected", "active_album", "library", "explicit_paths"]},
                "paths": {"type": "array", "items": {"type": "string"}},
                "field": {"type": "string", "enum": standard_field_values()},
                "pattern": {"type": "string"},
                "group_index": {"type": "number"}
            },
            "required": ["target_scope", "field", "pattern"]
        }),
        "chinese_convert" => serde_json::json!({
            "type": "object",
            "properties": {
                "target_scope": {"type": "string", "enum": ["selected", "active_album", "library", "explicit_paths"]},
                "paths": {"type": "array", "items": {"type": "string"}},
                "fields": {"type": "array", "items": {"type": "string", "enum": ["title", "artist", "artists", "album", "albumArtist", "albumArtists", "genre", "composer", "comment", "description", "lyrics"]}},
                "direction": {"type": "string", "enum": ["s2t", "t2s"]}
            },
            "required": ["target_scope", "direction"]
        }),
        "infer_tags_from_filenames" => serde_json::json!({
            "type": "object",
            "properties": {
                "target_scope": {"type": "string", "enum": ["selected", "active_album", "library", "explicit_paths"]},
                "paths": {"type": "array", "items": {"type": "string"}},
                "fields": {"type": "array", "items": {"type": "string", "enum": ["title", "artist", "artists"]}},
                "prettify": {"type": "boolean"}
            },
            "required": ["target_scope"]
        }),
        "organize_files" => serde_json::json!({
            "type": "object",
            "properties": {
                "source_dir": {"type": "string"},
                "criterion": {"type": "string", "enum": ["extension", "pattern", "date_created", "size"]},
                "pattern_string": {"type": "string"},
                "target_dir_name": {"type": "string"}
            },
            "required": ["source_dir", "criterion", "target_dir_name"]
        }),
        "library.run_task" | "run_library_task" => serde_json::json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "enum": ["auto_tag", "audit"]},
                "target_scope": {"type": "string", "enum": ["selected", "active_album", "library", "explicit_paths"]},
                "paths": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["task", "target_scope"]
        }),
        "create_plan" => serde_json::json!({
            "type": "object",
            "properties": {
                "plan_description": {"type": "string"},
                "steps": {"type": "array", "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "label": {"type": "string"},
                        "tool": {"type": "string"},
                        "args": {"type": "object"},
                        "depends_on": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["id", "tool"]
                }}
            },
            "required": ["steps"]
        }),
        "metadata.patch" => serde_json::json!({
            "type": "object",
            "properties": {
                "target_scope": {"type": "string", "enum": ["selected", "active_album", "library", "explicit_paths"]},
                "paths": {"type": "array", "items": {"type": "string"}},
                "changes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tag_kind": {"type": "string", "enum": ["standard", "extra"]},
                            "field": {"type": "string"},
                            "action": {"type": "string", "enum": ["set", "remove", "upsert"]},
                            "value": {},
                            "only_if_missing": {"type": "boolean"}
                        },
                        "required": ["field", "action"]
                    }
                },
                "per_track_changes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"},
                            "changes": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "field": {"type": "string"},
                                        "action": {"type": "string", "enum": ["set", "remove"]},
                                        "value": {}
                                    },
                                    "required": ["field", "action"]
                                }
                            }
                        },
                        "required": ["path", "changes"]
                    }
                }
            },
            "required": ["target_scope"]
        }),
        "metadata.transform" => serde_json::json!({
            "type": "object",
            "properties": {
                "target_scope": {"type": "string", "enum": ["selected", "active_album", "library", "explicit_paths"]},
                "paths": {"type": "array", "items": {"type": "string"}},
                "source": {
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string", "enum": ["tag", "filename"]},
                        "field": {"type": "string"}
                    },
                    "default": {"kind": "tag", "field": "title"}
                },
                "destination": {
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string", "enum": ["tag", "filename"]},
                        "field": {"type": "string"}
                    },
                    "default": {"kind": "tag", "field": "title"}
                },
                "operations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {"type": "string", "enum": ["regex_replace", "regex_extract", "strip_prefix", "strip_suffix", "literal_replace", "trim", "lowercase", "uppercase", "title_case", "prettify", "split_artists", "chinese_to_simplified", "chinese_to_traditional"]},
                            "pattern": {"type": "string"},
                            "replacement": {"type": "string"},
                            "group_index": {"type": "number"},
                            "prefix": {"type": "string"},
                            "suffix": {"type": "string"},
                            "find": {"type": "string"}
                        },
                        "required": ["op"]
                    }
                }
            },
            "required": ["target_scope", "operations"]
        }),
        "files.transform" => serde_json::json!({
            "type": "object",
            "properties": {
                "target_scope": {"type": "string", "enum": ["selected", "active_album", "library", "explicit_paths"]},
                "paths": {"type": "array", "items": {"type": "string"}},
                "source": {
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string", "enum": ["tag", "filename"]},
                        "field": {"type": "string"}
                    },
                    "default": {"kind": "filename"}
                },
                "operations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {"type": "string", "enum": ["regex_replace", "regex_extract", "strip_prefix", "strip_suffix", "literal_replace", "trim", "lowercase", "uppercase", "title_case", "prettify", "chinese_to_simplified", "chinese_to_traditional"]},
                            "pattern": {"type": "string"},
                            "replacement": {"type": "string"},
                            "group_index": {"type": "number"},
                            "prefix": {"type": "string"},
                            "suffix": {"type": "string"},
                            "find": {"type": "string"}
                        },
                        "required": ["op"]
                    }
                }
            },
            "required": ["target_scope", "operations"]
        }),
        _ => serde_json::json!({"type": "object", "properties": {}, "required": []}),
    }
}

fn standard_field_values() -> Value {
    serde_json::json!([
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
        "musicbrainzArtistId"
    ])
}

fn standard_update_schema() -> Value {
    serde_json::json!({
        "title": {"type": "string"}, "artist": {"type": "string"},
        "artists": {"type": "array", "items": {"type": "string"}},
        "album": {"type": "string"}, "albumArtist": {"type": "string"},
        "albumArtists": {"type": "array", "items": {"type": "string"}},
        "year": {"type": "string"}, "trackNumber": {"type": "number"},
        "trackTotal": {"type": "number"}, "discNumber": {"type": "number"},
        "discTotal": {"type": "number"}, "genre": {"type": "string"},
        "composer": {"type": "string"}, "comment": {"type": "string"},
        "description": {"type": "string"}, "lyrics": {"type": "string"},
        "compilation": {"type": "boolean"}, "musicbrainzTrackId": {"type": "string"},
        "musicbrainzAlbumId": {"type": "string"}, "musicbrainzArtistId": {"type": "string"}
    })
}

pub(crate) fn validate_registered_tool_args(name: &str, args: &Value) -> Result<(), String> {
    let definition = assistant_tool_definitions()
        .into_iter()
        .find(|definition| definition.name == name)
        .ok_or_else(|| format!("Unknown tool: {name}"))?;
    validate_tool_args(&definition.input_schema, args)
}

pub(crate) fn registered_tool_is_read_only(name: &str) -> Option<bool> {
    assistant_tool_definitions()
        .into_iter()
        .find(|definition| definition.name == name)
        .map(|definition| definition.read_only)
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

pub(crate) fn prettify_tag(text: &str) -> String {
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
    let offset = args
        .get("offset")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .round()
        .max(0.0) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_f64)
        .unwrap_or(20.0)
        .round()
        .clamp(1.0, 500.0) as usize;
    let total = paths.len();
    let paths = paths
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
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
    let next_offset = (offset + paths.len() < total).then_some(offset + paths.len());
    tool_ok(
        format!("Inspecting {} track(s).", tracks.len()),
        Some(serde_json::json!({
            "total": total,
            "offset": offset,
            "limit": limit,
            "nextOffset": next_offset,
            "paths": paths,
            "tracks": tracks
        })),
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
    let offset = args
        .get("offset")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .round()
        .max(0.0) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_f64)
        .unwrap_or(20.0)
        .round()
        .clamp(1.0, 100.0) as usize;
    let limited = matches
        .iter()
        .skip(offset)
        .take(limit)
        .copied()
        .cloned()
        .collect::<Vec<_>>();
    let next_offset = (offset + limited.len() < matches.len()).then_some(offset + limited.len());
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
            "offset": offset,
            "limit": limit,
            "nextOffset": next_offset,
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

        // All definitions (27 total) still present for internal dispatch
        assert_eq!(names.len(), 27);
        assert_eq!(unique.len(), names.len());
        // All definitions must have descriptions
        assert!(definitions.iter().all(|d| !d.description.is_empty()));

        // Public tools are the orthogonal set (10 read-only + 5 mutating = 15)
        let public_count = definitions.iter().filter(|d| d.public).count();
        let public_read_only = definitions
            .iter()
            .filter(|d| d.public && d.read_only)
            .count();
        let public_mutating = definitions
            .iter()
            .filter(|d| d.public && !d.read_only)
            .count();
        assert_eq!(public_count, 15, "public tool count should be 15");
        assert_eq!(public_read_only, 10, "10 public read-only tools");
        assert_eq!(public_mutating, 5, "5 public mutating tools");

        // New tools are present in the full definitions
        assert!(names.contains(&"metadata.patch"));
        assert!(names.contains(&"metadata.transform"));
        assert!(names.contains(&"files.transform"));
        // Public catalog only exposes the orthogonal set
        assert_eq!(context_tool_catalog().as_array().map(Vec::len), Some(15));
    }

    #[test]
    fn catalog_entries_include_descriptions() {
        let catalog = context_tool_catalog();
        let entries = catalog.as_array().unwrap();
        assert_eq!(entries.len(), 15, "public catalog should have 15 tools");
        for entry in entries {
            let desc = entry
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            assert!(
                !desc.is_empty(),
                "Tool {} is missing a description",
                entry["name"]
            );
        }
    }

    #[test]
    fn mutating_tool_schemas_validate_nested_arguments_and_reject_invented_fields() {
        validate_registered_tool_args(
            "edit_metadata",
            &serde_json::json!({
                "target_scope": "selected",
                "standard_updates": {"album": "Album", "artists": ["A", "B"]},
                "extra_upserts": [{"key": "MOOD", "value": "Calm"}]
            }),
        )
        .unwrap();
        let error = validate_registered_tool_args(
            "edit_metadata",
            &serde_json::json!({
                "target_scope": "selected",
                "standard_updates": {"madeUpTag": "value"}
            }),
        )
        .unwrap_err();
        assert_eq!(error, "Unknown field: standard_updates.madeUpTag");

        validate_registered_tool_args(
            "metadata.transform",
            &serde_json::json!({
                "target_scope": "selected",
                "source": {"kind": "tag", "field": "artists"},
                "operations": [{"op": "split_artists"}]
            }),
        )
        .unwrap();

        validate_registered_tool_args(
            "create_plan",
            &serde_json::json!({
                "steps": [{
                    "id": "inspect", "tool": "tracks.search",
                    "args": {"missingGenre": true}, "depends_on": []
                }]
            }),
        )
        .unwrap();
    }

    #[test]
    fn public_library_task_schema_requires_task_and_target_scope() {
        let missing_scope = validate_registered_tool_args(
            "library.run_task",
            &serde_json::json!({
                "task": "auto_tag"
            }),
        )
        .unwrap_err();
        assert_eq!(missing_scope, "Missing required field: target_scope");

        validate_registered_tool_args(
            "library.run_task",
            &serde_json::json!({
                "task": "auto_tag",
                "target_scope": "library"
            }),
        )
        .unwrap();
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
    fn deterministic_track_search_pages_through_every_match() {
        let tracks = (0..45)
            .map(|index| {
                serde_json::json!({
                    "path": format!("/music/Artist/Album/{index:02}.flac"),
                    "title": format!("Track {index}"),
                    "genre": null
                })
            })
            .collect();
        let input = AssistantSendInput {
            tracks,
            ..Default::default()
        };

        let first = execute_context_tool(
            "tracks.search",
            &serde_json::json!({"missingGenre": true, "limit": 20}),
            &input,
        );
        let first = first.data.unwrap();
        assert_eq!(first["total"], 45);
        assert_eq!(first["tracks"].as_array().unwrap().len(), 20);
        assert_eq!(first["offset"], 0);
        assert_eq!(first["nextOffset"], 20);

        let last = execute_context_tool(
            "tracks.search",
            &serde_json::json!({"missingGenre": true, "offset": 40, "limit": 20}),
            &input,
        );
        let last = last.data.unwrap();
        assert_eq!(last["tracks"].as_array().unwrap().len(), 5);
        assert_eq!(last["offset"], 40);
        assert!(last["nextOffset"].is_null());
        assert_eq!(
            last["paths"][0],
            serde_json::json!("/music/Artist/Album/40.flac")
        );
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
    fn track_inspection_pages_through_selected_tracks_without_broadening_scope() {
        let problematic_path =
            "/Volumes/downloads/谭咏麟/1982 - 爱人女神/谭咏麟&彭健新-我爱大自然.flac";
        let selected_track_paths = (0..46)
            .map(|index| {
                if index == 20 {
                    problematic_path.to_string()
                } else {
                    format!("/music/selected/{index:02}.flac")
                }
            })
            .collect::<Vec<_>>();
        let mut tracks = selected_track_paths
            .iter()
            .map(|path| {
                serde_json::json!({
                    "path": path,
                    "artist": "谭咏麟 & 彭健新",
                    "artists": ["谭咏麟 & 彭健新"]
                })
            })
            .collect::<Vec<_>>();
        tracks.push(serde_json::json!({
            "path": "/music/unselected.flac",
            "artist": "Outside Artist",
            "artists": ["Outside Artist"]
        }));
        let input = AssistantSendInput {
            selected_track_paths,
            tracks,
            ..Default::default()
        };

        let page = execute_context_tool(
            "tracks.inspect",
            &serde_json::json!({"offset": 20, "limit": 50}),
            &input,
        );

        assert!(page.ok, "{}", page.error.unwrap_or_default());
        let data = page.data.unwrap();
        assert_eq!(data["total"], 46);
        assert_eq!(data["offset"], 20);
        assert!(data["nextOffset"].is_null());
        assert_eq!(data["tracks"].as_array().unwrap().len(), 26);
        assert_eq!(data["paths"][0], problematic_path);
        assert!(
            data["paths"].as_array().unwrap().iter().all(|path| path
                .as_str()
                .is_some_and(|path| path != "/music/unselected.flac")),
            "inspection must remain within selected-track scope"
        );
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
