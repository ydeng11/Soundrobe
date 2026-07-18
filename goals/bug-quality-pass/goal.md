# Bug/Quality Fix Pass

Systematically fix bugs and code quality issues in the Soundrobe Electron app: Windows path handling in the renderer (10 sites), reducer mutation anti-pattern, cancel emission timing gap, and test setup duplication.

- **Facts**: `goals/bug-quality-pass/facts.md`
- **Plan**: `goals/bug-quality-pass/plan.md`

## Done Condition

All of the following are true:
1. `frontend/src/utils/path.ts` exists with `toPosixPath()`, `dirname()`, `basename()`, `shortPath()`
2. All 10 renderer `split("/")` sites across `App.tsx`, `FileGrid.tsx`, `FolderTree.tsx` use the shared utility
3. `MetadataEditor.tsx` and `ExtraTagsEditor.tsx` use the shared utility instead of inline `replace`
4. `UndoManager.cloneAndPush()` exists and `PUSH_UNDO` uses it without mutation
5. `cancelTask()` in `auto-tag.ts` emits a "cancelled"-type event directly
6. `auto-tag.test.ts` test setup uses a shared helper, no duplication
7. `test/utils/path.test.ts` has unit tests for path utilities
8. `npm run typecheck` passes
9. `npm test` passes (557 tests, same 2 skips)
10. `git diff --check` passes
