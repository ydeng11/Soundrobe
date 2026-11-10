# Non-authorizing performer evidence exporter

## Outcome and delivery

The existing offline native evaluator now exports original prewrite observations and raw-source guard diagnostics. An offline exporter joins them to the exact received fixture receipts. Every result has an empty writable delta, `acceptedCertificate: false`, and an insufficient overall authorization decision. No resolver, writer, source authority, production guard, certificate acceptance, oracle, or historical finding changes in this task.

This work remains local and uncommitted on `artist-library-inventory`, HEAD `d7659077a5030c5ca627d99b6e967343d3597e28`. That HEAD is the existing pushed branch checkpoint, not local or remote main. Incoming uncommitted production and evaluation work remains present. No commit, push, original-media access, live provider discovery, paid model, new thread/worktree, or deployment occurred.

The contract is the reviewed `.planning/design/field-scoped-performer-certificate-20261109.md` and its review outcome. This exporter implements the separately authorized observation-only unit, not the proposed future performer certificate or write integration.

## Changed paths

- `src-tauri/src/commands/auto_tag_eval.rs`: declares a test-only evidence module and writes one phase observation beside existing donor traces. It reuses the preflight request, baseline reader/hashes, donor trace, and already fetched fixture inventory. It adds no provider call.
- `src-tauri/src/commands/auto_tag_evidence.rs`: new test-only raw-receipt diagnostic module and native regression. It invokes the unchanged track and recording guards in two explicitly labeled conventions, retaining arguments, results, recording disambiguation, selectors, query, and receipt hash. Diagnostics are not proof of production route execution.
- `scripts/auto-tag-eval-replay/export-evidence.py`: new offline exporter. It validates profile/prewrite bindings, immutable physical hashes, exact source receipt/query/status/body identity, and complete diagnostic matrices/arguments. It separates mapping, raw duration and credit levels, catalog support, qualifier applicability, authorization-donor versus conflict-source scope, unresolved evidence, and bounded-attempt limits. It reads no media and makes no requests.
- `scripts/auto-tag-eval-replay/test_evidence_export.py`: eighteen export-contract regressions using genuine frozen captures and explicitly labeled synthetic mutations.
- `scripts/auto-tag-eval-replay/README.md` and `docs/CHANGELOG.md`: observational-export usage, scope, and limits. Existing entries are preserved.
- `.planning/plans/performer-evidence-export-20261109.md` and this report: local plan and evidence record. Review artifacts and reproducible comparison tools remain under `.amp/in/artifacts/performer-evidence-export-20261109/`.

The native evaluator itself is under `#[cfg(test)]`. No production tagging module is changed relative to this task's incoming snapshot. Comparing against HEAD alone would wrongly attribute earlier work to this task.

## Red and green contract evidence

All logs are retained under `.amp/in/artifacts/performer-evidence-export-20261109/`.

- `native-red.log` records the native regression failing on missing raw `explicit` disambiguation, followed by the passing native regression in `native-green.log`.
- `export-red.log` records seven behavior-facing export-contract failures against the initial empty implementation. Subsequent receipt, profile, totals, query-format, and independent-review failures remain separately retained.
- `contracts-final.log` records eighteen passing Python export tests. A fresh final rerun also passed all eighteen. `safety-audit-contracts.log` and a fresh rerun passed the existing eight safety-auditor contracts.
- Genuine negative captures cover the wrong-bonus 22-track packages, standard 20-track Discogs 2058854, the 26-track single/video package, and unrelated Radio Hits guest occurrence. Missing selected Discogs principals/durations and genuine track/recording co-billing versus featuring disagreement remain insufficient, not invented complete credits.
- Explicitly synthetic controls cover incompatible guest identity sets, reversed featuring direction, name-to-ID swaps, missing/ambiguous correspondence, one-sided and contradictory qualifiers, release-context contradictions, bad nested durations, bounded-source incompleteness, receipt/guard tampering, and cross-profile evidence leakage.

Exporter SHA-256 is `91cefad3c1d780d65d3468f359ce2c3cb95d92fd8bf67d6b3301689ee73d04ad`. Test SHA-256 is `25e0ef0c0f2c1070e3f97beed78a45d95aadd9174d949f4d3efd7f1f3764e45b`. These match the final independent re-review.

## Fresh paired native replay

`replay.cjs` freezes identical source manifests, case selection, process configuration, and default budgets. It runs Relapse and Desperado across `folder_filename`, `assisted_without_ids`, and `tagged_recovery`, with cold and warm phases. Each condition has six isolated jobs, twelve evaluations, and 198 physical-track evaluations/readbacks. These are repeated silent dummy inputs, not 198 independent recordings. Warm phases use a reused cache and separate disposable media.

The before executable SHA-256 is `e1a41164aa9413822335ef3a6aba835463c0c0082ceabe498a8fe7df76b2fb70`. The after executable is `b1fe3ca00bee4c05f27e5dd9dfc45da951d4603b59ddae783b7e3bb53ef0d394`. Both binaries, build logs, source archives, selections, wire inventories, native records, and final exports remain retained. The provider manifest SHA-256 is `d9f63a13681cba375b29e8d304c7ce8e2cf6ae6fc692fdb4ec1b3dec7cb55521`.

Final cumulative wire-request counts match before and after for every job:

| Profile | Desperado | Relapse |
| --- | ---: | ---: |
| Filename only | 53 | 60 |
| Existing tags without IDs | 53 | 90 |
| Tagged recovery | 40 | 13 |

Each condition consumed 309 wire requests, with zero misses and zero model exchanges. The independent comparator compares every phase field, donor trace, cumulative request trace, final fixture inventory, and execution record. That includes tag values, selected IDs, mappings, writes, persisted provenance, review state after restart, progress, payload/readback, cache observations, exchanges, and observed budget results. It does not infer unobserved scheduler state or exhaustive discovery.

The initial strict comparison retained 192 differences in `comparison.json`. All are disposable held-file inode/ctime/mtime or source observation clocks. Source code in `src-tauri/src/infra/audio_evidence.rs` derives these from held-file metadata and current time. Fresh copies necessarily differ. The revised `compare.py` retains every raw difference but classifies only those exact paths as expected, requiring different disposable roots, matching relative paths, identical expected/before/after source hashes, equal byte counts/device, and complete stable held-file observations without errors or aliases. It also requires exact native/persisted diagnostic equality within each run. It still compares all content, decoder, integrity, PCM, and other provenance fields. `semantic-comparison.json` records zero unexplained differences and all 192 expected runtime facts. Thirteen synthetic comparator controls check this boundary, key-presence/type comparison, and root-only timing normalization. The only general normalizations are evaluation-root elapsed time and the disposable UUID prefix. No speed claim is made.

`final-exports/` contains twelve newly generated observations bound to actual native snapshot bytes, semantic snapshot digests, compiled policy hashes, exact manifest, and final exporter bytes. All twelve are insufficient and non-authorizing. The exploratory `first-real-export.json` and `first-relapse-export.json` retain older exporter hashes and are not final-freeze evidence.

## Real-case decisions and existing limitations

Relapse M `10e769d6-c110-4e1f-934e-2d12677d4fbb` has supported deluxe catalog content and direct matching track/recording Eminem and Dr. Dre credits. Track 13 retains 278-second release-track duration and 278.16-second recording duration. Four diagnostic guards can pass while raw explicit applicability remains unresolved. This is supported catalog evidence, never write authorization. M has no genre/style evidence and cannot repair selected Discogs S's missing principals or bonus durations.

Selected Discogs 36441795 remains selected wherever available. Actual prewrite observations preserve the existing richer-profile My Darling 22 and Careful What You Wish For 21 values separately from filename-derived 21/22. Filename-only native clearing really removes those numeric tags; the exporter does not restore them from corpus descriptors or other profiles.

The unchanged baseline native workflow writes those bonus tracks as 21/22 and can reorder collaborator display in the existing-tag profile. It also already adds Dr. Dre on Old Time's Sake. The exporter does not cause, repair, or newly authorize those effects. The paired records reproduce them unchanged. This is not a clean tagging-safety verdict or a claim that production output preserves all proposed future certificate requirements.

Desperado's filename-only instrumental applicability remains unresolved. Captured richer-profile instrumental tags are assisted metadata claims, not independent acoustic truth. The durationless Discogs 20260261 remains insufficient, not a proven wrong composition. No new genre or ID authority follows from this export.

Every export explicitly states that exhaustive pending/cancelled relevant-source closure and the separate local decision/cleanup store are unavailable. Cumulative received telemetry can prove which receipts arrived, not which relevant work was never scheduled. A closed process is not a closed relevant-source set. New relevant receipts or changed inputs/policies invalidate the observation. No full general variant parser, artist-alias authority, or performer-write decision is implemented.

## Independent audits

Read-only reviewer `TU-034utkUzdGK51xM57e6fpX` challenged the exporter against raw captures, not native acceptance. It found six reproducible observation errors and a filename-position binding gap. Red controls reproduced them. Accepted corrections bind exact guard query/matrix/arguments, distinguish missing principals from full performer assertions, include Discogs Featuring in complete sets, bind names/IDs/relationship direction, retain unscoped release variants as unresolved, require both duration levels for conflict eligibility, and distinguish missing correspondence from demonstrated wrong bonuses.

Its follow-up found all original controls resolved at the frozen exporter/test hashes. It verified all nineteen source-body hashes and raw-field equality, regenerated four actual cold/warm native snapshots in memory with the actual receipt manifest, and rejected actual-snapshot guard/query/input mutations. It confirmed Relapse's four passing guards still yield unresolved applicability. This is one independent same-model reviewer with a follow-up, not human or multi-model sign-off. It did not independently re-execute Rust guard outcomes or establish acoustic truth.

Read-only auditor `TU-034uu3oQsEsmPGbpW5MBLV` independently compared key-preserving complete before/after records and found zero substantive mismatches. It exactly reproduced all 192 runtime differences, verified sixteen actual audio observations per condition duplicated in native/persisted records, and checked complete stable source/PCM/decoder identity. It additionally verified all twelve donor and request pairs, six fixture inventories, six execution records, checkpoints, aggregate summaries, and 1,534 selection locks per condition. Each condition consumed 162 MusicBrainz and 147 Discogs calls, including eight additional warm calls, without a model request or fixture miss.

That auditor caught a comparison-tool gap: dictionary `.get()` treated an absent key like explicit null. `comparison-key-red.log` reproduces the failing contract. The comparator now preserves keys and types, only removes evaluation-root timing, and checks the narrow runtime boundary. `compare-key-green.log` records the fresh zero-mismatch result. The independent key-preserving audit had already found no absent/null mismatch in the actual records. The auditor accidentally regenerated `semantic-comparison.json` while extracting concurrently revised helper definitions; it disclosed the side effect. The owner subsequently reran and froze the final comparator. No source or native output was edited by either auditor.

Neither exporter contracts nor these audits change the 84 historical findings, old exact-edition certificate, or oracles. Scripted assistance grounded-accuracy credit remains zero.

## Closing verification

`postflight.json` verifies all 3,564 incoming protected files, with exactly the three intended existing-file changes: evaluator, README, and changelog. Staging, branch, HEAD, other incoming source work, old findings/oracles/certificates, frozen evidence, dummy media, the unrelated iOS archive, and `scripts/__pycache__/` remain unchanged. It validates 1,534 selection locks per condition, all 95 files in the after-source archive, task-source hashes, and both frozen executables. `exports-verification.json` independently binds all twelve final observations to current compiled policies, raw snapshots, actual receipts, and final exporter bytes. Formatting and whitespace checks pass.

Fresh `just --no-dotenv check` exits 0. It passes typechecking, 856 renderer tests, 1,250 desktop-library tests with 29 ignored, 716 server-library tests with seventeen ignored, fourteen inventory-audio tests with twelve ignored, five recovery-CLI tests, nine mock-provider tests, and ten evaluation-tool tests. `check.log` and `check.exit` retain the complete result. Credentialed/live tagging, real-display smokes, and strict Clippy were not run. Ignored suites are not claimed as passing. The native exporter regression is included in both library gates, and the separate eighteen Python exporter and eight safety-auditor regressions pass without skips.

The task has no implementation blocker. Its exporter-contract verdict is passing within the bounded scope, with zero substantive paired replay differences. The existing tagging-safety verdict remains non-clean and all 84 findings remain unchanged. Raw logs, generated dummy sources, frozen before/after binaries/source, original failed controls, and final exports remain local. No delivery beyond local uncommitted changes is claimed.

The next bounded work is a review of the exported evidence and unresolved applicability/presentation policies. Performer writes need separate authorization, a genuinely supported applicability-positive case, protected-decision observations, and red/green validation. No second tagger or general certificate framework is proposed.
