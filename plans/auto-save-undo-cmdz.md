# Plan: Auto-Save + Cmd+Z Undo (Remove Save/Revert Buttons)

## Context

The current UI has a confusing dual mechanism:
- **Manual "Save Changes" button** in MetadataEditor/BatchEditor — edits are NOT written until the user clicks it.
- **"Save" + "Revert" toolbar buttons** in TitleBar — "Save" clears dirtyTracks AND the undo history (destroying the ability to undo). "Revert" pops the undo stack and writes old values back.
- **Cmd+Shift+Z** is bound to revert but **Cmd+Z** is unbound.
- **Auto-tag** writes directly to disk but does NOT push undo snapshots, so auto-tag changes cannot be reverted.

The user wants a simpler model:
1. **Edits auto-save immediately** — no "Save" button needed.
2. **Cmd+Z undoes** even after tags are written (metadata editor changes AND auto-tag changes).
3. **"Save" and "Revert" buttons removed** from UI — keyboard undo replaces them.

## Approach

### Core idea

Remove the draft/save-gate from MetadataEditor and BatchEditor. On every field edit, debounce (800ms) then auto-write to disk with an undo snapshot pushed beforehand. Likewise, before auto-tag runs, snapshot all tracks that will be touched so Cmd+Z can revert the auto-tag operation.

The existing `UndoManager` and `handleRevert` already work correctly — they write old field values back to disk. The changes are:
1. Trigger saves automatically instead of waiting for a click.
2. Bind Cmd+Z instead of Cmd+Shift+Z.
3. Push undo snapshots before auto-tag.
4. Remove the Save/Revert buttons from TitleBar and the Save/Discard buttons from MetadataEditor/BatchEditor.

### What happens to dirtyTracks?

The `dirtyTracks` set is currently populated by `UPDATE_TRACK` (every write) and cleared by the toolbar "Save" button. Since auto-save means there's no "unsaved" state anymore, we can:
- Keep `dirtyTracks` around briefly for visual feedback during the debounce window (show "Unsaved" dot while debounce timer is active)
- Remove references to `dirtyCount` from TitleBar status display since everything is always saved

Actually, simpler: replace `dirtyTracks` with a local `isSaving` / `wasJustSaved` state in the editor components. The global `dirtyTracks` + "Save" button flow is what we're removing.

## Files to Modify

| File | Change |
|------|--------|
| `frontend/src/App.tsx` | Remove `handleSave`, simplify `handleRevert`; wire auto-tag to push undo snapshots; change keyboard shortcut from Cmd+Shift+Z to Cmd+Z; update TitleBar props |
| `frontend/src/components/MetadataEditor.tsx` | Remove Save/Discard buttons; auto-save fields on change with 800ms debounce; push undo snapshot before write |
| `frontend/src/components/BatchEditor.tsx` | Remove Apply button; auto-save fields with debounce; push undo snapshots before write |
| `frontend/src/components/TitleBar.tsx` | Remove Save/Revert toolbar buttons; remove dirtyCount references |
| `frontend/src/state/AppState.ts` | Remove `dirtyTracks`, `SET_DIRTY`, `CLEAR_DIRTY` from state/actions; remove `POP_UNDO` if unused after simplification; clean up |
| `frontend/src/state/UndoManager.ts` | No changes needed — already works correctly |
| `frontend/electron/handlers/auto-tag.ts` | No changes needed (writes remain the same on backend) |

## Reuse

- **`UndoManager.push()` / `UndoManager.pop()`** (`frontend/src/state/UndoManager.ts`) — existing, works correctly.
- **`handleRevert`** in `App.tsx` — existing, pops undo stack + writes old values + dispatches UPDATE_TRACK. Keep as-is.
- **`window.api.writeTrack()`** — existing IPC call, used by all save paths.
- **Debounce pattern** — the existing `debounce` helper in `main.ts` can be adapted as a local `useDebounce` hook, or we can use a simple `setTimeout`/`clearTimeout` in `useEffect`.

## Steps

- [ ] **1. Remove `dirtyTracks` from AppState**
  - Delete `dirtyTracks` field from `AppState` interface and `initialAppState`
  - Remove `SET_DIRTY` and `CLEAR_DIRTY` action types
  - Remove `dirtyTracks` mutations from `appReducer` (the `UPDATE_TRACK` case currently adds to `dirtyTracks` — remove that line)
  - Remove `POP_UNDO` action type (the revert dispatch can just use `PUSH_UNDO` + a re-render trigger, or keep a simpler `UNDO` action)

- [ ] **2. Auto-save in MetadataEditor**
  - Remove "Save Changes" and "Discard" buttons from JSX
  - Add a `useRef` debounce timer
  - On every `setField` call (when value changes), clear the previous timer and set a new one for 800ms
  - When debounce fires:
    1. Collect all draft fields that differ from original values
    2. Push undo snapshot (current values before the edit) to UndoManager
    3. Call `onSave(changedFields)` — the existing `handleSaveMetadata` which writes to disk
    4. Reset draft state
  - Show a small "Saving…" indicator while the write is in flight (using the existing `saving` prop)
  - Keep draft state for fast optimistic UI between debounce and write completion
  - Remove `onCancel` prop from MetadataEditor (no longer needed since there's no Discard button)

- [ ] **3. Auto-save in BatchEditor**
  - Remove "Apply to N files" button from JSX
  - Add debounce similar to MetadataEditor
  - When debounce fires:
    1. Push undo snapshots for all selected tracks
    2. Call `onSave(fields)` — the existing `handleBatchSave` which writes to disk
    3. Clear values

- [ ] **4. Push undo snapshots before auto-tag**
  - In `App.tsx` `handleAutoTag`, before starting the auto-tag task:
    - Determine which tracks will be touched (all tracks in the target album(s))
    - Build `TrackSnapshot[]` from the current state of each track
    - Dispatch `PUSH_UNDO` with description `"Auto-tag"` and the snapshots
  - This way, after auto-tag completes, user presses Cmd+Z → `handleRevert` writes all original values back

- [ ] **5. Change keyboard shortcut: Cmd+Z for undo**
  - In App.tsx keyboard `useEffect`:
    - Change `if ((e.metaKey || e.ctrlKey) && e.key === "z" && e.shiftKey)` → `if ((e.metaKey || e.ctrlKey) && e.key === "z" && !e.shiftKey)`
    - Keep `e.preventDefault()` and `handleRevert()`
    - Remove the old `Cmd+Shift+Z` binding (or leave it as redo if desired — but user didn't ask for redo)

- [ ] **6. Remove Save/Revert buttons from TitleBar**
  - Remove `onSave`, `onRevert`, `dirtyCount`, `canUndo` from `TitleBarProps`
  - Remove the ToolbarButton group that renders "Save" and "Revert"
  - Remove `dirtyCount > 0` status indicator in the right side
  - Simplify to just show track count / saving indicator
  - Update the `<TitleBar>` call site in App.tsx to omit the removed props

- [ ] **7. Clean up App.tsx**
  - Remove `handleSave` callback entirely
  - `handleRevert` stays as-is (used by keyboard shortcut)
  - Remove `showTrackContextMenu` and any other dead code related to `dirtyTracks` if present
  - Remove `onSave` prop from TitleBar and remove `onCancel` prop logic from MetadataEditor

- [ ] **8. Update tests**
  - `frontend/test/components/MetadataEditor.test.tsx` — update to test auto-save behavior instead of save-button click
  - `frontend/test/components/BatchEditor.test.tsx` — update similarly
  - `frontend/test/components/TitleBar.test.tsx` — remove tests for Save/Revert buttons
  - `frontend/test/state/app-reducer.test.ts` — remove dirtyTracks tests, update for removed actions

## Verification

1. **Auto-save in MetadataEditor**: Open a track, edit the title field. Wait 800ms. Verify the tag is written to the file (close app, reopen, see updated title).
2. **Cmd+Z undo**: After auto-save, press Cmd+Z. Verify the title reverts to its previous value in both UI and file.
3. **Multiple field undo**: Edit title and artist quickly (within 800ms). Wait for debounce. Press Cmd+Z. Verify both fields revert together.
4. **Auto-tag undo**: Run auto-tag on an album. After completion, press Cmd+Z. Verify all tracks revert to their pre-auto-tag values.
5. **Batch editor**: Select multiple tracks, type a common artist. Wait for debounce. Verify all selected tracks get the new artist. Press Cmd+Z — all revert.
6. **No Save/Revert buttons**: Confirm TitleBar no longer has Save/Revert buttons. Confirm MetadataEditor has no Save/Discard buttons.
7. **Regression**: Open library, navigate albums, switch tracks — no console errors.
8. **Unit tests**: `cd frontend && npm test` passes.
