# Graph Report - .  (2026-06-09)

## Corpus Check
- 130 files · ~120,289 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1163 nodes · 2134 edges · 73 communities (62 shown, 11 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 20 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Artist Alias Data|Artist Alias Data]]
- [[_COMMUNITY_Extra Tag Writer|Extra Tag Writer]]
- [[_COMMUNITY_Folder Tree & Audit|Folder Tree & Audit]]
- [[_COMMUNITY_Conversation Logger|Conversation Logger]]
- [[_COMMUNITY_Library Scanner|Library Scanner]]
- [[_COMMUNITY_Folder Organizer|Folder Organizer]]
- [[_COMMUNITY_Convert Dialog & Service|Convert Dialog & Service]]
- [[_COMMUNITY_Assistant Handler|Assistant Handler]]
- [[_COMMUNITY_Sidebar UI|Sidebar UI]]
- [[_COMMUNITY_Assistant Runtime|Assistant Runtime]]
- [[_COMMUNITY_Dataset Reader|Dataset Reader]]
- [[_COMMUNITY_Assistant Integration Tests|Assistant Integration Tests]]
- [[_COMMUNITY_Batch Extra Tags Editor|Batch Extra Tags Editor]]
- [[_COMMUNITY_Audit UI|Audit UI]]
- [[_COMMUNITY_Debug Logger|Debug Logger]]
- [[_COMMUNITY_OpenRouter  LLM Client|OpenRouter / LLM Client]]
- [[_COMMUNITY_Track Metadata Service|Track Metadata Service]]
- [[_COMMUNITY_TypeScript Config|TypeScript Config]]
- [[_COMMUNITY_App State Manager|App State Manager]]
- [[_COMMUNITY_Cache Service|Cache Service]]
- [[_COMMUNITY_Discogs Client|Discogs Client]]
- [[_COMMUNITY_Dev Dependencies|Dev Dependencies]]
- [[_COMMUNITY_Artist Aliases|Artist Aliases]]
- [[_COMMUNITY_Task Manager|Task Manager]]
- [[_COMMUNITY_IPC Handlers|IPC Handlers]]
- [[_COMMUNITY_Album Cache|Album Cache]]
- [[_COMMUNITY_NPM Scripts|NPM Scripts]]
- [[_COMMUNITY_Assistant Panel UI|Assistant Panel UI]]
- [[_COMMUNITY_File Grid UI|File Grid UI]]
- [[_COMMUNITY_Electron Main Process|Electron Main Process]]
- [[_COMMUNITY_Auto-Tag Handler|Auto-Tag Handler]]
- [[_COMMUNITY_Track Tag Service|Track Tag Service]]
- [[_COMMUNITY_Assistant Tool Registry|Assistant Tool Registry]]
- [[_COMMUNITY_LLM Task Runner|LLM Task Runner]]
- [[_COMMUNITY_Safe API Request|Safe API Request]]
- [[_COMMUNITY_Metadata Editor UI|Metadata Editor UI]]
- [[_COMMUNITY_Batch Update Plan|Batch Update Plan]]
- [[_COMMUNITY_Fallback Tagger|Fallback Tagger]]
- [[_COMMUNITY_Plan Executor|Plan Executor]]
- [[_COMMUNITY_Auto-Tag Compilation E2E|Auto-Tag Compilation E2E]]
- [[_COMMUNITY_FLAC Test Helpers|FLAC Test Helpers]]
- [[_COMMUNITY_Plan Executor Doc|Plan Executor Doc]]
- [[_COMMUNITY_Settings Modal|Settings Modal]]
- [[_COMMUNITY_Runtime Dependencies|Runtime Dependencies]]
- [[_COMMUNITY_MusicBrainz Client|MusicBrainz Client]]
- [[_COMMUNITY_Cover Organizer|Cover Organizer]]
- [[_COMMUNITY_Response Schemas|Response Schemas]]
- [[_COMMUNITY_Error Boundary|Error Boundary]]
- [[_COMMUNITY_Package Metadata|Package Metadata]]
- [[_COMMUNITY_Lyrics Service|Lyrics Service]]
- [[_COMMUNITY_Filename Inference|Filename Inference]]
- [[_COMMUNITY_Batch Editor UI|Batch Editor UI]]
- [[_COMMUNITY_Title Bar UI|Title Bar UI]]
- [[_COMMUNITY_Cover Art|Cover Art]]
- [[_COMMUNITY_Directory Listing|Directory Listing]]
- [[_COMMUNITY_Mock Database|Mock Database]]
- [[_COMMUNITY_Mock Statement|Mock Statement]]
- [[_COMMUNITY_SQLite Type Defs|SQLite Type Defs]]
- [[_COMMUNITY_Package Scripts Test|Package Scripts Test]]
- [[_COMMUNITY_HTML Entry Point|HTML Entry Point]]
- [[_COMMUNITY_Electron Builder Config|Electron Builder Config]]
- [[_COMMUNITY_Native Check Test|Native Check Test]]
- [[_COMMUNITY_Alias File Path|Alias File Path]]

## God Nodes (most connected - your core abstractions)
1. `AssistantRuntime` - 34 edges
2. `AlbumCandidate` - 27 edges
3. `TrackData` - 26 edges
4. `DatasetReader` - 22 edges
5. `TaskManager` - 21 edges
6. `AssistantToolRegistry` - 21 edges
7. `FolderOrganizerService` - 21 edges
8. `basename()` - 20 edges
9. `compilerOptions` - 19 edges
10. `ConversationLogger` - 18 edges

## Surprising Connections (you probably didn't know these)
- `parseAlbumPath()` --calls--> `basename()`  [INFERRED]
  frontend/electron/handlers/fallback.ts → frontend/src/utils/path.ts
- `parseAlbumPath()` --calls--> `dirname()`  [INFERRED]
  frontend/electron/handlers/fallback.ts → frontend/src/utils/path.ts
- `readAlbumTagsFromFirstFile()` --calls--> `basename()`  [INFERRED]
  frontend/electron/handlers/fallback.ts → frontend/src/utils/path.ts
- `readAlbumTagsFromFirstFile()` --calls--> `dirname()`  [INFERRED]
  frontend/electron/handlers/fallback.ts → frontend/src/utils/path.ts
- `trackHintsFromPath()` --calls--> `basename()`  [INFERRED]
  frontend/electron/handlers/fallback.ts → frontend/src/utils/path.ts

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **Plan Executor System Components** — planexecutor, createtooltool, topologicalsort, scratchpad, argrefsyntax [INFERRED 0.95]

## Communities (73 total, 11 thin omitted)

### Community 0 - "Artist Alias Data"
Cohesion: 0.02
Nodes (122): GAI周延, 万能青年旅店, 万芳, 久保田利伸, 久石让, 五月天, 伍佰, 何欣穗 (+114 more)

### Community 1 - "Extra Tag Writer"
Cohesion: 0.05
Nodes (53): ExtraTag, BatchWriteConflictError, batchWriteExtraTags(), buildFlacPictureBlock(), buildWavId3Chunk(), EXTRA_TAG_RESERVED_EXCEPTIONS, ExtraTagUpdate, fieldsToID3v2() (+45 more)

### Community 2 - "Folder Tree & Audit"
Cohesion: 0.09
Nodes (31): FolderTree(), FolderTreeProps, DirEntry, applyAuditFixes(), AUDIO_EXTENSIONS, AUDIT_SCHEMA, auditAlbum(), AuditEvent (+23 more)

### Community 3 - "Conversation Logger"
Cohesion: 0.07
Nodes (13): ConversationEntry, ConversationEntryType, ConversationLogger, epochRandomId(), NullConversationLogger, SessionSummary, attemptRebuild(), BetterSqlite3Statement (+5 more)

### Community 4 - "Library Scanner"
Cohesion: 0.09
Nodes (19): AlbumInfo, collectAudioFiles(), isAudioFile(), isHiddenDir(), parseArtistAlbumHint(), registerLibraryHandlers(), scanDirectory(), ScanResult (+11 more)

### Community 5 - "Folder Organizer"
Cohesion: 0.09
Nodes (9): AlbumLookup, FileOrganizeCriterion, FileOrganizeInput, FolderGroupInput, FolderOrganizerPlan, FolderOrganizerService, MoveAction, MoveResult (+1 more)

### Community 6 - "Convert Dialog & Service"
Cohesion: 0.14
Nodes (22): ConvertDialog(), ConvertDialogProps, ConvertResult, track, buildFilenameFromConvertPattern(), CONVERT_FIELD_LABELS, CONVERT_SOURCE_TAGS, ConvertDirection (+14 more)

### Community 7 - "Assistant Handler"
Cohesion: 0.09
Nodes (16): buildMutatingTools(), buildReadOnlyTools(), currentAppState, FolderPlanForBatch, getOrCreateRuntime(), hasBlankPerTrackFields(), LibraryTaskKind, PER_TRACK_UNIQUE_FIELDS (+8 more)

### Community 8 - "Sidebar UI"
Cohesion: 0.09
Nodes (19): Sidebar(), SidebarProps, AlbumInfo, AssistantAction, AuditEvent, AuditTrackResult, AutoTagEvent, CONSOLE_METHOD (+11 more)

### Community 9 - "Assistant Runtime"
Cohesion: 0.11
Nodes (3): IConversationLogger, AssistantRuntime, redactSensitive()

### Community 10 - "Dataset Reader"
Cohesion: 0.14
Nodes (5): DatasetReader, DEFAULT_DB_PATH, TRACK_TABLES, TrackTableConfig, BetterSqlite3Database

### Community 11 - "Assistant Integration Tests"
Cohesion: 0.12
Nodes (15): AssistantAction, AssistantActionBatchKind, AssistantEvent, AssistantEventCallback, ConversationMessage, detectToolIntentMismatch(), hasExplicitFileMoveIntent(), hasTrackNumberIntent() (+7 more)

### Community 12 - "Batch Extra Tags Editor"
Cohesion: 0.12
Nodes (9): BatchExtraTagsEditor(), BatchExtraTagsEditorProps, createNewRow(), DraftRow, DraftRow, ExtraTagsEditor(), ExtraTagsEditorProps, ExtraTag (+1 more)

### Community 13 - "Audit UI"
Cohesion: 0.12
Nodes (13): AuditBanner(), AuditBannerProps, AuditTrackResult, AuditPanel(), AuditPanelProps, AuditTrackResult, STATUS_COLORS, ScanProgressBar() (+5 more)

### Community 14 - "Debug Logger"
Cohesion: 0.14
Nodes (8): forwardToWindows(), DebugLogger, debugSubscriptions, forwardRendererLog(), LogCallback, LogEntry, logger, registerDebugIpc()

### Community 15 - "OpenRouter / LLM Client"
Cohesion: 0.16
Nodes (8): estimateCost(), formatCost(), LLMResponse, MODEL_COST_RATES, OpenRouterClient, OpenRouterConfig, RETRYABLE_STATUSES, LlmTaskRunner

### Community 16 - "Track Metadata Service"
Cohesion: 0.16
Nodes (20): COVER_EXTS, COVER_NAMES, detectExternalCover(), findFlacBlock(), firstComment(), hasFlacPictureBlock(), isRecord(), mapLimit() (+12 more)

### Community 17 - "TypeScript Config"
Cohesion: 0.10
Nodes (20): compilerOptions, allowImportingTsExtensions, esModuleInterop, forceConsistentCasingInFileNames, isolatedModules, jsx, lib, module (+12 more)

### Community 18 - "App State Manager"
Cohesion: 0.14
Nodes (8): AppAction, appReducer(), AppState, AuditResultEntry, initialAppState, TrackSnapshot, UndoManager, UndoOperation

### Community 19 - "Cache Service"
Cohesion: 0.22
Nodes (16): albumCandidateFromJson(), albumCandidateToJson(), buildLookupVariantPairs(), candidatesFromJson(), candidatesToJson(), LookupRequest, lookupRequestFromJson(), lookupRequestToJson() (+8 more)

### Community 20 - "Discogs Client"
Cohesion: 0.18
Nodes (6): artistDisplayName(), splitArtistNames(), DiscogsClient, DiscogsRateLimiter, mergeGenreStyle(), parseDiscogsArtists()

### Community 21 - "Dev Dependencies"
Cohesion: 0.11
Nodes (19): devDependencies, autoprefixer, electron, electron-builder, jsdom, patch-package, @playwright/test, postcss (+11 more)

### Community 22 - "Artist Aliases"
Cohesion: 0.19
Nodes (17): artistMatchesAny(), BUNDLED_ALIAS_FILE, charactersOverlap(), convertScript(), DEFAULT_ALIAS_FILE, __dirname, __filename, getAliases() (+9 more)

### Community 24 - "IPC Handlers"
Cohesion: 0.20
Nodes (14): cancelTask(), filterCandidatesForAutoApply(), getConfig(), getDatasetStatus(), getProgress(), getRawApiConfig(), getTaskManager(), onAutoTagEvent() (+6 more)

### Community 25 - "Album Cache"
Cohesion: 0.22
Nodes (7): contentHash(), folderNameHash(), MatchCache, pathHash(), sha256(), VALID_STATUSES, getBetterSqlite3()

### Community 26 - "NPM Scripts"
Cohesion: 0.12
Nodes (16): scripts, build, dev, dev:rebuild, dist, dist:linux, dist:mac, dist:win (+8 more)

### Community 27 - "Assistant Panel UI"
Cohesion: 0.18
Nodes (11): AssistantPanel(), AssistantPanelProps, AssistantStatus, ChatMessage, STATUS_CONFIG, StatusDetail, mockApi, AssistantActionBatch (+3 more)

### Community 28 - "File Grid UI"
Cohesion: 0.15
Nodes (12): ALL_COLUMNS, Column, COLUMN_FLEX, EMPTY_SELECTED_TRACK_PATHS, FileGrid(), FileGridProps, FileGridRow, FileGridRowProps (+4 more)

### Community 29 - "Electron Main Process"
Cohesion: 0.15
Nodes (12): createWindow(), debounce(), __dirname, loadWindowState(), WINDOW_STATE_PATH, WindowState, initializeAssistantServices(), registerAssistantHandlers() (+4 more)

### Community 30 - "Auto-Tag Handler"
Cohesion: 0.21
Nodes (12): AUDIO_EXTENSIONS, AutoTagConfig, CONFIG_KEY_MAP, hintsAreAmbiguous(), pathSegments(), taskEvents, TaskProgress, buildFallbackMessages() (+4 more)

### Community 31 - "Track Tag Service"
Cohesion: 0.19
Nodes (9): extractNativeTag(), readTrackMetadata(), batchWriteTags(), WriteFields, FilenameTagInference, PlannedActionBatch, PlannedTagAction, TagUpdateInstruction (+1 more)

### Community 33 - "LLM Task Runner"
Cohesion: 0.18
Nodes (11): TokenUsage, ApiCallCallback, AssistantLoopInput, AssistantLoopResult, AssistantLoopStep, AssistantToolDef, LlmTaskConfig, ParsedResponse (+3 more)

### Community 34 - "Safe API Request"
Cohesion: 0.21
Nodes (4): ALLOWED_HOSTS, SafeApiRequest, SafeApiRequestService, SafeApiResult

### Community 35 - "Metadata Editor UI"
Cohesion: 0.22
Nodes (6): formatDetailedTags(), formatDuration(), formatSize(), hasDetailedTags(), MetadataEditor(), MetadataEditorProps

### Community 36 - "Batch Update Plan"
Cohesion: 0.19
Nodes (10): applyLegacyExtraTagBatch(), applyMetadataUpdateBatch(), isInsideDirectory(), metadataBatchToExtraInputs(), metadataBatchToStandardUpdates(), planStripFilenamePrefixes(), planStripTitlePrefixes(), planTrackNumbering() (+2 more)

### Community 37 - "Fallback Tagger"
Cohesion: 0.35
Nodes (11): makeTrackCandidate(), candidateFromFolder(), cleanAlbumFolderName(), cleanFolderName(), COMPILATION_FOLDER_SET, extractYearFromName(), isCompilationFolder(), parseAlbumPath() (+3 more)

### Community 38 - "Plan Executor"
Cohesion: 0.21
Nodes (6): AssistantActionBatch, Plan, PlanResult, PlanStepDef, PlanStepError, PlanStepOutput

### Community 40 - "FLAC Test Helpers"
Cohesion: 0.36
Nodes (8): syntheticFlac(), writeSyntheticFlac(), flacHeader(), flacHeaderWithDuration(), paddingBlock(), vorbisCommentBlock(), syntheticFlac(), writeSyntheticFlac()

### Community 41 - "Plan Executor Doc"
Cohesion: 0.22
Nodes (10): Argument Reference Syntax, AssistantRuntime, AssistantToolRegistry, create_plan Assistant Tool, Plan-and-Solve Strategy, PlanExecutor, Plan Executor — Multi-Step Agent Pipeline, Plan Definition Types (+2 more)

### Community 42 - "Settings Modal"
Cohesion: 0.22
Nodes (4): SettingsModal(), SettingsModalProps, SettingsState, defaultMockConfig

### Community 43 - "Runtime Dependencies"
Cohesion: 0.20
Nodes (10): dependencies, better-sqlite3, dotenv, jschardet, music-metadata, node-id3, opencc-js, react (+2 more)

### Community 44 - "MusicBrainz Client"
Cohesion: 0.31
Nodes (5): TrackCandidate, createRateLimiter(), escapeQuery(), MusicBrainzClient, RateLimiter

### Community 45 - "Cover Organizer"
Cohesion: 0.27
Nodes (8): collectAllAudioFiles(), registerOrganizerHandlers(), sanitizeDirName(), sortByAlbum(), SortByAlbumEntry, SortByAlbumFile, SortByAlbumResult, isAudioFile()

### Community 46 - "Response Schemas"
Cohesion: 0.20
Nodes (8): AuditResponse, AuditTrackResult, CandidateSelectionResponse, CorrectedTrack, FallbackTagResponse, FolderExtractionResponse, GeneratedTrackTags, GenreEnrichmentResponse

### Community 47 - "Error Boundary"
Cohesion: 0.22
Nodes (4): renderPanel(), ErrorBoundary, ErrorBoundaryProps, ErrorBoundaryState

### Community 48 - "Package Metadata"
Cohesion: 0.22
Nodes (8): author, description, homepage, license, main, name, type, version

### Community 49 - "Lyrics Service"
Cohesion: 0.36
Nodes (3): LyricsClient, normalizeLyricsEncoding(), readLocalLyrics()

### Community 50 - "Filename Inference"
Cohesion: 0.39
Nodes (5): cleanStem(), FilenameTagInferenceOptions, FilenameTagInferenceService, normalizeArtistForSplit(), splitArtistTitle()

### Community 51 - "Batch Editor UI"
Cohesion: 0.33
Nodes (3): BATCH_FIELDS, BatchEditor(), BatchEditorProps

### Community 53 - "Cover Art"
Cohesion: 0.29
Nodes (3): COVER_EXTS, COVER_NAMES, registerCoverHandlers()

### Community 54 - "Directory Listing"
Cohesion: 0.38
Nodes (5): DirEntry, DirTreeData, listDirectoryEntries(), readDirectory(), registerDirectoryHandlers()

### Community 57 - "SQLite Type Defs"
Cohesion: 0.50
Nodes (3): Database, DatabaseOptions, Statement

## Knowledge Gaps
- **362 isolated node(s):** `陶喆`, `林憶蓮`, `李宗盛`, `張惠妹`, `林俊杰` (+357 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **11 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `FolderOrganizerService` connect `Folder Organizer` to `Assistant Integration Tests`, `Assistant Handler`?**
  _High betweenness centrality (0.033) - this node is a cross-community bridge._
- **Why does `TrackData` connect `Batch Extra Tags Editor` to `Metadata Editor UI`, `Assistant Handler`, `Sidebar UI`, `Audit UI`, `App State Manager`, `Batch Editor UI`, `Assistant Panel UI`, `File Grid UI`?**
  _High betweenness centrality (0.032) - this node is a cross-community bridge._
- **Why does `ConversationLogger` connect `Conversation Logger` to `Assistant Runtime`, `Dataset Reader`, `Assistant Handler`?**
  _High betweenness centrality (0.032) - this node is a cross-community bridge._
- **What connects `陶喆`, `林憶蓮`, `李宗盛` to the rest of the system?**
  _363 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Artist Alias Data` be split into smaller, more focused modules?**
  _Cohesion score 0.016260162601626018 - nodes in this community are weakly interconnected._
- **Should `Extra Tag Writer` be split into smaller, more focused modules?**
  _Cohesion score 0.051577152600170505 - nodes in this community are weakly interconnected._
- **Should `Folder Tree & Audit` be split into smaller, more focused modules?**
  _Cohesion score 0.08658536585365853 - nodes in this community are weakly interconnected._