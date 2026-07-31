# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Work since the 1.0.0 milestone. This period covers the migration of the
project from the legacy Python CLI to the current Tauri 2 desktop application
**Soundrobe**, followed by the hardening of that application toward its first
desktop release.

### Added

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
- **Configuration** — unified `~/.auto-tagger/config.yaml` with environment
  precedence and secret redaction, LLM provider selection with a test
  connection, `AUTO_TAG_CHINESE_SCRIPT` for Simplified/Traditional conversion,
  and configurable write concurrency.
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

- **Application rewritten** — the project moved from the Python `auto-tag`
  CLI to the Tauri desktop application; the intermediate Electron prototype
  was dropped in favor of the native Rust shell.
- **Configuration path** — consolidated to the single canonical
  `~/.auto-tagger/config.yaml`; renderer responses redact secrets.

### Fixed

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
- Undo: auto-tag and batch writes are now correctly undoable.
- Tag hygiene: stale TXXX album-artist aliases shadowing TPE2 and leftover
  Vorbis ALBUMARTIST variants are cleared before writes.
- Configuration: `AUTO_TAG_CHINESE_SCRIPT` validated at load, concurrent
  config writes serialized, API-key credential flow fixed, and the configured
  lyrics URL honored for single-track fetches.
- UI: stale invoke callbacks after page reload, rapid right-click promise
  rejection, and optimistic updates shown before writes complete.

### Removed

- Standalone `fix-medium` toolbox command (the underlying
  `fix-medium-flac-tracks.js` tool remains available in `scripts/`).
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
