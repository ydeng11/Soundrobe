# Soundrobe — Agent Guide

## Project overview

Soundrobe is a Tauri 2 + React desktop app for editing and enriching audio metadata. The maintained application is entirely under `frontend/`:

- `frontend/src/` — React renderer and the renderer-neutral `DesktopAPI` contract.
- `frontend/src/shared/tauri-adapter.ts` and `install-desktop-api.ts` — the renderer bridge from `DesktopAPI` calls and startup subscriptions to Tauri `invoke` and events.
- `frontend/src-tauri/src/commands/` — Tauri commands and orchestration.
- `frontend/src-tauri/src/state/` — managed configuration, tasks, caches, providers, and write queue.
- `frontend/src-tauri/src/infra/` — tag I/O, HTTP, SQLite, artwork, logging, encoding, and OpenRouter.
- `frontend/test/` — renderer component/state/adapter tests and shared media fixtures.
- `frontend/e2e-tauri/` — WebdriverIO workflows against the built native app; credentialed and real-display smokes are selected explicitly.
- `frontend/src-tauri` inline `#[cfg(test)]` modules — Rust unit and integration contracts.

Tauri is the only application backend. Do not reintroduce Python application code, Electron, native Node modules, an Electron preload, or a second desktop backend.

## Stack and boundaries

- Desktop: Tauri 2 / Rust
- Renderer: React 19 / TypeScript / Vite / Tailwind
- Metadata: Lofty plus bounded format-specific Rust readers and writers
- Storage: rusqlite, using existing files in place
- HTTP: reqwest with Rustls
- Native bridge: Tauri commands for request/response and Tauri events for pushed progress
- Tests: Cargo test, Vitest, and WebdriverIO with the Tauri driver

Renderer code should use the shared `DesktopAPI` contract. Keep direct Tauri `invoke` and `listen` calls inside the bridge modules under `frontend/src/shared/`; do not leak Tauri transport details into components or state. Tauri commands receive renderer requests and wire services, while filesystem access, HTTP, SQLite, secrets, and tag I/O remain in the native process. Pure deterministic logic should remain independently testable.

All media writes must go through the shared Rust `WriteQueue`; never create a parallel writer or bypass atomic validation. Register every new command in the Tauri `generate_handler!` list, expose it through the existing adapter contract, and fail explicitly when a command or format is unsupported.

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
- `just fe-smoke-openrouter` — credentialed native OpenRouter integration gate
- `just fe-smoke-assistant` — credentialed packaged assistant loopback
- `just fe-smoke-cover-picker` — macOS native picker cancellation gate

Targeted checks:

- `cd frontend && npm run test:web`
- `cd frontend && npm run test:e2e`
- `cd frontend && npm run typecheck`
- `cd frontend/src-tauri && cargo test <filter>`
- `cd frontend/src-tauri && cargo clippy --all-targets -- -D warnings`

`just fe-check` is the deterministic default gate and does not run credentialed or real-display smoke tests. Run the relevant smoke explicitly when changing OpenRouter, assistant loopback, native dialogs, or packaged-app integration.

## Changelog

`CHANGELOG.md` at the repository root follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and documents
user-visible changes between releases.

- Every commit that changes behavior — a `feat`, `fix`, `perf`, breaking
  change, or removed feature — must add or update an entry under
  `[Unreleased]` in the same commit. Test-only, refactor, docs, and chore
  commits that do not change behavior need no entry.
- Entries describe what a user or operator observes; leave internal plumbing,
  commit hashes, and implementation details out.
- When cutting a release (see `docs/release-checklist.md`), move the
  `[Unreleased]` entries into a new dated section for the released version and
  start a fresh, empty `[Unreleased]` heading on top.

## Configuration, persistence, and logs

The root `.env.local` is ignored by Git and loaded by `just` for local commands. Use it for development-only process environment variables such as `LLM_API_KEY` and `LLM_MODEL`; never commit it, print its values, or expose them through renderer responses. User-editable application settings remain in `~/.auto-tagger/config.yaml`, with process environment precedence handled by the Rust config state.

Use existing user data in place; do not reset or migrate formats silently:

- `~/.auto-tagger/config.yaml`
- `~/.auto-tagger/cache.db`
- `~/.auto-tagger/dataset-index.sqlite`
- `~/.auto-tagger/artist-aliases.json`
- `~/.auto-tagger/window-state.json`
- `~/.auto-tagger/auto-tagger.log`
- `~/.auto-tagger/auto-tag-debug-YYYY-MM-DD.log`

For active-app troubleshooting, inspect the Tauri process, the current debug log, config, cache tables, and Vite at `http://localhost:5173/`. Use copied real media under `/private/tmp` for live read/write validation; never mutate the original music library during verification.

## Metadata invariants

- Preserve per-track artist credits, provider IDs, multi-disc positions, duplicate track numbers, and audio payload bytes.
- Vorbis keys may be uppercase on disk even though readers normalize them.
- Cover resolution order is local → Cover Art Archive → Discogs → TheAudioDB; cover suppression must be honored.
- Local lyrics take precedence over optional remote lyrics.
- Assistant mutations are preview-first and require explicit approval.
- Config secrets stay in the native process and renderer responses remain redacted.
