# Bounded-storage integration review

## Current frozen result171

Frozen native source155, tools169 and server-tests155 passed the maintained six-job workload in run171. Audit172 accounts for all 216 requested slots, twelve closed sessions and 497 arrived/completed operations. Original sequential handle64170 settled zero. Helper169 passes seven with no skips; faults170 passes eleven controls. Final independent read-only evidence review supports trusted mock-only bounded-storage integration acceptance, with no unresolved blocker in that scope. The reviewer independently matched all seven native snapshots, both script snapshots, nine frozen hash entries and all 1,666 receipt bindings. It reconstructed the exact 329,299-byte split-binding JSON and mutable state, independently replayed every ledger record, and checked publication/checkpoint/result/observation links. Supervisor exit preceded acceptance by 30 ms. The review ran only read-only forensic scripts, not gates or workloads.

Reservations include **321,297,203 growth bytes and 502,354,528 cumulative application/VFS-write bytes**, including the unchanged 2-MiB/8-MiB terminal grant. All 2,505 records and 204 owner claims reconcile. No ticket refund, cap increase or net-file-length substitution occurred. Preparation was 215,685 ms of 300,000 ms. Acceptance was 445,700 ms into the 600,000-ms execution scope, after observed supervisor exit. Native receiver and launcher exited zero. Audit172 stderr is empty; frozen172-verified and final174 retain the hash and static-check results.

The final fixture correction parks the driver after owner-lease acknowledgement and before preparation writers. A separate report confirms ownerLive=true and empty sessions before the external reserved writer lock. Journal167-red lacks the required boundary and fails; journal168-green passes. Same-model independent review accepts the isolation. The retained journal-entered marker proves pre-transaction dispatch under the lock, not independently confirmed SQLite contention or a valid completed-preparation Freeze. Faults163's exact historical scheduling is not reconstructed.

Check157 passed TypeScript, 858 renderer tests, 1,361 desktop tests with 38 ignored, 861 server tests with 29 ignored, fourteen audio tests with twelve ignored, five recovery CLI tests, twelve mock-provider tests and ten review-tool tests. Native source did not change afterward. The later script controls are covered by helper169, faults170 and run171. Final174 passes Rust formatting, whitespace, changed CJS syntax, Python AST parsing and frozen executable/tool hashes. No credentialed or display smokes ran.

Process-dispositions173 records fourteen completed roots with all recorded actors/groups absent by ESRCH. For run171, receiver64805, supervisor64806, anchor/group64807, broker64808 and driver64809 are absent. No expected slow run was killed for startup delay. Faults170's anchor-running-loss intentionally records unknown containment at launcher return, then observes its finite orphan exit. The other ten controls establish actual group absence. Storage-denial additionally requires actual source-bound quota-denial evidence, denied-target absence and all 216 unavailable slots with unknown effects.

All twenty-five intended changed repository paths are listed below and remain uncommitted on artist-library-inventory above6894e38f. The only additional local records are the hidden plan and evidence. No commits, pushes, fetches, OS changes, restart, host cleanup, live requests, private-config or credential activity, or original-library operations occurred. Raw evidence, generated media, unrelated archive and Python cache remain retained. No persistent startup blocker or concrete design impossibility is currently established.

This validates trusted mock-only infrastructure fit, not useful tagging of all 216 slots. Grounded assistance credit remains zero; user agreement remains0/22. Physical-device I/O bounds, malicious-process isolation, arbitrary host/power-loss durability, live-provider safety and model quality remain unproved. Run80 remains genuine cap-denial evidence, never fit. Every failed, interrupted or unfrozen historical gate remains in its original disposition.

## Historical source128 review

The remaining sections preserve the earlier checkpoint. Their open preflight/handle findings were subsequently corrected and reviewed through source155; they are not current-source blockers.

## What the independent review checked

The same-model, read-only reviewer inspected the complete integration diff, untracked storage boundaries, retained native executable, run123 receipts, split Journal state and accounting ledger. It launched no gates or workloads and changed no files.

- Native storage and ownership matched retained117 source snapshots. The executable matched its receipt digest.
- It verified 116 relevant source-lock entries without mismatches. The full 132-entry manifest was an exact prefix of the 1,666-entry receipt binding. The 1,534 additional fixture bindings matched. It did not independently inspect the remaining label and corpus inputs. The executed maintained auditor separately checks every receipt binding.
- Split immutable binding JSON matched its stored SHA256 and provisional receipt. Mutable state contained twelve sessions and 497 arrived/completed operations.
- Independent replay of all 2,506 ledger records found no bad hashes, identity/counter disagreement, ambiguous unused records, or stopped bytes. Reservations matched audit124: 321,297,203 growth bytes and 502,346,336 cumulative write bytes, including the original terminal grant. There were 204 valid owner claims.
- Receiver/result/provisional/launcher hash links matched. Preparation was 215,317 ms. Acceptance was 445,809 ms into execution, after observed successful supervisor exit and before the unchanged 600-second deadline. Launcher evidence recorded receiver exit zero.
- Authority117's 94 passes/nine ignored, helper118's seven passes, and successful check122 are supported by retained logs. The review did not rerun them.

## Accepted corrections and their limits

Directory117 guards close reproduced recursive deletion through child/.. and reserved-owner directory creation. Negative finalization observes actual group absence within the remaining original eight-second fixture bound; positive success still requires immediate absence. BEGIN IMMEDIATE preserves report reads while blocking the intended writer, with a required journal-entered witness. Faults121 passed eleven controls. Ten establish group absence. Anchor-running-loss deliberately records unknown containment at return.

The reviewed path includes bootstrap, Journal/cache/worker SQLite, FLAC/staging/aliases, FFmpeg pipe capture, Node/Python output, logs, and partitioned terminal publication. This is source/workload evidence for the maintained mock path, not isolation against malicious processes or arbitrary host corruption. Application/VFS reservation accounting is not physical-device I/O. Power-loss durability, live-provider safety, and model quality remain unproved.

## Cleanup failure finding and source128

The reviewer found that remove_file and remove_dir_all returned delegated errors without closing the stop latch. The real FLAC fallback discards temporary-file cleanup errors. An otherwise successful or skipped result could therefore retain open ordinary admission after EIO or PermissionDenied. Caps still held, but the fail-stop contract did not.

Cleanup126-red reproduced the first bypass. Cleanup127-red exercised all three public-facade swallowing controls and failed with remove-file, remove-directory, and create-directory. Each child ignored a real filesystem type-conflict error, then successfully wrote after-cleanup-error. Expected absent cleanup was independently shown to remain allowed.

Source128 uses the existing read/stat error classifier for deletion, preserving expected NotFound/Interrupted handling. Direct directory creation now stops on any delegated error, matching recursive creation and writable-handle policy. The child fixture duplicates the real caller's ignored-error pattern; it does not force the FLAC fallback branch itself. Authority128 passes 95 tests, zero failures, nine ignored. The source delta adds two checked wrappers and one stop-on-error boundary, with no debit refunds or cap changes.

## Fresh frozen verification

Original handle19265 runs one sequential gate: check129, helper130, faults131, run132, audit133. Native source128, executable inputs and replay tools remain unchanged throughout. Only unbound operator documentation and planning records change during execution. The independent source128 correction review accepts the delegated-error fix and considers the public-facade swallowing regression appropriate. It found no unclassified delegated result in these four directory functions. It separately identifies raw Path::exists preflight probes in FLAC temporary cleanup and recursive ancestor discovery: metadata errors can become false before classification. That source-reasoned concern remains open, so do not claim universal filesystem-error containment or final integration acceptance. Do not substitute run123/audit124 for source128 final evidence.

Original verification handle18470 finished zero. Whole-project formatting and whitespace checks passed, changed Python ASTs parsed, and repeated audit125 was byte-for-byte identical to audit124. These checks preceded source128 and are not its final verification.

## Historical adverse evidence remains retained

Run80 is genuine 1-GiB cap-denial evidence with all 216 slots unavailable/effects unknown. It is never workload-fit proof. Authority16 remains non-green with unresolved readiness cause. Gate81/handle81599 was aborted only after a sample identified a coordinator-return/server-accept join deadlock; its mixed-source failures remain non-final and non-green. Its Cargo88706/native88888 processes were absent afterward. Original89834 settled101. Original91213 settled zero but was unfrozen. Authority110, directories116, faults119 and failed finalization observer invocations remain adverse evidence tied to their source/fixture snapshots. No expected slow run was killed merely for startup delay.

Prior startup delay has no established cause. Current gates have run without a persistent startup blocker. No OS changes, restart, cleanup, cap increases, commits, pushes, fetches, live requests, credentials/private-config access, or original-library writes were performed. Raw logs, generated media, unrelated archive and Python cache remain retained.

## Changed paths

All edits remain uncommitted on artist-library-inventory above local HEAD 6894e38ffa2f2771e76f9f62137a10e4f4904993. These are the intended task paths; the unrelated archive and scripts/__pycache__/ are excluded from this list.

- docs/offline-provider-mock.md
- scripts/auto-tag-eval-replay/audit-supervised.py
- scripts/auto-tag-eval-replay/bounded-storage.cjs (new)
- scripts/auto-tag-eval-replay/check-mock-authority.cjs
- scripts/auto-tag-eval-replay/check-supervision.cjs
- scripts/auto-tag-eval-replay/finalization-control.cjs
- scripts/auto-tag-eval-replay/mock-authority.cjs
- scripts/auto-tag-eval-replay/run-bounded.cjs
- scripts/auto-tag-eval-replay/run-supervised.cjs
- scripts/auto-tag-eval-replay/score-observations.py
- scripts/auto-tag-eval-replay/test_supervision.cjs
- scripts/bounded_storage.py (new)
- scripts/mock-provider-service.cjs
- scripts/review_auto_tag_subset.py
- src-tauri/src/commands/auto_tag.rs
- src-tauri/src/commands/auto_tag_eval.rs
- src-tauri/src/commands/mutations.rs
- src-tauri/src/infra/aliases.rs
- src-tauri/src/infra/mock_run_authority.rs
- src-tauri/src/infra/mock_run_broker.rs
- src-tauri/src/infra/mock_run_ownership.rs
- src-tauri/src/infra/mock_run_receiver.rs
- src-tauri/src/infra/mock_run_storage.rs (new)
- src-tauri/src/state/sqlite.rs
- src-tauri/src/state/worker.rs

The local planning record is .planning/plans/bounded-storage-admission-20261112.md. Retained snapshots/logs/binaries and run roots are under this evidence directory. Grounded assistance accuracy credit remains zero and user agreement remains 0/22. Storage fit does not complete the broader unified-tagging quality goal.
