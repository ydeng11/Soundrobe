# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Reliable embedded lyrics** — lyrics now preserve valid embedded content
  before checking `.lrc`, `.txt`, or LRCLIB sources; strictly reject uncertain
  or malformed text instead of writing mojibake; embed synchronized and plain
  lyrics in Navidrome-compatible tags without modifying sidecar files; and
  report written, preserved, unavailable, unsupported, and failed tracks.

- **Explicit release artifact names** — macOS and Linux GitHub release
  downloads are now named `soundrobe-{version}-{os}-{arch}` (for example
  `soundrobe-0.1.0-macos-arm64.dmg`, `soundrobe-0.1.0-macos-intel.dmg`, or
  `soundrobe-0.1.0-linux-arm64.deb`), so the app name, version, OS, and
  architecture are visible in each filename.

- **Linux ARM64 releases** — the release pipeline now builds and publishes
  ARM64 AppImage and deb packages alongside the existing x64 ones.

- WAV files whose terminal ID3 chunk omits its optional RIFF padding byte can
  now be edited without changing their PCM audio.

- Pull requests now report a single required test result, while cross-platform
  release bundles are built nightly only when the app version is unpublished.

Work since the 1.0.0 milestone. This period covers the migration of the
project from the legacy Python CLI to the current Tauri 2 desktop application
**Soundrobe**, followed by the hardening of that application toward its first
desktop release.

### Added

- **Offline assistant organization previews** — unambiguous requests to group
  files into album folders now create a preview without requiring an LLM
  connection.

- **Isolated advisor consultations** — project agents can explicitly consult a
  separate Codex or Claude session with bounded evidence, filtered environment
  access, and no workspace tools before the main agent decides how to proceed.

- **New-conversation button in the AI Assistant** — the panel header now has
  a "New chat" button that starts a fresh conversation (same reset as the
  `/clear` command), disabled while a request is in flight.

- **Multi-disc cue-splitting** — `cue-split` now detects folders holding
  multiple CUE sheets (2CD/3CD sets) and slices each disc into its own
  subfolder under `<album>-tracks/<disc>` with `disc=N/M` tags, instead of
  overwriting tracks when discs restart at track 01.

- **Unified media toolbox script** — `scripts/toolbox.sh` is now the single
  entry point for media/library utilities: cue-splitting (FLAC/WAV album
  images per CUE sheet, with image copies and optional doctor health check),
  DSF/DSD-to-FLAC conversion, audio ISO slicing (K2HD SACD and raw CD), RAR
  extraction, and the FLAC doctor/QA pipeline. The `doctor` command renders
  an HTML corruption report automatically whenever it saves a scan report.

- **Tauri 2 desktop application** — Rust backend with a React/TypeScript
  renderer, replacing the legacy Python CLI and the intermediate Electron
  prototype. Renderer code talks to the native process exclusively through the
  shared `DesktopAPI` contract, and all media writes are serialized through a
  shared Rust write queue with atomic validation.
- **Auto-tag pipeline** — folder hints → exact provider IDs / artist release
  browsing → MusicBrainz → Discogs → conditional LLM fallback, with genre
  fill, per-track artist credit preservation, multi-disc and compilation
  handling, provider-ID persistence (MusicBrainz + Discogs), and
  Simplified/Traditional Chinese artist matching.
- **Manual release search** — MusicBrainz/Discogs search dialog with release
  preview and an explicit confirm-to-write step.
- **AI Assistant** — LLM-driven assistant with a native tool protocol
  (`metadata.patch`, `metadata.transform`, `files.transform`,
  `auto_numbering_tracks`), deterministic routing, durable task state, session
  persistence, self-review, and approval-gated, preview-first mutations.
- **Cover and artwork** — resolution in the order local → Cover Art Archive →
  Discogs → TheAudioDB, artist artwork, embedded-cover removal and
  suppression, stale-cache invalidation, and an in-memory cover cache with
  preload for fast track selection.
- **Lyrics** — LRCLIB fetch and album lyrics download with encoding repair;
  local lyrics take precedence over remote ones.
- **Batch editing** — origin-scoped extra-tags editor, per-track write
  resilience with structured results, configurable write concurrency,
  worker-backed write queue, and undo restricted to successful writes.
- **Format support** — atomic readers/writers for MP3, FLAC, M4A/MP4,
  OGG/Opus, APEv2, WAV, and AIFF; FLAC covers stored as native `PICTURE`
  blocks; legacy UTF-16 MP3 artwork; WAV reads prefer ID3v2 over LIST INFO.
- **Configuration** — unified `~/.soundrobe/config.yaml` with environment
  precedence and secret redaction, LLM provider selection with a test
  connection, `AUTO_TAG_CHINESE_SCRIPT` for Simplified/Traditional conversion,
  and configurable write concurrency. Existing `~/.auto-tagger` application
  data is migrated into the new directory on startup.
- **Diagnostics** — real-write probe that tests the write path against a copy
  of a track, and a volume write-probe command.
- **UI** — tabbed settings, audit panel with findings highlighted in the track
  grid, redesigned Convert dialog with presets and live preview, ctrl-click
  track selection, right-click context menu, refreshed app icons, and an
  explicit Apply Changes button for right-panel metadata edits.
- **Local dataset** — optional MusicMoveArr dataset lookup with read-only
  status reporting.
- **Platform packaging** — macOS (Apple Silicon and Intel) app/DMG, Windows
  NSIS, and Linux AppImage/deb bundles validated by a CI matrix.

### Changed

- **AI Assistant side sheet** — the assistant now uses Soundrobe's native
  light and dark styling, a responsive overlay layout, context-aware composer,
  prompt suggestions, clearer progress states, and streamlined review cards
  for approval-gated changes.

- **Application rewritten** — the project moved from the Python `auto-tag`
  CLI to the Tauri desktop application; the intermediate Electron prototype
  was dropped in favor of the native Rust shell.
- **Configuration path** — the canonical configuration and application-data
  directory is now `~/.soundrobe`; existing `~/.auto-tagger` data is migrated
  there before the legacy directory is removed.

### Fixed

- **Manual Track Match artist and genre consistency** — provider collaborator
  identities now survive track mapping and reassignment, edited collaborative
  credits produce repeated artist tags, and missing provider genre is resolved
  when available or explicitly preserved when unavailable.

- **Canonical CJK artist credits during auto-tagging** — verified provider
  aliases now resolve back to the trusted folder artist across strong and weak
  track matches, while preserving genuine collaborators and only using unique
  track identities—scoped by disc whenever multi-disc evidence exists—to align
  alphabetically enumerated files.

- **Cover removal priority** — removing album artwork now selects the highest-
  priority standard cover name consistently, regardless of filesystem directory
  enumeration order.

- **Windows album selection** — album filters and album-scoped track actions now
  work correctly with native Windows backslash paths.

- **Complete MusicBrainz artist searches** — manual Search now resolves artist
  aliases to the exact MusicBrainz artist, loads every matching release page,
  lets users filter the cached release titles, and reuses an identical completed
  search after returning to the form or reopening the dialog without additional
  provider requests.

- **Discogs release title filtering** — manual Search now provides the same
  release-title filter for the currently loaded Discogs results page.

- **Faster AI Assistant applies** — approved standard metadata/tag batches now
  write album folders concurrently using the configured write limit, show
  preflight, writing, and verification progress, and still finish only after
  native readback confirms the requested changes.

- **Safer AI Assistant approvals** — file-operation previews now show their
  exact source and destination paths, approved moves refuse newly occupied
  destinations, dependent plan steps enforce approval order, and ordinary
  answers ending in a question no longer trigger the clarification-loop guard.
- **Auto-tag folder years** — release years at the start of a quoted album
  title are now detected even when the artist name precedes the title.

- **Faster WAV library reads** — standard WAV metadata loading now seeks past
  PCM audio and reads only chunk headers and metadata, while padded and
  malformed files retain the compatibility parser.

- **Faster metadata reads** — Opus, M4A/MP4, AIFF, and Monkey's Audio
  property/tag fallbacks now seek around encoded audio instead of loading the
  complete file, while unusual layouts retain their compatibility readers.

- **Faster metadata writes** — WAV writes reuse one loaded source, stream
  LIST/INFO cleanup, and compare PCM in bounded blocks; common FLAC edits now
  update only the metadata prefix; MP4 and OGG/Opus validation avoids cloning
  encoded audio and reuses already loaded bytes for local staging. Batch writes
  now use four folder workers by default after controlled local and SMB tests.

- **Auto-tag genre fallback** — structured LLM genre responses now retain
  numeric confidence values serialized as text, allow longer repair responses,
  and explicitly disclose when genre inference fails or remains below the
  confidence threshold instead of ending with a silent missing genre.

- **Assistant IME input** — confirming a candidate with Enter in Chinese and
  other input methods no longer sends the message prematurely, including the
  post-composition Enter sequence emitted by macOS WebKit.

- Batch edit: tag changes are no longer written when you click anywhere
  inside the batch editor panel — edits persist only when the Apply changes
  button is clicked, matching the single-track inspector.

- Manual release search: confirming a track match now writes the exact
  disc/track numbers and titles shown in the match table, honors the
  Simplified/Traditional Chinese script setting like auto-tag, keeps manual
  assignments and unused-track counts stable while writing, and leaves rows
  set to "Do not update" entirely untouched. Track totals from MusicBrainz
  and Discogs are now per disc (e.g. 14/14 for a two-CD set) instead of the
  whole-release count.
- A candidate without a year no longer removes an existing year tag when
  writing auto-tag or manual-search results; a year is written only when the
  matched release provides one.

- Manual MusicBrainz release search: resolving a release that exists no
  longer reports "MusicBrainz release not found" when the failure is a
  transient network error or a MusicBrainz rate limit — the dialog now shows
  the actual HTTP status (including the `Retry-After` hint when present)
  instead of claiming the release is missing.

- Assistant folder-derived metadata: instructions like "set album based on
  their folder name" now derive each track's album from its containing
  folder instead of writing the instruction text as a literal tag value. The
  preview shows the derived per-track values, verification re-checks the
  folder source after writing, and one identical literal value planned across
  many different folders raises a preview and post-write warning.

- WAV metadata: garbled LIST INFO reads, trailing junk/null-byte padding,
  over-eager orphan-tail repair, and reads preferring ID3v2 over stale LIST
  INFO.
- FLAC integrity: ghost Vorbis Comment blocks, stray APEv2 tags, ID3v2
  prefixes, the 20 MB block-size limit with broken-chain detection, and audio
  offset preservation during metadata rewrites.
- Auto-tag matching: `feat.` tracks with missing titles, album-subtitle
  poisoning, `leading_cjk` collisions, accidental `Various Artists`
  overwrites, artist-only fallback regressions, and direct MBID lookups with
  mismatched folder names.
- Batch writes: masked per-track failures, silently ignored disc fields, and
  SMB volume failures (data-only copy instead of `fs::copy`).
- Assistant reliability: false completions, LLM timeouts, malformed JSON,
  planned-action verbosity, and low-confidence candidate rejection.
- Assistant folder grouping: the assistant can now group tracks into album
  folders via the new `files.relocate` tool (destination derived from a tag
  field or filename transformation), and it stops asking the same clarifying
  question repeatedly — after a user answers, it either creates an approval
  preview or states the limitation.
- Assistant no longer guesses an action after a clarifying question stays
  unanswered: if the model repeats a question the user already answered, the
  app refuses to run a guessed mutating tool and asks the user to pick an
  option instead.
- Assistant `plan.create` now shows its real step schema to the model and to
  validation (it previously fell through to an empty schema), so multi-step
  requests like "group tracks into albums by the title before (" produce a
  valid two-step preview instead of repeatedly failing argument validation.
- Assistant tool guidance for `metadata.transform`/`files.relocate` now
  documents the regex_extract capture-group requirement and the {value}-only
  destination placeholder, so grouping by a title prefix no longer silently
  skips paren-less tracks or creates placeholder-named folders.
- Assistant title-based album grouping: `metadata.transform` regex extraction
  now writes the captured value even when it equals the whole source string,
  so setting the album to the title before '(' works for paren-less original
  tracks too (previously only version-suffixed tracks such as `(伴奏)` were
  fixed, or the request reported no changes at all). No-change tool results
  now state how many tracks were scanned, skipped, or already correct, and
  the assistant's clarification guard recognizes Chinese questions ending in
  the full-width `？`.
- Undo: auto-tag and batch writes are now correctly undoable.
- Tag hygiene: stale TXXX album-artist aliases shadowing TPE2 and leftover
  Vorbis ALBUMARTIST variants are cleared before writes.
- Configuration: `AUTO_TAG_CHINESE_SCRIPT` validated at load, concurrent
  config writes serialized, API-key credential flow fixed, and the configured
  lyrics URL honored for single-track fetches.
- UI: stale invoke callbacks after page reload, rapid right-click promise
  rejection, and optimistic updates shown before writes complete.

### Removed

- **Legacy `grill-with-docs` agent skill** — the retired planning interview
  workflow is no longer installed in the project skill catalog.

- Standalone `fix-medium` toolbox command and its obsolete
  `fix-medium-flac-tracks.js` implementation.
- Legacy Python CLI and Electron prototype artifacts (2026-07-18).

## [1.0.0] - 2026-05-10

Initial release milestone of the legacy Python `auto-tag` CLI for
Navidrome-oriented libraries:

- `tag` and `batch` commands with `--dry-run`, `--interactive`, and `--yolo`
  modes.
- Album matching via MusicBrainz, Discogs, and beets, with smart compilation
  detection and Chinese folder-name parsing.
- Health reports (`--health-report`) with cover-art checks.
- ReplayGain calculation and ffprobe-based audio validation.
- Local MusicMoveArr dataset lookup.

[Unreleased]: https://github.com/ydeng11/Soundrobe/compare/v1.0...HEAD
[1.0.0]: https://github.com/ydeng11/Soundrobe/releases/tag/v1.0
