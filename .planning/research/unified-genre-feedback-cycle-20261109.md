# First unified tagging feedback cycle

The existing coordinator now retains provider identity while marking an AI-inferred genre for review. This cycle establishes repeatable safety evidence. It does not increase measured source-grounded tagging coverage.

## Failure and decision

The coordinator already combines provider matching with optional genre inference. An accepted inference previously produced `Applied` with no assisted-field provenance or durable review checkpoint. The writer could also replace an existing per-file genre with the inferred album genre.

The hypothesis was that field-specific provenance and a prewrite review checkpoint would expose the uncertainty without discarding provider identity. The fix stays in that coordinator. It does not add a second tagger or a research architecture.

Native regressions first failed for the missing checkpoint and the overwritten `User choice` genre. Those failures are in `.amp/in/genre-native-red-2.log`. Renderer regressions also failed before their fixes. Display conversion and whitespace-only original genres have explicit label contracts.

## Frozen comparison

Both conditions use Ocean Eyes, case `fb0a478438a7`, from the reviewed development population. One unique physical track is evaluated 24 times per condition. The selection includes all three input profiles, cold and warm caches, and four assistance scenarios: accepted, rejected for low confidence, terminal service failure with HTTP 400, and disabled.

| Evidence | Before | After |
| --- | --- | --- |
| Directory under `.amp/in/artifacts/` | `unified-genre-baseline-2-20261109/` | `unified-genre-after-2-20261109/` |
| Frozen executable SHA-256 | `d9924e24c965a9fd57399cdc8ea55f81d1dde9632e3c7822ab22213712b35fac` | `f7ea2f4ac7ae8ae6f36be28f9ba182fd6d64637f5f05cc2f91d19ea96b13c776` |
| Evaluations | 24 | 24 |
| Missing review/provenance checks | 18 across six accepted evaluations | 0 |
| Provider requests | 212 | 212 |
| Scripted model requests | 18 | 18 |
| Missing fixtures | 0 | 0 |
| Grounded assisted accuracy credit | 0 | 0 |

The independent comparison checks all exported readback fields, apart from each run's disposable absolute file path. Every tag value is identical between conditions. The audio payload checks pass in both. Provider and model requests are identical, and 1,581 fixture and dummy-input hashes are unchanged. Only three locked source files differ: `auto_tag.rs`, `auto_tag_eval.rs`, and `server.rs`.

The six accepted evaluations now return `needs_review`, retain MusicBrainz authority, carry genre-only model/input-hash provenance, and retain a pending review flag after reopening the worker SQLite database. The other 18 evaluations retain ordinary provider success without an assisted-field marker. A low-confidence response or model service failure does not discard the provider match.

The comparison is reproducible with `.amp/in/artifacts/unified-genre-cycle-20261109/compare.py`. Its machine-readable result is `comparison.json` beside it. Each condition retains `snapshot.json`, `source.diff`, `source.tgz`, the frozen executable, request inventories, native logs, and original/readback values. The tracked procedure is in `scripts/auto-tag-eval-replay/README.md`.

## Evaluator correction and retained failed runs

The first baseline attempt caught a profile-specific title-capitalization mismatch in the strict scripted request fixture. That attempt remains in `unified-genre-baseline-20261109/` and is not comparison evidence.

The first changed-code replay found a separate evaluator defect. `readback_matches` required `Applied` even when a `needs_review` result had written correct tags. That replay remains in `unified-genre-after-20261109/` with six `write_readback` findings. A regression failed with this message: `review status must not turn correct written tags into a readback failure`. The corrected check still verifies every written track against its candidate and rejects deliberately wrong readback. Its red and green logs are `.amp/in/genre-readback-red.log` and `.amp/in/genre-readback-green.log`.

No oracle, provider receipt, version rule, title comparison, collaborator policy, or discovery allowance changed to obtain the clean result. The audit exits nonzero for non-clean results. Historical failed runs remain available.

## User-visible review

`AutoTagReview` labels each newly filled genre `AI-inferred · unverified`. It does not label a retained existing genre as inferred. The label survives **Keep changes**, including a configured Chinese display conversion. The explanation separates acknowledgment from verification. Written results requiring review no longer falsely say that no changes were written.

The actual component was rendered using native exported before/readback values at 1920×1080 and 390×844. Four inspected screenshots show pending and kept states:

:::gallery grid
![Pending, 1920×1080](file:///Users/ihelio/code/auto_tagger/.amp/in/artifacts/genre-review-desktop-pending-final.png)
![Pending, 390×844](file:///Users/ihelio/code/auto_tagger/.amp/in/artifacts/genre-review-mobile-pending-final.png)
![Kept, 1920×1080](file:///Users/ihelio/code/auto_tagger/.amp/in/artifacts/genre-review-desktop-kept-final.png)
![Kept, 390×844](file:///Users/ihelio/code/auto_tagger/.amp/in/artifacts/genre-review-mobile-kept-final.png)
:::

DOM checks show one genre marker, no false no-writes claim, a disabled Keep button after acknowledgment, and no horizontal overflow. Computed styles show the existing 12px semibold marker at 15.60:1 contrast against its row. Keyboard focus is visible. No animation was added. The mobile filename header wraps within its card.

This is component verification with an isolated DesktopAPI response built from native exports, not a packaged-app smoke. Only the displayed temporary path prefix was shortened. The native exporter independently tested persisted review state. The component used a stub Keep response, and the existing native Keep implementation preserves the review's evidence while changing its decision.

## Changed paths

- `src-tauri/src/commands/auto_tag.rs` adds genre-only provenance, review handling, per-file genre preservation, and regressions.
- `src-tauri/src/commands/auto_tag_eval.rs` adds explicit offline assistance configuration, before/readback and persisted-worker exports, and the readback-status regression.
- `src-tauri/src/server.rs` uses the same candidate review reason as desktop tagging.
- `src/components/AutoTagReview.tsx` and `test/components/AutoTagReview.test.tsx` implement and test the field-level explanation and acknowledgment behavior.
- `scripts/mock-provider-service.cjs` and `test/scripts/mock-provider-service.test.ts` add the bounded scripted genre endpoint.
- `scripts/auto-tag-eval-replay/run-assisted.cjs` and `audit-assisted.py` are the new repeatable driver and independent gate.
- `scripts/auto-tag-eval-replay/README.md` documents reproduction and its limits. `docs/CHANGELOG.md` records the observable fix.
- Local working records are `.planning/plans/2026-11-09-unified-genre-feedback.md` and this decision record. Runtime evidence remains under `.amp/in/`.

## Verification and delivery

`just --no-dotenv check` passed. Its desktop library ran 1,243 passing tests with 29 ignored. Its server library ran 709 passing tests with 17 ignored. Audio integration ran 14 passing tests with 12 ignored, and the recovery CLI ran five passing tests. The gate also passed typecheck, eight mock-provider tests, and ten review-tool tests.

The long gate's renderer phase ran before the last display edge-case tests and passed 854 tests. A fresh final-source renderer run passed all 856 tests, including seven review component tests. Typecheck and the 18 mock/review-tool tests were rerun and passed. Final-source desktop checks passed 14 readback tests and 12 genre tests, with one gated genre smoke ignored. This also rechecked the evaluator correction after the desktop phase of the long gate. Focused server checks passed ten readback tests and seven genre tests.

The frozen suite check passed for 378 cases and 3,506 tracks. Replay archive hashes, script syntax, and diff whitespace checks passed. The source diff was reviewed for unused code, duplicated branches, and scope changes. No matching or version policy changed. Credentialed, packaged-app, and real-display smokes were not selected. Strict Clippy was not rerun.

Evidence logs are `.amp/in/unified-genre-check.log`, `genre-final-web.log`, `genre-final-tools.log`, `genre-final-typecheck.log`, `genre-desktop-final-readback.log`, and `genre-desktop-final-genre.log`. The final source, UI, screenshot, and comparison hashes are in `.amp/in/artifacts/unified-genre-cycle-20261109/final-source-ui-hashes.json`. `tool-versions.json` beside it records Node, Python, Rust, agent-browser, and browser versions. The temporary preview sources were removed. The Vite verification server remains on port 5173.

The same checkout and branch, `artist-library-inventory`, remain in use. This continuation is uncommitted and unpushed. The previous checkpoint series was already pushed through [d7659077](https://github.com/ydeng11/Soundrobe/commit/d7659077a5030c5ca627d99b6e967343d3597e28). No PR, deployment, paid model call, or original-library write occurred. Existing raw logs, dummies, the unrelated iOS archive, and `scripts/__pycache__/` remain untouched.

## Limits and next development selection

The scripted genre is not independently grounded provider evidence. Its confidence is a controlled test input, not a correctness score. This cycle earns no additional useful-tagging coverage or live-model accuracy credit. One inspected development release is not generalization evidence.

The full 51-case provider matrix was not rerun. Bellyache's standalone year, Enya remaster grounding, missing Vince Staples credits, and the Gaelic-title oracle finding remain unresolved. No untouched holdout was used for tuning. Cancellation, live research, agent-discovered releases, and new bilingual aliases are outside this diagnostic selection.

The review dialog and full undo remain session-scoped. Worker provenance and the pending flag persist, but restart-safe review/undo and the existing acknowledgment-to-verification coupling are not repaired here. The next cycle should widen the independently reviewed safety selection before architecture expansion. A paid or live model evaluation still requires explicit approval.
