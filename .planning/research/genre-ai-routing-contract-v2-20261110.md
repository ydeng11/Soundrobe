# Enya routing contract correction, version 2

## Authorization and exact scope

The user authorized: "Correct the Enya routing test expectation from source evidence, preserve the old results, and rerun the audit."

The original experiment and its report remain unchanged at `.amp/in/artifacts/genre-ai-gaps-20261110/` and `.planning/research/genre-ai-gaps-20261110.md`. The addendum evidence is `.amp/in/artifacts/genre-ai-routing-contract-v2-20261110/`. Native inputs and outputs are read-only references to the original experiment, not new executions.

Only `/jobs/215/expectedModelRequests` changes, from 2 to 0, for `hold-accepted-folder_filename-f7756354af37`. This is the total per cold/warm process, so each phase changes from one expected metadata request to zero. The script remains registered, with its original response and `maxRequests: 2`. All other jobs, scripts, budgets, provider captures, profile baselines, expected outcomes, and field oracles are unchanged.

The original selection SHA-256 is `135fea8766e5a01355065b5a0a1d56bff8b8f11098bc33d018ecca2d200d5fc3`. The corrected selection SHA-256 is `5bdfa3f5af43ed1237bd6ca3a1dc3937072419120c264794f036c165f8256d86`. `freeze.json.diff` shows the sole changed line. `contract-correction.json` records authorization, selectors, source/excerpt hashes, the original seal, and phase-specific discovery evidence.

## Derivation from source and discovery evidence

`DiscogsDiscovery::detail` in `src-tauri/src/commands/auto_tag_editions.rs` rejects another uncached detail request after the existing forty-detail limit. It returns `Discogs discovery incomplete: release detail limit reached`. This leaves relevant evidence unexamined. It does not establish that all eligible releases were searched and none matched.

In `src-tauri/src/commands/auto_tag.rs`, `provider_error_status` classifies incomplete/budget diagnostics as `Incomplete`. With no credible selected candidate, `provider_authority_decision` prioritizes an incomplete attempt over `Providerless`. `providers_confirmed_no_match` requires `Providerless` and, when present, an AcoustID `NoMatch`. The production metadata caller additionally requires no candidate. Thus an incomplete Discogs search cannot enter `resolve_tags_via_llm`, even when no candidate was selected from the examined subset. The later limit/budget diagnostic maps the held result to `discovery_budget_limited`.

Both frozen Enya phases independently contain zero credible candidates, MusicBrainz `no_match`, Discogs `incomplete`, forty-detail discovery traces, and the explicit incomplete-discovery receipt. The preceding provider-only baseline has identical provider attempts. These source-bound discovery facts establish the false no-match gate before considering observed model-request counts. The metadata script must remain unexercised because that branch is ineligible, not because its scripted answer is rejected or the model is disabled.

The full source and seven selected excerpts are hashed in the correction record and match the frozen compiled-source locks. Source archive SHA-256 remains `ee38c6c50b9726a2a47e3cc3eaaaf07c1640086c17d2d6de9e77aab0c3ed00a4`; executable SHA-256 remains `69452eb3021d8c2f215a8e45e3b416ec0bd47a8d70221d7b95bc033ef5ac6388`.

## Old and corrected verdicts

| Check | Original frozen contract | Corrected routing contract |
|---|---|---|
| Primary audit | Exit 1, two Enya `model_count` findings | Exit 0, no routing/behavior-contract findings |
| Strict exchange checker | Exit 1 on Enya count expectation | Exit 0, all 440 phase records reconcile |
| Strict postflight | Exit 1 on the same count expectation | Exit 0, `pass_under_corrected_routing_contract_v2` |
| Construction comparison | Exit 0, no findings across 348 writes | Exit 0, byte/value-identical comparison result |
| Paired hash comparison | Exit 0, no unexpected changes across 5,322 observations | Exit 0, identical result |
| Supplemental source-contract audit | Exit 0, no findings | Exit 0, unchanged except removal of the two routing labels |

The primary audit, strict exchange checker, supplemental checker, construction comparison, paired-hash checker, and original negative-control checker are byte-identical copies of their sealed tools. They run against the separately corrected selection and write only new outputs. The new postflight preserves the original execution-selection binding rather than pretending the native driver used the corrected selection. It also checks the exact one-field correction and permits only removal of the two declared routing labels from audit results. No other per-file, provenance, review, ID, cancellation, or local-choice result changes.

The focused checker first fails under the old selection, then passes all six tests under the correction. It rejects an unexpected metadata request, an unintended write, and unauthorized fallback candidate/authority. Its positive controls preserve all five other scripted held-release jobs, cold and warm. Their completed provider no-match attempts still require one metadata request per phase, and removing that request fails the audit. The four original collaborator/provenance/model-role/local-choice negative controls also pass.

## Reused coverage and remaining findings

The reanalysis covers the same 220 jobs/440 existing evaluations, 258 distinct physical input paths, and 5,322 repeated file observations. **New native evaluations, provider requests, and model requests are all zero.** Existing receipts still contain 10,529 provider requests and 352 scripted model requests, with 342 genre and ten metadata roles, no fixture misses, and the original bounded budgets. No model script or provider capture is edited.

Outputs remain 852 AI-inferred/unverified, 26 provider-sourced, ten retained explicit local, and 4,434 missing genre observations. The 348 written evaluations, 24 held evaluations, and 68 no-write cancellations are unchanged. All 440 evidence-only exports remain insufficient, with `writableDelta: []`, `acceptedCertificate: false`, and zero grounded-assistance accuracy credit. Export results and 2,200 compiled-policy bindings are reused under verified unchanged hashes, not regenerated.

The unchanged field auditor still reports 360 assistance-boundary, seventy missing-genre-receipt, seventy unsupported-genre, and two source-eligibility findings in this repeated selection. Legacy results remain 42 selected-credit, ten collaborator, eight numbering, and zero title findings across 278 applied rows. The historical 84 certification findings and original full-matrix disagreements remain intact. A clean corrected routing audit is not overall clean tagging safety, genre truth, provider-source completeness, or live-agent verification.

The original broad and native targeted gates are reused under exact source/executable bindings, not rerun. Fresh work consists of Python checker tests, audits, comparisons, and integrity verification. The original ignored-test limits remain. Twenty-four independent payload hashes remain redacted, every unexported tag is not independently proven, and earlier Keep UI acknowledgment remains component-stub evidence rather than packaged integration. No new UI check is claimed.

## Integrity and delivery

The preflight verifies all 5,358 files in the original seal and 3,691 source/2,233 input locks before deriving the correction. The original manifest SHA-256 remains `86d69ab72c3c8e62806763f72ddd063d90eb5453bd1e54728e99b2b5334bdfd8`. The active preview log is outside that original seal. Final streaming postflight passes and rechecks those original files, 11,751 incoming/historical locks, all 3,361 preceding-matrix files, all 220 inventories, the source archive, executable, actual input/script/driver bindings, exports, and unchanged branch/HEAD/staging state. Its result records `originalFrozenAuditPassed: false` and `correctedContractAuditPassed: true`, with zero new native evaluations/requests. Its result and the addendum seal are separate from the immutable original verdicts. Original `frozenAuditPassed: false` records are never changed.

Reanalysis runs use `python3 -B` to prevent imports from creating bytecode files inside the sealed evidence. The retained commands are `prepare-correction.py`, `test-contract.py`, `test-audit.py`, `audit.py`, `exchange-summary.py`, `construction-diff.py`, `paired-hashes.py`, `supplement.py`, and `postflight.py` in the new directory. Each has a fresh log/exit record. `contract-red.log` records the initial failing contract test, and `contract-green.log` records all six tests passing after the one-field correction. `checker-locks.json` binds the executed checkers and corrected selection. The addendum manifest binds new files and records symlink targets to old inputs separately. A reproduction needs a fresh sibling evidence directory because outputs use exclusive creation; it must not write into either sealed directory.

New paths are this addendum and its local evidence directory. Production, maintained test/tool sources, the original experiment/report, unrelated work, branch, and HEAD remain unchanged. Nothing is committed or pushed. There are no original-library reads/writes, network/model calls, deployments, new threads/worktrees, policy changes, budget changes, or additional write authority. Nothing needed from you right now.
