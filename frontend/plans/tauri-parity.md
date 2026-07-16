# Tauri Migration Parity Inventory

Source of truth for the Electron→Tauri cutover. Every row must be green
(Rust command + adapter implemented, parity test passing, or explicitly blocked)
before `electron/` is removed. Generated from `electron/preload.ts`,
`electron/main.ts`, the per-file `register*` handlers, the test corpus, and
`electron-builder.yml`.

## Migration status

| Slice | Status | Evidence |
|---|---|---|
| Step 1 — characterization + dual-shell scaffold | ✅ green | shared `DesktopAPI` contract extracted; Tauri crate/adapter/loader + contract tests (60 shared tests); cargo fmt/clippy/test + electron typecheck/vitest/build green |
| Window state persistence + off-screen recovery | ⏳ logic implemented, GUI parity pending | `state/window_state.rs` (11 unit tests encoding Electron intent, incl. missing-axis LeaveUnspecified); wired in `run()`. PENDING: `ready-to-show` show timing (Tauri equivalent needs `unstable` feature) — verified only under a display session; adapter command-name normalization regressions fixed |
| Config (`config:get`/`config:set`) | ✅ green (vertical slice) | `state/config.rs` `ConfigState` (init/refresh/set/redacted) + wired sync `config_get`/`config_set` commands. 18 config unit tests incl. lifecycle, poisoned-lock no-panic fallback (`config_get` → `{}`, raw → default; live state remains unchanged until restart), and normalized Electron-vs-Rust redaction fixture (Rust + Vitest pass the same expected JSON). DEFERRED: `config:set` assistant sync (`setStoredConfig` for llmApiKey/llmModel) wired when the assistant slice lands. |
| Native shell: folder dialog / context menu / focus / quit guard | ⏳ wired; GUI validation pending | Folder/context/focus commands are wired with E2E overrides. Quit guard observes managed queued+running writes on `RunEvent::ExitRequested`, prevents exit, coalesces duplicate requests, shows a nonblocking warning, and uses a force flag before re-requesting exit; 2 state-machine tests cover cancel/re-prompt, coalescing, and force bypass. PENDING: real-display folder/popup/modal timing, default-button behavior, and cancellation-vs-native-dialog-error distinction. |
| Persistence (config/cache/dataset/aliases/logs) | ⏳ pending | |
| Read-only library slice — traversal/grouping/read core | ⏳ partial | `library_scan` has a shared fixture. Internal `TrackData` reader uses Lofty 0.24 plus bounded MPEG/OGG/FLAC/APE and MP4/Opus/AIFF property parsers. Eleven-file differential corpus is green: rich MP3 ID3, FLAC/WAV/OGG/APE, malformed inputs, M4A/MP4, true Opus, and AIFF. Fractional Electron bitrate is preserved (`f64`); MP4 `mdhd`/`stsz`, Opus granule+11.9 `lastPos` quirk, and AIFF COMM/SSND calculations match exact DTO values. `directory:read`, `album:read`, `album:refresh` are wired. PENDING: `directory_list` ICU collation and cover data URLs/provider discovery. |
| Mutation + media-safety slice | ⏳ ordinary writes + batch green; FLAC repair edges pending | `track_write` supports MP3, FLAC, OGG, Opus, M4A, MP4, WAV, and APE through one queue; AIFF rejects identically. `tracks_batch_write` holds that queue for the full sequential batch, returns void, supports empty batches, and stops on first error after preserving prior commits (Electron is non-transactional). Electron exposes no batch progress/cancellation event, so none is invented. FLAC repair edges are green on shared deterministic fixtures: trailing APE is bounded/removed; appended ghost VC only has its guarded vendor-length word zeroed; duplicate metadata VC collapses to one at the same audio boundary; absent VC is created; and 8-byte padding growth falls back to full rewrite. Every path preserves the exact prepared audio payload and validates readback before atomic replacement. Extra tags are green end-to-end. `track_extra_tags_read` covers ID3v2, Vorbis, and raw APEv2 with editor filtering/provider canonicalization/native dedup and graceful `[]`. `track_extra_tags_write` replaces editable extras while preserving reserved standard tags and exact MP3/WAV/FLAC/OGG/Opus/APE audio boundaries; it supports repeated values, COMMENT, ARTISTS, provider aliases, clearing, malformed safety, and unsupported rejection. `tracks_batch_write_extra_tags` skips unsupported formats, holds one queue lease, attempts all supported files, aggregates failures, and returns every original track with unreadable fallback. `track_rename` is green: one WriteQueue lease excludes concurrent media writes, target parents are created recursively, platform `rename` collision and normal `..` resolution semantics match Electron, readback returns the supplied new path, and failed rename leaves source bytes intact. The core track/extra-tag/rename mutation rows are complete. |
| Providers/auto-tag/audit/assistant | ⏳ partial | Audit is wired end-to-end: deterministic checks, targeted OpenRouter review, 0.92 approval threshold, approval-only writes, two-album concurrency, specified-track grouping, cancellation/cleanup, and the Electron event/summary contract. Chinese alias preflight reuses pure-Rust OpenCC-aware Discogs validation, tries MusicBrainz aliases first, validates any LLM suggestion against Discogs, and persists only confirmed suggestions. Shared OpenRouter preserves schema/auth/deadline/retry/repair behavior. Auto-tag and assistant send/apply remain pending. |
| E2E→WebdriverIO + CI + cutover | ⏳ pending | |

> GUI runtime behavior (window placement, quit-guard dialog, WebdriverIO E2E on
> macOS/Windows/Linux, installer smoke) requires a display session and CI
> runners this sandbox lacks; pure logic is unit-tested and compiled, and those
> runtime gates are recorded as **environment-blocked** rather than skipped silently.

Legend:
- **Ch** = Electron IPC channel (request/response via `ipcMain.handle`, unless
  marked `evt` for pushed events).
- **Renderer method** = the `window.api.*` call in `preload.ts`.
- **Owner** = Electron file registering the handler.
- **Parity tests** = existing tests that encode the current intent and that the
  Rust port must keep green (or be replaced by an equivalent Rust test).

## A. Request/response IPC handlers (51)

### Library & directories

| Ch                     | Renderer method   | Owner            | Parity tests                              | Notes |
|------------------------|-------------------|------------------|-------------------------------------------|-------|
| `library:scan`         | `scanLibrary`     | library.ts       | `library.test.ts`, `LibraryService.test.ts` | pure-fs grouping ported; shared Electron/Rust fixture validates normalized response |
| `album:refresh`        | `refreshAlbum`    | library.ts       | `library.test.ts`                         | wired delegation to album:read; same local cover/status/fallback behavior |
| `directory:list`       | `listDirectory`   | directory.ts     | `directory.test.ts`                       | logic ported; `localeCompare` collation parity pending (case/accents/CJK) |
| `directory:read`       | `readDirectory`   | directory.ts     | `directory.test.ts`                       | wired: subdirs + sorted direct tracks + audioCount; per-file malformed fallback keeps basename/real size |

### Tracks (metadata read/write/rename/extra-tags/delete)

| Ch                        | Renderer method          | Owner      | Parity tests                                                            | Notes |
|---------------------------|--------------------------|------------|-------------------------------------------------------------------------|-------|
| `album:read`              | `readAlbum`              | tracks.ts  | `tracks.test.ts`                                                        | wired AlbumDetail: sorted direct tracks, parent/dir hints, status, local external/embedded/missing cover state |
| `track:write`             | `writeTrack`            | tracks.ts  | `writer.test.ts`, `writer-discogs-ids.test.ts`                         | single-file queue write + readback |
| `tracks:batch-write`      | `writeTracks`           | tracks.ts  | `writer.test.ts`, `TagWriteQueue.test.ts`                              | batch lock, serialization, fail-loud |
| `track:extra-tags:read`   | `readExtraTags`         | tracks.ts  | `tracks.test.ts`, `ExtraTagService.test.ts`                            | limited-format support |
| `track:extra-tags:write`  | `writeExtraTags`        | tracks.ts  | `tracks.test.ts`, `ExtraTagService.test.ts`                            | queue one |
| `tracks:batch-write-extra-tags` | `writeExtraTagsBatch` | tracks.ts | `tracks.test.ts`, `ExtraTagService.test.ts`                         | skip unsupported formats, warn |
| `track:rename`            | `renameTrack`           | tracks.ts  | `tracks.test.ts`                                                        | mkdir dest dir, rename, readback |
| `file:exists`             | `checkFileExists`       | tracks.ts  | Rust filesystem tests + adapter contract | wired: raw path existence for files/directories; missing false |
| `track:context-menu`      | `showTrackContextMenu`  | main.ts    | Rust shell unit tests + E2E override | wired native menu + clipboard; E2E action override, exact Copy All payload, single-popup action state. GUI popup/dismissal timing pending display smoke. |
| `track:delete-files`      | `deleteFiles`          | main.ts    | Rust filesystem tests + adapter contract | wired: one queue lease, ordered per-file success/error, continues after missing/duplicate/directory failures; catastrophic task failure becomes per-file errors |

### Covers

| Ch                          | Renderer method        | Owner     | Parity tests                                  | Notes |
|-----------------------------|------------------------|-----------|-----------------------------------------------|-------|
| `cover:data-url`            | `getCoverDataUrl`     | cover.ts  | Rust cover tests + `cover.test.ts` | wired: suppression → ordered external names → embedded artwork; decode/resize JPEG data URL, malformed/missing returns null |
| `cover:set`                 | `setCover`            | cover.ts  | Rust cover tests + `cover.test.ts` | wired: native image picker, max-500 JPEG quality 90 `cover.jpg`, clears suppression, returns quality-85 data URL; real-display cancel/error distinction pending |
| `cover:remove`              | `removeCover`         | cover.ts  | Rust cover tests | wired: remove first ordered external cover, write `.auto-tagger-cover-removed`, hide remaining external/embedded sources; missing/write failure returns false |
| `cover:download`            | `downloadCoverArt`    | cover.ts  | Rust routed-provider/local-write tests + `cover.test.ts`, `cover-download.test.ts` | ✅ metadata prerequisite; local→CAA→Discogs direct/artist/validated-search→TheAudioDB, per-provider invalid-image fallthrough, CAA transport retry once, double 1000px quality-90 JPEG normalization, queued `cover.jpg`, suppression clear, data URL |
| `cover:download-artist-art` | `downloadArtistArt`  | cover.ts  | Rust routed-provider/local-write tests + `cover-download.test.ts`, `ArtworkResolverService.test.ts` | ✅ metadata/artist prerequisite; local→Discogs direct/exact/MB-alias identity→Wikidata P18/Wikimedia, retry policy, queued parent `artist.jpg`, `{path,source}`, album suppression unchanged |

### Lyrics

| Ch            | Renderer method | Owner   | Parity tests                 | Notes |
|---------------|-----------------|---------|------------------------------|-------|
| `lyrics:fetch` | `fetchLyrics`   | main.ts | Rust local-HTTP tests + `lyrics.test.ts` | wired LRCLIB RustLS client: required names, URL encoding, optional album, rounded positive duration, 10s timeout/user-agent, synced→plain preference, null for non-OK/network/JSON/empty/instrumental. Encoding/local-file work belongs album-download slice. |
| `album:download-lyrics` | `downloadAlbumLyrics` | main.ts | Rust encoding/batch tests + `lyrics.test.ts` | wired sorted direct audio files: `.lrc` then `.txt` local priority with UTF-16 BOM/chardetng+encoding_rs (Shift-JIS/Big5 tested), sequential API fallback using metadata, one atomic WriteQueue batch, per-track failure continuation, success count, outer failures→0 |

### Configuration & dataset

| Ch               | Renderer method     | Owner    | Parity tests        | Notes |
|------------------|---------------------|----------|---------------------|-------|
| `config:get`     | `getConfig`        | main.ts  | `config.test.ts`    | **redacted**: keys show `****`+last4 or null |
| `config:set`     | `setConfig`        | main.ts  | `config.test.ts`    | flat YAML, comments/unknown keys preserved, env precedence, key map |
| `dataset:status` | `getDatasetStatus` | main.ts  | Rust SQLite tests + `auto-tag.test.ts` | wired configured/default read-only SQLite status: exact available/musicbrainz/totalRecords/lastUpdated shape; missing/corrupt/wrong schema degrades unavailable. Shared DTO stale `spotify` corrected to runtime `available`. |

### Tasks / auto-tag manager

| Ch              | Renderer method    | Owner    | Parity tests        | Notes |
|-----------------|--------------------|----------|---------------------|-------|
| `album:auto-tag` | `autoTagAlbum`    | main.ts  | `auto-tag.test.ts`, `auto-tag-chinese.test.ts`, `candidates.test.ts`, `fallback.test.ts`, `musicbrainz.test.ts`, `discogs.test.ts` | returns taskId; pipeline folder hints→MusicBrainz→Discogs→LLM |
| `task:progress`  | `getTaskProgress` | main.ts  | Rust task registry + `auto-tag.test.ts` | wired nullable unknown lookup and exact running/completed/failed/cancelled progress/total/message/result DTO; shared API corrected nullable |
| `task:cancel`    | `cancelTask`      | main.ts  | Rust task registry + `auto-tag.test.ts` | wired no-op unknown cancellation, status/message update, shared atomic token; task creation/events attach in auto-tag/audit slices |

### Audit

| Ch                       | Renderer method      | Owner    | Parity tests                                          | Notes |
|--------------------------|----------------------|----------|-------------------------------------------------------|-------|
| `audit:run`              | `runAudit`          | audit.ts | Rust audit/provider/OpenRouter contracts + frontend suite | ✅ discovery, configured semantic review, two-worker pool, events, cancellation, summary |
| `audit:run-specified`     | `runAuditOnTracks` / `runAuditOnAlbums` | audit.ts | Rust grouping/pool contracts + frontend suite | ✅ track-parent first-seen grouping, album precedence, exact empty-input error, shared run lifecycle |
| `audit:run-album`         | `runAlbumAudit`     | audit.ts | Rust audit/provider/OpenRouter contracts + frontend suite | ✅ read-only deterministic + configured semantic review with validated Discogs alias context |
| `audit:apply-fixes`       | `applyAuditFixes`   | audit.ts | Rust audit/write tests + `audit.test.ts`             | ✅ registered; only eligible plans, merged per-file jobs, explicit-null clearing, omitted-field preservation, queue serialization, atomic writer validation, per-file success marking, and continuation after a failed file |
| `audit:cancel`            | `cancelAudit`       | audit.ts | Rust current-token tests + `audit.test.ts` | ✅ no-op without run; atomically aborts and clears only current token; runner owns cancelled event/zero-summary emission |

### Assistant (LLM)

| Ch                        | Renderer method          | Owner        | Parity tests                                                                 | Notes |
|---------------------------|--------------------------|--------------|------------------------------------------------------------------------------|-------|
| `assistant:send`           | `assistantSend`         | assistant.ts | `assistant.test.ts`, `AssistantRuntime.test.ts`, `assistant-paths.test.ts`   | routing + tools + previews |
| `assistant:cancel`         | `assistantCancel`        | assistant.ts | Rust runtime/conversation tests + `assistant.test.ts` | ✅ no-op before runtime; sets cancellation, persists system entry, emits exact cancelled event |
| `assistant:clear`         | `assistantClear`         | assistant.ts | Rust runtime/conversation tests + `assistant.test.ts` | ✅ no-op before runtime; resets cancellation and creates new `session-{epoch}-{base36}` ID/number; pending batch map preserved like Electron |
| `assistant:apply-actions`  | `assistantApplyActions` | assistant.ts | `assistant.test.ts`, `assistant-folder-group.integration.test.ts`, `assistant-organize-files.integration.test.ts` | returns undo snapshots + optional task trigger |
| `assistant:reject-actions` | `assistantRejectActions`| assistant.ts | Rust batch-transition tests + `assistant.test.ts` | ✅ no-op missing/no-runtime; pending→rejected and exact event payload |
| `assistant:get-batches`    | `assistantGetBatches`   | assistant.ts | Rust batch-transition tests + `assistant.test.ts` | ✅ [] before runtime; insertion-order pending-only DTOs |
| `assistant:init-runtime`    | `assistantInitRuntime`  | assistant.ts | Rust session/schema tests; LLM runtime tests pending | 🟡 initializes idempotent UUID/session-number + configured/default compatible cache.db; LLM runner/tool registry enhancement remains pending |
| `assistant:init-services`   | `assistantInitServices` | assistant.ts | Rust managed-state tests + `assistant.test.ts` | ✅ stores unredacted apiKey/model, recreates optional Discogs/lyrics/library service configuration on every call, preserves prior key/model for empty/redacted values, rejects unavailable state |
| `assistant:list-sessions`   | `listSessions`          | assistant.ts | Rust existing-Electron-schema query tests + `conversation-logger.test.ts` | ✅ runtime-gated; default/explicit limit, newest first, first user message truncated to 200 characters, API entry count/cost |
| `assistant:get-conversation`| `getConversation`       | assistant.ts | Rust existing-Electron-schema query tests + `conversation-logger.test.ts` | ✅ UUID or session number, ascending ID, exact nullable/token/cost fields; unavailable/error→[] |
| `assistant:get-session`     | `getSession`            | assistant.ts | Rust existing-Electron-schema query tests + `conversation-logger.test.ts` | ✅ UUID/number resolution through latest 1000 summaries; unavailable/error→null |
| `assistant:current-session` | `getCurrentSession`     | assistant.ts | Rust lifecycle tests + `conversation-logger.test.ts` | ✅ null before init; stable `{sessionId, sessionNumber}` after idempotent init |

### Organizer

| Ch                    | Renderer method | Owner        | Parity tests                                    | Notes |
|-----------------------|------------------|--------------|-------------------------------------------------|-------|
| `files:sort-by-album` | `sortByAlbum`   | organizer.ts | Rust organizer tests + `FolderOrganizerService.test.ts` | wired under one queue lease: recursive sorted audio collection, metadata/Unknown Album grouping, exact cross-platform sanitization, copy default or move, per-file failure/skip accounting |

### Window / dialog / debug

| Ch                    | Renderer method     | Owner    | Parity tests   | Notes |
|-----------------------|---------------------|----------|----------------|-------|
| `window:focused`      | `onFocus`          | main.ts  | shell unit suite | wired no-op hook (matches Electron; no main-process state change) |
| `dialog:open-folder`  | `openFolderDialog` | main.ts  | (E2E)          | wired; selected/null parity only — plugin GUI-error rejection remains pending display validation |
| `debug:subscribe`     | `subscribeDebugLogs`| debug.ts | Rust debug tests + adapter listener tests | wired `{subscribed:true}`; loader listener receives Tauri `debug:log` events |
| `debug:set-mode`      | `setDebugMode`     | main.ts  | Rust debug/config tests | wired managed toggle + config persistence + daily JSONL truncate-once + enable event; automatic generic tracing-layer forwarding remains pending |
| `debug:status`        | (internal/test)    | debug.ts | Rust debug tests | wired enabled/logFile/forwardedCount snapshot |
| `debug:toggle`        | (internal/test)    | debug.ts | Rust debug tests | wired non-persistent toggle, matching internal Electron distinction from set-mode |

**Subtotal:** 51 `ipcMain.handle` channels. 49 are surfaced through `window.api`
in `preload.ts`; `debug:status` and `debug:toggle` are main/internal-only but
must still be ported for test parity.

## B. Pushed event streams (4)

| Channel          | Renderer listener | Emit mechanism                                            | Payload type     | Parity tests                        |
|------------------|-------------------|-----------------------------------------------------------|------------------|-------------------------------------|
| `auto-tag:event` | `onAutoTagEvent`  | `forwardToWindows(onAutoTagEvent, …)` → all windows `.send` | `AutoTagEvent`   | `auto-tag.test.ts`                  |
| `audit:event`    | `onAuditEvent`    | `forwardToWindows(onAuditEvent, …)` → all windows `.send`   | `AuditEvent`     | `audit.test.ts`                     |
| `assistant:event`| `onAssistantEvent`| `win.webContents.send("assistant:event", …)`               | `AssistantEvent` | `assistant.test.ts`                 |
| `debug:log`      | (inline `ipcRenderer.on`) | Tauri global event from `DebugState::emit`; renderer listener installed before React | `LogEntry` | adapter/install tests; enable event wired, future subsystems call managed emitter |

Unsubscribe contract: each `on*` returns a disposer `() => void` that calls
`ipcRenderer.removeListener(channel, listener)`. Tauri adapter must mirror this
 (`unlisten()` from `listen()`).

## C. Window lifecycle & native shell

| Behavior | Electron impl | Tauri requirement | Verified by |
|---|---|---|---|
| Window state persistence | `~/.auto-tagger/window-state.json` (`{x,y,width,height,isMaximized}`), debounced 300ms on resize/move/maximize/unmaximize + on close | read/write same file in place; no move | smoke test |
| Off-screen position recovery | `screen.getAllDisplays()` workArea check; center if saved `x/y` not on any display | same logic | smoke test |
| Min size | `minWidth:900, minHeight:600`, default `1200×800`, bg `#1a1a2e` | window config `tauri.conf.json` | build |
| Title treatment | `titleBarStyle:"hiddenInset"` + `titleBarOverlay` (rgba 0.95, symbol #1d1d1f, height 38) | macOS hidden-inset equivalent; inset-compatible | build/launch |
| First paint | `show:false` + `ready-to-show` → `show()` | wait for `window` event then show | smoke |
| Dev vs prod load | dev: `VITE_DEV_SERVER_URL` (5173) or `AUTO_TAGGER_E2E_RENDERER_PATH`; prod: `../dist/index.html` | `frontendDist` + devUrl in `tauri.conf.json` | dev/build |
| Quit-during-write guard | `before-quit` → `isBatchWriteInProgress()` → confirm dialog → `app.exit()` | Tauri close-requested event + same guard | manual/WDIO |
| macOS reactivation | `window-all-closed` keeps app alive on darwin | standard Tauri macOS | smoke |
| Native context menu + clipboard | `Menu.buildFromTemplate` + `clipboard.writeText` | `tauri-plugin-clipboard-manager` + menu | WDIO/E2E |
| Folder dialog | `dialog.showOpenDialog` | `tauri-plugin-dialog` | E2E |
| ABI/native guard | `ensureNativeModules()` rebuild/quit prompt | removed (no native Node) | n/a |

## D. Persistence (must use in place; no reset/schema conversion)

| File | Path | Owner | Format / notes |
|---|---|---|---|
| Config | `~/.auto-tagger/config.yaml` (single path; `getConfigPaths()` returns only this one) | auto-tag.ts `getConfigPaths()` | flat YAML, comments, unknown keys, env precedence, redaction |
| Artist aliases | `~/.auto-tagger/artist-aliases.json` | aliases.ts `DEFAULT_ALIAS_FILE` | JSON |
| Cache DB | `~/.auto-tagger/cache.db` | auto-tag.ts (configurable `cachePath`) | SQLite — `lookup_cache`, `album_state`, `conversation_log` (names/ns/hashes unchanged) |
| Dataset index | `~/.auto-tagger/dataset-index.sqlite` | dataset.ts `DEFAULT_DB_PATH` | SQLite |
| Debug log | `~/.auto-tagger/auto-tag-debug-YYYY-MM-DD.log` (truncated per session) | debug.ts | JSON lines |
| General log | `~/.auto-tagger/auto-tagger.log` | — | |
| Window state | `~/.auto-tagger/window-state.json` | main.ts | JSON |

## E. Packaging targets (electron-builder.yml → Tauri bundles)

| Platform | Electron target | Tauri target | Arch |
|---|---|---|---|
| macOS | dmg + zip | dmg + app | arm64, x64 |
| Windows | nsis | nsis | x64 |
| Linux | AppImage + deb | AppImage + deb | x64 |

`appId: com.auto-tagger.app`, productName `Auto Tagger`, copyright 2026.
Unsigned-development posture preserved (no signing/notarization in this work).

## F. Renderer `DesktopAPI` surface (preload)

All methods in `ElectronAPI` (≈49) plus the inline `debug:log` console forwarder.
Shared renderer-neutral types to extract to `frontend/src/shared/desktop-api.ts`:
`AlbumInfo`, `CoverInfo`, `TrackData`, `AlbumDetail`, `TaskProgress`,
`AuditTrackResult`, `AuditEvent`, `AuditRunSummary`, `AuditApplyFixesSummary`,
`AutoTagEvent`, `DatasetStatus`, `DirEntry`, `DirectoryData`,
`TrackUndoSnapshot`, `ExtraTagUndoSnapshot`, `AssistantAction`,
`AssistantActionBatch`, `AssistantEvent`, `LogEntry`, `ExtraTag`,
`ExtraTagUpdate`, `SortByAlbumResult`, `SessionSummary`, `ConversationEntry`,
and the `DesktopAPI` (a.k.a. `ElectronAPI`) method interface.

## G. Backend module ownership map (port groups)

| Domain | Electron handler | Electron services | Approx LOC |
|---|---|---|---|
| Library/tracks read | library.ts, tracks.ts, directory.ts | LibraryService, TrackTagService, FilenameTagInferenceService | ~1100 |
| Writer (media safety) | writer.ts | TagWriteQueue, ApeTagEngine | ~2150 |
| Covers | cover.ts | ArtworkResolverService | ~1300 |
| Providers | musicbrainz.ts, discogs.ts, fallback.ts, candidates.ts, openrouter.ts, lyrics.ts | DiscogsService, RemoteTrackMatcher, SafeApiRequestService, SafeQueryService, ArtistIdentityResolver | ~3000 |
| Auto-tag | auto-tag.ts | LlmTaskRunner, PlanExecutor, tags | ~2500 |
| Audit | audit.ts | AuditRuleEngine | ~1350 |
| Assistant | assistant.ts, prompts.ts, conversation-logger.ts, schemas.ts | AssistantRuntime, AssistantToolRegistry | ~4200 |
| Config/cache/dataset | auto-tag.ts(cfg), cache.ts, dataset.ts, debug.ts | — | ~1300 |
| Organizer | organizer.ts | FolderOrganizerService, TrackNumberingService, ConvertService | ~800 |
| Aliases/chinese/extra-tags | aliases.ts, (chinese), ExtraTagService, TagPrettifyService, ProviderTagKeys | ~900 |

**Total Electron backend:** ~15.2k handlers + ~8.3k services + ~1.2k main/preload ≈ **24.7k LOC** to port to Rust.

## H. Rust crate crate inventory (port order)

1. **infra**: fs, sqlite (rusqlite), http (reqwest+boring/rustls), logging, artwork (image), encoding (chardetng/encoding_rs), tag I/O (lofty).
2. **state**, **commands** per behavioral group above.
3. Provider clients → auto-tag → audit → assistant.

## I. Cutover gate (final)

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test`
- [ ] frontend typecheck + Vitest green
- [ ] WebdriverIO Tauri E2E on macOS/Windows/Linux
- [ ] copied-real-media smoke tests under `/private/tmp`
- [ ] `git diff --check`
- [ ] packaged app launch + installer smoke
- [ ] every row above green or explicitly blocked
- [ ] `electron/`, preload/main/worker, vite-plugin-electron*, electron-builder.yml, Electron deps, ABI rebuild scripts/tests, `better-sqlite3` patch removed
- [ ] `just fe-*` recipes drive Vite + Cargo/Tauri
