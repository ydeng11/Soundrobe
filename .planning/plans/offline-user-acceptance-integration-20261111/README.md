# Offline user-acceptance integration delivery

This record accompanies the locally committed, independently reviewed offline scoring integration. It enables no live capability. The reviewed five-file source diff against `6cb93861c12628aff6d87332f1721e81ba761080` has SHA-256 `f88b8cc807b7c0b50f19abbcad5c671d038a4e29ee5100832538e498b3b1f850`.

## What is included

- The maintained scorer, mock driver and tests, with operator documentation and changelog.
- The immutable `crowd-lu-user-acceptance-v1-20261111` packet, its original seven-content-file seal, annotation snapshot and original row-binding/sheet records.
- The linked immutable `scorable-assistance-contract-20261111` packet and all eight files named by its original seal. That historical proposal is not execution authority or the current five-job diagnostic selection.
- This integration's `verify.py`, historical `validation.json`, `review.md`, byte-preserved independent review start/end receipts, and `annotation-provenance.json` transcription receipt.
- `delivery-checks.json` records fresh delivery checks separately from reused native/mock evidence. `delivery-SHA256SUMS` binds the selected source and records, not itself or the eventual commit ID.

Historical records retain their original source HEAD, then-current delivery state and test outcomes. Statements such as "uncommitted" or "no integration" inside those sealed records describe their earlier checkpoints, not the delivery state of this commit. They have not been rewritten or resealed. The current commit ID and no-push state are reported in the thread artifact and Git history.

## Reproduce checks from the repository root

```sh
python3 -B scripts/auto-tag-eval-replay/test_observation_scoring.py ObservationScoring NativeProjection FrozenUserProjection
python3 -B scripts/auto-tag-eval-replay/checkpoints/runtime-artist-diagnostic/test_comparison.py
shasum -a 256 -c .planning/plans/crowd-lu-user-acceptance-v1-20261111/SHA256SUMS
shasum -a 256 -c .planning/plans/scorable-assistance-contract-20261111/SHA256SUMS
shasum -a 256 -c .planning/plans/offline-user-acceptance-integration-20261111/delivery-SHA256SUMS
node --check scripts/auto-tag-eval-replay/run-bounded.cjs
```

The selected metadata inputs are committed so the frozen-packet admission and synthetic tests can be inspected and checked without original music. **Full native replay and paired-export reproduction are not fresh-clone/self-contained.** `verify.py`, the real-export projection controls, and historical generators additionally depend on locally retained ignored inputs under `.amp/in/`: generated corpus/media, native exports, strict-fixture inventories, request records, manifests, and matching test executables. None of those media, binaries, caches or large raw logs is committed. The historical receipts contain their paths and hashes; missing inputs must remain missing/unavailable, not be fabricated. Do not run the native driver as a side effect of checking these records. It materializes disposable media and requires separate authorization for a new execution.

## Current tools versus historical bindings

The frozen labels pin the historical scorer hash `1222426479f9180b0184f5a27e42b6441c7736ae0d0c131d136a75e08b994afe`. The reviewed current scorer hash is `276b624c7785deb434f34d4dc55264f3d580b594398c5523ed567701dd85e5a8`. The importer verifies the exact frozen label/seal bytes and immutable data bindings. Its historical scorer/coordinator hashes remain provenance; current native specs/manifests separately bind current tools. The historical coordinator bytes still match.

Consequently, the old label `freeze.py` current-source equality check reports drift. Comparing `derive()` with frozen `labels.json` must show **only the scorer entry in `bindings` differing**, not labels, rows, annotations or other data. This expected failure is not repaired by changing historical seals. Other historical generators may also require their bound ignored receipts and historical tools; their earlier green results are reused evidence, not newly passing delivery tests.

## Known limits

- Exactly `Pop` OR `Mandopop` OR the single combined string `Pop, Mandopop` is accepted for every one of eleven bound Crowd Lu rows and their common album answer. No normalization or additional spelling is accepted. The combined value is not comma-split.
- These are exposed user-acceptance labels, not acoustic truth, independently verified listening, original-audio authentication, holdouts or causal AI improvement. The common album answer is not a twelfth independent example. Grounded AI credit remains zero, certificates false and writable deltas empty.
- The existing runtime still supplies one album-level genre. This integration changes neither eligibility nor runtime, prompt construction, retries, guards, provider precedence, meaningful versions, collaborator credits or user decisions. It adds no live endpoint or approval capability.
- Four repeated Amala `02. Cookie Jar.flac` postwrite hash comparisons in the retained paired runs are unavailable because their baseline and postwrite entries are redacted. Equality of redacted values and the native boolean are not independent postwrite proof. All observations remain in denominators.
- The retained native/model observations are scripted loopback evidence. Entire parsed request JSON matched with and without the sidecar, but this is not raw HTTP byte identity, universal contamination prevention or executable provenance attestation. Native successes, cancellations and unknown-usage controls were not rerun for delivery.
- The initial unoptimized run retains scope-expiry failures. Later optimized runs used the documented build settings without increasing limits. Neither record is deleted or silently promoted to a stronger result.
- Checksums bind recorded bytes, not user approval or tamper-resistant storage. Live approval, privacy, billing enforcement and musical truth remain unresolved and outside this delivery.

No push, fetch, live model/provider request, original-library access, media write or activation is authorized by this record.
