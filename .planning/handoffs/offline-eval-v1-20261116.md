# Offline eval handoff, 2026-11-16

## Goal

Build a reproducible benchmark that demonstrates additional correct metadata recoveries without losing correct results, assigning wrong versions or parent albums, changing audio payloads, authorizing unintended writes, or exceeding request budgets and cooldowns.

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

The shared branch also contains unrelated native/mock work. This commit excludes those changes. No push, merge, deployment, or hosted execution is part of this handoff.

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

## Planned offline v1, not yet completed

1. Freeze one authoritative versioned suite and explicit per-profile scoring scopes. Keep parent-package, selected-medium, and core-content claims distinct.
2. Connect one reproducible entry point to current native loopback replay using frozen provider responses and disposable silent media. Keep hosted dispatch disabled.
3. Exercise the three profiles with separate cold and warm caches. Including all 53 metadata positives would imply 318 evaluations, subject to explicit native eligibility. This is a job count, not measured execution time.
4. Check independent tag readback, payload preservation, artist credits, numbering, write approval, and request-budget accounting. Report fixture gaps and product failures separately rather than relaxing truth.
5. Publish a compact result with the exact binary, input locks, exclusions, and failure list.

Completing v1 does not require labeling all 325 unscored cases, acquiring more provider evidence, or proving acoustic or genuine hosted-model quality. Further receipt hardening is closed unless a concrete new defect appears.

## Known coverage limits

There are no verified abstentions or positive compilation-batch cases. Twenty-eight positives are Enya, and all eight positive holdout cases are Billie Eilish. The split is retrospective, not blind. Additional representative truth is future evidence work, not hidden execution credit.

## Suggested skills

For the next implementation session, use `graphify` for code ownership, `writing-plans` for the bounded execution plan, `test-driven-development` for code changes, and `verification-before-completion` before results claims. Existing workflow documentation is in `docs/auto-tag-evaluation.md` and `docs/tagging-feedback-status.md`.
