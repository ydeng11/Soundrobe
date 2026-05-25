# Phase 7 Execution Plan: Terminal UI (Textual)

## Overview

**Goal**: Build a terminal UI for auto-tagger that provides a visual tag editor, auto-tag workflow with audit, and manual fix capability — inspired by MP3Tag but purpose-built for auto-tagger's automated-first workflow.

**Tech Stack**: [Textual](https://textual.textualize.io/) (TUI framework built on Rich)
**New Dependency**: `textual>=1.0.0`
**Entry Point**: `auto-tag ui [PATH]`
**New Package**: `src/auto_tagger/ui/`

**Duration**: ~25-35 hours
**Phase**: 7 (after Phase 6 distribution readiness)

---

## Design Decisions (from grill-me session)

### Layout (MP3Tag-inspired but flipped)
```
┌─────────────────────────────────────────────────────────────────────────────┐
│  [Open] [Auto-Tag] [Stop] [Undo] [Filter]                [Settings] [ⓘ]    │  ← Toolbar
├─────────────────────┬───────────────────────────────────────────────────────┤
│                     │  # │ Filename   │ Title    │ Artist  │ Album  │Status │
│  Title        [   ] │  1 │ 01 Come... │ Come To… │ Beatles │ Abbey… │ ✅    │
│  Artist       [   ] │  2 │ 02 Some... │ Somet…   │ Beatles │ Abbey… │ ✅    │
│  ARTISTS      [   ] │  3 │ 03 Maxwe…  │ Maxwel…  │ Beatles │ Abbey… │ ⚠️    │
│  Album        [   ] │  4 │ 04 Oh! D…  │ Oh! Da…  │ Beatles │ Abbey… │ ✅    │
│  Album Artist [   ] │  5 │ 05 I Wan…  │ I Wan…   │ Beatles │ Abbey… │ ✅    │
│  Year         [   ] │  6 │ 06 Here …  │ Here C…  │ Beatles │ Abbey… │ ✅    │
│  Track        [   ] │                                                    │  ← Track table
│  Disc         [   ] │                                                    │     (spreadsheet)
│  Genre        [   ] │                                                    │
│  Composer     [   ] │                                                    │
│  Comment      [   ] │                                                    │
│                     │                                                    │
│  ┌───────────────┐  │                                                    │
│  │  Cover Art    │  │                                                    │
│  │  Preview      │  │                                                    │
│  │  (right-click)│  │                                                    │
│  └───────────────┘  │                                                    │
├─────────────────────┴───────────────────────────────────────────────────────┤
│  Filter: [____________________________________________]  42 files │ 23 min │  ← Status + Filter
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key Design Points
- **Left panel**: Tag editor (form fields) + cover art preview at bottom
- **Right panel**: Track table (DataTable) with sortable columns, per-track status
- **Top toolbar**: Action buttons for Open, Auto-Tag, Stop, Undo, Filter focus
- **Bottom bar**: Filter input (real-time text/regex) + status bar (file count, duration)
- **Auto-save**: Field edits write to disk immediately
- **Undo**: Stack of operations (auto-tag album, manual edit, batch edit); Ctrl+Z reverts last
- **Incremental load**: Directory tree shown immediately, tags read lazily on album click

### Health/Audit System (Two-Tier)

**Tier 1 — Hard Rules** (instant, deterministic):
- Missing required fields (title, artist, album, track number)
- Track total < actual track count
- Track sequence gaps
- Duplicate track numbers
- Inconsistent album_artist across tracks
- ARTISTS empty when track_artist ≠ album_artist (non-compilation)
- Cover missing (no embedded, no external cover.jpg)
- Genre mismatch between tracks in same album

**Tier 2 — LLM Audit** (automatic after auto-tag, batch per album):
- Batch all tracks in album → structured JSON to LLM
- Assess: artist, title, album, album_artist, artists, path per track
- Return per-track issues with suggestions
- Cost: ~$0.0001 per album (negligible)

### Status Indicators (per track)
- ✅ **Healthy** — all Tier 1 pass, audit passed or not yet run
- ⚠️ **Warning** — Tier 1 warnings or LLM flagged suggestions
- ❌ **Error** — Tier 1 failures (missing required fields etc.)
- **Bold/green highlight**: Recently auto-tagged (changed fields)
- **Yellow highlight**: Flagged by audit
- **Red highlight**: Hard-rule failure

### Album-Level Aggregation
- ✅ All tracks healthy
- ⚠️ Any track with warnings, none failing
- ❌ Any track failing

---

## Architecture

### New Modules

```
src/auto_tagger/ui/
  __init__.py
  app.py                  # Textual App definition, screens, keybindings
  screens/
    __init__.py
    main_screen.py        # Main layout (left panel + table + toolbar + status)
    settings_screen.py    # Inline settings editor (modal or side panel)
    about_screen.py       # Version, health, etc.
  widgets/
    __init__.py
    tag_panel.py          # Left sidebar: form fields + cover preview
    track_table.py        # Right pane: DataTable with columns, sorting, inline edit
    toolbar.py            # Top bar: action buttons
    status_bar.py         # Bottom bar: filter + file count + duration
    cover_viewer.py       # Cover art display with right-click menu
    filter_input.py       # Real-time filter input
  workflow.py             # Subprocess management for auto-tag + audit
  undo.py                 # Undo stack (snapshot/restore per operation)
  state.py                # In-memory state: track data, selections, loading state
```

### Data Flow

```
User clicks "Auto-Tag"
  → UI saves pre-state snapshots of all tracks in library (for undo)
  → Launches subprocess: auto-tag batch /path --json-stream
  → Parses incremental JSON lines from stdout
  → Updates track_table rows: changed fields highlighted green, status updated
  → When batch finishes, triggers LLM audit
  → Launches subprocess: auto-tag audit /path --json-stream (new command)
  → Parses audit results, highlights flagged rows yellow
  → User reviews flagged tracks, selects a row → left panel populates
  → User edits a field → auto-saves to file → updates undo stack
  → User can Ctrl+Z to revert last operation
```

### Undo System

```python
@dataclass
class UndoOperation:
    description: str  # "Auto-Tag: Abbey Road" or "Edit Title: Come Together"
    timestamp: float
    snapshots: list[TrackSnapshot]  # one per affected track

@dataclass
class TrackSnapshot:
    path: Path
    metadata: TrackMetadata  # complete pre-operation state
```

- Stack of up to 50 operations
- Ctrl+Z pops the top, restores all snapshots to disk
- Snapshot = full TrackMetadata for each affected track
- Batch operations (auto-tag) snapshot all tracks in the album
- Manual edits snapshot just that one track

### Subprocess Protocol (New CLI Commands)

**`auto-tag batch --json-stream`** (new flag):
- Outputs one JSON line per album processed
- Each line: `{"type": "album_result", "path": "...", "status": "ok|error|skipped", "tracks": [...], "changes": [...]}`
- Final line: `{"type": "summary", "processed": N, "failed": N}`
- UI reads lines as they arrive, updates table incrementally

**`auto-tag audit <path> --json-stream`** (new subcommand):
- Reads album metadata from files (no network except LLM)
- Outputs one JSON line per album audited
- Each line: `{"type": "audit_result", "path": "...", "tracks": [{"index": N, "field": "...", "status": "correct|warning|error", "message": "...", "suggestion": "..."}]}`
- UI applies audit results to track highlighting

---

## Implementation Waves

### Wave 7.1: Foundation (6-8 hours)

**Task 7.1.1**: Add `textual` dependency + scaffold UI package
- `pip install textual`
- Create `ui/__init__.py`, `ui/app.py`
- Register `auto-tag ui` CLI command
- Verify `auto-tag ui` launches an empty Textual app

**Task 7.1.2**: Implement state model
- `state.py`: `AppState` dataclass with:
  - `library_path: Path | None`
  - `tracks: dict[Path, TrackMetadata]` — all loaded tracks
  - `pre_auto_tag_state: dict[Path, TrackMetadata]` — before last auto-tag
  - `selected_paths: set[Path]` — currently selected rows
  - `loaded: bool`
  - `auto_tagging: bool`
  - `auditing: bool`
  - `audit_results: dict[Path, TrackAuditResult]`
  - `undo_stack: list[UndoOperation]`
- `TrackAuditResult`: per-track audit outcome

**Task 7.1.3**: Implement undo system
- `undo.py`: `UndoManager` class
  - `push(description, snapshots)`
  - `undo() -> UndoOperation | None`
  - `can_undo: bool`
  - Max 50 entries
- Integration with auto-save and auto-tag

### Wave 7.2: Layout Shell (6-8 hours)

**Task 7.2.1**: Main layout screen
- `main_screen.py`: Full-screen app with:
  - `Header` (toolbar) — dock: top
  - `TagPanel` (left) — dock: left, width: 35
  - `TrackTable` (right) — dock: right, rest of space
  - `StatusBar` (bottom + filter) — dock: bottom, height: 3

**Task 7.2.2**: Toolbar widget
- `toolbar.py`: `Horizontal` with `Button` widgets
  - Open Library (`Button("📂 Open", id="open")`)
  - Auto-Tag (`Button("▶ Auto-Tag", id="auto_tag", variant="primary")`)
  - Stop (`Button("⏹ Stop", id="stop", disabled=True)`)
  - Undo (`Button("↩ Undo", id="undo", disabled=True)`)
  - Filter button (`Button("🔍", id="filter")`)
  - Separator
  - Disk usage / library size indicator
  - Settings button ("⚙")

**Task 7.2.3**: Status bar + filter
- `status_bar.py`: Bottom bar with two sections:
  - Left: `Label` showing "42 files | 2h 23m | 3 selected"
  - Right: `FilterInput` extending full remaining width
- `filter_input.py`: `Input` with `on_change` handler
  - Filters track table by text match across all visible columns
  - Supports regex: `%_extension% IS flac` or `%artist% HAS "Beatles"`

### Wave 7.3: Track Table (6-8 hours)

**Task 7.3.1**: DataTable setup
- `track_table.py`: `DataTable` subclass
  - Static columns (always shown): `#`, `Filename`, `Title`, `Artist`, `Album Artist`, `Album`, `Year`, `Track`, `Disc`, `Status`
  - Optional columns (toggle via right-click header): `Path`, `Extension`, `Size`, `Genre`, `ARTISTS`, `Composer`, `Bitrate`, `Codec`, `Cover`, `MusicBrainz IDs`, `ReplayGain`, `Sample Rate`
  - Default visible optional columns: `Genre`, `Cover`, `Bitrate`, `Codec`

**Task 7.3.2**: Populate from library
- On "Open Library": scan directory tree
  - Walk folders recursively (respect `recursive`, `recursive_depth`, exclude patterns)
  - Group files by album directory → `list[Album]`
  - For each album: read tags from first file to get album-level metadata
  - Populate table rows with per-track data
  - Do NOT read all files' tags upfront — lazy: read tags only when an album row is expanded or clicked
  - Show album rows as collapsible groups in the table (or use a Tree widget + table combination)

Wait — after reconsidering the design, a flat table of all tracks across the library would be too large. Let me reconsider the table structure:

**Revised Track Table Approach**:
- Two modes:
  1. **Album view** (default): Table shows one row per **album**, columns: `Artist`, `Album`, `Year`, `Track Count`, `Status`, `Cover`
  2. **Track view** (click an album): Table shows individual tracks in that album
  
- Or simpler: show the library as a **Tree** on the left (above the tag panel or in a separate toggle pane):
  - Artist folders
  - Album sub-folders
  - Click album → right table populates with that album's tracks

**Simpler approach for Wave 7.3**: Since the left panel is already the tag editor, make the right table per-album:
- The library is loaded as an **album list** (a DataTable with album rows)
- Click an album → the table switches to show that album's tracks (breadcrumb at top)
- Or: use a separate sidebar for the album tree, keep the table flat

Actually, re-reading the MP3Tag layout agreed earlier: the left panel is the tag editor + cover, the right panel is the track table. But MP3Tag operates on a single directory at a time, not a full library. For auto-tagger, the user wants to browse the full library.

**Final decision for Wave 7.3**: 
- **Album tree** as a collapsible vertical panel on the far left (or integrated into the left tag panel as a collapsible section at top)
- Or: top section of the app is an album breadcrumb + album selector
- The **right table** shows tracks for the currently selected album only
- This keeps the table manageable and the tag panel relevant

Let me redesign the layout slightly:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  [Open] [Auto-Tag] [Stop] [Undo] [Filter]           [Settings] [ⓘ]        │
├──────────┬───────────────────┬──────────────────────────────────────────────┤
│  LIBRARY │                   │  # │ Filename   │ Title    │ Artist  │Status │
│  TREE    │ TAG EDITOR        │  1 │ 01 Come... │ Come To… │ Beatles │ ✅    │
│          │                   │  2 │ 02 Some... │ Somet…   │ Beatles │ ✅    │
│  ├── Beatles   │  Title  [   ]│  3 │ 03 Maxwe…  │ Maxwel…  │ Beatles │ ⚠️    │
│  │ ├── Abbey.. │  Artist [   ]│                                            │
│  │ ├── Revol.. │  ARTISTS[   ]│                                            │
│  │ └── Sgt. P..│  Album  [   ]│                                            │
│  ├── Miles D.  │  ...         │                                            │
│  │ ├── Kind..  │              │                                            │
│  │ └── Bitche..│  ┌────────┐ │                                            │
│  └── Radiohead │  │ Cover  │ │                                            │
│     ├── OK Co..│  │ Art    │ │                                            │
│     └── In Ra..│  └────────┘ │                                            │
├──────────┴───────────────────┴──────────────────────────────────────────────┤
│  Filter: [____________________________________________]  3 files │ 12:34  │
└─────────────────────────────────────────────────────────────────────────────┘
```

Wait — this is starting to get complex. Let me simplify. The user specifically said the left panel is the tag editor + cover, and the right panel is the track table. Let me keep it at that and handle the library browsing differently:

**Simplified: Two screens**
1. **Album Browser screen** (on launch): Shows a scrollable list of all albums in the library with their status — click one to open
2. **Track Editor screen** (on album click): Shows the tag panel (left) + track table (right) for that album

This is cleaner and avoids the three-panel cram. Let me proceed with this for the plan.

**Task 7.3.4 (revised)**: Album Browser screen
- `DataTable` with columns: `Artist`, `Album`, `Year`, `Tracks`, `Status`, `Cover`
- Sortable by any column
- Status indicator per album (aggregate of all tracks)
- Click row → navigate to Track Editor screen

**Task 7.3.5**: Track Editor screen
- Left: tag panel (form fields + cover)
- Right: track table for this album
- Top: breadcrumb showing "Library > Beatles > Abbey Road"

### Wave 7.4: Tag Panel (4-6 hours)

**Task 7.4.1**: Form field widget
- `tag_panel.py`: `Vertical` container with:
  - Scrollable list of labeled `Input` widgets
  - Fields: Title, Artist, ARTISTS, Album, Album Artist, Year, Track, Disc, Genre, Composer, Comment
  - Multi-value field (ARTISTS): comma-separated `Input`, split on save
  - Track/Disc: numeric `Input` with validator
  - When no track selected: all fields grayed out with "Select a track" message
  - When multiple tracks selected: fields show value, or `<keep>` if diverging

**Task 7.4.2**: Cover art viewer
- `cover_viewer.py`: Fixed-size `Static` widget showing cover art
  - Uses rich `Renderable` (or Textual's built-in image support)
  - Right-click → context menu: "Embed from file...", "Extract to folder", "Remove", "Auto-fetch from Cover Art Archive"
  - Auto-fetch invokes auto-tagger's cover art flow via subprocess
  - Shows source indicator: "External: cover.jpg" or "Embedded" or "Missing"

**Task 7.4.3**: Auto-save + validation
- On `Input.Changed` → validate field:
  - Track/Disc: must be positive integer or empty
  - Year: 4-digit number or empty
  - All fields: non-empty for required fields
  - If valid: write tag to file immediately, update undo stack
  - If invalid: show red border on field, don't save

### Wave 7.5: Auto-Tag Integration (4-6 hours)

**Task 7.5.1**: Subprocess launcher
- `workflow.py`: `AsyncSubprocess` class
  - `run_auto_tag(library_path)`: launches `auto-tag batch <path> --json-stream`
  - `run_audit(library_path)`: launches `auto-tag audit <path> --json-stream`
  - `run_album_auto_tag(album_path)`: launches `auto-tag tag <path> --yolo --json`
  - `run_cover_fetch(album_path)`: launches appropriate cover-art subprocess
  - Parse stdout lines as JSON, yield events to the UI
  - Handle: subprocess cancellation (Stop button), timeout, error

**Task 7.5.2**: JSON stream protocol
- Define the JSON line schema (shared between CLI and UI)
- Album result line: `{"type": "album", "path": "...", "tracks": [...]}`
- Audit result line: `{"type": "audit", "path": "...", "tracks": [...]}`
- Progress line: `{"type": "progress", "current": 5, "total": 42}`
- Error line: `{"type": "error", "path": "...", "message": "..."}`

**Task 7.5.3**: UI integration
- On "Auto-Tag" click:
  1. Save pre-state snapshots for all albums (undo)
  2. Disable toolbar buttons (except Stop)
  3. Navigate to Album Browser screen if not already there
  4. Launch subprocess, listen for events
  5. For each `album` event: update that album's status row in the browser, highlight changed fields
  6. When done: automatically trigger audit
  7. For each `audit` event: update album/track status with audit results
  8. Re-enable toolbar, show summary in status bar (e.g., "Auto-tag complete: 38 albums, 3 errors")

**Task 7.5.4**: Stop button
- Kill subprocess gracefully (SIGTERM, then SIGKILL after 5s)
- Restore UI state, show partial results
- Note in status bar: "Auto-tag interrupted — partial results"

### Wave 7.6: LLM Audit (3-4 hours)

**Task 7.6.1**: New CLI subcommand `auto-tag audit`
- Reads album metadata from files (no remote lookups, just tag state)
- Batches all tracks per album into structured JSON
- Calls LLM with positive/negative examples
- Outputs per-track issues as JSON stream
- New prompt/schema in `llm/`:
  - `prompts.py`: `build_audit_messages(request)` — album context + tracks + examples
  - `schemas.py`: `AuditResponse(BaseModel)` — list of per-track issues

**Task 7.6.2**: Auto-run after auto-tag
- In Wave 7.5.3, the audit automatically follows auto-tag completion
- Runs with `parallel=1` (rate limit)
- Progress shown in status bar: "Auditing album 5/42..."
- Results immediately update table highlighting

### Wave 7.7: Settings + Polish (3-4 hours)

**Task 7.7.1**: Settings screen
- `settings_screen.py`: Modal overlay
  - Editable fields: Parallel jobs, Auto-audit toggle, LLM model, Output format
  - Library path selector (file browser)
  - Save to config

**Task 7.7.2**: Keyboard shortcuts
- Ctrl+O: Open library
- Ctrl+T: Auto-Tag
- Ctrl+Z: Undo
- Ctrl+F: Focus filter bar
- Ctrl+S: Force save (currently redundant but familiar)
- Escape: Close settings / go back
- Tab / Shift+Tab: Navigate between fields
- Arrow keys: Navigate track table
- Enter: Open selected album (in browser) / edit selected cell (in track view)

**Task 7.7.3**: Performance optimizations
- Lazy tag reading: only read tags when album is selected
- Cache read tags in memory (no re-read unless Refresh)
- Limit initial scan to `os.scandir` + audio file count per album
- Debounce filter input (150ms)

---

## Dependencies

### New Python dependency
- `textual>=1.0.0` (add to `pyproject.toml` optional deps `ui = ["textual"]`)

### New CLI commands (in auto-tagger)
- `auto-tag ui [PATH]` — launch the TUI
- `auto-tag batch --json-stream` — incremental JSON output mode for batch
- `auto-tag audit [PATH]` — LLM-based metadata audit subcommand

---

## Test Strategy

- **Unit tests**: Isolate undo logic, state management, subprocess protocol parsing
- **Integration tests**: Mock subprocess output to test UI event handling
- **No visual/end-to-end tests** for the initial TUI (Textual has a testing framework, but manual verification is adequate for Wave 7)
- **Snapshot tests**: Verify JSON stream protocol schema

---

## Success Criteria

1. `auto-tag ui /path/to/library` opens and shows the album browser
2. Click an album → track table + tag panel populate
3. Auto-Tag button processes the library, results stream into the table with green highlights for changed fields
4. Audit auto-runs after auto-tag, yellow highlights for suspicious tracks
5. Click a flagged track → tag panel shows its metadata
6. Edit a field → auto-saves to file
7. Ctrl+Z → reverts last edit
8. Stop button cancels a running auto-tag
9. Filter bar filters tracks in real-time
10. Right-click cover art → add/extract/remove/auto-fetch
11. No crashes on large libraries (1000+ albums)

---

## Progress

### Wave 7.1 — Foundation ✅
- `textual>=1.0.0` optional dep in `pyproject.toml` under `ui` extras
- `ui/__init__.py`, `app.py`, `state.py`, `undo.py` scaffolds
- `auto-tag ui [PATH]` CLI command registered
- 15 tests for AppState, AlbumData, TrackAuditResult, UndoManager

### Wave 7.2 — Layout Shell ✅
- MainScreen with Toolbar (dock:top), StatusBar (dock:bottom), content-area (horizontal)
- TagPanel left (38 cols) + TrackTable right (1fr)
- DirectoryBrowser modal screen
- CSS fixes: `layout: vertical`, `align: left middle`, no Python theme hacks

### Wave 7.3 — Track Table ✅
- Column sorting, toggling, row highlighting/selection, multi-select via Ctrl+click
- Breadcrumb bar, lazy loading, dual-mode (album browser + per-album track list)
- Tested: sorting, toggling, multi-select, row selection, initial view

### Wave 7.4 — Tag Panel ✅
- 13 form fields with validation (year 4-digit, track/disc ≥1, required fields)
- Multi-track `<keep>` system for divergent values
- Cover art detection (embedded + external) in lazy loading
- Cover art right-click context menu stubs
- Auto-save with `dataclasses.replace()`, _populating guard, undo integration
- 16 tests for validation + integration

### Wave 7.5 — Auto-Tag Integration ✅
- `--json-stream` flag added to `auto-tag batch` CLI command
- `_execute_json_stream()` emits per-album, progress, and summary JSON lines
- JSON stream protocol: `album`, `progress`, `summary` event types with schema
- `workflow.py` subprocess launcher with cancellation via `cancel_running()`
- `MainScreen.start_auto_tag()` creates asyncio task; `stop_auto_tag()` terminates subprocess
- Toolbar Stop button wired up, disabled when idle, auto-tag/undo disabled during run
- Status bar shows "Auto-tagging..." and "Auditing..." states
- `_handle_event()` and `_handle_audit_event()` process JSON events into state updates
- Snapshot-based undo support (full TrackMetadata per track)
- 16 tests for JSON stream protocol, schema validation, event handling, toolbar state

### Wave 7.6 — LLM Audit ✅
- New `AuditTrackResult` + `AuditResponse` schemas in `llm/schemas.py`
- New `build_audit_messages()` prompt builder in `llm/prompts.py` with positive/negative examples
- New `auto-tag audit <PATH> [--json-stream]` CLI command in `cli.py` + `commands/audit.py`
  - Discovers albums, reads tags, batches per album, calls LLM via `OpenRouterClient`
  - Emits `progress`, `audit`, and `summary` JSON stream events
  - Filters out `correct` results, only outputs `warning`/`error`
- `audit` command auto-runs after auto-tag completes (wired in `workflow.py`)
- `_handle_audit_event()` processes audit results into `AlbumData.audit_results` and `TrackData.status`
- 14 tests for schemas, prompts, command helpers, event handler

### Wave 7.7 — Settings + Polish ✅
**Settings Screen** (`screens/settings_screen.py`):
- Modal overlay with checkbox (auto-audit toggle), input (LLM model), select (output format)
- Cancel/Save buttons, Save returns dict of changed values → applied to `AppState`
- Wired to toolbar Settings button and Ctrl+S keyboard shortcut

**Keyboard Shortcuts** (`app.py`):
- Ctrl+O: Open library, Ctrl+T: Auto-Tag, Ctrl+Z: Undo, Ctrl+F: Filter, Ctrl+S: Settings
- Escape: Go back, Enter: Select current row, Ctrl+Q: Quit
- `action_select_row()` triggers `DataTable.RowSelected` on focused table
- `action_settings()` opens settings modal via screen handler

**Filter Debounce** (`widgets/status_bar.py`):
- `on_input_changed` sets a `set_timer(150ms)` instead of applying immediately
- Timer resets on each keystroke (only fires 150ms after last keystroke)
- Prevents expensive filter re-computation on every character

**Library Scan Optimization** (`screens/main_screen.py`):
- Replaced `path.rglob("*")` with `os.scandir` + manual queue-based recursive walk
- Safety limit of 100,000 entries to prevent runaway scans
- `PermissionError` silently skipped per-directory
- Reduces startup scan from O(n²) stat overhead to iterative directory reads

**AppState Extensions**:
- `parallel_jobs`, `llm_model`, `output_format` — UI-relevant settings
- `recent_workspaces: list[Path]` — max 5, no duplicates, added on library load

**19 tests** for settings screen, keyboard bindings, state extensions, debounce, library scan, recent workspaces

| Risk | Mitigation |
|---|---|
| Textual DataTable slow with 10,000+ tracks | Only show per-album tracks (typically 8-20). Album browser paginated or scrollable. |
| Subprocess protocol fragile (CLI + UI coupling) | Define JSON schema explicitly, version it, validate both sides |
| Auto-save on edit could corrupt files on crash | Mutagen saves are atomic (write temp, rename). Undo stack in memory only — no disk persistence for v1. |
| LLM audit cost surprise | Show estimated cost before batch audit. Hard-coded `parallel=1`, respect rate limits. |
| Terminal size too small | Set minimum 100x30; show "Resize terminal" message below minimum. |
