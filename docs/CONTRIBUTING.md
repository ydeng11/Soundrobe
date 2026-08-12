# Contributing to Soundrobe

Thanks for helping improve Soundrobe. The project is under active development,
so small, focused changes with clear verification are especially helpful.

## Before opening an issue or discussion

- Search existing issues and discussions first.
- Use a bug report for reproducible broken behavior.
- Use a feature request for a concrete request that is ready to be tracked.
- Use Discussions for questions, self-hosting and configuration help, early
  ideas, and sharing setups.
- Never include API keys, credentials, private library paths, or unredacted
  logs. Use a copy of real media under `/private/tmp` when reproducing media
  behavior locally.

## Development setup

Soundrobe is a Tauri 2 desktop application with a Rust backend and a
React/TypeScript renderer.

Prerequisites:

- Node.js 22 and npm
- A stable Rust toolchain (Rust 1.85 or newer)
- Tauri 2 platform dependencies for your operating system
- `just`

Install dependencies and start the development app:

```bash
just install
just dev
```

The maintained application lives at the repository root. Renderer code should use
the shared `DesktopAPI` contract. Direct Tauri transport calls belong in the
bridge modules under `src/shared/`; filesystem, HTTP, SQLite, secrets,
and metadata I/O stay in the native Rust process.

## Tests and quality checks

Run the default local gate before opening a pull request:

```bash
just check
```

Useful targeted checks are:

```bash
just typecheck
just test
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

Credentialed or real-display checks are separate and should be run when the
change affects those workflows:

```bash
just smoke-openrouter
just smoke-assistant
just smoke-cover-picker
```

Tests should explain the behavior they protect. For metadata changes, preserve
audio payload bytes, provider IDs, per-track credits, multi-disc positions, and
the shared Rust `WriteQueue` and verify readback where applicable.

## Pull requests

1. Create a focused branch from the default branch.
2. Keep unrelated formatting and refactoring out of the change.
3. Describe the user-visible behavior, the implementation boundary, and the
   verification commands in the pull request.
4. Include screenshots or a short recording for UI changes.
5. Update `docs/CHANGELOG.md` under `[Unreleased]` for behavior changes. Test-only,
   documentation-only, refactor, and chore changes do not need a changelog
   entry.
6. Confirm that no secrets, local configuration, generated bundles, or real
   media files are included.

All media writes must use the existing native write queue and atomic validation.
Do not introduce a second desktop backend, Python application path, Electron
preload, or direct renderer filesystem access.

## Documentation changes

Prefer durable user and operator documentation under `docs/`. Keep agent
working artifacts under `.planning/` and do not add root-level planning files.
