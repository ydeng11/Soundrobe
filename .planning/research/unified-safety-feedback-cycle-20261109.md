# Broader offline tagging safety cycle

## Evidence boundary

This continuation uses the existing provider and assistance coordinator, real native writers and readers, and disposable silent FLACs. It does not run a live model or change the research architecture. Scripted answers receive zero grounded-assistance accuracy credit.

The selection contains 31 unique physical files in seven already inspected development cases. Twelve scenarios, three input profiles and separate cold/warm media request 72 evaluations and 246 physical-file evaluations per complete run. These are repeated evaluations, not 246 independent recordings. Neither these cases nor the earlier inspected partitions are untouched holdouts.

`scripts/auto-tag-eval-replay/safety-selection.json` fixes field expectations before this cycle's tuning. Its source review is independent of model answers and native outputs. No new human or separate-reviewer sign-off is claimed. Existing review manifests and title, performer and version oracles remain unchanged.

The source review covers two Guitar Songs tracks with unequal local genres, the Billie Eilish and ROSALÍA duet, nine dont smile at me tracks including Vince Staples, my future with missing release duration, reciprocal studio/remix negatives, and sixteen Enya compilation tracks with fourteen unsupported remaster claims. Each source receipt is named in the selection manifest. Explicit local genres are controlled inputs, not claims about the original library.

## Controls and failures retained

Every process uses a fresh cache. Its warm phase reuses that cache with separate, newly prepared media. Provider budgets remain production defaults. A process allows at most two scripted model requests. Authentication headers and real credentials are not captured.

The two wrong-release negatives deliberately expose only the incompatible captured detail. Their discovery wrappers and exact supplemental queries are scripted counterfactual visibility, not authentic provider responses. Once providers reject the detail, metadata assistance receives an exact-payload terminal HTTP 400 response. That checks safe rejection and retention, not an accurate model answer. The remaster case receives an explicitly adversarial, high-confidence ordinary-title response with no remaster evidence. Its no-write expectation is unchanged.

All failed runs remain under `.amp/in/artifacts/`. The first broader baseline exposed missing negative queries, unsupported metadata request schemas, an evaluator panic on cancellation, and serialization drift between the selection file and its copied hash. The second run completed 72 evaluations but still had missing metadata-rejection controls and selection-copy drift. Neither run is a clean safety baseline. Native cancellation evidence must be an object, not JSON null. Held worker rows and unacknowledged review flags are separate persisted mechanisms, so the export now records both.

## Observed production defects and bounded correction

The baseline reproduced loss of local genres on content-only output. That early coordinator branch bypassed genre assistance and the writer explicitly cleared unknown genre. Ordinary provider genres could also overwrite explicit per-file choices. An older integration assertion deliberately required this clearing. The user's newer requirement to preserve per-file decisions supersedes that assertion; the replacement must retain distinct values and still honor explicit genre cleanup.

The correction is limited to this field-authorization defect. Content-only output now reuses the existing genre assistance and preserves file-bound local genres, assisted provenance and review obligations. Edition identity remains unresolved. Rejection, service failure and disabled assistance leave provider-backed basic tags useful. Cancellation after genre inference stops before any write. This is not in-flight HTTP or mid-write recovery evidence.

The broader baseline also reproduced unsupported metadata writes on the Enya negative. The high-confidence scripted response wrote sixteen files in each existing-tag profile despite missing remaster support. Pending review and intact payloads do not authorize those writes. The known Vince Staples omission also remains a safety failure. Neither oracle may be broadened to accept these results.

## Final results

The complete comparison uses `unified-safety-baseline3-20261109/` and `unified-safety-after-20261109/` under `.amp/in/artifacts/`. Each condition executes all 72 evaluations and 246 physical-file evaluations. The audit finds 119 failed checks across 55 evaluations before the correction, and 12 failed checks across eight evaluations after it. These are checks and repeated evaluations, not counts of independently corrected recordings.

| Audited finding category | Before | After |
| --- | ---: | ---: |
| Field authorization | 72 | 8 |
| Persisted review/highlight | 14 | 0 |
| Missing or extra agent requests | 11 | 0 |
| Assisted provenance | 10 | 0 |
| Cancellation | 8 | 0 |
| Collaborator credit | 4 | 4 |

Both conditions have zero fixture misses, input-integrity failures, physical-file mapping failures, native write/readback failures, payload changes, and cold/warm disagreements. All 1,614 locked inputs and expectations are identical. Per-job provider manifests and request observations are identical, with 849 provider requests per condition. Only the coordinator, its collection regression file and the native evaluator differ in the source snapshots. Non-genre readback metadata remains identical outside the deliberately changed cancellation outcomes.

Scripted model requests increase from 26 to 48 because content-only output now reaches existing assistance. Each process retains its two-request ceiling. No additional provider allowance, real model, token measurement or model cost is involved. Grounded-assistance accuracy credit remains zero.

The final accounting separates 44 written evaluations satisfying the reviewed checks, fourteen held evaluations without writes, six prewrite cancellations, and eight evaluations with findings. Native status alone would misleadingly report 34 `applied`, 32 `needs_review` and six resolver errors. The last six are the deliberate cancellations. A correct abstention does not count as useful tagged completion.

The independent comparator is `.amp/in/artifacts/unified-safety-cycle-20261109/compare.py`. Its `comparison.json` verifies comparable inputs, budgets, exact per-job provider manifests, provider observations, payloads, absence of new findings and unchanged non-genre metadata. `coverage.json` records the final accounting. Each run retains its executable, source archive and diff, source and input locks, exact scripted controls, raw request inventories, native logs, field ledger and reopened worker evidence.

Two regressions failed before production changes. The existing writer replaced `User choice` with `Pop`, and content-only output cleared `Rock`. Red logs are `.amp/in/safety-genre-native-red.log` and `safety-content-native-red.log`. Both pass after the correction, including an explicit genre-cleanup counterexample and unchanged wrong-title, wrong-version and wrong-duration negatives. A separate evaluator regression first rejected a correctly retained per-file genre. Its correction still rejects a deliberately overwritten genre rather than dropping genre validation. Readback checks pass ten server tests.

The earlier Ocean Eyes diagnostic was rerun fresh against the final native source. All 24 evaluations pass its independent audit, with no fixture misses and zero grounded-assistance accuracy credit. Its evidence is `unified-safety-previous-success-20261109/`.

## Remaining failures and coverage limits

- Four `dont smile at me` evaluations, the two existing-tag profiles cold and warm, still omit Vince Staples from `&burn`. The reviewed source requires that credit. The selected provider's omission remains a failure.
- Four Enya evaluations, the two existing-tag profiles cold and warm, still write all sixteen files from the scripted ordinary-title response without source-bound remaster support. Eight failed checks record altered bytes and altered tags. These are 64 repeated file writes over sixteen unique physical files, not 64 different tracks. Review flags do not make these writes safe. The audit exits **1** and remains a release blocker for a clean safety verdict.
- The filename-only Enya path holds without writes after discovery-budget exhaustion. That result does not isolate the remaster veto. The reciprocal wrong-version negatives also combine title, duration and known filename-year parsing effects, so they do not isolate each guard.
- Scripted service failures exercise terminal HTTP 400 rejection, not transient retries, throttling or transport recovery. Cancellation is injected after genre inference, before writing; it does not cover cancellation during a request or write.
- No new artist-family holdout, bilingual alias gap, mixed-disc or partial-album safety sample, live research, acoustic identity, in-flight cancellation or crash recovery was evaluated. The corpus remains artist-concentrated. The complete reviewed 51-case matrix was not rerun.
- Field ledgers record provenance claims and source references, not blanket semantic verification of every field. The content-only representative-year policy remains an explicitly unresolved risk. Broader acknowledgment and restart-safe undo behavior is unchanged.

The next bounded safety task is preventing metadata assistance from bypassing unsupported version evidence. This report does not claim that every physical track is now usefully or safely tagged.

## User-visible review

The actual `AutoTagReview` component was rendered from native exported fields. DOM checks cover mixed retained/inferred genres, unequal retained genres, provider-backed duet credits with a retained local genre, held wrong-version output, and cancelled output. Only the newly filled second-file genre carries `AI-inferred · unverified`; the first file retains `User acoustic` without that marker. The marker survives **Keep changes**, and the Keep button becomes disabled. The held case shows no writes and retry/manual-search controls. Cancelled files show zero changed fields and no applied-assistance marker.

The pending desktop and acknowledged mobile views were inspected at 1920×1080 and 390×844. Neither has horizontal overflow. The unchanged marker computes to 12px, weight 600. The baseline capture shows the local genre being lost. Its native field values were checked to be identical to the compared third baseline, apart from disposable paths.

:::gallery side-by-side
![Before: local genre lost](file:///Users/ihelio/code/auto_tagger/.amp/in/artifacts/unified-safety-cycle-20261109/content-before.png)
![After: retained local choice and highlighted fill](file:///Users/ihelio/code/auto_tagger/.amp/in/artifacts/unified-safety-cycle-20261109/content-after-desktop.png)
:::

:::gallery side-by-side
![Acknowledged mobile inference remains unverified](file:///Users/ihelio/code/auto_tagger/.amp/in/artifacts/unified-safety-cycle-20261109/content-after-mobile-kept-final.png)
![Wrong-version output held without writes](file:///Users/ihelio/code/auto_tagger/.amp/in/artifacts/unified-safety-cycle-20261109/held-negative-desktop.png)
:::

These are component checks with a stub DesktopAPI response and stub acknowledgment, not a packaged-app or desktop navigation smoke. SQLite reopen checks run independently in the native evaluator. Dialog, error and undo lifecycle limits remain. DOM exports, screenshot files and preview reconstruction sources are retained in `unified-safety-cycle-20261109/`. Temporary preview routes and data were removed. Vite remains on port 5173. Failed browser-command attempts were retained and replaced by successful semantic-locator checks before screenshots were accepted.

## Reproduction and changed paths

The maintained replay commands are in `scripts/auto-tag-eval-replay/README.md`. Run the safety driver into a fresh directory, then run `audit-safety.py`. A complete run can have driver exit 0 and safety-auditor exit 1. Never suppress or reinterpret that distinction. For this comparison, run `python3 -B .amp/in/artifacts/unified-safety-cycle-20261109/compare.py .amp/in/artifacts/unified-safety-baseline3-20261109 .amp/in/artifacts/unified-safety-after-20261109`.

This continuation changes these paths:

- `src-tauri/src/commands/auto_tag.rs`, `auto_tag_collections_tests.rs` and `auto_tag_eval.rs` implement the bounded genre correction, regressions, offline test inputs, cancellation evidence and separate persisted review exports.
- `scripts/mock-provider-service.cjs` and `test/scripts/mock-provider-service.test.ts` extend strict bounded replay to metadata schemas and disposable path normalization.
- `scripts/auto-tag-eval-replay/run-safety.cjs`, `audit-safety.py` and `test_safety_audit.py` provide the new repeatable matrix and independent field gate.
- `scripts/auto-tag-eval-replay/safety-selection.json`, `safety-negative-queries.json` and `safety-model-inputs.json` retain pre-tuning expectations and explicitly scripted controls.
- `scripts/auto-tag-eval-replay/README.md` and `docs/CHANGELOG.md` document reproduction, limits and observable genre behavior.
- Local working records are `.planning/plans/2026-11-09-unified-safety-expansion.md` and this report. Runtime and review evidence remains under `.amp/in/`.

The incoming milestone is preserved in `unified-safety-cycle-20261109/incoming-milestone.tgz` and its manifest. Its UI, server bridge, first-cycle replay scripts and renderer regression file remain byte-identical. No new UI source change was needed.

## Verification and delivery

Fresh `just --no-dotenv check` exited 0. Renderer tests pass 856/856; the desktop Rust library passes 1,244 with 29 ignored; the server library passes 710 with 17 ignored. Server integration checks pass fourteen inventory-audio and five recovery-CLI tests, with twelve inventory-audio tests ignored. Mock and review-tool suites pass nine and ten tests. These counts describe suite executions, not independent behaviors; shared Rust tests run under both feature sets. Ignored native/live/credentialed tests are not claimed as executed. The complete log is `.amp/in/safety-full-check.log`.

Targeted checks also pass the new native regressions, ten readback tests, five auditor contracts, seven review component tests, typecheck, the 378-case/3,506-track frozen-suite check, script syntax and diff whitespace checks. Final source and input hashes match the final replay. The final independent comparison exits 0 for comparability; a fresh final safety audit still exits 1 with the twelve findings above. Its log is `unified-safety-cycle-20261109/final-audit.log`. A passing deterministic gate does not override the failed safety verdict.

Work remains local, uncommitted and unpushed on `artist-library-inventory` in the same checkout. The pushed base is still [d7659077](https://github.com/ydeng11/Soundrobe/commit/d7659077a5030c5ca627d99b6e967343d3597e28). No PR, deployment, live/paid model, original-library write, new thread or worktree was used. Existing frozen evidence, generated dummies, failed attempts, the unrelated iOS archive and `scripts/__pycache__/` were not removed or changed.
