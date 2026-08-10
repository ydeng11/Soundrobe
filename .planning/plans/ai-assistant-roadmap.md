# Plan: In-App AI Assistant With Bounded Tools

## Context

Soundrobe already has most of the primitives needed for an assistant:

- OpenRouter client and app config
- Electron IPC bridge between renderer and main process
- Track reading and writing
- Extra tag reading and writing
- Library scanning and album discovery
- Dataset and cache SQLite readers
- Existing long-running task/event patterns for auto-tag and audit

The assistant should build on those primitives instead of adding a general agent
framework in v1. The goal is a safe, app-specific assistant that can chat with
the user and use a small set of validated tools to organize the current music
library.

## Decisions

1. **No external agent framework for v1**
   - Use a lightweight local tool loop around the existing `OpenRouterClient`.
   - Keep tool execution in the Electron main process.
   - Keep the model constrained to a fixed registry of app tools.

2. **Refactor reusable app behavior into shared services**
   - Existing buttons and assistant tools should call the same main-process
     service methods.
   - IPC handlers should become thin wrappers around those methods.
   - Renderer handlers should stay focused on UI state, undo integration, and
     refresh behavior.

3. **Do not convert all LLM features into an agent**
   - Auto-tag candidate selection, tag correction, fallback generation, and
     audit should stay as structured LLM tasks.
   - The assistant gets the tool loop because it needs conversation, planning,
     and tool selection.
   - Both paths should share a common LLM task runner, config, retry behavior,
     schema parsing, and cost accounting.

4. **Default safety mode is preview then apply**
   - Any mutating action creates a preview batch first.
   - The user must approve the batch before files, tags, folders, or external
     state change.
   - Settings may enable autonomous mode, but only for current-library tools
     and validated safe presets.

5. **Scope is current library only**
   - All filesystem reads, writes, and moves must resolve inside the selected
     library root.
   - No arbitrary shell, arbitrary SQL, arbitrary filesystem path access, or
     arbitrary HTTP request execution in v1.

## Target Architecture

```text
Renderer
  TitleBar / AssistantPanel / SettingsModal
        |
        v
Preload API
  assistant:send
  assistant:apply-actions
  assistant:cancel
  assistant:event
        |
        v
Main-process IPC handlers
        |
        v
Shared app services
  LlmTaskRunner
  AssistantRuntime
  AssistantToolRegistry
  LibraryService
  TrackTagService
  ExtraTagService
  FolderOrganizerService
  SafeQueryService
  SafeApiRequestService
  AutoTagTaskService
  AuditTaskService
        |
        v
Existing handlers/utilities
  openrouter.ts
  writer.ts
  tracks.ts
  library.ts
  dataset.ts
  cache.ts
  musicbrainz.ts
  discogs.ts
  lyrics.ts
```

## Shared Service Refactor

### LlmTaskRunner

Create a small orchestration layer over `OpenRouterClient`.

Responsibilities:

- Load current config via existing config loader.
- Run structured JSON tasks with schema names and schemas.
- Run assistant tool loops with the same client.
- Normalize retry and parse errors.
- Track usage and estimated cost.
- Redact API keys and tokens from logs, prompts, and tool results.

Suggested API:

```ts
interface LlmTaskRunner {
  runStructuredTask<T>(input: {
    taskName: string;
    messages: Array<{ role: string; content: string }>;
    schemaName: string;
    schema: Record<string, unknown>;
    model?: string;
    maxTokens?: number;
  }): Promise<{ data: T; usage: TokenUsage; model: string }>;

  runToolLoop(input: AssistantLoopInput): Promise<AssistantLoopResult>;
}
```

Existing LLM use should migrate gradually:

- Candidate selection calls `runStructuredTask`.
- Tag correction calls `runStructuredTask`.
- Fallback generation calls `runStructuredTask`.
- Audit calls `runStructuredTask`.
- Assistant chat calls `runToolLoop`.

### LibraryService

Extract reusable library operations from `library.ts`, `directory.ts`, and
renderer refresh flows.

Responsibilities:

- Scan the selected library.
- Read an album.
- Read a directory.
- Return current-library summaries for prompts.
- Resolve and validate paths inside the selected library root.

Suggested methods:

```ts
scanLibrary(libraryPath: string): Promise<AlbumInfo[]>
readAlbum(albumPath: string): Promise<AlbumDetail>
readTracksForAlbums(albumPaths: string[]): Promise<TrackData[]>
assertInsideLibrary(libraryPath: string, targetPath: string): string
summarizeLibraryContext(context: AssistantAppContext): LibrarySummary
```

### TrackTagService

Extract reusable tag writing from the current IPC handler path.

Responsibilities:

- Plan standard tag updates.
- Apply standard tag updates via `writeTags` and `batchWriteTags`.
- Re-read updated metadata.
- Return per-track success or failure results.

Suggested methods:

```ts
planTagUpdates(input: {
  tracks: TrackData[];
  instructions: TagUpdateInstruction[];
}): PlannedActionBatch

applyTagUpdates(input: {
  updates: Array<{ path: string; fields: WriteFields }>;
}): Promise<TrackData[]>
```

The existing `track:write` and `tracks:batch-write` IPC handlers should call
this service after the extraction.

### ExtraTagService

Extract reusable extra-tag behavior from the current `tracks.ts` IPC handlers.

Responsibilities:

- Read extra tags for a track.
- Plan extra-tag upsert and remove operations.
- Apply single-track or batch extra-tag changes.
- Preserve the current reserved-key behavior in `writer.ts`.

### FolderOrganizerService

Add a new service for file moves and album-title grouping.

Responsibilities:

- Plan moves before applying them.
- Sanitize folder names.
- Avoid overwrites with deterministic suffixes.
- Keep every destination inside the current library root.
- Skip tracks with missing album titles unless user gives a fallback rule.
- Apply approved move batches.
- Produce a reversible move manifest.

Default grouping behavior:

1. Use selected tracks when any tracks are selected.
2. Else use active album tracks when an album is active.
3. Else use all loaded tracks in the current library.
4. Destination is `<libraryRoot>/<sanitized album title>/<original filename>`.
5. Files already in the correct folder are reported as no-ops.

### SafeQueryService

Support read-only, typed database and metadata queries. Do not expose raw SQL in
v1.

Initial query capabilities:

- Find tracks by title, artist, album, genre, year, codec, bitrate, missing tags,
  missing cover, missing lyrics, or duplicate title/artist/album patterns.
- Summarize counts by album, artist, genre, year, codec, and tag completeness.
- Read dataset status via existing `DatasetReader`.
- Read cache stats only through typed aggregate methods.

### SafeApiRequestService

Support safe preset HTTP requests, not arbitrary HTTP.

Initial presets:

- MusicBrainz release search for an artist/album query.
- Discogs release search when a token is configured.
- Lyrics GET request to the configured lyrics API host.

Rules:

- Only GET requests in v1.
- Only known hosts or the configured lyrics host.
- Timeout every request.
- Redact tokens in request previews and responses.
- Return small summarized bodies to the assistant.

## Assistant Runtime

### Tool Loop

The assistant runtime should implement a simple bounded loop:

1. Build a compact prompt with current app context, tool list, and recent chat.
2. Ask the model for either a natural-language response or a JSON tool call.
3. Validate tool name and arguments.
4. Execute read-only tools immediately.
5. For mutating tools, create a pending action batch unless autonomous mode is on.
6. Feed the tool result back to the model.
7. Stop after a final response or a max-step limit.

Use a max of 6 tool steps per user message in v1.

### Tool Call Shape

Use a schema that is explicit and easy to validate:

```ts
type AssistantModelResponse =
  | {
      type: "message";
      content: string;
    }
  | {
      type: "tool_call";
      toolName: string;
      args: Record<string, unknown>;
      reason: string;
    };
```

Tool results should also be structured:

```ts
interface AssistantToolResult {
  ok: boolean;
  summary: string;
  data?: unknown;
  pendingActionBatchId?: string;
  error?: string;
}
```

### App Context Sent To Assistant

Keep context compact and local to the current app state:

```ts
interface AssistantAppContext {
  libraryPath: string | null;
  activeAlbumPath: string | null;
  selectedTrackPaths: string[];
  visibleTrackCount: number;
  selectedTrackSummaries: TrackSummary[];
  activeAlbumSummary: AlbumSummary | null;
  assistantAutonomous: boolean;
}
```

Do not send:

- Full config
- API keys
- Discogs token
- Unbounded track lists
- Full lyrics bodies unless directly requested and bounded

## Initial Tool Registry

### Read-only tools

| Tool | Purpose |
| --- | --- |
| `library.summarize` | Summarize current library, active album, and selection |
| `tracks.search` | Search current loaded tracks by safe fields |
| `tracks.inspect` | Inspect selected tracks or explicit current-library paths |
| `albums.inspect` | Inspect an active/current-library album |
| `query.metadata` | Run typed read-only metadata queries |
| `query.datasetStatus` | Return local dataset availability and counts |
| `api.musicbrainzSearch` | Safe MusicBrainz GET search |
| `api.discogsSearch` | Safe Discogs GET search if enabled |
| `api.lyricsSearch` | Safe configured lyrics API GET |

### Preview-first mutating tools

| Tool | Purpose |
| --- | --- |
| `tags.planUpdate` | Plan add/edit/remove standard tag fields |
| `extraTags.planUpdate` | Plan add/edit/remove extra tags |
| `folders.planGroupByAlbum` | Plan file moves into album-title folders |
| `autoTag.planRun` | Plan running existing auto-tag task on selected scope |
| `audit.planRun` | Plan running existing audit task on selected scope |

### Apply tools

The model should not call apply tools directly in preview mode. The renderer
should call apply by action batch ID after user approval:

- `assistant.applyActionBatch(actionBatchId)`
- `assistant.rejectActionBatch(actionBatchId)`

In autonomous mode, the runtime may apply approved tool categories directly
after emitting progress events, but still through the same action-batch path.

## Action Batch Design

All mutating operations become action batches.

```ts
interface AssistantActionBatch {
  id: string;
  createdAt: string;
  kind:
    | "tag-update"
    | "extra-tag-update"
    | "folder-move"
    | "auto-tag-run"
    | "audit-run";
  title: string;
  summary: string;
  riskLevel: "low" | "medium" | "high";
  actions: AssistantAction[];
  reversible: boolean;
}
```

Examples:

- Tag updates are reversible through existing undo snapshots.
- Extra-tag updates are reversible by storing previous extra tags per file.
- Folder moves are reversible by storing source and destination pairs.
- Auto-tag and audit runs are medium risk because they can write many tags.

The action preview UI should show:

- Number of files affected
- Field-level diffs for tags
- Source and destination for moves
- Skipped files with reasons
- Apply and Reject buttons

## UI Plan

### TitleBar

Add a compact assistant button near settings. Use a familiar icon button with a
tooltip such as "Assistant".

### AssistantPanel

Add a right-side drawer or modal panel. A drawer is preferred because the user
can keep seeing the file grid and inspector while chatting.

Required UI states:

- Empty chat
- Sending
- Tool running
- Assistant message
- Tool result summary
- Pending action batch preview
- Apply in progress
- Applied
- Rejected
- Error

The assistant panel should include:

- Chat transcript
- Input box
- Stop/cancel button while running
- Pending action preview area
- "Use selected tracks" context hint when selection exists

### SettingsModal

Add assistant settings:

- `assistantAutonomous` toggle, default false
- Short warning copy in the setting description
- Persist as `assistant_autonomous`

Optional later settings:

- Max tool steps
- Allowed API presets
- Allowed autonomous action types

## IPC Plan

Add preload types:

```ts
assistantSend(input: AssistantSendInput): Promise<AssistantResponse>
assistantApplyActions(actionBatchId: string): Promise<AssistantApplyResult>
assistantRejectActions(actionBatchId: string): Promise<void>
assistantCancel(sessionId: string): Promise<void>
onAssistantEvent(callback: (event: AssistantEvent) => void): () => void
```

Add main-process handlers:

- `assistant:send`
- `assistant:apply-actions`
- `assistant:reject-actions`
- `assistant:cancel`
- event channel `assistant:event`

Use the existing event forwarding pattern from auto-tag and audit.

## Roadmap

### Phase 1: Service extraction

Goal: make current app actions reusable without changing behavior.

Deliverables:

- Add `LlmTaskRunner` wrapper over `OpenRouterClient`.
- Extract `LibraryService`.
- Extract `TrackTagService`.
- Extract `ExtraTagService`.
- Update existing IPC handlers to call services.
- Keep current renderer button behavior unchanged.

Acceptance:

- Existing unit tests pass.
- Existing auto-tag, audit, metadata edit, batch edit, and extra-tags flows still
  work.
- No assistant UI yet.

### Phase 2: Assistant runtime skeleton

Goal: chat with the assistant and run read-only tools.

Deliverables:

- Add `AssistantRuntime`.
- Add `AssistantToolRegistry`.
- Add `assistant:send`, `assistant:cancel`, and `assistant:event` IPC.
- Add read-only tools:
  - `library.summarize`
  - `tracks.search`
  - `tracks.inspect`
  - `albums.inspect`
  - `query.metadata`
  - `query.datasetStatus`
- Add minimal `AssistantPanel`.

Acceptance:

- User can ask "what is selected?" and get correct track context.
- User can ask "find tracks missing album titles" and get current-library
  results.
- Tool calls are schema-validated.
- Unknown tools and invalid args fail safely.

### Phase 3: Preview action batches

Goal: allow the assistant to propose changes without applying them.

Deliverables:

- Add action-batch store in main process.
- Add `tags.planUpdate`.
- Add `extraTags.planUpdate`.
- Add `folders.planGroupByAlbum`.
- Add action preview UI with Apply and Reject buttons.
- Add `assistant:reject-actions`.

Acceptance:

- "Remove genre from selected tracks" creates a tag diff preview.
- "Add mood=night to selected tracks" creates an extra-tag preview.
- "Group these tracks by album title" creates a move preview with skipped files
  and conflicts.
- No mutation happens before Apply in default mode.

### Phase 4: Apply action batches

Goal: approved assistant actions mutate through shared services.

Deliverables:

- Add `assistant:apply-actions`.
- Apply tag batches through `TrackTagService`.
- Apply extra-tag batches through `ExtraTagService`.
- Apply folder move batches through `FolderOrganizerService`.
- Refresh library, active album, and visible tracks after apply.
- Push undo snapshots for tag changes.
- Store reversible manifests for folder moves.

Acceptance:

- Applying tag changes updates disk and UI.
- Applying extra-tag changes updates disk and UI.
- Applying folder grouping moves files and rescans library.
- Failed per-file actions are reported without hiding successful actions.

### Phase 5: Safe API presets and existing task tools

Goal: let the assistant use external lookup helpers and existing long-running
tasks safely.

Deliverables:

- Add `SafeApiRequestService`.
- Add `api.musicbrainzSearch`.
- Add `api.discogsSearch`.
- Add `api.lyricsSearch`.
- Add `autoTag.planRun`.
- Add `audit.planRun`.
- Route apply for auto-tag and audit through existing task/event flows.

Acceptance:

- User can ask "search MusicBrainz for this album" and see summarized results.
- User can ask "audit selected tracks" and get a preview before running.
- Tokens are redacted from logs, previews, prompts, and errors.

### Phase 6: Autonomous mode

Goal: allow trusted direct execution while preserving bounded tools.

Deliverables:

- Add `assistant_autonomous` config loading and saving.
- Add Settings toggle.
- Runtime applies allowed action batches automatically when enabled.
- High-risk actions still require manual confirmation:
  - folder moves affecting many files
  - auto-tag on whole library
  - audit fixes across whole library

Acceptance:

- Default remains preview-first.
- With autonomous mode on, simple selected-track tag edits can apply directly.
- The assistant still emits action summaries and progress events.
- Path containment and tool schemas are still enforced.

### Phase 7: Polish and hardening

Goal: make the assistant feel stable in real library workflows.

Deliverables:

- Chat transcript persistence per app session.
- Better summaries for large action batches.
- Stop/cancel behavior for long tool loops and long tasks.
- More prompt examples for common music-library commands.
- Optional prompt history shortcuts.

Acceptance:

- Long conversations stay responsive.
- Cancelling stops future tool steps and reports the current state.
- Large batches remain understandable in the preview.

## Testing Plan

### Unit tests

- `LlmTaskRunner`
  - structured task success
  - malformed JSON retry behavior
  - missing API key behavior
  - token redaction

- `AssistantRuntime`
  - final message with no tools
  - read-only tool call
  - invalid tool name
  - invalid args
  - max-step cutoff
  - mutating tool creates pending action in preview mode
  - autonomous mode applies allowed low-risk batch

- `AssistantToolRegistry`
  - duplicate tool names rejected
  - schemas required for every tool
  - read-only vs mutating classification

- `FolderOrganizerService`
  - album-title grouping
  - path containment
  - filename conflict suffixing
  - missing album skip
  - no-op for already grouped files
  - reversible manifest creation

- `SafeQueryService`
  - missing tag queries
  - aggregate summaries
  - duplicate detection
  - no raw SQL access

- `SafeApiRequestService`
  - host allowlist
  - GET-only enforcement
  - timeout handling
  - token redaction

### Component tests

- Assistant panel opens and closes.
- User can send a message.
- Sending state disables duplicate sends.
- Tool result rows render.
- Pending action preview renders diffs and skipped files.
- Apply and Reject buttons call expected APIs.
- Settings autonomous toggle loads and saves.

### Integration tests

- Metadata edit button and assistant tag update both use the same service path.
- Batch edit button and assistant batch tag update produce equivalent writes.
- Extra tag editor and assistant extra-tag update preserve reserved keys.
- Folder grouping moves temp fixture files and rescans.

### E2E smoke test

Use mocked assistant IPC first:

1. Open app with fixture library.
2. Open assistant panel.
3. Send "group selected tracks by album title".
4. See action preview.
5. Apply.
6. Verify visible refresh or success state.

Later add a model-backed smoke test only when API configuration is available.

## Prompting Guidelines

The assistant system prompt should say:

- You are an assistant for Soundrobe.
- Use tools only through the provided registry.
- Default to the current selection, then active album, then current library.
- For destructive or mutating work, create a preview action batch.
- Ask a concise clarification only when the target scope is genuinely unclear.
- Never request or expose API keys.
- Never invent file paths outside the current library.
- Prefer small, reversible batches.

Tool descriptions should include examples:

- "Remove genre from selected tracks" -> `tags.planUpdate`
- "Put all tracks into folders by album" -> `folders.planGroupByAlbum`
- "Which tracks are missing album artist?" -> `query.metadata`
- "Search Discogs for this selected album" -> `api.discogsSearch`

## Risks and Mitigations

| Risk | Mitigation |
| --- | --- |
| Model calls wrong tool | Fixed registry, schema validation, max-step limit |
| Model mutates too much | Preview batches by default, scoped context, action risk levels |
| Path traversal | Canonical path resolution and current-library containment checks |
| Raw SQL damage | No raw SQL in v1, typed query methods only |
| Unsafe HTTP | Safe GET presets only, host allowlist, redacted tokens |
| Prompt leaks secrets | Never include config secrets, redact logs/results |
| Duplicate business logic | Shared services used by buttons and assistant |
| UI gets stale after apply | Central refresh result from action apply path |

## Implementation Order Checklist

- [ ] Create service folders/files and move reusable logic without behavior change.
- [ ] Add tests around extracted services before wiring assistant.
- [ ] Add `LlmTaskRunner` and migrate one structured LLM task as proof.
- [ ] Add assistant runtime with read-only tools.
- [ ] Add assistant IPC and minimal panel.
- [ ] Add preview action-batch model.
- [ ] Add tag and extra-tag planning tools.
- [ ] Add folder grouping planner.
- [ ] Add apply flow for approved batches.
- [ ] Add safe API presets.
- [ ] Add existing task wrappers for auto-tag and audit.
- [ ] Add autonomous setting and guarded autonomous execution.
- [ ] Run `cd frontend && npm test`.
- [ ] Run `cd frontend && npm run typecheck`.

## Out of Scope for v1

- External agent frameworks.
- Arbitrary shell commands.
- Arbitrary SQL.
- Arbitrary HTTP requests.
- Arbitrary local filesystem paths outside the current library.
- Background autonomous monitors.
- Multi-session persisted assistant memory.
- Refactoring the legacy Python CLI.

