# Offline feedback-loop handoff, updated 2026-11-17

The bounded provider-only baseline/candidate comparison is implemented, reviewed, and shipped to `origin/main` in [b16c8084](https://github.com/ydeng11/Soundrobe/commit/b16c8084bf854ae82a6054932da6285638b722a7). It completed a five-case comparison with six improvements and no regressions or wrong matches. The broader unified provider-and-assistance feedback loop is not complete. This section supersedes the older checkout-state and next-implementation instructions below.

## Resume the existing local checkout

Use `/Users/ihelio/code/auto_tagger` on runner `soundrobe-local`. The implementation shipped at `b16c8084bf854ae82a6054932da6285638b722a7`; local and remote `main` matched after that push. The final fetch/rebase found no changes or conflicts, and the remote tree matched the tested tree. This handoff is a documentation-only follow-up. Inspect current Git state and resume the latest main, which may include later documentation commits.

The intentional `ios-native-probe-0d104d95-source.tgz` and four files in `scripts/__pycache__/` remain untracked and unchanged. The reconciled original edits remain backed up in `.amp/in/artifacts/reconcile-original-20261116/`, including `original-uncommitted-files.tar.gz`, `tracked-edits.patch`, `backup.json`, and `result.json`. The former shipping checkout at `.amp/in/owner-eval-commit-20261116/` remains detached at `3d041679` with its evidence intact. Do not reapply its superseded source edits or delete it to clean up test discovery.

A fresh cloud checkout does not contain the retained executables, dummy media, or historical receipts. Continue locally or explicitly transfer the required inputs. Do not silently substitute a fresh clone. Read `AGENTS.md`, `docs/auto-tag-evaluation.md`, and `scripts/auto-tag-eval-replay/README.md` before implementation.

## Confirm exact-head CI before further implementation

At this refresh, [CodeQL](https://github.com/ydeng11/Soundrobe/actions/runs/44013604108) for `b16c8084` passed; [Tests](https://github.com/ydeng11/Soundrobe/actions/runs/44013604179) is still in progress. Do not infer green Tests from local checks or earlier commits. This thread has active exact-commit CI subscription `0f450d6d-8b2e-403c-82df-5e1142bceee9`. Inspect the final results and resolve any demonstrated blocker caused by these changes.

The owner authorized the completed direct push. GitHub accepted it while required checks were pending. That shipment does not authorize future pushes, deployments, releases, or hosted execution.

## What this thread completed

- Added `scripts/auto-tag-eval-replay/run-paired.cjs` with `freeze` and `compare` commands, an independent `audit-paired.py`, reviewed selections, and regression tests wired into `npm run test:review-eval`.
- Froze both native executable identities, source snapshots, identical inputs, scoring rules, and budgets. Every case runs under `folder_filename`, `assisted_without_ids`, and `tagged_recovery`, with separate cold caches and warm caches inherited only from their own cold phase. The assisted profile uses existing tags, not a model.
- Closed the observed Bellyache year-annotation defect with a failing regression, targeted matcher fix, and paired rerun. Comparison ignores only top-level bracketed bare years. Canonical provider titles replace filename year pollution; nested live-performance years and named remix/remaster years remain meaningful.
- Expanded from two single-track controls to five cases and twenty-two physical files. The added cases cover two-track Guitar Songs, distinct Remix/LP Version mappings in Storms In Africa, and sixteen-track Paint The Sky With Stars. Unsupported remaster claims require unchanged no-write abstentions.
- Corrected demonstrated report failures through successive red/green review cycles. Incomplete or misbound exports cannot pass no-write controls. Malformed ancillary fields retain both reports and every requested row. Observed wrong writes remain counted despite process or fixture failures. Null/count/binding diagnostics do not invent wrong matches. Partial errors grade independently observed changed files, including excluded MusicBrainz and Discogs IDs, without counting unchanged bad input as a new wrong match. Optional `skipped` status must be Boolean.
- The final independent standards and specification reviews found no new substantive findings in the latest three fixes. The specification reviewer exercised twenty-five current-versus-archived scenarios. Those probes simulate native-supported exports or malformed diagnostic inputs; they are not induced filesystem faults.

Use the [comparison report](https://ampcode.com/artifacts/offline-tagging-pair-51263e1c-5b73-40db-a048-a2f29e899bf1) and [review history](https://ampcode.com/artifacts/offline-comparison-review-e15528f2-ee00-4fec-a9b1-3488b905cf84) for the detailed iterations rather than reconstructing them from older handoff sections. The implementation thread is [the offline comparison continuation](https://ampcode.com/threads/T-01a1e634-6657-725f-a4bd-74c89e65df48).

## Latest executed comparison

The final pair is `.amp/in/artifacts/paired-safety-classification-20261117/`. All thirty native processes and sixty evaluations completed, retaining 264 physical-file slots. Baseline recovered 18/30 evaluations and abstained twelve times; candidate recovered 24/30 and abstained six times. Each profile improved from 6/10 to 8/10. There were six improvements, zero regressions, and zero wrong matches.

Unavailable evaluations, missing fixtures, skips, process failures, independent metadata-readback failures, and write-safety failures were separately zero for both builds. Both passed all six required no-write controls. Each retained 132 physical-file slots, including 96 unchanged no-write file slots. `validation-summary.json` confirms all 1,622 locks, equal initial media, isolated caches, zero model requests, unchanged protected files, and unchanged reconciliation archive/patch hashes. No required local dependency was missing.

The frozen builds both use base revision `3d04167955ee1b9bc57ffa784459669290ce24b1`. Baseline source is clean; candidate source was dirty and archived before shipping. Do not relabel these historical binaries as builds of the later shipping commit.

- Baseline: `.amp/in/artifacts/paired-baseline-20261116/`, executable SHA-256 `a318fe98d51e5e9d444d9cd9959e2818db283216f6c474ae933b81b12c42ce01`.
- Candidate: `.amp/in/artifacts/paired-candidate-reviewed-20261117/`, executable SHA-256 `edd052b0b68e9efbabd2815e237aef714ef368e760d4671c2cb73e784700db3e`.
- Shared limits: 600 seconds per native folder, 1,300 seconds per process, zero model requests. Fixtures, selection, expectations, and limits were identical between builds. The final auditor was archived in the run's `harness.tgz` before execution.

To repeat that exact pair, choose a new output directory and use the existing command. Never rerun into a historical directory.

```sh copy Copy Paired Comparison
node scripts/auto-tag-eval-replay/run-paired.cjs compare \
  .amp/in/artifacts/NEW_SAFETY_PAIR \
  .amp/in/artifacts/paired-baseline-20261116 \
  .amp/in/artifacts/paired-candidate-reviewed-20261117 \
  scripts/auto-tag-eval-replay/paired-safety-selection.json
```

The README declares all local dependencies, including `.amp/in/eval-dummies/original/{corpus.json,media/}`, retained provider fixtures, Python 3, Node, FFmpeg, and cached Rust build dependencies for a new freeze. Missing dependencies must remain explicit. A new candidate requires a new frozen executable and source snapshot; never overwrite either retained build.

## Shipping verification and the baseline exception

Ran `just --no-dotenv check` before pushing. Desktop Rust passed 1,388 tests with 39 explicitly ignored. Server Rust passed 907 unit tests with 34 ignored, plus nineteen integration tests with twelve ignored. All seventeen Python report contracts and both Node CLI controls passed. Mock-provider and review/suite checks passed; detached-checkout duplicates are not extra current-source coverage. Typecheck, the renderer production build, and an explicitly selected offline background-tagging smoke also passed. Credentialed, real-display, and other gated tests were not enabled.

The full gate exited one because Vitest discovered the preserved nested checkout and loaded a second React installation. A temporary unmodified `origin/main` control reproduced all 523 failed-test identifiers exactly; all belonged to the nested checkout. The owner explicitly allowed ignoring failures only after that baseline demonstration. With `.amp` evidence excluded from discovery, the current renderer suite passed all 858 tests in 49 files. No test expectations or tracked test configuration were weakened. The temporary control worktree was removed; the historical worktree was not changed or deleted.

Shipping evidence is `.amp/in/artifacts/ship-offline-comparison-20261117/`, especially `validation-summary.json`, `check.log`, `origin-main-web-control-copied.log`, `current-web-isolated.log`, `background-smoke.log`, and `build-web.log`. No source changed after testing. The paired comparison was not rerun merely for shipment.

## Preserve evidence and keep the remaining scope explicit

The latest correction evidence is `.amp/in/artifacts/report-classification-fixes-20261117/`, including red/intermediate/green logs, the exact auditor diff, and copied review-probe results. Earlier corrections remain in `paired-review-fixes-20261117/` and `three-gap-fixes-20261117/`; the original final-review probes remain in `three-fixes-review-20261117/`. Superseded and incomplete runs are historical evidence, not current success. The source tree contains the portable command, not the ignored execution archive.

The provider-only loop is usable. The broader unified provider-and-assistance comparison, persisted per-field assisted review/highlight checks, real-model quality, acoustic identity, exact-edition proof, multi-disc/duplicate-file expansion, and the full 53-positive matrix remain outside this result. The five cases are inspected development evidence, not blind generalization. At 53 eligible positives, 636 paired evaluations remains a planned count, not an executed run or measured runtime.

When further implementation is requested, first resolve exact-head CI if necessary, then extend this working comparison for the requested bounded case or unified-workflow behavior. Reuse the native coordinator, WriteQueue, retained replay, and owner-offline components. Do not rebuild the comparison command, start another inventory, or reopen general receipt hardening without a demonstrated blocker. Retain the unsupported-remaster no-write expectation.

Hosted/model calls remain disabled. Use retained responses, loopback endpoints, fake credentials, disposable keys, and disposable media. Never write to the original music library or rewrite historical receipts. The graph index references retired Electron code; use current native source when it is stale.

## Suggested skills for continuation

Use `systematic-debugging` for a demonstrated CI or product failure, `test-driven-development` for code changes, and `verification-before-completion` for result claims. Use `graphify` for ownership questions with the stale-index caveat above. For unified behavior, read the existing combined-workflow design before using `brainstorming` or `writing-plans`. Use `writing-prompts`, `pstack:technical-writing`, and `handoff` when updating continuation instructions.

---

## Historical handoff, 2026-11-16

The record below preserves the earlier owner-offline and fixture work. Its statements that the paired command is unimplemented, its old checkout instructions, and its pending older CI instructions are superseded by the current section above.

## Resume from main

The reviewed owner-evaluation component is on `main` in [a4c34ee0](https://github.com/ydeng11/Soundrobe/commit/a4c34ee040502028c866b17482a74a3286c4e748), followed by handoff commit [cf8980c5](https://github.com/ydeng11/Soundrobe/commit/cf8980c57ec82c99de05e4e03b30d1552fd5bd1a). GitHub reported bypass of the pull-request rule while checks were pending. The [completed CI run](https://github.com/ydeng11/Soundrobe/actions/runs/43937108892) is red: desktop/server Rust and the combined unit job failed; lint, CodeQL, and all four desktop platform jobs passed. Its exact-head subscription ended. Check current main/CI state before continuing.

Confirm green CI for the follow-up below before starting the baseline/candidate comparison command described under [Next implementation task](#next-implementation-task). The owner-offline component is implemented; the paired feedback-loop command is not. No feedback-loop implementation or new owner-evaluation rehearsal was started during this follow-up.

### CI follow-up and pending remote verification

Two corrections accompany this handoff: `src-tauri/Cargo.toml` uses test-only line-table debug information, retaining the unchanged 512 MiB executable source limit; the broker approval test attaches a publication latch only when publication actually occurs, while preserving its explicit pending-publication rejection control. The prewritten-file path now runs on macOS too, reproducing the Linux setup defect before the fix. Production authority and hosted-call denial are unchanged.

The owner failures occurred while validating the current test executable. A controlled executable padded to 536,870,913 bytes reproduced the exact panic and rejection; CI executable size was not retained, so oversize remains the supported hypothesis, not a directly measured CI fact. Corrected local default/server test executables are 123,837,152 and 127,461,808 bytes. The Linux broker failure independently came from a false publication flag that only macOS code could set.

Fresh offline checks passed: thirteen owner tests in each configuration, nineteen server broker tests with two explicit subprocess-entry ignores, changed-file formatting, and default/server all-target Clippy with warnings denied. The exact approval control failed before correction and passed afterward. Repository-wide `cargo fmt --check` still flags three unchanged hunks in `commands/auto_tag.rs`; they were left alone. Full suites and Linux/Rust 1.100 CI have not been rerun for this correction. Logs, reproduction details, source bindings, and the original-checkout preservation check are retained under the isolated checkout's `.amp/in/artifacts/ci-executable-size-20261116/`. Hosted calls stayed blocked by the offline sandbox. The owner authorized committing and pushing this follow-up to main and following CI. Confirm this follow-up's exact-head CI results in the [source thread](https://ampcode.com/threads/T-01a1b234-730d-7557-9ba6-fc105c23f683); earlier red results do not describe a fresh run, and local checks do not establish remote success.

### Deadline-test follow-up and pending remote verification

The [completed CI run for d0012fb8](https://github.com/ydeng11/Soundrobe/actions/runs/43944929389) passed server Rust, lint, CodeQL, and all four desktop platform jobs. Desktop Rust and the combined unit job each failed only `schema_repair_and_body_read_share_the_original_deadline`; required tests consequently failed. The earlier owner failures are no longer present. This exact-head subscription has ended.

A controlled scheduling probe reproduced the test's one-millisecond watchdog failure: repair headers can be ready when the completion future is next polled at virtual millisecond 90, so the unchanged ten-millisecond body polling interval settles at millisecond 100. The local test-only correction allows eleven milliseconds after expiry and asserts total virtual elapsed time is 90–101 ms. A deliberate fresh-repair-budget mutation still failed this corrected test. Production code, timeouts, retries, and hosted-call denial are unchanged; the temporary scheduling probe and mutation were removed after retaining their diffs and logs.

The final source passed 100 consecutive exact-test repetitions and seventeen OpenRouter tests in each configuration, with two explicitly ignored tests per configuration. Changed-file formatting and both all-target Clippy configurations passed. Full suites were not repeated. The owner authorized committing and pushing this test-only correction and handoff update to main, then following fresh exact-head CI. Confirm those results in the source thread before continuing; local passes do not establish remote success. Evidence and the fresh original-checkout preservation inventory are alongside the earlier CI records, under `ci-executable-size-20261116/deadline-*`.

### Protect the original working tree

The original checkout at `/Users/ihelio/code/auto_tagger` remains on local branch `artist-library-inventory` at `615243e0`, with all 23 original uncommitted files unchanged. Those files include predecessor owner-evaluation edits, generated bytecode, and the unrelated iOS archive. Do not reapply, commit, discard, or overwrite them while continuing from main.

The isolated reviewed checkout at `/Users/ihelio/code/auto_tagger/.amp/in/owner-eval-commit-20261116` now follows local `main`. Use an updated clean main checkout for continuation. A different machine or fresh clone will not have this local path or the ignored evidence. Shipment permission for this thread does not authorize later pushes, hosted calls, releases, or deployments.

## Goal

Build a repeatable feedback loop for changes to auto-tagging capabilities. Compare baseline and changed implementations on the same frozen inputs, show improvements and regressions, and verify safe metadata writes. The eval set is supporting material, not the final deliverable. The [current plan](../../docs/auto-tag-evaluation.md#goal-and-planned-offline-feedback-loop) defines acceptance and exclusions.

The earlier documentation-only pause was followed by the owner's separate authorization to review, verify, and commit the existing owner-evaluation work. That component is now on main. The broader baseline/candidate loop remains planned.

Offline metadata review establishes compatible content, not acoustic identity, exact source pressing, genuine model quality, or current native-app success. Hosted calls remain disabled. Verification uses retained provider responses and disposable media, never writes to the original library, and preserves earlier evidence.

## Completed work

The earlier committed inventory contains 378 cases and 3,506 tracks across 27 batches. Twelve batches are compilation albums. The maintained `test/fixtures/tauri/auto-tag-eval/batch-suite.json` has 51 reviewed positives and 539 track mappings. Its other cases remain explicitly unscored.

This handoff accompanies two opt-in manifests under `test/fixtures/tauri/auto-tag-eval/`:

- `reviewed-selected-media-20261115.json` adds one Enya selected-medium metadata positive with 16 mappings and retains four unresolved parent-package conflicts.
- `reviewed-explicit-extras-20261116.json` adds one Relapse core-album metadata representative with 20 mappings and retains nineteen unresolved cases. It does not authorize replacing the Refill package or disc identity.

Together with the maintained reviews, these produce 53 metadata positives with 575 mappings, 45 unresolved cases, and 280 unverified cases. All 378 cases and 3,506 tracks remain present. The optional additions do not replace the maintained checkpoint or establish exact-edition native write expectations.

Local evidence-only passes also corrected coverage classification, skip reporting, duplicate JSON keys, canonical provider-position uniqueness in the versioned diagnostic comparator, packaging reconciliation, and historical receipt consistency. Independent reviews and actual-entry mutation probes found no remaining proven blocker within the declared fixed-input contracts. These corrections are retained locally, not promoted into the application or maintained comparator.

## Owner-offline component and verification

The [owner-offline reference](../../docs/auto-tag-evaluation.md#owner-authorized-offline-evaluation) documents the actual native entry, proposal and signed intent, disposable custody, and independent audit API. The maintained code is under `src-tauri/src/infra/mock_run_owner*.rs` and `scripts/auto-tag-eval-replay/`. Reuse these components instead of adding a second backend or another signing/archival path.

Review corrected two demonstrated blockers before commit: non-public source eligibility now depends on an independently selected evaluator path, and native completion archival requires observed successful exit of the owned receiver child. Withholding can complete an observation without an accepted genre or actionable genre review. Disconnect remains incomplete. Neither scripted outcome earns real-model quality credit.

Local results for the reviewed source are retained in `.amp/in/artifacts/owner-eval-commit-review-20261116/verification-summary.json` and its logs:

- Default Rust: 1,387 passed, zero failed, 39 explicitly ignored. Renderer: 858 passed. Mock-provider: twelve passed. Reviewed-eval: ten passed. Node controls: 68 passed with zero skipped.
- Default and server all-target Clippy passed with warnings denied. Typechecking and the production renderer build passed. The local Rust version was 1.97.1, not CI's Rust 1.100.
- Supplementary server controls: 132 passed, one failed because the outer macOS sandbox denied process inspection, and thirteen explicitly ignored. The exact affected test passed separately with a PATH-only environment outside that outer sandbox. Final owner-launch controls passed five tests and explicitly ignored one private subprocess entry. Do not describe the sandboxed supplementary run as entirely green.
- A fresh final-source six-job withheld-response rehearsal exited successfully in about twelve minutes. The maintained independent auditor accepted 216 field-slot observations and twelve closed sessions, with zero real-model quality credit and no downstream success stubs. This is one component rehearsal, not the planned 636-evaluation comparison.
- An earlier rehearsal hit the unchanged native deadline. Its incomplete audit retained all 216 requested slots as unavailable. Both attempts are preserved in `owner-rehearsal-attempts.tgz`; the successful standalone retry does not prove that load caused the first failure.

Hosted calls remained disabled. Validation used fake credentials, disposable keys, and copied silent dummy media, never original music or provisioned credentials. `commit-result.json` binds the 21-file commit and original-file preservation check. Historical receipts still describe their original local-only delivery state; the main-branch update above supersedes that state without rewriting them.

## Evidence and delivery boundary

The [running report](https://ampcode.com/artifacts/dummy-eval-set-integrity-and-offline-replay-6b0dd7d6-6b86-4dee-bce3-d9741a2b219b) records the detailed review history and limits. Its older two-to-four-day estimate was an unmeasured planning allowance, not a runtime measurement. The bounded next step is packaging, followed by measured replay.

Evidence remains under Git-ignored `.amp/in/artifacts/`. The earlier fixed-input receipt correction and review are `historical-receipt-correction-20261116/` and `historical-receipt-review-20261116/`. The latter records eighteen isolated regression reruns with observed zero exits, six complementary probes, ordinary replay, and verification of 3,527 protected hashes and 38 exact directory inventories. Historical shutdown and missing-acknowledgement anomalies remain preserved, not rewritten as successful original executions.

These are local, unsealed consistency checks. They do not authenticate capture history. A fresh clone receives the manifests and this handoff, not the ignored correction sources, generated suites, private inventory evidence, or receipt archive. Reproducing the complete historical audit therefore requires the preserved local evidence. This commit does not claim to make that archive portable.

The original fixture-only commit excluded native/mock edits. The reviewed owner-evaluation component has since been committed separately and shipped as described above. This handoff update changes documentation only. Hosted execution and deployment remain excluded.

## Historical fixture wrap-up checks

The maintained builder reproduced the original checkpoint and the optional expansion in memory, with network and subprocess creation denied. The expansion exactly matched retained `offline-eval-expansion-20261116/additive-suite.json`, including both manifest input hashes. Recomputed positive mappings totaled 575. The original checkpoint bytes remained unchanged.

All four tests in `test/scripts/build-auto-tag-eval-suite.test.ts` passed. No native workload or provider call was run for this wrap-up.

## Reproduce the optional expansion

From the repository root, this read-only command uses committed inputs, refuses network and subprocesses, and prints the optional suite. It does not require the ignored archive or overwrite the maintained checkpoint.

```sh
python3 -B - <<'PY'
import argparse
import json
from pathlib import Path
import runpy
import sys


def offline(event, args):
    if event.startswith('socket.') or event in ('subprocess.Popen', 'os.system'):
        raise RuntimeError('offline build forbids network and subprocesses')


sys.addaudithook(offline)
sys.path.insert(0, 'scripts')
root = Path('test/fixtures/tauri')
eval_root = root / 'auto-tag-eval'
baseline = json.loads((eval_root / 'batch-suite.json').read_text())
reviews = [root / source for source in sorted({row['expectationSource'] for row in baseline['cases']})
           if source.startswith('auto-tag-eval/reviewed-')]
api = runpy.run_path('scripts/build_auto_tag_eval_suite.py')
result = api['build'](argparse.Namespace(
    fixture_root=root,
    corpus=[root / source for source in sorted({row['corpus'] for row in baseline['cases']})],
    reviewed_manifest=reviews + [eval_root / 'reviewed-selected-media-20261115.json',
                                 eval_root / 'reviewed-explicit-extras-20261116.json'],
    candidate_pools=eval_root / 'candidate-pools.json',
    discovery=eval_root / 'provider-discovery.json'))
print(json.dumps(result, ensure_ascii=False, indent=2))
PY
```

## Next implementation task

Build the first usable baseline/candidate comparison command, using the authoritative [feedback-loop plan](../../docs/auto-tag-evaluation.md#goal-and-planned-offline-feedback-loop) and existing native replay components. Start with a small explicitly eligible subset to validate paired execution and report generation before expanding to the full eligible suite. Do not start another inventory-expansion or general receipt-review cycle.

The command must identify both revisions and executables, freeze identical inputs and budgets, and compare the same requested cases across the three profiles with separate cold and warm caches. Its compact before/after report must expose improvements, regressions, wrong matches, abstentions, missing fixtures, skipped cases, and process failures. Independent readback and write-safety checks remain separate from recovery counts. Preserve the full requested denominator and the plan's field-specific truth scopes; a metadata-content positive does not authorize exact-edition writes. A confirmed product defect should support a focused regression test, targeted fix, and paired rerun without weakening reviewed expectations.

Packaging remains real work: the owner-offline component's reviewed invocation still uses retained public plans and setup helpers under ignored artifact paths. A fresh clone does not receive those inputs. Make the minimal comparison reproducible from declared maintained inputs, or report missing input dependencies explicitly rather than substituting success or silently skipping cases. The [replay instructions](../../scripts/auto-tag-eval-replay/README.md) distinguish portable entry points from local historical dependencies.

For 53 eligible positives across three profiles and cold/warm phases, the planned count is 318 evaluations per implementation, or 636 per baseline/candidate comparison. Eligibility is explicit; these are not measured runtimes. Run the long rehearsal independently of broad builds/test suites when possible, while retaining the unchanged limits and any failures. The prior deadline failure's cause is not established.

Completing v1 does not require labeling all 325 unscored cases, acquiring more provider evidence, or proving acoustic or genuine hosted-model quality. Further receipt hardening is closed unless a concrete defect blocks the loop. Hosted calls remain disabled; use loopback responses, fake credentials, disposable keys, and disposable media. Practical implementation belongs to the continuation thread when the user requests it.

## Known coverage limits

There are no verified abstentions or positive compilation-batch cases. Twenty-eight positives are Enya, and all eight positive holdout cases are Billie Eilish. The split is retrospective, not blind. Additional representative truth is future evidence work, not hidden execution credit.

## Suggested skills

For the next implementation session, use `graphify` for code ownership, `writing-plans` for the bounded execution plan, `test-driven-development` for code changes, and `verification-before-completion` before results claims. Existing workflow documentation is in `docs/auto-tag-evaluation.md` and `docs/tagging-feedback-status.md`.
