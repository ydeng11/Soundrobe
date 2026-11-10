# Donor eligibility and genre gaps

The investigation explains the native predicates behind the earlier findings without changing tagging behavior or certifying uncertain donors. All 152 scoped evaluations complete. Their audited outputs and provider-request multisets match the sealed matrix. The independent field audit still exits **1** with the same 84 source-eligibility findings. Genre remains absent in 68 written evaluations. A passing driver or comparison does not override those findings.

Desperado's filename-only genre donors reach the native helper but fail its existing version or alignment checks. Relapse's collaborator donor passes native album normalization and full file mapping, while its album label still fails the independent certificate. Neither result authorizes relaxing the version guard or accepting the donor by inspection of its useful-looking credit.

## Coverage and separate verdicts

The frozen selection is the union of the earlier source-eligibility findings and written genre gaps. It contains 76 case/profile jobs across 27 inspected development cases, 327 distinct physical input paths, and 1,830 repeated physical-file evaluations. Every job runs cold and warm against fresh disposable silent FLACs and its own isolated cache. These are not untouched holdouts or acoustic identities. A preliminary focused run adds four evaluations of Desperado and Relapse over the same 33 input paths already in the union. It is not added to the primary coverage denominator.

The earlier full matrix remains 306 evaluations across 51 cases and 539 inputs. This task does **not** freshly rerun the other 154 evaluations.

| Profile | Cold evaluations | Warm evaluations | Written file evaluations, both phases | Source-finding evaluations | Genre-incomplete evaluations | Missing genre file evaluations |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `folder_filename` | 26 | 26 | 632 | 32 | 20 | 216 |
| `assisted_without_ids` | 25 | 25 | 610 | 34 | 16 | 242 |
| `tagged_recovery` | 25 | 25 | 588 | 18 | 32 | 388 |
| Total | 76 | 76 | 1,830 | 84 | 68 | 846 |

All 152 native outcomes are `applied`, with writes and readbacks. That status is not an independently clean completion count. The deliberately selected population contains either an unresolved source finding or a genre gap. There are no holds or cancellations in this selection. The twelve holds in the earlier complete matrix remain earlier evidence, not fresh abstentions here.

| Gate | Actual result |
| --- | --- |
| Native driver | Exit 0; all 76 processes complete |
| Comparison with sealed outputs | Exit 0; zero changed audited evaluations and zero changed provider-request multisets |
| Unchanged independent field audit | Exit 1; 84 `source_eligibility` findings, no other field category |
| Fixture coverage | Zero missing routes |
| Payload, native readback, source media, cache isolation | Checks pass; twelve redacted payload exports still limit independent hash visibility |
| Persisted field receipts, review flags, selected identity | Unchanged against sealed exports |
| Model requests and grounded-assistance accuracy credit | Zero requests; zero credit |
| Fresh `just --no-dotenv check` | Exit 0, with ignored tests disclosed below |

All assistance is disabled, including the profile named `assisted_without_ids`. This is provider safety and completeness evidence, not a full unified live-agent evaluation. Scripted answers elsewhere continue to receive zero grounded-accuracy credit.

## What the native probes measure

Test-only hooks record the actual request, parsed provider candidates, original provider tracks, selected candidate, and request aliases immediately before genre and performer completion. The collector re-evaluates the existing pure native predicates. It does not change candidate selection, provider budgets, caches, assistance routing, or writes.

`strict_provider_evaluation` checks provider backing, normalized album identity, artist identity or request aliases, applicable year/country constraints, and strong coverage of every physical input. Positional-only or unresolved mapping fails. It need not require that the provider's entire release has exactly the local file count.

For a fallback genre donor, `complete_provider_genre` clears edition year, country, release IDs, and per-track IDs from a cloned request. It retains artist, album, mapping, duration, and both existing version predicates. Every mapped track must satisfy `track_versions_compatible` and `recording_versions_compatible`. An already selected provider's nonblank genre follows its prior selection instead of this fallback path. The exported `genreEligible` is a fallback probe result, not a statement that every selected genre executed that branch.

`complete_provider_credits` uses the unmodified request and strict mapping, then preserves nested performer sets and rejects non-nested disagreements. The trace's extra version booleans are diagnostic results; they are not additional executed branches in the credit helper.

Phase inventories contain cumulative HTTP receipts. The analysis subtracts the cold prefix to identify new warm requests. A warm candidate without new detail HTTP is labeled `available_after_cold_detail_without_new_phase_detail_http`, not categorically cache-originated. Candidate traces establish actual availability at the helper; the combined job inventory alone does not.

```{ .diff file="src-tauri/src/commands/auto_tag.rs" }
 ) -> Option<serde_json::Value> {
// @annotation Keep observability in test builds rather than changing production donor authority.
+    #[cfg(test)]
+    eval_tests::trace_provider_donors("genre", request, selected, candidates);
// @endannotation
     if !matches!(
```

## Desperado has a demonstrated version-predicate differential

Case `9373204e076a`, `folder_filename`, reproduces the same result in the focused run and the larger replay, cold and warm. The genre helper receives all ten genre-bearing Discogs candidates. Nine pass strict alignment, but each fails `recording_versions_compatible` for local track 7. The local original title is `Doolin-Dalton`; the mapped provider title is `Doolin-Dalton (Instrumental)`. `track_versions_compatible` returns true, while `recording_versions_compatible` returns false. All eleven local genres remain empty because donation requires every mapped track to pass.

Those nine sources are Discogs `1151167`, `1571575`, `2336317`, `2398669`, `3512330`, `5390450`, `5455029`, `6425118`, and `6800188`. The tenth, `20260261`, fails strict alignment with `ambiguous_comparison` on tracks 1 and 7. No eligible fallback donor is silently skipped in this case.

Representative source receipt:

- Discogs `/releases/2398669`, captured 2026-11-07T16:52:00.897Z.
- Raw capture `test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/folder-identities-2026-11-07/discogs-1eca3fc5640cce90.json`.
- SHA-256 `3d28e8416591895e91d7e645bdba00e19114f99e2cff04ee725e5afe98f7d62b`.
- Genre `Rock, Country Rock`; the independent certificate accepts the reviewed content mapping, while the native fallback additionally compares the original unqualified local title.
- Actual probes `matrix/folder_filename/9373204e076a/donors-{cold,warm}-9373204e076a.json` beneath the evidence directory.

This resolves the earlier availability hypothesis, not the policy question. Feeding already rewritten titles back as independent local evidence, reversing predicate arguments, or removing the instrumental guard would weaken the existing evidence boundary. No such change is made. A bounded next step is to review the original occurrence-to-recording mapping independently, including both `Doolin-Dalton` occurrences, the instrumental, and the reprise, before considering any general change.

## Relapse remains uncertified despite native full mapping

Case `8452b6f2b3ea`, `tagged_recovery`, requests `Relapse (Deluxe)` and retains selected Discogs `36441795`. MusicBrainz `10e769d6-c110-4e1f-934e-2d12677d4fbb` is titled `Relapse`. `normalized_release_identity` explicitly removes bracketed `deluxe` annotations. Native album identity therefore accepts those labels; artist, year, and all 22 local file mappings also pass. The diagnostic version checks report no conflict.

The independent certificate requires the reviewed album identity and does not share that normalization. It rejects the donor with `album_identity_not_certified`. Its frozen raw-title/duration diagnostic has no failed tracks for this donor. That narrows the disagreement to album-label policy but does not establish pressing, mastering, or authorization to supplement credits.

- MusicBrainz `/release/10e769d6-c110-4e1f-934e-2d12677d4fbb`, captured 2026-11-07T14:52:43.686Z with recordings, artist credits, labels, and URL relations.
- Raw capture `test/fixtures/tauri/auto-tag-eval/provider-snapshots/discovery/details-2026-11-07/musicbrainz-22be98f32a7f4174.json`.
- SHA-256 `cfa111b6bf92a5689262e13929fa8451443668b273306deaaaf017f3bdad6a62`.
- Actual cold/warm request, parsed donor, and mapping are in `matrix/tagged_recovery/8452b6f2b3ea/donors-*.json`.

`Old Time's Sake` retains Dr. Dre through this donor. The useful-looking credit is not used to certify its album identity. The separate selected Discogs genre receipt still has missing-duration certificate gaps. Neither certificate is changed. The legacy selected-source credit mismatch remains recorded in the earlier matrix and the unchanged readback comparison.

The no-ID Relapse profile separately lacks genre. Its considered donors fail album identity or full coverage. Standard editions missing the two bonus tracks are not compatible 22-file donors. Its search also includes genuinely unrelated Discogs `38833854`, `Radio Hits 06-34 (Diciembre - A)`, rejected by album identity. That capture has SHA-256 `071d2d15c8ceb1e0403a5cda75e817fc982b85d9d01536eb0c3abd88337b34e1`. A decorated or bilingual label rejection elsewhere must not automatically be grouped with this demonstrated unrelated catalog result.

## All source findings retain their independent limitations

All 86 previously uncertified field-receipt contexts have a corresponding executed native probe. The referenced donors pass native strict evaluation and have no diagnostic version rejection in those contexts. Native acceptance is therefore established separately from certificate eligibility. There are 24 case/source pairs across twenty cases, not 86 different releases or 84 proven wrong writes.

The groups below describe demonstrated failed certificate predicates. They do not issue replacement certificates or collapse coexisting failures.

| Independent limitation | Case/source pairs | Receipt contexts | Disposition |
| --- | ---: | ---: | --- |
| Missing duration, sometimes with artist, album, or title gaps | 10 | 32 | Insufficient source evidence; retain every coexisting gap |
| Full-release count versus scoped local content | 6 | 24 | The certificate cannot express the native scoped mapping; independently review scope before a future certificate revision |
| Single-artist equality or unreviewed artist alias | 4 | 16 | Joint credits and Romanized/CJK names need separate identity evidence |
| Native normalized edition label versus exact certificate | 2 | 4 | Relapse and Amala labels require an explicit field-scope decision |
| Native title alias or bilingual mapping versus exact certificate | 2 | 10 | Preserve the title, artist, and album gaps shown in each receipt |

For example, the four selected-media pairs involving `222668c9d053` and `4147560c89fd` use Discogs `14782838` and `7832914`. Native strong local mapping can accept a scoped disc while the certificate rejects the full source count. The two duet cases reference Discogs `17028138`; native aliases accept the joint artist, while the certificate's single-artist test rejects it. These are concrete certificate limitations, not a reason to accept the ten missing-duration pairs.

`source-classifications.json` binds every category to capture SHA, failed certificate predicates, native strict results, and all case/profile/phase contexts. `receipt-predicates.json` preserves the original failures and exact probes. `case-breakdown.md` lists every covered context, not just representatives. No oracle, certificate, title/version mapping, or performer expectation is revised.

## Genre gaps have two observed availability classes

Fifty genre-incomplete evaluations have genre-bearing candidates at the native helper, but none passes all fallback predicates. Eighteen have no genre-bearing considered candidate. Across repeated candidate occurrences, the first rejection counts are 142 album identity, 98 strong file coverage, 40 artist identity, and eighteen recording-version conflicts. These are candidate occurrences, not evaluation or distinct-release counts. Earlier failing predicates prevent later ones from being evaluated, so these are not complete semantic diagnoses of every rejected release.

The eighteen no-genre-candidate evaluations include existing-tag profiles of Amala, The Memory of Trees, Watermark, and Shepherd Moons, plus tagged-recovery What a Folk !!!!!!. Their inventories and parsed traces, rather than the global capture manifest, establish absence at the helper. Amala's existing-tag profiles contain genre-less MusicBrainz candidates even though a genre-bearing Discogs capture exists elsewhere in the frozen fixture inventory. This does not prove that a fetched eligible donor was lost or authorize a larger search budget.

The Crowd Lu recovery example `036e0a00e45b` requests artist `盧廣仲` with an empty alias list. Discogs `37231524`, artist `Crowd Lu`, fails the native artist predicate. Its filename-only request instead contains `卢广仲`, `クラウド・ルー`, `Crowd Lu`, and `盧廣仲`, and that donor passes. This is a difference in available identity evidence, not a requested profile-specific exception. Recovery has no artist-discovery HTTP lookup that could have supplied the absent aliases. The evaluator isolates its alias file as well as its cache; no user alias history is imported. No dropped fetched alias has been demonstrated.

Exile `e8a8a8d6b27d` rejects its decorated soundtrack album labels before track alignment. Those may describe related content, unlike the unrelated Relapse search hit, but the present predicates do not authorize donation. Every such case retains its exact first rejection rather than being declared genuinely incompatible from the error string alone.

`genre-predicates.json` contains each missing evaluation, its file count, phase availability, donor capture receipts, exact error, and diagnostic path. All twenty missing-genre cases and all profiles/phases appear in `case-breakdown.md`. Assistance is disabled in every one. That setting explains why these gaps receive no AI fill, but does not establish whether a grounded assistant could resolve them.

## Verification, failures, and reproducibility

The diagnostic regression fails before hook implementation with zero collected rows, then passes. It distinguishes a wrong album from a compatible donor while retaining provider genre authority and selected edition identity. An initial green attempt has a compile error from calling `.len()` on an optional group value; the corrected attempt and both failures remain retained. Two provider-genre tests, one provider-credit guard, and four unchanged field-auditor positive/negative contracts pass. The first credit filter matched zero tests; the corrected filter runs one and is the claimed result. Rustfmt and diff whitespace pass.

The fresh deterministic gate passes 856 renderer tests, 1,249 desktop-library tests with 29 ignored, and 715 server-library tests with seventeen ignored. Integration suites pass fourteen inventory-audio tests with twelve ignored and five recovery-CLI tests. Mock-provider and review-tool suites pass nine and ten tests. Shared Rust tests execute under both feature sets. Ignored live, credentialed, and native-display tests are not claimed as run. This fresh pass does not explain historical FFmpeg launch stalls or all earlier timing failures.

The instrumented executable SHA-256 is `6eb14677154d4175c8993cb90f1cfbcde892026a7cc6eb7aed46e89b23006f2e`. Its native source archive SHA-256 is `e3826d32b9be8e83d2a99d0badadb6fd3af7635cb67027255515675784afe0a4`. Postflight verifies 2,231 current selection locks and all 136 prior preflight locks, with exactly the two intentional native instrumentation files changed. All ninety archived native sources match the current files. Unrelated tracked milestone changes match the incoming diff.

The first postflight fails on macOS tar's AppleDouble metadata entries, not on a source mismatch. That failure remains `postflight-attempt1.log`. The corrected checker verifies the AppleDouble magic bytes, verifies the exact ninety-file source inventory, and hashes every source without a waiver. Ninety metadata entries remain in the untouched archive. Postflight then exits 0 and binds 1,312 retained evidence files in `evidence-locks.json`.

Evidence is under `.amp/in/artifacts/donor-eligibility-investigation-20261109/`. The executable, incoming source and diff, frozen jobs, current source archive, raw traces, phase inventories, requests, readbacks, payload hashes, worker evidence, isolation receipts, failures, checks, analysis scripts, and locks remain local. The unchanged provider manifest and sources are referenced from the sealed 51-case run. Temporary evaluator media and actual cache databases are removed by the maintained evaluator after export; they are not claimed as archived databases.

The reproduction entry points, from repository root, are:

```sh
# The driver intentionally refuses existing output directories.
# Preserve these sealed directories before a separately authorized fresh run.
node .amp/in/artifacts/donor-eligibility-investigation-20261109/run-probes.cjs --focus
node .amp/in/artifacts/donor-eligibility-investigation-20261109/run-probes.cjs
python3 -B .amp/in/artifacts/donor-eligibility-investigation-20261109/analyze.py
python3 -B .amp/in/artifacts/donor-eligibility-investigation-20261109/analyze.py --field-audit
python3 -B .amp/in/artifacts/donor-eligibility-investigation-20261109/classify.py
python3 -B .amp/in/artifacts/donor-eligibility-investigation-20261109/postflight.py
```

The field-audit command is expected to exit 1 with the retained findings. Analysis and classification can be regenerated without native execution. A fresh native reproduction needs a copy of the diagnostic directory with empty `focused/` and `matrix/` destinations, the bound executable, and unchanged referenced sources. Do not delete the sealed outputs or rebuild over its frozen executable. `build.jsonl` retains the server-feature build receipt.

## Delivery and bounded next steps

Only `src-tauri/src/commands/auto_tag.rs` and `src-tauri/src/commands/auto_tag_eval.rs` change during this investigation. The former adds two test-only hook sites. The latter owns task-local collection, the diagnostic regression, loopback-only opt-in, and phase exports. Rustfmt also wraps an existing expression without changing it. No production behavior fix is justified by the demonstrated evidence without resolving field-scope or ground-truth uncertainty first.

The plan is `.planning/plans/2026-11-09-donor-eligibility-investigation.md`; this report and the evidence directory are the other new records. The existing uncommitted milestone, frozen baselines, old oracles, failed attempts, raw logs, generated media, unrelated iOS archive, and `scripts/__pycache__/` are retained. Work remains local and uncommitted on `artist-library-inventory` above [d7659077](https://github.com/ydeng11/Soundrobe/commit/d7659077a5030c5ca627d99b6e967343d3597e28). No original media, live/paid model, commit, push, PR, deployment, new thread, or worktree is used.

The next bounded work is independent source review of the Desperado occurrence/version mapping and Relapse's content-only donor album-label policy. Any resulting certificate change must precede new output evaluation and retain the old certificate. Separately, scoped-media and joint-credit certificates can be reviewed without borrowing native acceptance as their oracle. Missing duration and unreviewed bilingual/alias cases remain evidence gaps, not automatic certificate expansions.

`Book of Days (Gaelic only version)` versus the reviewed `Book of Days` remains unresolved. This task does not freshly rerun or independently adjudicate that case. Version and performer expectations are unchanged. No new human or separate-reviewer sign-off is claimed. There is no claim of acoustic/mastering verification, complete library coverage, live assistance accuracy, fresh UI highlighting, packaged-app navigation, transport recovery, in-flight cancellation, or crash recovery. No immediate user action is required to retain and review this investigation.
