# Soundrobe — Agent Guide

These rules apply to every task in this project unless explicitly overridden. Bias toward caution on non-trivial work.

## Working rules

1. State assumptions and success criteria before substantial changes.
2. Make the smallest change that solves the verified problem.
3. Read exports, callers, and shared utilities before writing.
4. Surface conflicting patterns; do not average them together.
5. Use deterministic code for routing, retries, status handling, and transforms. Use a model only for judgment.
6. Use TDD for every code change. Add integration coverage for structural changes.
7. Tests must encode intent and fail when the business rule changes.
8. Checkpoint after significant steps and fail loud about skipped or unverified work.
9. Match existing conventions and preserve unrelated worktree changes.
10. After changes exceeding 50 non-test lines, run `pi-simplify` and manually review the diff. If the command is unavailable, report that and perform the manual review.

## Project overview

Soundrobe is a Tauri 2 + React desktop app for editing and enriching audio metadata. The maintained application is entirely under `frontend/`:

- `frontend/src/` — React renderer and the renderer-neutral `DesktopAPI` contract.
- `frontend/src-tauri/src/commands/` — Tauri commands and orchestration.
- `frontend/src-tauri/src/state/` — managed configuration, tasks, caches, providers, and write queue.
- `frontend/src-tauri/src/infra/` — tag I/O, HTTP, SQLite, artwork, logging, encoding, and OpenRouter.
- `frontend/test/` — renderer component/state/adapter tests and shared media fixtures.
- `frontend/src-tauri` inline `#[cfg(test)]` modules — Rust unit and integration contracts.

Tauri is the only application backend. Do not reintroduce Python application code, Electron, native Node modules, an Electron preload, or a second desktop backend.

## Stack and boundaries

- Desktop: Tauri 2 / Rust
- Renderer: React 19 / TypeScript / Vite / Tailwind
- Metadata: Lofty plus bounded format-specific Rust readers and writers
- Storage: rusqlite, using existing files in place
- HTTP: reqwest with Rustls
- Tests: Cargo test and Vitest

Tauri commands receive renderer requests and wire services. Pure deterministic logic should remain independently testable. All media writes must go through the shared Rust `WriteQueue`; never create a parallel writer or bypass atomic validation.

The metadata pipeline remains: folder hints → exact provider IDs / artist release browsing → MusicBrainz → Discogs → conditional LLM fallback. Higher-confidence fields are not overwritten by lower-priority sources. Prefer explicit no-change or unsupported outcomes over a guessed write.

## Commands

From the repository root:

- `just fe-install` — install renderer and Tauri CLI dependencies
- `just fe-dev` — run Tauri with Vite HMR
- `just fe-build` — build the Tauri app/bundle
- `just fe-test` — run renderer and Rust tests
- `just fe-typecheck` — TypeScript typecheck
- `just fe-check` — typecheck plus all tests
- `just fe-dist mac|win|linux` — build a platform bundle

Targeted checks:

- `cd frontend && npm run test:web`
- `cd frontend && npm run typecheck`
- `cd frontend/src-tauri && cargo test <filter>`
- `cd frontend/src-tauri && cargo clippy --all-targets -- -D warnings`

## Persistence and logs

Use existing user data in place; do not reset or migrate formats silently:

- `~/.auto-tagger/config.yaml`
- `~/.auto-tagger/cache.db`
- `~/.auto-tagger/dataset-index.sqlite`
- `~/.auto-tagger/artist-aliases.json`
- `~/.auto-tagger/window-state.json`
- `~/.auto-tagger/auto-tag-debug-YYYY-MM-DD.log`

For active-app troubleshooting, inspect the Tauri process, the current debug log, config, cache tables, and Vite at `http://localhost:5173/`. Use copied real media under `/private/tmp` for live read/write validation; never mutate the original music library during verification.

## Metadata invariants

- Preserve per-track artist credits, provider IDs, multi-disc positions, duplicate track numbers, and audio payload bytes.
- Vorbis keys may be uppercase on disk even though readers normalize them.
- Cover resolution order is local → Cover Art Archive → Discogs → TheAudioDB; cover suppression must be honored.
- Local lyrics take precedence over optional remote lyrics.
- Assistant mutations are preview-first and require explicit approval.
- Config secrets stay in the native process and renderer responses remain redacted.
