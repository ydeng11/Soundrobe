# Plan: Upgrade Electron 34 → 37 → 40 → 42

## Solution Approach

Incrementally bump the `electron` devDependency in `package.json` through intermediate versions (37.x → 40.x → 42.x). At each stop:
1. Run `npm run rebuild:electron` to compile native modules for the new Electron ABI
2. Fix any TypeScript compilation errors (API deprecations)
3. Fix any `vite-plugin-electron` incompatibilities
4. Verify the full gate: typecheck + tests + dev launch
5. Commit the step

Related dependencies (`vite-plugin-electron`, `vite-plugin-electron-renderer`, native modules) are upgraded only when they block progression.

## Ordered Steps

### Step 0 — Baseline confirmation (skip if clean)
- Run `npm run typecheck` ✅
- Run `npm test` ✅  
- Run `npm run dev` briefly (verify app launches)
- `git log --oneline -3` — note current HEAD

### Step 1 — Electron 34 → 37.x
**Files touched:**
| File | Change |
|------|--------|
| `frontend/package.json` | `"electron": "^34.0.0"` → `"electron": "^37.0.0"` |
| `frontend/package-lock.json` | Auto-updated by npm install |

**Procedure:**
1. `cd frontend && npm install` (fetches Electron 37.x, updates lockfile)
2. `npm run rebuild:electron` (native modules compiled for Electron 37's ABI)
3. `npm run typecheck` — fix any TypeScript errors from API changes
4. `npm test` — fix any test failures
5. `npm run dev` — verify app launches (1-2 seconds, then Ctrl+C)
6. If blocking: skip to Step 2 (Electron 40) and see if it resolves there
7. `git add -A && git commit -m "Upgrade Electron 34→37"`

**Verification:**
- `npm run typecheck` exits 0
- `npm test` — all tests pass
- `npm run dev` — dev server starts, Electron window opens
- `node -e "require('better-sqlite3/build/Release/better_sqlite3.node')"` — ABI is Electron 37's

### Step 2 — Electron 37 → 40.x
**Files touched:** Same as Step 1, plus potentially:
- `vite-plugin-electron` if 0.29.x is incompatible with Electron 40
- `vite-plugin-electron-renderer` if version mismatch

**Procedure:** Same as Step 1, but target `"electron": "^40.0.0"`.

**Known risk — vite-plugin-electron compatibility:**
- Current: `vite-plugin-electron@0.29.1`
- If electron@40 + vite-plugin-electron@0.29 fails, upgrade to `vite-plugin-electron@1.0.2`
  - This is a 0.x → 1.x major bump. The API changed significantly in 1.x:
    - `electron()` plugin config structure changed (the array-based multi-entry config may need adjustment)
    - If incompatible, the fallback is to stay on 0.29 with Electron 40

**Verification:** Same as Step 1.

### Step 3 — Electron 40 → 42.x
**Files touched:** Same pattern.
**Procedure:** Same as Step 1, but target `"electron": "^42.3.3"`.

### Step 4 — Final check and commit
**Files touched:**
| File | Change |
|------|--------|
| `frontend/package.json` | Final electron dep at `^42.3.3` |

- Run full verification gate one more time
- If any deps were upgraded along the way, verify nothing regressed
- `git add -A && git commit -m "Upgrade Electron 40→42"`

## Risks / Open Questions

| Risk | Mitigation |
|------|------------|
| `vite-plugin-electron` 0.29 incompatible with Electron 37+/40+/42+ | Upgrade to 1.0.2 if it blocks. The configuration format changed — the array-based multi-entry (`[{entry: "main.ts"}, {entry: "preload.ts"}]`) is still supported in 1.x, just the import path may differ. |
| Electron 35+ removed deprecated APIs breaking `main.ts` | Fix any `tsc` errors at each step. Common areas: `Menu.setApplicationMenu` signature, `webContents` event signatures, `dialog` return types. Most are backward-compatible until 38+. |
| `electron-rebuild` (via npx) doesn't support Electron 42 | Electron 42 is a very recent release. If `electron-rebuild` hasn't been updated, install `@electron/rebuild@4.0.4` explicitly and use that. |
| The full ABI mismatch fix (`;` operator) works with Electron 42 | Already confirmed — the inline script is ABI-agnostic. `electron-rebuild` handles any target version. |
| Tests that use `better-sqlite3` directly (dataset.test.ts, conversation-logger.test.ts) | `pretest` in the `npm test` script handles this — it rebuilds for Node ABI before tests, then back to Electron ABI after. |
