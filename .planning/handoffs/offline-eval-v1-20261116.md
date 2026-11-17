# Unified tagging feedback-loop handoff

Updated 2026-11-17. The small provider-only comparison works and is shipped. The remaining target is a verified provider-and-assistance workflow with per-field review state.

## Target

Use one repeatable offline baseline/candidate loop to improve useful tagging of every physical track. Combine provider evidence and grounded assistance, preserve stronger evidence and meaningful version distinctions, and highlight assisted or uncertain fields for review. Verify written metadata independently. A populated field or review acknowledgment is not proof of correctness.

Follow the [combined-workflow design](https://ampcode.com/artifacts/combined-tagging-workflow-design-c015f92a-30f9-4830-a776-b021c6b3a6f0). Reuse the native coordinator, WriteQueue, replay, and owner-offline components. The command is implemented; do not rebuild it or start another inventory or general receipt-hardening exercise without a demonstrated blocker.

## Gap

The existing loop compares provider-only tagging across `folder_filename`, `assisted_without_ids`, and `tagged_recovery`, each cold and warm. The assisted profile means existing tags, not active model assistance.

The next bounded milestone is one paired native run that verifies provider-plus-assistance cooperation and persisted per-field review/highlight state. Include a grounded assisted fill, preservation of a stronger provider value, and a conflict or assistance failure. Use retained or scripted loopback responses and label them accordingly. Such a run establishes deterministic workflow behavior, not real-model quality.

Use TDD for any code change. Freeze both executables, identical inputs, reviewed expectations, and budgets. Preserve all requested cases and physical-file slots. Report recoveries, wrong assignments, abstentions, unavailable work, missing fixtures, skips, and process failures separately from independent metadata-readback and write-safety failures. Retain the unsupported-remaster no-write control.

The full 53-positive matrix, multi-disc/duplicate coverage, acoustic identity, exact-edition proof, and real-model quality remain unverified by this work. The five inspected cases are development evidence, not blind generalization.

## Verification

### Completed provider-only pair

The final run at `.amp/in/artifacts/paired-safety-classification-20261117/` covered five cases, 22 files, three profiles, and cold/warm phases: 30 native processes, 60 evaluations, and 264 physical-file slots.

- Baseline recovered 18/30 evaluations with 12 abstentions. Candidate recovered 24/30 with six abstentions.
- Six improvements, zero regressions, and zero wrong matches. Both builds passed all six no-write controls.
- Unavailable work, missing fixtures, skips, process failures, independent readback failures, and write-safety failures were separately zero. No required local dependency was missing.
- Input locks, isolated caches, zero model requests, and protected-file preservation passed. Frozen builds are `paired-baseline-20261116/` and `paired-candidate-reviewed-20261117/` under `.amp/in/artifacts/`. Their source base is `3d041679`; the candidate includes archived dirty source. They are not builds of the later shipping commit.

The native year-annotation fix and reporting failure-path corrections have regression coverage. Final review found no substantive findings in the latest three fixes. Failure-export probes simulate errors; they do not establish actual filesystem fault recovery.

See the [paired results](https://ampcode.com/artifacts/offline-tagging-pair-51263e1c-5b73-40db-a048-a2f29e899bf1) and [review record](https://ampcode.com/artifacts/offline-comparison-review-e15528f2-ee00-4fec-a9b1-3488b905cf84) for executable hashes, per-case outcomes, and correction evidence.

### Shipping checks and CI

Before shipment, desktop Rust passed 1,388 tests, server Rust passed 907 unit and 19 integration tests, and the isolated renderer suite passed 858 tests. Seventeen Python report contracts, two Node CLI controls, typecheck, renderer build, and the selected offline background smoke passed. Gated credentialed and real-display tests were not enabled.

The full `just --no-dotenv check` exited 1 because Vitest discovered the preserved nested checkout and a second React installation. An unmodified `origin/main` control reproduced all 523 failed-test identifiers. The owner accepted this demonstrated baseline exception. No tracked expectations or test configuration were weakened. Logs and ignored-test counts are in `.amp/in/artifacts/ship-offline-comparison-20261117/validation-summary.json` and adjacent logs.

Implementation shipped in [b16c8084](https://github.com/ydeng11/Soundrobe/commit/b16c8084bf854ae82a6054932da6285638b722a7); the prior handoff shipped in [717cf746](https://github.com/ydeng11/Soundrobe/commit/717cf74680320b8ea7c384e270154b38d304dc3b). At this refresh, exact-head [Tests](https://github.com/ydeng11/Soundrobe/actions/runs/44014205191) and [CodeQL](https://github.com/ydeng11/Soundrobe/actions/runs/44014205465) for `717cf746` are still running. Confirm their final state before implementation and resolve demonstrated blockers. Nothing was deployed. Future pushes and hosted execution need explicit approval.

## Resume safely

Use `/Users/ihelio/code/auto_tagger` on runner `soundrobe-local`. Inspect current Git state. Read `AGENTS.md`, [evaluation guidance](../../docs/auto-tag-evaluation.md), and the [replay README](../../scripts/auto-tag-eval-replay/README.md). The README declares inputs, dependencies, and `freeze`/`compare` commands. Use a new frozen candidate and fresh output directory for changed code.

Hosted/model calls remain disabled. Use retained responses, loopback endpoints, fake credentials, disposable keys, and disposable media. Never write to the original music library or overwrite historical receipts.

Preserve the intentional untracked iOS archive and four Python caches, `.amp/in/artifacts/reconcile-original-20261116/`, and detached `.amp/in/owner-eval-commit-20261116/`. A fresh cloud checkout lacks the local executables, media, and evidence. Do not silently substitute one.

Detailed history remains in the [earlier evidence report](https://ampcode.com/artifacts/dummy-eval-set-integrity-and-offline-replay-6b0dd7d6-6b86-4dee-bce3-d9741a2b219b), Git history, and immutable local run directories. This brief supersedes older checkout and next-task instructions.
