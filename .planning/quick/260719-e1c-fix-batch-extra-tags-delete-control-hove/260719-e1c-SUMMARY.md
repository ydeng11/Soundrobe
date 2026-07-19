---
status: complete
commit: e4b7683
---

# Quick Task 260719-e1c Summary

Changed the Batch Extra Tags delete button from `transition-all` to `transition-colors`, so row hover visibility switches immediately while the button's color feedback remains animated.

Added a regression test covering the no-opacity-transition invariant across multiple rows.

Verification:
- `npm exec vitest -- run test/components/BatchExtraTagsEditor.test.tsx` — 24 passed
- `npm run typecheck` — passed
- `git diff --cached --check` — passed before commit
