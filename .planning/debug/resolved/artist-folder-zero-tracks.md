---
status: resolved
trigger: "The real artist folder /Volumes/downloads/弦子 opens in the app with zero tracks; it must support load, auto-tag, artwork download, audit, AI assistant metadata updates, and Convert tag updates."
created: 2026-07-19
updated: 2026-07-19
---

# Artist Folder Loads With Zero Tracks

## Symptoms

- expected: Selecting `/Volumes/downloads/弦子` loads its tracks and enables auto-tag, artwork download, audit, AI assistant metadata updates, and Convert tag updates.
- actual: The library opens with zero tracks.
- errors: No visible error was supplied.
- timeline: Not supplied.
- reproduction: Open the real artist folder `/Volumes/downloads/弦子` in the Tauri app.

## Current Focus

- hypothesis: The reported zero-track behavior came from an older running app; the current build with the pending non-blocking Tauri folder-picker fix loads the real folder correctly.
- test: Run the current Tauri WebDriver build through Open Library against `/Volumes/downloads/弦子`, then exercise all requested workflows on independent copied real FLACs.
- expecting: The UI reports 12 albums and 136 tracks; copied-media auto-tag, artwork, audit, assistant update, and Convert complete with readback.
- next_action: Restart/rebuild the user's running Tauri app so it uses the validated current worktree.

## Evidence

- timestamp: 2026-07-19
  observation: The real folder contains 136 supported FLAC files in 12 direct album subdirectories.
  implication: The input is neither empty nor an unsupported format/layout.
- timestamp: 2026-07-19
  observation: A read-only native diagnostic scanned 12 albums, read 136 tracks, and serialized every AlbumDetail successfully.
  implication: The Rust scanner, FLAC reader, Unicode paths, and response DTO are healthy for the exact real media.
- timestamp: 2026-07-19
  observation: The current Tauri WebDriver build loaded `/Volumes/downloads/弦子` through the Open Library button and displayed 12 albums and 136 files.
  implication: The complete native picker, IPC adapter, React load orchestration, and renderer state path works in the current build.
- timestamp: 2026-07-19
  observation: Independent copies of a real 23 MB FLAC passed auto-tag completion, local artwork download/write, audit findings with proposed corrections, assistant preview/apply/readback, and Convert dialog/write/readback.
  implication: All requested local workflows operate on representative real media without mutating the original library.
- timestamp: 2026-07-19
  observation: A live provider/LLM run was blocked before launch because it could disclose copied real metadata to external services; the safe rerun disabled remote lookup.
  implication: Local workflow behavior is verified, while remote-provider/LLM accuracy remains intentionally unverified.

## Eliminated

- hypothesis: The scanner does not recognize FLAC or CJK paths.
  reason: Both native and Tauri IPC diagnostics returned all 136 tracks.
- hypothesis: One malformed FLAC causes every album read to reject.
  reason: All 12 albums read and serialized successfully, and each requested workflow passed on copied real FLAC data.

## Resolution

- root_cause: The current source does not reproduce the zero-track defect. The worktree already contained the Tauri folder/runtime fix (async non-blocking picker plus correct drag-region handling), while the reported behavior is consistent with an older running build that had not incorporated it.
- fix: Preserve the pending Tauri folder-picker and title-bar fixes; restart/rebuild the app from the current worktree. No additional scanner or metadata change is justified.
- verification: Current Tauri Open Library UI loaded 12 albums/136 real tracks. Five requested workflows passed on independent copied real FLACs. The original mounted library was read-only throughout.
- files_changed: [frontend/src-tauri/src/commands/shell.rs, frontend/src/components/TitleBar.tsx, frontend/test/components/TitleBar.test.tsx]
