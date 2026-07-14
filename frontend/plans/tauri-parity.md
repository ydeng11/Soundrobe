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
| Native shell: folder dialog / context menu / focus | ⏳ wired; GUI validation pending | `dialog_open_folder` honors `AUTO_TAGGER_E2E_LIBRARY_PATH`; plugin exposes only `Option<FilePath>`, so cancellation vs native GUI error rejection remains display-pending. `track_context_menu` honors `AUTO_TAGGER_E2E_TRACK_CONTEXT_ACTION`, builds Electron-equivalent action/copy labels, routes selection through managed single-popup state, and writes clipboard text; 4 focused tests cover override/filtering, exact Copy All output, action/null semantics, and overlap rejection. `window_focused` is wired as Electron’s no-op. PENDING: real-display popup/dismissal timing and quit-during-write (depends on `TagWriteQueue`). |
| Persistence (config/cache/dataset/aliases/logs) | ⏳ pending | |
| Read-only library slice — traversal/grouping/read core | ⏳ partial | `library_scan` has a shared fixture. Internal `TrackData` reader uses Lofty 0.24 plus bounded MPEG/OGG/FLAC/APE and MP4/Opus/AIFF property parsers. Eleven-file differential corpus is green: rich MP3 ID3, FLAC/WAV/OGG/APE, malformed inputs, M4A/MP4, true Opus, and AIFF. Fractional Electron bitrate is preserved (`f64`); MP4 `mdhd`/`stsz`, Opus granule+11.9 `lastPos` quirk, and AIFF COMM/SSND calculations match exact DTO values. `directory:read`, `album:read`, `album:refresh` are wired. PENDING: `directory_list` ICU collation and cover data URLs/provider discovery. |
| Mutation + media-safety slice | ⏳ MP3 characterization green; Rust writer pending | Shared reader corpus is green across eleven files. Electron MP3 writer characterization covers identical patch, omitted-vs-null, title-only, and rich updates: legacy outcome is always `full_rewrite/format_requires_full_rewrite`; identical patches still alter ID3 bytes; every case preserves MPEG payload SHA-256. Raw ID3 lyrics leakage and compilation readback `null` remain separately characterized. No Rust writer/queue/mutation yet. Next: atomic tri-state MP3 core with intentional true no-op and payload-hash validation. |
| Providers/auto-tag/audit/assistant | ⏳ pending | |
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
| `file:exists`             | `checkFileExists`       | tracks.ts  | `tracks.test.ts`                                                        | fs.existsSync |
| `track:context-menu`      | `showTrackContextMenu`  | main.ts    | Rust shell unit tests + E2E override | wired native menu + clipboard; E2E action override, exact Copy All payload, single-popup action state. GUI popup/dismissal timing pending display smoke. |
| `track:delete-files`      | `deleteFiles`          | main.ts    | — (integration)                                                         | unlink, per-file success/error |

### Covers

| Ch                          | Renderer method        | Owner     | Parity tests                                  | Notes |
|-----------------------------|------------------------|-----------|-----------------------------------------------|-------|
| `cover:data-url`            | `getCoverDataUrl`     | cover.ts  | `cover.test.ts`, `ArtworkResolverService.test.ts` | base64 data URL |
| `cover:set`                 | `setCover`            | cover.ts  | `cover.test.ts`                              | pick image, embed |
| `cover:remove`              | `removeCover`         | cover.ts  | `cover.test.ts`                              | strip embedded |
| `cover:download`            | `downloadCoverArt`    | cover.ts  | `cover.test.ts`, `cover-download.test.ts`     | resolver chain local→cover-art-archive→discogs→theaudiodb→google |
| `cover:download-artist-art` | `downloadArtistArt`  | cover.ts  | `cover-download.test.ts`, `ArtworkResolverService.test.ts` | local→wikimedia→google |

### Lyrics

| Ch            | Renderer method | Owner   | Parity tests                 | Notes |
|---------------|-----------------|---------|------------------------------|-------|
| `lyrics:fetch` | `fetchLyrics`   | main.ts | `lyrics.test.ts`, `lyrics-smoke.test.ts` | chardet/encoding for non-UTF8 |
| `album:download-lyrics` | `downloadAlbumLyrics` | main.ts | `lyrics.test.ts` | batch per album |

### Configuration & dataset

| Ch               | Renderer method     | Owner    | Parity tests        | Notes |
|------------------|---------------------|----------|---------------------|-------|
| `config:get`     | `getConfig`        | main.ts  | `config.test.ts`    | **redacted**: keys show `****`+last4 or null |
| `config:set`     | `setConfig`        | main.ts  | `config.test.ts`    | flat YAML, comments/unknown keys preserved, env precedence, key map |
| `dataset:status` | `getDatasetStatus` | main.ts  | `dataset.test.ts`, `dataset-lookup.test.ts` | musicbrainz/spotify flags, record count, lastUpdated |

### Tasks / auto-tag manager

| Ch              | Renderer method    | Owner    | Parity tests        | Notes |
|-----------------|--------------------|----------|---------------------|-------|
| `album:auto-tag` | `autoTagAlbum`    | main.ts  | `auto-tag.test.ts`, `auto-tag-chinese.test.ts`, `candidates.test.ts`, `fallback.test.ts`, `musicbrainz.test.ts`, `discogs.test.ts` | returns taskId; pipeline folder hints→MusicBrainz→Discogs→LLM |
| `task:progress`  | `getTaskProgress` | main.ts  | `auto-tag.test.ts` | running/completed/failed/cancelled, progress/total/message/result |
| `task:cancel`    | `cancelTask`      | main.ts  | `auto-tag.test.ts` | cooperative cancellation |

### Audit

| Ch                       | Renderer method      | Owner    | Parity tests                                          | Notes |
|--------------------------|----------------------|----------|-------------------------------------------------------|-------|
| `audit:run`              | `runAudit`          | audit.ts | `audit.test.ts`, `AuditRuleEngine.test.ts`            | whole-library deterministic rules |
| `audit:run-specified`     | `runAuditOnTracks` / `runAuditOnAlbums` | audit.ts | `audit.test.ts` | body `{trackPaths}` or `{albumPaths}` |
| `audit:run-album`         | `runAlbumAudit`     | audit.ts | `audit.test.ts`                                      | single-album results |
| `audit:apply-fixes`       | `applyAuditFixes`   | audit.ts | `audit.test.ts`                                      | approval-gated writes |
| `audit:cancel`            | `cancelAudit`       | audit.ts | `audit.test.ts`                                      | |

### Assistant (LLM)

| Ch                        | Renderer method          | Owner        | Parity tests                                                                 | Notes |
|---------------------------|--------------------------|--------------|------------------------------------------------------------------------------|-------|
| `assistant:send`           | `assistantSend`         | assistant.ts | `assistant.test.ts`, `AssistantRuntime.test.ts`, `assistant-paths.test.ts`   | routing + tools + previews |
| `assistant:cancel`         | `assistantCancel`        | assistant.ts | `assistant.test.ts`                                                          | |
| `assistant:clear`         | `assistantClear`         | assistant.ts | `assistant.test.ts`                                                          | |
| `assistant:apply-actions`  | `assistantApplyActions` | assistant.ts | `assistant.test.ts`, `assistant-folder-group.integration.test.ts`, `assistant-organize-files.integration.test.ts` | returns undo snapshots + optional task trigger |
| `assistant:reject-actions` | `assistantRejectActions`| assistant.ts | `assistant.test.ts`                                                          | |
| `assistant:get-batches`    | `assistantGetBatches`   | assistant.ts | `assistant.test.ts`                                                          | |
| `assistant:init-runtime`    | `assistantInitRuntime`  | assistant.ts | `assistant.test.ts`                                                          | |
| `assistant:init-services`   | `assistantInitServices` | assistant.ts | `assistant.test.ts`                                                          | apiKey/model/discogsToken/lyricsHost/libraryPath |
| `assistant:list-sessions`   | `listSessions`          | assistant.ts | `conversation-logger.test.ts`                                               | limit? |
| `assistant:get-conversation`| `getConversation`       | assistant.ts | `conversation-logger.test.ts`                                               | session uuid or number |
| `assistant:get-session`     | `getSession`            | assistant.ts | `conversation-logger.test.ts`                                               | |
| `assistant:current-session` | `getCurrentSession`     | assistant.ts | `conversation-logger.test.ts`                                               | `{sessionId, sessionNumber}` |

### Organizer

| Ch                    | Renderer method | Owner        | Parity tests                                    | Notes |
|-----------------------|------------------|--------------|-------------------------------------------------|-------|
| `files:sort-by-album` | `sortByAlbum`   | organizer.ts | `FolderOrganizerService.test.ts`, `group-by-album.integration.test.ts` | move/copy into album folders |

### Window / dialog / debug

| Ch                    | Renderer method     | Owner    | Parity tests   | Notes |
|-----------------------|---------------------|----------|----------------|-------|
| `window:focused`      | `onFocus`          | main.ts  | shell unit suite | wired no-op hook (matches Electron; no main-process state change) |
| `dialog:open-folder`  | `openFolderDialog` | main.ts  | (E2E)          | wired; selected/null parity only — plugin GUI-error rejection remains pending display validation |
| `debug:subscribe`     | `subscribeDebugLogs`| debug.ts | `debug.test.ts`| renderer opts into live `debug:log` forwarding |
| `debug:set-mode`      | `setDebugMode`     | main.ts  | `debug.test.ts`| toggle + persist to config |
| `debug:status`        | (internal/test)    | debug.ts | `debug.test.ts`| not in preload; parity still required |
| `debug:toggle`        | (internal/test)    | debug.ts | `debug.test.ts`| not in preload; parity still required |

**Subtotal:** 51 `ipcMain.handle` channels. 49 are surfaced through `window.api`
in `preload.ts`; `debug:status` and `debug:toggle` are main/internal-only but
must still be ported for test parity.

## B. Pushed event streams (4)

| Channel          | Renderer listener | Emit mechanism                                            | Payload type     | Parity tests                        |
|------------------|-------------------|-----------------------------------------------------------|------------------|-------------------------------------|
| `auto-tag:event` | `onAutoTagEvent`  | `forwardToWindows(onAutoTagEvent, …)` → all windows `.send` | `AutoTagEvent`   | `auto-tag.test.ts`                  |
| `audit:event`    | `onAuditEvent`    | `forwardToWindows(onAuditEvent, …)` → all windows `.send`   | `AuditEvent`     | `audit.test.ts`                     |
| `assistant:event`| `onAssistantEvent`| `win.webContents.send("assistant:event", …)`               | `AssistantEvent` | `assistant.test.ts`                 |
| `debug:log`      | (inline `ipcRenderer.on`) | `win.webContents.send("debug:log", entry)`        | `LogEntry`       | `debug.test.ts`                     |

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