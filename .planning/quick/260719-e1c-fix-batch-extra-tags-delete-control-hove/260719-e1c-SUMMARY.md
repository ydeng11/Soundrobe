---
status: complete
commit: 76b5565
---

# Quick Task 260719-e1c Summary

Changed Batch Extra Tags delete visibility from CSS-only hover behavior to one editor-level active-row state. Entering a row now deactivates every other row, and leaving a row or the tag list clears the delete control even if the native hover pseudo-state sticks.

Added a regression test covering activation, transfer to another row, and mouse-leave cleanup.

Verification:
- `npm exec vitest -- run test/components/BatchExtraTagsEditor.test.tsx` — 24 passed
- `npm run typecheck` — passed
- `git diff --cached --check` — passed before commit
