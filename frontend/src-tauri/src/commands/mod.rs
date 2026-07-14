//! Tauri commands, grouped to mirror `electron/handlers/*`.
//!
//! Each submodule is a parity owner for the rows in
//! `frontend/plans/tauri-parity.md`. Modules are intentionally empty until
//! their slice is ported behind a failing contract test (TDD); a command is
//! wired into `generate_handler!` only once its parity row is green.

/// Build identity + scaffold health-check (currently wired).
pub mod meta;
/// Native shell: folder dialog, context menu, window activation, quit guard
/// (the `electron/main.ts` GUI handlers ported per step 2).
pub mod shell;

// ── Parity owners (slices; wired as they turn green) ────────────────
/// `assistant:*` — `electron/handlers/assistant.ts`.
pub mod assistant;
/// `audit:*` — `electron/handlers/audit.ts`.
pub mod audit;
/// `config:get`/`config:set` (redacted) — `electron/handlers/auto-tag.ts`.
pub mod configuration;
/// `assistant:list-sessions` / `get-conversation` / `get-session` / `current-session`.
pub mod conversation;
/// `cover:*` — `electron/handlers/cover.ts`.
pub mod covers;
/// `dataset:status` — `electron/handlers/dataset.ts`.
pub mod dataset;
/// `directory:list`, `directory:read` — `electron/handlers/directory.ts`.
pub mod directories;
/// `library:scan`, `album:refresh` — `electron/handlers/library.ts`.
pub mod library;
/// `files:sort-by-album` — `electron/handlers/organizer.ts`.
pub mod organizer;
/// `task:progress`, `task:cancel`, `album:auto-tag` — `electron/handlers/auto-tag.ts`.
pub mod tasks;
/// `album:read`, `track:write`, `tracks:batch-write`, extra-tags, rename, exists.
pub mod tracks;
