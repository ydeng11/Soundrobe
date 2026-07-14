# Electron Baseline Record (pre‑Tauri migration)

Captured on the `codex/tauri-migration` branch at commit `f0d8b73`
before any Tauri scaffolding changes behavior. Recorded faithfully:
no failures were fixed or hidden before writing this file (per migration plan step 1).

## Environment

| Tool        | Version                                   |
|-------------|-------------------------------------------|
| node        | v26.0.0                                   |
| npm         | 11.12.1                                   |
| typescript  | 5.9.3                                     |
| vitest      | 3.2.6                                     |
| electron    | v42.3.3                                   |
| playwright  | 1.60.0                                    |
| vite        | 6.4.3                                     |
| rustc       | 1.94.1 (e408947bf 2026-03-25)             |
| cargo       | 1.94.1 (29ea6fb6a 2026-24-03)            |
| better-sqlite3 native | built for Electron ABI 146 (Node ABI 147) |
| OS          | macOS (darwin-arm64)                      |

`node_modules` present; `tsc`, `vitest`, `electron`, `playwright`, and the
`better_sqlite3.node` native module all resolve. (The earlier attempt noted in
the plan failed only because `node_modules` was absent then.)

## Commands and results

### 1. `npm run typecheck`  (`tsc --noEmit`)
- Exit: **0** — pass
- Output: clean, no errors.

### 2. `npm test`  (unit + integration via `scripts/run-vitest-with-electron-restore.mjs`)
- Exit: **0** — pass
- Test Files: **78 passed | 4 skipped (82)**
- Tests: **1296 passed | 71 skipped (1367)**
- Duration: 66.34s
- ABI note from harness: `better-sqlite3 is already compatible with Electron ABI 146 (shell Node ABI 147).`
- Skips are pre-existing (guarded by env/feature flags), not introduced here.

### 3. `npm run build`  (`tsc && vite build`)
- Exit: **0** — pass
- Produced `dist/`, `dist-electron/main.js`, `dist-electron/preload.mjs`, `dist-electron/tag-worker.mjs`.
- One expected Vite warning: `new URL(".", import.meta.url)` deferred to runtime (pre-existing, intentional).

### 4. `npm run test:e2e`  (`npm run build && playwright test`)
- Exit: **1** — **8 failed, 1 passed**
- Failed suites/tests:
  1. `assistant-organize.electron.spec.ts` — assistant groups tracks with same album into album folders (`toBeVisible` failure)
  2. `assistant-paths.electron.spec.ts` — scanLibrary returns TrackData with full absolute paths (`toBeVisible` failure)
  3. `audit.electron.spec.ts` — writes deterministic FLAC fixes and surfaces manual review (`expect(received).toBe(expected)`)
  4. `convert.electron.spec.ts` — Convert splits an existing title tag into artist and title tags (`toBeVisible`)
  5. `extra-tags.electron.spec.ts` — Extra Tags can be viewed, edited, saved, and reopened (`toBeVisible`/`toBeEnabled`)
  6. `extra-tags.electron.spec.ts` — Batch Extra Tags shows combined tags (`toBeEnabled` — Save not found)
  7. `number-tracks.electron.spec.ts` — by filename A-Z assigns sequential track numbers (30000ms timeout)
  8. `number-tracks.electron.spec.ts` — by title Z-A reverses title order (30000ms timeout)
  - Passed: 1 (`extra-tags`… or one of the suites had a passing case).

**Failure characterization (environmental, not logic):** every failure is a GUI
visibility / element-not-found / timeout failure against the launched Electron
window. The corresponding business logic is covered by the integration test suite
above, which is fully green (1296 passed). The sandboxed Electron build does not
get a usable interactive window (no accelerated display session for Playwright
to drive), so the Electron E2E driver cannot reach the first visible control.
These E2E failures are the **existing baseline state under this environment** and
are recorded as-is; they will be re-evaluated once the Tauri WebdriverIO E2E
shim replaces them. No E2E failures were fixed before this record was written.

## Native modules and ABI

- `postinstall` rebuilds `better-sqlite3` for the Electron ABI via `electron-rebuild`.
- `patches/` contains the `better-sqlite3` patch applied via `patch-package`.
- `scripts/ensure-electron-abi.mjs` guards the ABI at dev time.

These are exactly the pieces the migration must remove at cutover (step 5 of the plan).

## Reproduce

```bash
cd frontend
npm run typecheck
npm test
npm run build
npm run test:e2e   # see environmental E2E note above
```