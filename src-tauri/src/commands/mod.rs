//! Tauri commands, grouped to mirror `electron/handlers/*`.
//!
//! Each submodule is a parity owner for the rows in
//! `.planning/plans/tauri-parity.md`. Modules are intentionally empty until
//! their slice is ported behind a failing contract test (TDD); a command is
//! wired into `generate_handler!` only once its parity row is green.

/// Build identity + scaffold health-check (currently wired).
#[cfg(feature = "desktop")]
pub mod meta;
/// Native shell: folder dialog, context menu, window activation, quit guard
/// (the `electron/main.ts` GUI handlers ported per step 2).
#[cfg(feature = "desktop")]
pub mod shell;

// ── Parity owners (slices; wired as they turn green) ────────────────
/// `album:search-releases` / `album:resolve-release` / `album:preview-release-match` / `album:search-apply-candidate`
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod album_search;
/// `assistant:*` — `electron/handlers/assistant.ts`.
#[cfg(feature = "desktop")]
pub mod assistant;
#[cfg(feature = "desktop")]
pub(crate) mod assistant_intent;
#[cfg(feature = "desktop")]
pub(crate) mod assistant_metadata_tools;
#[cfg(feature = "desktop")]
pub(crate) mod assistant_tools;
/// `audit:*` — `electron/handlers/audit.ts`.
#[cfg(feature = "desktop")]
pub mod audit;
/// `album:auto-tag` candidate normalization and orchestration.
#[cfg(feature = "desktop")]
pub mod auto_tag;
/// `config:get`/`config:set` (redacted) — `electron/handlers/auto-tag.ts`.
#[cfg(feature = "desktop")]
pub mod configuration;
/// `assistant:list-sessions` / `get-conversation` / `get-session` / `current-session`.
#[cfg(feature = "desktop")]
pub mod conversation;
/// `cover:*` — `electron/handlers/cover.ts`.
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod covers;
/// `dataset:status` — `electron/handlers/dataset.ts`.
#[cfg(feature = "desktop")]
pub mod dataset;
/// `debug:subscribe` / `debug:set-mode` and renderer log forwarding.
#[cfg(feature = "desktop")]
pub mod debug;
/// `directory:list`, `directory:read` — `electron/handlers/directory.ts`.
#[cfg(feature = "desktop")]
pub mod directories;
/// `library:scan`, `album:refresh` — `electron/handlers/library.ts`.
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod library;
/// `lyrics:fetch` and later album lyric download.
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod lyrics;
/// Pure, media-safe writer cores; command/queue wiring follows separately.
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod mutations;
/// `files:sort-by-album` — `electron/handlers/organizer.ts`.
#[cfg(feature = "desktop")]
pub mod organizer;
/// `task:progress`, `task:cancel`, `album:auto-tag` — `electron/handlers/auto-tag.ts`.
#[cfg(feature = "desktop")]
pub mod tasks;
/// Deterministic provider-track alignment used by auto-tag.
#[cfg(any(feature = "desktop", feature = "server"))]
mod track_matcher;
/// `album:read`, `track:write`, `tracks:batch-write`, extra-tags, rename, exists.
#[cfg(any(feature = "desktop", feature = "server"))]
pub mod tracks;
/// Signed packaged-app update checks and coordinated installation.
#[cfg(feature = "desktop")]
pub mod updater;
