# Goal — Improve save+reload UX

Eliminate unnecessary disk re-reads during sidebar navigation (album clicks, "All Files") and after save operations. Sidebar navigation becomes instant (in-memory filter only). After save, only modified albums are re-read. A manual refresh button + `Cmd+R` triggers full re-scan on demand.

**Shared understanding:** `goals/improve-reload-ux/facts.md`

**Execution plan:** `goals/improve-reload-ux/plan.md`

**Done when:**
- Clicking an album or "All Files" shows tracks instantly with no progress bar
- A manual refresh button exists and full re-scan works on click or `Cmd+R`
- After single-track save, no reload occurs (optimistic update is sufficient)
- On save failure, rollback is skipped if the user made further edits
- After auto-tag/audit, only modified albums are re-read from disk
- All 100+ existing tests pass
- New tests verify the filtering, rollback guard, and refresh button behavior
