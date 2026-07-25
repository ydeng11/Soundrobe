# Plan: Make AI Assistant More Flexible with General Tools

## Context

The Soundrobe AI assistant currently has 22 registered tools, many of which are
specialized "macro" tools that each handle a narrow transformation:

- `auto_numbering_tracks`
- `strip_track_title_prefixes`
- `extract_tag_value`
- `chinese_convert`
- `strip_filename_prefixes`
- `infer_tags_from_filenames`
- `group_by_album`
- `remove_embedded_cover`
- `edit_metadata` (the most general, but still limited to uniform updates)

In addition, `derive_assistant_task_contract` performs **deterministic semantic
routing** of common user messages to these specialized code paths, bypassing the
LLM entirely for those requests.

The user's request: **the AI assistant should be more flexible with more general
tools.** For example, if the user says "remove title", "remove parts of title",
or "convert the title", the AI should figure out the desired transformation and
use an agnostic tool to set the new value — instead of relying on a dedicated
macro or a deterministic route.

## Current Limitations

1. **Too many narrow tools.** The LLM receives 22 tool definitions. Many are
   narrow macros that could be expressed as a small set of orthogonal primitives.

2. **Semantic routing preempts the LLM.** `derive_assistant_task_contract`
   intercepts messages like "strip title prefixes" and routes them directly to a
   code macro, so the LLM never gets to reason about the task.

3. **No per-track value support.** `edit_metadata` applies the same
   `standard_updates` to all tracks in the scope. It cannot say "for track A set
   title to X, for track B set title to Y". The LLM would need one tool call per
   track, consuming many steps.

4. **No transformation pipeline.** There is no way for the LLM to compose
   operations (e.g., "extract a regex group, then trim, then title-case").

5. **Monolithic code file.** 5k+ lines in `assistant.rs` with no clear boundary
   between action planning, transformation logic, and LLM orchestration.

6. **No tool descriptions.** `context_tool_catalog()` exposes names and schemas
   but no semantic descriptions, impairing model selection reliability.

## Approach

Replace the many narrow macros with a small set of **orthogonal, structured
primitives** that the LLM composes at inference time. The LLM reasons about the
task and chooses the right combination of tools, operations, and parameters.
Rust validates each input and deterministically compiles it into an
`AssistantActionBatch` for preview/apply.

### Target Public Tool Set

| Tool | Read-only | Purpose |
|---|---|---|
| `library.summarize` | Yes | Summarize current library, active album, and selection |
| `tracks.search` | Yes | Search current loaded tracks by safe fields |
| `tracks.inspect` | Yes | Inspect selected tracks or explicit library paths |
| `albums.inspect` | Yes | Inspect an active/library album |
| `query.metadata` | Yes | Run typed read-only metadata queries |
| `query.datasetStatus` | Yes | Return local dataset availability and counts |
| `api.musicbrainzSearch` | Yes | Safe MusicBrainz GET search |
| `api.discogsSearch` | Yes | Safe Discogs GET search if enabled |
| `api.lyricsSearch` | Yes | Safe configured lyrics API GET |
| — | — | — |
| `metadata.patch` | No | Uniform or per-track set / remove / upsert of tag fields |
| `metadata.transform` | No | Deterministic transformation pipeline on a tag field or filename |
| `files.transform` | No | Filename/path transformations (higher risk) |
| `metadata.prettify` | No | Prettify tag fields (casing/punctuation normalization) |
| `library.run_task` | No | Run auto-tag or audit on a scope |
| `plan.create` | No | Compose multiple tool calls with dependency ordering |

**Narrow macros removed from the public catalog** (kept as internal helpers for
compatibility, but no longer advertised to the LLM):
- `auto_numbering_tracks` → internal, or expressed via `metadata.transform` with incremental numbering
- `strip_track_title_prefixes` → internal; LLM uses `metadata.transform` with `regex_replace`
- `extract_tag_value` → internal; LLM uses `metadata.transform` with `regex_extract`
- `chinese_convert` → internal; LLM uses `metadata.transform` with `chinese_convert`
- `strip_filename_prefixes` → `files.transform`
- `infer_tags_from_filenames` → `metadata.transform` with `source: {kind: "filename"}`
- `group_by_album` → `files.transform`
- `remove_embedded_cover` → keep as `metadata.patch` variant or separate low-risk tool
- `edit_metadata` → superseded by `metadata.patch` (or retained as alias)

### Structured Schemas

#### `metadata.patch`

Uniform changes and per-track overrides in a single call:

```json
{
  "target_scope": "selected",
  "paths": [],
  "changes": [
    {"tag_kind": "standard", "field": "genre", "action": "remove"},
    {"tag_kind": "extra", "field": "MOOD", "action": "upsert", "value": "Night"}
  ],
  "per_track_changes": [
    {
      "path": "/music/a.mp3",
      "changes": [
        {"field": "title", "action": "set", "value": "Intro"},
        {"field": "trackNumber", "action": "set", "value": 1}
      ]
    },
    {
      "path": "/music/b.mp3",
      "changes": [
        {"field": "title", "action": "set", "value": "First Song"},
        {"field": "artists", "action": "set", "value": ["Artist A", "Artist B"]}
      ]
    }
  ]
}
```

Rules:
- `action` enum: `set`, `remove`, `upsert` (extra only)
- `set` with a null/empty value for unique fields (title, artist, trackNumber) is rejected
- `per_track_changes` must reference paths in the resolved scope
- Uniform `changes` apply first, then `per_track_changes` override

#### `metadata.transform`

Typed pipeline of deterministic operations on a source to produce new field values:

```json
{
  "target_scope": "selected",
  "paths": [],
  "source": {"kind": "tag", "field": "title"},
  "destination": {"kind": "tag", "field": "title"},
  "operations": [
    {"op": "regex_replace", "pattern": "^\\d+\\s*[-.]?\\s*", "replacement": ""},
    {"op": "trim"},
    {"op": "title_case"}
  ]
}
```

Supported `source.kind` values:
- `"tag"` — read from a tag field (requires `source.field`)
- `"filename"` — read from the filename (stem, without extension)
- `"path"` — read from the relative directory path

Supported `destination.kind` values:
- `"tag"` — write to a tag field (requires `destination.field`)
- `"filename"` — rename the file (higher risk, preview-first)

Supported `op` values:

| op | Parameters | Description |
|---|---|---|
| `regex_replace` | `pattern`, `replacement` | Replace regex matches |
| `regex_extract` | `pattern`, `group_index` (default 1) | Extract capture group |
| `strip_prefix` | `prefix` | Remove leading string |
| `strip_suffix` | `suffix` | Remove trailing string |
| `literal_replace` | `find`, `replacement` | Simple string replacement |
| `trim` | — | Trim whitespace |
| `lowercase` | — | Convert to lowercase |
| `uppercase` | — | Convert to uppercase |
| `title_case` | — | Title-case each word |
| `prettify` | — | Normalize casing/punctuation (reuses `prettify_tag`) |
| `chinese_to_simplified` | — | Convert Chinese to simplified |
| `chinese_to_traditional` | — | Convert Chinese to traditional |

Each operation is a pure function `String -> Option<String>` (None means no
change). The pipeline chains them: output of op N is input to op N+1. If any op
returns None, the track is skipped (no action produced).

#### `files.transform`

Same pipeline model as `metadata.transform` but restricted to:
- `source.kind`: `"filename"` or `"path"`
- `destination.kind`: `"filename"` only
- Higher risk level (`"medium"` or `"high"`)
- Additional path containment checks

### Code Structure

Extract transformation logic and action planning into a new file:

```
frontend/src-tauri/src/commands/
  ├── assistant.rs                 # LLM orchestration, dispatch, routing (shrunk)
  ├── assistant_tools.rs           # Tool registry, schemas, catalog, validation
  └── assistant_metadata_tools.rs  # metadata.patch, metadata.transform, files.transform
```

- `assistant_metadata_tools.rs` contains:
  - `execute_metadata_patch()` — validates and compiles patch changes into actions
  - `execute_metadata_transform()` — runs the pipeline per track, produces actions
  - `execute_files_transform()` — same pipeline model for filenames with path containment
  - Pure pipeline operation functions (each independently testable)
  - Shared helpers (`tool_scope_paths`, `push_string_action`, etc.) remain in `assistant.rs` or move to a shared module

### Tool Descriptions

Extend `AssistantToolDefinition` in `assistant_tools.rs` with a `description`
field. Include descriptions in `context_tool_catalog()`. Examples:

```rust
AssistantToolDefinition {
    name: "metadata.transform",
    description: "Apply a pipeline of deterministic transformations to a tag field or filename. Operations are composed sequentially: regex_replace -> trim -> title_case. Source can be a tag field or the filename stem.",
    examples: &[
        "Strip leading track numbers: {\"source\":{\"kind\":\"tag\",\"field\":\"title\"},\"operations\":[{\"op\":\"regex_replace\",\"pattern\":\"^\\\\d+\\\\s*[-.]?\\\\s*\",\"replacement\":\"\"},{\"op\":\"trim\"}]}",
        "Extract first word: {\"operations\":[{\"op\":\"regex_extract\",\"pattern\":\"^(\\\\w+)\"}]}",
    ],
    input_schema: ...,
    read_only: false,
    operation_kind: Kind::MetadataEdit,
}
```

### Routing Changes

Change `derive_assistant_task_contract()` from **semantic macro routing** to
**safety classification only**:

- Detect auto-tag / audit intent → still route deterministically (these are
  complex multi-step tasks that benefit from dedicated code paths)
- Empty or nonsense input → `ClarificationRequired`
- Everything else → let the LLM reason about it

This means "strip title prefixes", "chinese convert", "extract tag value", etc.
are no longer intercepted. The LLM sees the track context, figures out what
transformation is needed, and calls `metadata.transform` with the right pipeline.

### Migration Table

| Old tool | New equivalent |
|---|---|
| `edit_metadata` (uniform) | `metadata.patch` with `changes` only |
| `edit_metadata` (per-track, not possible) | `metadata.patch` with `per_track_changes` |
| `auto_numbering_tracks` | Keep as internal helper; LLM can also use `metadata.transform` with position-based numbering |
| `strip_track_title_prefixes` | `metadata.transform(source:{tag:title}, operations:[{op:regex_replace, pattern:"^\\d+\\s*[-.]?\\s*", replacement:""}, {op:trim}])` |
| `extract_tag_value` | `metadata.transform(operations:[{op:regex_extract, pattern:"..."}])` |
| `chinese_convert` | `metadata.transform(operations:[{op:chinese_to_simplified}])` |
| `strip_filename_prefixes` | `files.transform(source:{kind:filename}, operations:[{op:regex_replace, pattern:"^\\d+[\\s.\\\\)-]+", replacement:""}])` |
| `infer_tags_from_filenames` | `metadata.transform(source:{kind:filename}, destination:{kind:tag, field:title}, operations:[{op:regex_extract, pattern:"^\\d+\\s*[-.]?\\s*(.+)"}])` |
| `group_by_album` | `files.transform(source:{kind:tag, field:album}, destination:{kind:filename})` |
| `remove_embedded_cover` | `metadata.patch(changes:[{field:"_cover", action:"remove"}])` or keep as dedicated tool |
| `tags.prettify` | `metadata.transform(operations:[{op:prettify}])` |
| `organize_files` | `files.transform` with directory classification operations |

## Files to modify

| File | Change |
|---|---|
| `frontend/src-tauri/src/commands/assistant_tools.rs` | Add `AssistantToolDefinition.description` field. Update `AssistantToolDefinition` struct. Update `context_tool_catalog()` to emit descriptions. Update `assistant_tool_definitions()` with new `metadata.patch`, `metadata.transform`, `files.transform`, `metadata.prettify`. Remove narrow macros from registry. |
| `frontend/src-tauri/src/commands/assistant_metadata_tools.rs` | **New file.** Contains `execute_metadata_patch()`, `execute_metadata_transform()`, `execute_files_transform()`, pure pipeline functions, action compilation. |
| `frontend/src-tauri/src/commands/assistant.rs` | Remove macro implementations (`plan_strip_track_title_prefixes`, `plan_chinese_conversion`, etc.) — moved to new file. Update `execute_mutating_assistant_tool()` dispatch to call new functions. Update `assistant_response_schema()` to reference new tool names. Simplify `derive_assistant_task_contract()` to safety classification only. Add `mod assistant_metadata_tools;`. |
| `frontend/src-tauri/src/commands/mod.rs` | Add `pub(crate) mod assistant_metadata_tools;`. |
| `frontend/src-tauri/src/state/assistant.rs` | No changes needed (action batch model is already general enough). |
| `frontend/src/shared/desktop-api.ts` | Update `AssistantActionBatch` type if new batch kinds are added. Update `AssistantAction` if new action types are added. |
| `frontend/src/components/AssistantPanel.tsx` | Likely no changes (handles action batches generically). |

## Reuse

- **`prettify_tag`** in `assistant_tools.rs` → reused as the `prettify` operation
- **`strip_track_title_prefix`** in `assistant.rs` → moved to new file, reused as regex in `metadata.transform` examples
- **Chinese conversion** (`convert_chinese_text` in `providers.rs`) → reused as `chinese_to_simplified` / `chinese_to_traditional` operations
- **`tool_scope_paths`** in `assistant.rs` → reused by all new tools
- **`push_string_action`**, **`push_numeric_action`**, **`action_value_string`** → moved to shared location or new file
- **`assistant_batch`**, **`AssistantAction`**, **`AssistantActionBatch`** → reused unchanged
- **`validate_registered_tool_args`** → reused for all new tool schemas

## Testing Plan

### Table-driven schema validation
- Every new tool and operation validated: required fields, type checks, enum values, nested schemas
- Invalid regex patterns rejected
- Unsupported source/destination combinations rejected
- Path containment for `files.transform`

### Pure pipeline operation tests
Each operation tested in isolation:
- `regex_replace` with no match, match, global flag, invalid pattern
- `regex_extract` with group 0, group 1, missing group
- `strip_prefix` / `strip_suffix` with match, no match, empty prefix
- `trim`, `lowercase`, `uppercase`, `title_case` with various inputs
- `prettify` (reuse existing tests, expand with edge cases)
- `chinese_to_simplified` / `chinese_to_traditional` (reuse existing)
- `literal_replace` with found, not found, empty find

### Pipeline composition tests
- Chaining 2+ operations produces correct final value
- Pipeline with an all-None skip produces no actions
- Pipeline on a list field (artists) applies operation per item

### `metadata.patch` planner tests
- Uniform changes produce correct actions with old/new values
- Per-track changes override uniform changes
- Blank-value rejection for unique fields (title, artist)
- Extra-tag upsert/remove
- Multiple tracks with different per-track changes
- No-op elimination (old value == new value)

### `metadata.transform` planner tests
- Tag source with each operation type
- Filename source (stem extraction)
- Per-track actions with old/new values
- No-op elimination
- Scope resolution (selected, active_album, library, explicit_paths)

### `files.transform` planner tests
- Path containment enforcement
- Filename sanitization
- No-overwrite checks
- Reversible manifest creation

### Compatibility tests
- Old tool behavior produces identical action batches through new tools
- `strip_track_title_prefixes("01. Song")` == `metadata.transform({source:{tag:title}, ops:[{op:regex_replace, pattern:"^\\d+\\s*[-.]?\\s*", replacement:""}, {op:trim}]})`

### Catalog tests
- Unique tool names
- All tools have descriptions and schemas
- Read-only vs mutating classification
- Schema JSON Schema validity checked

### Mocked assistant-response tests
- Mock the LLM response to verify the tool dispatch and action batch production
- Test each user example from the plan:
  - "remove title from these tracks" → `metadata.patch` with remove
  - "strip track numbers from titles" → `metadata.transform` with regex_replace
  - "convert titles to lowercase" → `metadata.transform` with lowercase
  - "extract first word as title" → `metadata.transform` with regex_extract
  - "prettify all titles" → `metadata.transform` with prettify

### Integration tests
- Preview creation still applies through existing `WriteQueue`
- Apply flow unchanged
- Undo snapshots still created

## Steps

- [ ] **1. Add `description` field to `AssistantToolDefinition`** — update struct,
  `context_tool_catalog()`, and all existing tool definitions with descriptions
- [ ] **2. Create `assistant_metadata_tools.rs`** — move shared helpers
  (`tool_scope_paths`, `push_string_action`, etc.) and add the new tool executors
- [ ] **3. Implement pure pipeline operation functions** — each op as a
  `fn(&str) -> Option<String>`, with unit tests
- [ ] **4. Implement `execute_metadata_patch()`** — uniform + per-track changes,
  validation, no-op elimination, action batch creation
- [ ] **5. Implement `execute_metadata_transform()`** — pipeline execution per
  track, source resolution (tag/filename), action batch creation
- [ ] **6. Implement `execute_files_transform()`** — same pipeline model with
  path containment and higher risk level
- [ ] **7. Update tool registry** — add new tools, remove narrow macros, update
  `context_tool_catalog()`
- [ ] **8. Update `assistant_response_schema()`** — replace old tool names with
  new ones in the `toolName` enum
- [ ] **9. Simplify `derive_assistant_task_contract()`** — remove semantic macro
  routing, keep only safety classification and auto-tag/audit detection
- [ ] **10. Update `execute_mutating_assistant_tool()` dispatch** — wire new
  tool executors
- [ ] **11. Write compatibility tests** — verify old macro behavior matches new
  tool output
- [ ] **12. Write catalog and schema validation tests**
- [ ] **13. Write mocked assistant-response tests** for each user example
- [ ] **14. Run quality gate** — `cd frontend && npm run typecheck && cd ../.. && cd frontend && npm test`

## Verification

1. **Rust unit tests pass** — `cd frontend/src-tauri && cargo test assistant`
   - Pipeline operations
   - `metadata.patch` planner
   - `metadata.transform` planner
   - `files.transform` planner
   - Schema validation
   - Catalog tests
2. **Compatibility tests pass** — old macro behavior matches new tool output
3. **TypeScript typecheck passes** — `cd frontend && npm run typecheck`
4. **Renderer tests pass** — `cd frontend && npm run test:web`
5. **Manual E2E check** — Open the app, open the assistant panel, send:
   - "strip track numbers from the selected titles" → produces a
     `metadata.transform` action batch with `regex_replace` pipeline
   - "convert all selected titles to lowercase" → produces `lowercase` operation
   - "set track 1 title to Intro" → produces `metadata.patch` with per-track change
   - "remove genre from all tracks" → produces `metadata.patch` with uniform change
   - "prettify all genres" → produces `metadata.transform` with `prettify`
6. **No semantic interception** — "strip title prefixes" goes through the LLM
   instead of being caught by `derive_assistant_task_contract`
