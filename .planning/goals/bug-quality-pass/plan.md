# Bug/Quality Fix Pass — Plan

## Context

The Electron app has several concrete bugs and code quality issues discovered during systematic audit:

1. **Windows path handling (critical)** — 10 renderer sites split on "/" without normalizing backslashes, breaking on Windows
2. **Reducer mutation (moderate)** — `PUSH_UNDO` mutates UndoManager instead of using a pure pattern
3. **Cancel emission gap (moderate)** — `cancelTask()` doesn't emit a "cancelled"-type event directly (only via signal path)
4. **Test setup duplication (minor)** — Env isolation duplicated across describe blocks in `auto-tag.test.ts`
5. **Missing path utility tests** — New shared utility needs its own unit tests

## Approach

Phase 1 adds a shared `toPosixPath()` utility and fixes all 10 renderer sites. Phase 2 fixes the reducer mutation and cancel emission. Phase 3 deduplicates test code and adds new tests. Each phase has its own verification step.

## Files to Modify

| File | Phase | Change |
|---|---|---|
| `frontend/src/utils/path.ts` (new) | 1 | Create `toPosixPath()`, `basename()`, `dirname()` utils |
| `frontend/src/App.tsx` | 1 | Replace inline `split("/")` at lines 23, 491, 529, 634, 909, 1431, 1494 |
| `frontend/src/components/FileGrid.tsx` | 1 | Fix `shortPath()` line 708 |
| `frontend/src/components/FolderTree.tsx` | 1 | Fix library label line 213 |
| `frontend/src/components/MetadataEditor.tsx` | 1 | Use shared util instead of inline replace |
| `frontend/src/components/ExtraTagsEditor.tsx` | 1 | Use shared util instead of inline replace |
| `frontend/src/state/UndoManager.ts` | 2 | Add `cloneAndPush()` method |
| `frontend/src/state/AppState.ts` | 2 | Use `cloneAndPush()` in PUSH_UNDO reducer |
| `frontend/electron/handlers/auto-tag.ts` | 2 | Emit "cancelled" event type in `cancelTask()` |
| `frontend/test/handlers/auto-tag.test.ts` | 3 | Extract shared test setup helper |
| `frontend/test/utils/path.test.ts` (new) | 3 | Unit tests for path utilities |

## Reuse

- The `MetadataEditor.tsx:24` and `ExtraTagsEditor.tsx:33` already use `replace(/\\/g, "/")` — the shared util replaces this inline pattern
- `UndoManager.push()` already exists; `cloneAndPush()` returns `new UndoManager` with the operation already pushed
- `auto-tag.test.ts` has identical `beforeEach`/`afterEach` in two describe blocks — extract into a function

## Steps

### Phase 1: Windows path fix

- [ ] **1.1** Create `frontend/src/utils/path.ts` with:
  - `toPosixPath(p: string): string` — replaces `\` with `/`
  - `dirname(p: string): string` — uses `toPosixPath` then split/join
  - `basename(p: string): string` — uses `toPosixPath` then split/pop
  - `shortPath(p: string, depth?: number): string` — for FileGrid's `-4` pattern
  - `parentPathSet(paths: string[]): Set<string>` — for album detection (line 491 pattern)
- [ ] **1.2** Fix `App.tsx`:
  - Line 23: `dirPath()` → use shared `dirname()`
  - Line 491: `t.path.split("/").slice(0, -1).join("/")` → use shared utility
  - Lines 529, 634, 1431: `albumPath.split("/").pop()` → use shared `basename()`
  - Line 909: `track.path.split("/").pop()` → use shared `basename()`
  - Line 1494: `state.selectedTrack.path.split("/").pop()` → use shared `basename()`
  - Remove local `dirPath()` function
- [ ] **1.3** Fix `FileGrid.tsx` line 708: `shortPath()` → use shared `shortPath()`
- [ ] **1.4** Fix `FolderTree.tsx` line 213: `libraryPath.split("/").pop()` → use shared `basename()`
- [ ] **1.5** Fix `MetadataEditor.tsx` line 24: inline replace → shared `basename()`
- [ ] **1.6** Fix `ExtraTagsEditor.tsx` line 33: inline replace → shared `basename()`

### Phase 2: Reducer & Cancel emission

- [ ] **2.1** Add `cloneAndPush(description, snapshots)` to `UndoManager.ts` — returns a new UndoManager with the operation pushed
- [ ] **2.2** Update `AppState.ts` `PUSH_UNDO` case to use `cloneAndPush` instead of mutation
- [ ] **2.3** Update `auto-tag.ts` `cancelTask()` to emit a "cancelled" event type directly (in addition to the `updateTask` status set)

### Phase 3: Test improvements

- [ ] **3.1** Extract env setup/teardown helper in `auto-tag.test.ts`
- [ ] **3.2** Create `test/utils/path.test.ts` with tests for `toPosixPath()`, `dirname()`, `basename()`, `shortPath()`

## Verification

Each phase verified independently before moving to the next:

### Phase 1 verify
```bash
cd frontend && npm run typecheck && npm test -- run test/utils/path.test.ts
```

### Phase 2 verify
```bash
cd frontend && npm run typecheck && npm test -- run test/state/
```

### Full verify
```bash
cd frontend && npx tsc --noEmit && npm test && git diff --check
```
