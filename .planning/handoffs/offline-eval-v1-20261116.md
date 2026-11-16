# Offline eval handoff, 2026-11-16

## Goal

Build a repeatable feedback loop for changes to auto-tagging capabilities. Compare baseline and changed implementations on the same frozen inputs, show improvements and regressions, and verify safe metadata writes. The eval set is supporting material, not the final deliverable. The [current plan](../../docs/auto-tag-evaluation.md#goal-and-planned-offline-feedback-loop) defines acceptance and exclusions.

Implementation and native execution are deferred at the user's request. This update reframes the plan and documentation for shipment only.

Offline metadata review establishes compatible content, not acoustic identity, exact source pressing, genuine model quality, or current native-app success. Hosted calls remain disabled. Verification uses retained provider responses and disposable media, never writes to the original library, and preserves earlier evidence.

## Completed work

The earlier committed inventory contains 378 cases and 3,506 tracks across 27 batches. Twelve batches are compilation albums. The maintained `test/fixtures/tauri/auto-tag-eval/batch-suite.json` has 51 reviewed positives and 539 track mappings. Its other cases remain explicitly unscored.

This handoff accompanies two opt-in manifests under `test/fixtures/tauri/auto-tag-eval/`:

- `reviewed-selected-media-20261115.json` adds one Enya selected-medium metadata positive with 16 mappings and retains four unresolved parent-package conflicts.
- `reviewed-explicit-extras-20261116.json` adds one Relapse core-album metadata representative with 20 mappings and retains nineteen unresolved cases. It does not authorize replacing the Refill package or disc identity.

Together with the maintained reviews, these produce 53 metadata positives with 575 mappings, 45 unresolved cases, and 280 unverified cases. All 378 cases and 3,506 tracks remain present. The optional additions do not replace the maintained checkpoint or establish exact-edition native write expectations.

Local evidence-only passes also corrected coverage classification, skip reporting, duplicate JSON keys, canonical provider-position uniqueness in the versioned diagnostic comparator, packaging reconciliation, and historical receipt consistency. Independent reviews and actual-entry mutation probes found no remaining proven blocker within the declared fixed-input contracts. These corrections are retained locally, not promoted into the application or maintained comparator.

## Evidence and delivery boundary

The [running report](https://ampcode.com/artifacts/dummy-eval-set-integrity-and-offline-replay-6b0dd7d6-6b86-4dee-bce3-d9741a2b219b) records the detailed review history and limits. Its older two-to-four-day estimate was an unmeasured planning allowance, not a runtime measurement. The bounded next step is packaging, followed by measured replay.

Evidence remains under Git-ignored `.amp/in/artifacts/`. The latest correction and review are `historical-receipt-correction-20261116/` and `historical-receipt-review-20261116/`. The latter records eighteen isolated regression reruns with observed zero exits, six complementary probes, ordinary replay, and verification of 3,527 protected hashes and 38 exact directory inventories. Historical shutdown and missing-acknowledgement anomalies remain preserved, not rewritten as successful original executions.

These are local, unsealed consistency checks. They do not authenticate capture history. A fresh clone receives the manifests and this handoff, not the ignored correction sources, generated suites, private inventory evidence, or receipt archive. Reproducing the complete historical audit therefore requires the preserved local evidence. This commit does not claim to make that archive portable.

The original fixture commit excludes unrelated uncommitted native/mock work. The documentation follow-up authorizes shipment of the existing work, not implementation or native evaluation. Hosted execution and deployment remain excluded.

## Fresh wrap-up checks

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

## Planned offline v1, implementation deferred

The authoritative [feedback-loop plan](../../docs/auto-tag-evaluation.md#goal-and-planned-offline-feedback-loop) replaces the earlier suite-first work order. Acceptance is one command that compares baseline and candidate native implementations on identical frozen inputs and emits a compact before/after report. The loop must retain failures and skipped cases, separate scoring scopes, verify readback and write safety, and support a targeted fix followed by rerun.

For 53 eligible positives across three profiles and cold/warm phases, the planned count is 318 evaluations per implementation, or 636 per baseline/candidate comparison. Eligibility is explicit, and these are not measured runtimes. No loop implementation or execution is authorized by this documentation-only shipment.

Completing v1 does not require labeling all 325 unscored cases, acquiring more provider evidence, or proving acoustic or genuine hosted-model quality. Further receipt hardening is closed unless a concrete defect blocks the loop.

## Known coverage limits

There are no verified abstentions or positive compilation-batch cases. Twenty-eight positives are Enya, and all eight positive holdout cases are Billie Eilish. The split is retrospective, not blind. Additional representative truth is future evidence work, not hidden execution credit.

## Suggested skills

For the next implementation session, use `graphify` for code ownership, `writing-plans` for the bounded execution plan, `test-driven-development` for code changes, and `verification-before-completion` before results claims. Existing workflow documentation is in `docs/auto-tag-evaluation.md` and `docs/tagging-feedback-status.md`.
