# Evidence-only primary artist retention corrections

The three reviewed retention defects are corrected locally. Master metadata no longer supplies concrete-release observations. Album-content conversion retains only its chosen representative's observations. Automatic and manual candidate JSON now expose the new field in the spelling declared by `DesktopAPI`, while accepting the earlier populated snake-case spelling.

No eligibility or identity diagnostic consumes the observations. These corrections do not change artist aliases, display credits, provider budgets, requests, donor predicates, or write authority. The proposed correspondence policy remains unapproved and unimplemented.

This report supplements, rather than rewrites, `.planning/research/candidate-primary-artist-variants-20261110.md` and the policy review. The earlier conversion and wire coverage claims were incomplete. Their frozen outputs, failed attempts, and source findings remain intact.

## Boundaries and regression results

Only three production files changed during this correction task: `src-tauri/src/state/providers.rs`, `src-tauri/src/commands/auto_tag.rs`, and `src-tauri/src/commands/album_search.rs`. Incoming `src/shared/desktop-api.ts`, `docs/CHANGELOG.md`, and both rehearsal tools are unchanged. The existing changelog entry describes evidence retention, not matching improvement, so no additional behavior claim was added.

| Defect | Red through the ownership boundary | Correction and green |
| --- | --- | --- |
| Master ID treated as release ID | Public `DiscogsClient::master_metadata` and concrete release fetch receive equal numeric IDs and the same authentic release body. Master incorrectly retains the observation. | Resource kind reaches `parse_discogs_release` from each real caller. Only explicit release origin retains observations. Master title and tracks remain unchanged. Unknown, search, and absent resource kinds retain no observation. |
| Content construction drops observations | Real `album_content_candidate` returns an empty vector from compatible competing candidates. | Both candidate orders retain exactly the selected representative's own pair, not another candidate's pair. Local edition claims do not rebind observations. Album, release, and recording IDs stay suppressed. |
| Native wire spelling differs from `DesktopAPI` | Actual native automatic and manual candidate JSON lacks `discogsArtistNameVariants`. Actual JSON through mocked invoke also fails typed consumption. | Both structs emit camel case for this field only. Nested observation casing matches. Populated prior snake-case caches deserialize and reserialize correctly; missing legacy vectors remain empty. Other wire fields retain their spelling. |

The combined native red run exits 101 with three intended failures and two passing controls. The green run exits 0 with five passing focused tests. Actual native JSON through the unchanged adapter records two red failures, then two green passes. That adapter check mocks invoke. It is not a packaged Tauri round-trip.

The master test is a synthetic resource-kind control on the genuine master call path, not an assertion that this release body is an authentic catalog master. The one-track content-construction test is also synthetic. Its accepted request alias and duration are test inputs, not added evidence for the real Crowd Lu case.

Existing controls remain for absent or blank ANV, missing or mismatched bindings, another entity, homonym search hits, guest-only assertions, separate joint primaries, preferred-candidate ownership, successive requests, display conversion, SQLite reopen, and legacy rows. Tests assert parsed or candidate evidence, not private call order. The raw motivating body remains Discogs `37231524`, `artists[0]`, ID `2461941`, canonical `Crowd Lu`, ANV `盧廣仲`, SHA-256 `1798a65076cb0ab25f376751bf30fbc42677702ec54b5ffe29f4139f0c939b1a`.

## Fresh paired observation without broader authority

The new strict-loopback replay uses the same eleven generated Crowd Lu inputs, authentic route pool, locked three-profile selection, job definitions, and assistance-disabled settings as the retained baseline. It executes three jobs and six cold/warm evaluations, with 66 repeated file observations. The comparison uses six historical baseline evaluations, not six additional fresh evaluations. These exposed diagnostic fixtures are not holdouts, original music, acoustic proof, or model-quality evidence.

The baseline executable SHA-256 is `add2754fa54176d54b8f857d346f951e700d45ce95ab38cc5bcfdc25a6c61cfb`. The new executable is `6fdbf49d0e16c662920581577432db3e6588a201997ddfce28ac374651b60d30`. Its execution plan binds the executable, source, generated inputs, authentic provider pool, selection, and zero model budget. All 1,780 current execution locks match after execution. Only the three authorized Rust sources differ from the prior plan's source hashes.

| Profile | Provider exchanges, cold + new warm | Written per phase | Empty genres per phase | Genre donor |
| --- | ---: | ---: | ---: | --- |
| `folder_filename` | 16 + 0 | 11 | 0 | Discogs `37231524` |
| `assisted_without_ids` | 16 + 0 | 11 | 0 | Discogs `37230261` |
| `tagged_recovery` | 15 + 5 | 11 | 11 | None, artist conflict |

There are 52 new provider exchanges, zero model exchanges, and zero fixture misses. Each profile has its own process, state, cache, and generated worker media. Warm reuses only its corresponding cold cache and receives fresh profile-equivalent media. Cold caches are empty; warm cache counters and profile baselines match the recorded contract.

The strict comparison passes. Full phase records, proposed and readback tags, writes, selected IDs, content-only suppression, physical mapping, payload hashes, provenance, review state, cache counters, donor predicates and reasons, and ordered provider request/query/status arrays are unchanged. It also compares the complete non-authorizing evidence snapshots. Allowed differences are the temporary UUID root, elapsed time, compiled `auto_tag.rs` hash, and outer observation key spelling. In this replay the underlying pairs, ownership, and locations are unchanged; no extra observations are accepted by the comparator. The master and content fixes are independently exercised by their regression boundaries, not falsely claimed as replay paths.

Every retained pair is checked against its own raw primary credit and candidate source/release. Comparator negative controls reject changed binding, cross-provider attachment, alias insertion, genre changes, eligibility changes, numbering changes, and writable/certificate changes. All twelve compared exports remain `decision: insufficient`, `acceptedCertificate: false`, and `writableDelta: []`.

The unchanged source-field auditor runs on all twelve compared phase records and reports four `source_eligibility` findings per arm, in the two non-recovery profiles. Recovery's earlier artist rejection still prevents later donor predicates from running. It does not establish that those predicates pass. Missing durations, final acoustic qualifier, independent album/title/credit certification, and selected-edition questions remain unresolved. The historical 84 findings and original four rehearsal findings are unchanged. Overall safety remains non-clean, with zero grounded-assistance accuracy credit.

## Independent re-review and remaining limits

A fresh, independently scoped read-only specification reviewer inspected the corrected full retention diff against HEAD and the approved plan. It found no remaining specification defects. It verified the three corrections, preferred-candidate ownership, raw display preservation, cache compatibility, unchanged request limits, and absence of eligibility or authority consumers. This is a new code review, not reuse of the earlier standards or proposed-policy reviews. The retained disposition is `.amp/in/artifacts/primary-artist-retention-corrections-20261110/independent-spec-review.md`.

The reviewer observed a still-running broad gate and did not certify its later result. The review and parser checks do not establish semantic artist equivalence. SQLite serialization retains values, but does not prove that a cached observation is associated with its originating HTTP receipt or was available to a particular later request. No receipt timestamp, origin, user decision, cross-provider equivalence, or performer certificate is invented. Legacy empty rows stay empty until an ordinary fetch. There is no cache migration, invalidation, forced refetch, or namespace expansion.

## Preservation and reproducible evidence

Evidence root: `.amp/in/artifacts/primary-artist-retention-corrections-20261110/`. It contains red/green logs and exit codes, actual native wire JSON, adapter checks, source archives, the preserved executable, replay execution plan and logs, ordered inventories, phase records, donor/request/evidence snapshots, strict comparison and negative controls, independent review, source audit, and preservation results.

Streamed verification checks 3,765 incoming files and all 2,132 entries in the old retention manifest. Only the three authorized Rust files differ. Old manifests, raw source receipts, contracts, packet, seals, earlier reports, unrelated archive and Python cache, and historical runtime evidence remain untouched. The 2,132-entry manifest itself retains SHA-256 `972bb2f8854c5704622b9897851feacb6b9c0fb7b3ca3dcdd9034346294765b4`. This is a disclosed source delta, not an unconditional old-manifest match.

Disk pressure was handled without deleting historical evidence. Four disposable incremental compiler session directories created during this correction task were removed after source/executable preservation; their exact paths are retained in `compiler-scratch-cleanup.json`. Six closed new evidence snapshots were losslessly compressed to xz, streamed back, and checked against their original SHA-256 before redundant raw copies were removed. Compression receipts retain the uncompressed lengths and hashes. No replay failure or earlier failed attempt was erased.

The new comparator and its three negative controls pass. Final-source checks also pass 18 exporter, eight safety-audit, four source-field, five preservation, ten frozen-packet, and four scheduler tests. No old expectation or checker was relaxed.

Fresh `just --no-dotenv check` exits 0 on final source. It passes TypeScript checks, 856 renderer tests, 1,269 desktop Rust library tests, 735 server Rust library tests, nine provider-mock tests, ten review-evaluation tests, fourteen media-evidence integration tests, and five recovery CLI tests. Ignored counts remain 29 desktop-library, seventeen server-library, and twelve media-evidence tests. They are not claimed as executed. `cargo fmt -- --check` and `git diff --check` also pass. Broad logs and exit codes remain under the evidence root. This is not a live-provider, model, acoustic, original-library, or packaged UI/Keep verification.

The new `evidence-manifest.json` binds the final report, independent review, source and executable archives, tools, source/input locks, and raw new runtime evidence. All 102 source files in `source-final.tgz` match their current source counterparts. The archive also retains 109 macOS AppleDouble metadata entries. The first finalization attempt incorrectly treated those metadata entries as physical source files and failed before writing a manifest; its error is retained. The corrected source check explicitly separates metadata without changing the archive. The six compressed streams have lossless verification receipts. This new manifest does not replace any earlier seal or source certificate.

These commands reproduce the focused native check and comparison without live provider or model calls:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server --lib primary_variants -- --nocapture
python3 -B .amp/in/artifacts/primary-artist-retention-corrections-20261110/test_comparison.py
python3 -B .amp/in/artifacts/primary-artist-retention-corrections-20261110/compare.py \
  .amp/in/artifacts/primary-artist-variants-20261110/after-retry \
  .amp/in/artifacts/primary-artist-retention-corrections-20261110/replay \
  /absolute/fresh/comparison.json
```

The last command needs an existing output parent and an unused filename. It requires the retained local artifacts, not merely a fresh clone. Actual native wire checks use `SOUNDROBE_VARIANT_WIRE` pointing at retained `green-wire.json` and the validation-only `native-wire.test.ts` through Vitest's programmatic runner with its test include overridden. They do not provide a live bridge certificate. Any future native replay requires another explicit bounded run and a fresh output directory; none is proposed or authorized by this report.

All work remains local and uncommitted on `artist-library-inventory` at `e9edf1c0086cc60ffffcea6e5970d4b90ae27327`. Nothing was pushed, deployed, or run against original media or live providers/models. No new thread or worktree was created. Using observations for identity or eligibility remains a separate unimplemented policy decision.
