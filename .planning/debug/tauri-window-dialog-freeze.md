---
status: awaiting_human_verify
trigger: "App cannot be moved after just fe-dev, and clicking Open the Library leaves the cursor loading instead of opening the file browser. Electron version works; this is the first Tauri attempt; no visible errors."
created: 2026-07-18
updated: 2026-07-19
---

# Tauri Window and Folder Dialog Regression

## Symptoms

- expected: The app window can be dragged and resized freely.
- expected: Open Library opens the native file browser and loads a selected folder and its files.
- actual: The Tauri dev window cannot be moved; Open Library changes the cursor to a loading symbol and does not open a usable browser.
- errors: No terminal, console, or app error observed by the user.
- timeline: First attempt with the Tauri app; both behaviors work in the Electron app.
- reproduction: Run `just fe-dev`, attempt to drag the window, then click Open Library.

## Current Focus

- hypothesis: Confirmed root cause and fix; deterministic verification is complete. The native picker smoke is environmentally blocked before test execution because `tauri-driver` is not installed.
- test: Run `just fe-dev` in a real macOS display session, drag the window from the title-bar background and non-interactive child surfaces, then open and cancel/select a library folder.
- expecting: The window moves immediately, interactive buttons remain clickable, and Open Library presents a responsive native picker whose cancel/selection returns control to the app.
- next_action: Await the user's real-display confirmation; rerun `just fe-smoke-cover-picker` only in an environment with `tauri-driver` installed.
- reasoning_checkpoint:
    hypothesis: "The window cannot be dragged because TitleBar renders only Electron's CSS drag region and omits Tauri's data marker; Open Library freezes because the synchronous Tauri command invokes the plugin's blocking picker on its command thread."
    confirming_evidence:
      - "TitleBar.tsx renders the custom title-bar root with `.drag-region` but no `data-tauri-drag-region`."
      - "shell.rs registers a synchronous `dialog_open_folder` command that directly calls `.blocking_pick_folder()`."
      - "Local tauri-plugin-dialog 2.7.1 documentation says blocking dialog APIs must not be used on the main thread and provides callback-based `pick_folder`; local Tauri 2.11.5 drag handling keys on `data-tauri-drag-region`."
      - "The default capability already grants `core:window:allow-start-dragging`, and tauri.conf already enables decorations and resizing."
    falsification_test: "The hypothesis would be false if the current title bar already carried Tauri's marker, or if the native command used the callback picker / returned without synchronously blocking the command thread."
    fix_rationale: "Marking the existing drag surface with Tauri's supported attribute activates the existing capability; awaiting the callback picker preserves the Promise contract while letting the native event loop present and dismiss the dialog without a synchronous wait."
    blind_spots: "Automated tests cannot prove macOS native-window interaction; a real-display human check is still required after deterministic renderer and Rust verification."
- tdd_checkpoint:
    test_file: "frontend/test/components/TitleBar.test.tsx; frontend/src-tauri/src/commands/shell.rs"
    test_name: "marks the custom title bar as a Tauri window drag region; folder_dialog_command_is_async"
    status: green
    failure_output: "Red: renderer received null instead of the Tauri drag attribute; Rust E0277 reported Option<String> is not a Future. Green: renderer suite 322/322 and async-command test passed."

## Evidence

- timestamp: 2026-07-18
  observation: `TitleBar.tsx` uses the `.drag-region` class, whose CSS is only `-webkit-app-region: drag`, and has no Tauri drag marker.
  implication: The custom overlay title bar has no supported Tauri drag region.
- timestamp: 2026-07-18
  observation: `dialog_open_folder` is synchronous and invokes `blocking_pick_folder()`.
  implication: Native picker setup can block the desktop UI event loop and leave the cursor busy.
- timestamp: 2026-07-18
  observation: `tauri.conf.json` already sets `decorations: true` and `resizable: true`.
  implication: Resize configuration is enabled; the failure is more likely UI-thread starvation or custom title-bar behavior than a disabled resizable flag.
- timestamp: 2026-07-18
  observation: `frontend/src-tauri/capabilities/default.json` already grants `core:window:allow-start-dragging`, and the adapter maps `openFolderDialog()` to the registered `dialog_open_folder` command.
  implication: Neither a missing capability nor a renderer/native command-name mismatch explains the symptoms.
- timestamp: 2026-07-18
  observation: Local `tauri-plugin-dialog` 2.7.1 source documents callback `pick_folder` as the main-thread-safe API and warns that blocking APIs must not be called on the main thread; local Tauri 2.11.5 recognizes `data-tauri-drag-region`.
  implication: The observed code violates the supported dialog threading contract and omits Tauri's required drag marker.
- timestamp: 2026-07-18
  observation: The new renderer test fails with `null` for `data-tauri-drag-region`; the new Rust type-level test fails with E0277 because `dialog_open_folder` returns `Option<String>` rather than a Future.
  implication: Both regressions are reproduced deterministically before implementation.
- timestamp: 2026-07-18
  observation: After the minimal implementation, the renderer suite passes 322/322 and `cargo test folder_dialog_command_is_async` passes.
  implication: Both exact regression contracts are green without changing the renderer API or Tauri command registration.
- timestamp: 2026-07-18
  observation: `just fe-check` passes outside the restricted sandbox: TypeScript typecheck, 322 renderer tests, and 278 Rust tests pass; 3 credentialed/live Rust tests remain explicitly ignored by the existing suite.
  implication: The fix passes the project's full deterministic gate; the initial sandbox-only failures were local mock-server bind denials, not product regressions.
- timestamp: 2026-07-18
  observation: Tauri 2.11.5 drag handling applies an empty marker only when the marked element is the direct event target; `deep` also covers non-clickable descendants, while interactive descendants remain excluded.
  implication: The initial empty marker would leave the traffic-light spacer and other title-bar child surfaces non-draggable, so the regression test must require `deep`.
- timestamp: 2026-07-18
  observation: The tightened renderer test failed with empty-versus-`deep` as expected, then passed 322/322 after the marker changed to `deep`; `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` also pass.
  implication: The nested drag-surface edge case is now regression-protected and the Rust change is formatted and warning-free.
- timestamp: 2026-07-18
  observation: Final `just fe-check` passes after the `deep` correction (322 renderer tests; 278 Rust tests passed, 3 existing credentialed/live tests ignored), `git diff --check` is clean, and manual diff review found only the intended three implementation/test files.
  implication: Deterministic verification and scoped review are complete; only real-display native interaction remains.
- timestamp: 2026-07-19
  observation: `just fe-smoke-cover-picker` built the WebDriver-enabled release binary successfully, then reported `Tauri Driver: tauri-driver not found`, followed by a WebDriver script timeout before the picker assertion ran. The process was stopped cleanly with Ctrl-C and exited 1; no picker opened and no media/config was selected or mutated.
  implication: The relevant native smoke is skipped due to missing test infrastructure, not failed product behavior; real-display verification remains required.
- timestamp: 2026-07-19
  observation: The patched `just fe-dev` build compiled and launched cleanly. Attempted macOS UI automation used unreliable physical-versus-CSS coordinates on the Retina display and closed the window through the traffic-light area, so the process was stopped cleanly without exercising drag or picker behavior.
  implication: Development launch is verified, but the automation attempt provides no valid evidence about the two interactive fixes; status must remain `awaiting_human_verify`.

## Eliminated

- hypothesis: The window is explicitly configured as non-resizable.
  reason: `tauri.conf.json` sets `resizable` to true.

## Resolution

- root_cause: The first Tauri port retained Electron-only window-drag CSS and used a synchronous blocking native folder picker. Tauri therefore had no drag-region marker on the custom title bar, while opening the folder dialog could block the desktop event loop before a usable picker was presented.
- fix: Added `data-tauri-drag-region="deep"` to the existing custom title bar and changed `dialog_open_folder` to await the callback-based non-blocking picker through a Tokio oneshot channel. Added renderer and Rust regression tests for both contracts.
- verification: TDD red/green confirmed for both regressions; `just fe-check` passes after final changes (322 renderer tests, 278 Rust tests passed, 3 pre-existing credentialed/live tests ignored); `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `git diff --check` pass. The patched `just fe-dev` build compiles and launches cleanly. `just fe-smoke-cover-picker` was attempted but skipped before test execution because `tauri-driver` is not installed, and Retina coordinate mismatch made a separate macOS automation attempt invalid; both processes were stopped cleanly without media/config mutation. Awaiting real-display user confirmation for dragging and native folder-picker interaction.
- files_changed: [frontend/src/components/TitleBar.tsx, frontend/test/components/TitleBar.test.tsx, frontend/src-tauri/src/commands/shell.rs]
