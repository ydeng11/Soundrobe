# Album-content collaborator preservation and full reviewed replay

## Result

The native resolver, queued writer, and reader now retain the actual composite
`Billie Eilish | ROSALÍA` display and one-entry artist list for Lo Vas A Olvidar,
case `4060f1de6662`, in `assisted_without_ids` cold and warm. The focused old/new
replay reproduces two unfulfilled preservation receipts before the correction
and none afterward. All 28 native preservation receipts in the complete new
matrix are fulfilled. No performer applicability or write authority was added.

All **153 jobs and 306 evaluations** completed across the original 51 reviewed
cases, three established profiles, and cold/warm phases. These represent **539
distinct physical input paths and 3,234 repeated file evaluations**, not independent
recordings. Overall tagging safety remains **not clean** because the independent
field audit retains all 84 source-eligibility findings across twenty cases.
Export comparison and streaming integrity postflight pass. The sealed manifest
binds all 3,361 closed evidence files, totaling 27,681,614,304 logical bytes.

The evidence root is `.amp/in/artifacts/reviewed-51-content-preservation-20261110/`.
Its immediate comparison is `reviewed-51-preservation-20261110`; its original
comparison is `reviewed-51-provider-first-20261109`. Both earlier runs, their raw
evidence, fixed oracles, certificates, and findings remain unchanged.

## Correction and red/green evidence

`complete_provider_credits` already restored the physical-file-bound presentation.
The subsequent `album_content_candidate` construction used the number of list
entries to decide whether to replace track credits with album-artist presentation.
That test misclassified a composite one-entry list as a solo performer.

The production correction changes one predicate in `src-tauri/src/commands/auto_tag.rs`.
The existing `split_collaborative_artists` determines the number of identities,
instead of `track.artists.len()`. This preserves completed collaborator credits
without adding a parser, donor rule, source selection, or receipt exemption.
Actual solo/missing track-credit album-artist fallback remains available.

`composite-fixture-red.log` records the native regression before the correction:
readback is `Billie Eilish & Rosalía`, not the independently captured
`Billie Eilish | ROSALÍA`. `composite-final-green.log` passes the same observable
boundary using the pinned final executable. `controls-green.log` covers actual
album-artist fallback, missing/stripped credits, incompatible guests, and explicit
disc-total cleanup. Incompatible guests require zero writes, unchanged media,
and zero mock-model requests. Existing numbering, genre, version, payload,
selected-ID, content-only ID-suppression, and cleanup assertions remain intact.

The actual frozen case also runs with old and new executables in all three
profiles and both phases. `focused-verification.json` binds six paired evaluations,
twelve total evaluations over one distinct input track. It confirms exactly two
old receipt violations, zero new violations, and identical ordered exchanges.
`focused-bindings.json` binds source inputs and executables.

Earlier invalid-fixture failures remain retained. `composite-red.log` reached an
artist conflict before the intended failure; `content-green.log` used a missing-input
fixture whose album/folder artist violated an existing source guard. The fixtures
were corrected to exercise the intended paths. Production guards were not relaxed.

## Complete matrix and unchanged findings

Each profile and each phase contains 51 evaluations, 49 written evaluations, and
two holds. Across all phases/profiles, 294 evaluations write 3,132 repeated files
over 522 distinct inputs. Twelve holds cover seventeen inputs. There are no
process failures, fixture misses, cancellations, cache-isolation failures, or
cold/warm output/classification differences.

| Profile | Source findings per phase | Title findings | Selected-credit mismatches | Reviewed-collaborator mismatches | Written evaluations missing genre |
| --- | ---: | ---: | ---: | ---: | ---: |
| `folder_filename` | 16 | 0 | 0 | 0 | 10 |
| `assisted_without_ids` | 17 | 0 | 7 | 5 | 8 |
| `tagged_recovery` | 9 | 1 | 9 | 5 | 16 |

Cold and warm have identical counts. Categories overlap and are not independent
failed recordings. The unchanged legacy auditor exits zero but reports two title
findings, 32 selected-credit mismatches, twenty reviewed-collaborator mismatches,
and four sequential-numbering disagreements. These counts equal the immediately
preceding matrix. The original provider-first matrix had counts 2, 4, 0, and 0.
Preserving composite list representations can disagree with selected-source
equality without deleting a guest. The equality oracle remains unchanged.

The field auditor exits one with 84 `source_eligibility` findings, identical to
the original locations. These are certification gaps, not 84 proven wrong writes.
The conservative legacy score remains 194 written evaluations, or 132 requiring
populated genre, compared with 206 and 138 originally. These are executed-contract
counts, not grounded accuracy. Scripted responses earn zero grounded accuracy.

Sixty-eight written evaluations remain genre-incomplete, covering 846 repeated
file observations over 229 distinct inputs. Bellyache, Marian Hill Remix,
`0e7c8a578cea`, remains held for `ai_not_configured` in all profiles/phases.
Paint The Sky With Stars, `f7756354af37`, retains `discovery_budget_limited` in
filename-only input and `ai_not_configured` in the richer profiles. No reviewed
case newly enters credit-conflict rejection. The native control separately proves
that incompatible credits stop before mock-model fallback.

`case-breakdown.md` retains every case/profile/phase finding, hold, and genre gap.
`supplement-case-profile-phase.csv` adds nested receipts and integrity details.
`matrix-summary.json`, `matrix-delta.json`, and `validation-facts.json` retain the
machine-readable totals and every changed readback field.

## Exact before/after scope

Against the immediately preceding matrix, only Lo Vas no-ID cold/warm changes
readback: four fields, its display and list in each phase. Every changed value
equals that physical file's actual prewrite value. Eight corresponding candidate
fields and two tag hashes change. No other readback, selected ID, mapping,
genre, review, provenance, request, or budget difference is unexplained.

Against the original provider-first matrix, 42 readback fields change, and all
42 equal actual file-bound prewrite values. This includes the earlier captured
numbering and collaborator corrections. Relapse retains My Darling 22 and
Careful What You Wish For 21 rather than sequential 21/22. The comparison also
classifies 84 candidate fields, 36 tag-hash consequences, 360 position-policy
labels, and two legacy-classification consequences. These consequences create
no safety authority. Filename-only readback fields remain unchanged.

Both comparisons account for 306 run identities and 1,104 bounded runtime facts.
Their 56 preservation-receipt rows are annotations of validated native and
persisted receipt occurrences, not 56 new receipts or receipt changes.
`preceding-comparison.json` and `comparison.json` report zero request changes and
zero unexplained differences. Full-file-hash consequences prove unchanged payload
and enumerated reader fields, not preservation of every unexported tag.

The prewrite audit checks 2,156 captured track-number observations, 216 track-total
observations, and 1,148 disc-number observations. It separately identifies 1,078
missing filename-profile track numbers. Captured values remain available metadata,
not certified truth or inferred user locks. No richer-profile tags or IDs are
restored into stripped inputs. The matrix contains no captured disc totals,
duplicate positions, mixed-disc positions, nonblank local genres, or explicit
cleanup inputs. Native preservation/cleanup controls cover those boundaries
separately; the matrix alone does not prove them.

The original narrower preservation checker sees ten receipts and passes its
limited explicit-multi-entry checks. The independent nested receipt audit checks
all 28 native receipts and finds zero violations. Existing source/version guards,
strict donor eligibility, explicit choices, rejection-before-fallback, and
persisted review obligations remain unchanged.

## Exchanges, assistance, and evidence limits

All 153 complete ordered exchange records match both preceding revisions.
There are **5,068 provider requests per revision**, 2,323 MusicBrainz and 2,745
Discogs, with zero misses and zero model exchanges. Warm cumulative receipts are
not counted again as separate requests. Each job has an isolated cold cache;
warm reuses only that job's cache with fresh disposable media.

Assistance is disabled despite the `assisted_without_ids` name. This is frozen
provider-side validation with silent generated media and scripted HTTP responses,
not a unified live-agent evaluation, acoustic identity proof, or provider-truth
validation. Thirty independent payload-hash exports are redacted. Other native
payload checks do not remove that independent-visibility limit. Cache isolation
receipts remain, but disposable cache databases are deleted rather than archived.

The evidence-only exporter remains unchanged and non-authorizing. Source-set
completeness and independent performer applicability remain unestablished.
All 306 new exports succeed with `decision: insufficient`, `writableDelta: []`,
`acceptedCertificate: false`, and zero grounded-assistance accuracy credit.
`export-comparison.json` checks 612 old/new exports, 306 pairs, and 3,060 actual
compiled-policy bindings with zero unexplained differences. Actual snapshot and
decision-revision digests are independently recomputed. The original provider-first
matrix lacks this export contract; no synthetic old snapshots or certificates
were created to fill that gap.

## Fresh gates and source binding

Fresh final-source `env RUST_TEST_THREADS=4 just --no-dotenv check` passes. It is
not reused from an earlier source freeze. Renderer tests pass 856/856; desktop
Rust passes 1,264 with 29 ignored; server Rust passes 730 with 17 ignored;
mock-provider and evaluation-script suites pass 9 and 10 tests; audio integration
passes 14 with twelve ignored; inventory CLI passes five. A separate fresh server
suite also passes but is not another independent population. Ignored credentialed
and original-media tests are not counted as passing.

Fresh exporter contracts pass eighteen, safety-auditor contracts eight, and
analysis contracts twenty-five. The frozen 378-case/3,506-track contract suite,
formatting, and whitespace checks pass. Old failed comparator-fixture tests remain
in the red logs. The correction binds the historical negative fixture to its
actual original run identity, not a relaxed comparison predicate. Locked analysis
tools retain old oracle checks and reject wrong guests or modified receipts.

`source.tgz` SHA-256 is
`ee38c6c50b9726a2a47e3cc3eaaaf07c1640086c17d2d6de9e77aab0c3ed00a4`.
The pinned `native-evaluator` SHA-256 is
`69452eb3021d8c2f215a8e45e3b416ec0bd47a8d70221d7b95bc033ef5ac6388`.
`preflight-locks.json`, `incoming-locks.json`, `selection.json`, and
`analysis-tool-locks.json` bind source, incoming/historical evidence, generated
inputs, reviewed sources, provider fixtures, and checker implementations.

`postflight.json` passes checks of 3,691 source locks, 11,751 historical/incoming
locks, 2,229 selection locks, all 153 inventories, the archive/executable hashes,
unchanged branch/HEAD/staging, and unrelated incoming work. All 34 locked analysis
entries match. The additional denominator helper has its own pre-execution hash
receipt, and the final seal binds it and its output with every closed evidence file.
Only closed new JSON evidence received transparent APFS compression, with raw
byte hashes checked before and after replacement. No old evidence was compressed
or rewritten. Temporary storage-probe files were removed.

`evidence-manifest.json` binds 3,361 files and 27,681,614,304 logical bytes. Its
SHA-256 is `3177b40527d7ef0b9ba5fed21341efe17c49bfc6274fc4382fe670e03cda5b5a`.
The manifest and its checksum receipt exclude themselves to avoid a circular hash.
No overall clean verdict is inferred from driver or integrity success.

## Reproduction and local delivery

The retained tools refuse to overwrite frozen outputs. Reproduction requires a
fresh evidence directory and newly bound source/executable, not deleting old runs.
`run-native.cjs` is the maintained harness used for all 153 jobs. `focused.cjs`
records the actual-case old/new experiment. `finish-audits.py` lists the unchanged
legacy/field audits, independent preservation/receipt audits, strict comparisons,
exports, and per-case breakdown generation. `validation-facts.py` independently
derives physical-input denominators and all changed readback values.
`postflight.py` uses one inventory at a time instead of retaining all large JSON
inventories in memory.

Only three source paths change relative to the incoming checkpoint:
`src-tauri/src/commands/auto_tag.rs`, its existing collection tests, and
`docs/CHANGELOG.md`. The hidden plan, this report, and local evidence are additional
review records. Incoming edits in all other paths are retained.

All work remains local and uncommitted on `artist-library-inventory`, above pushed
HEAD `d7659077a5030c5ca627d99b6e967343d3597e28`. No original-media writes, live/paid
models, commits, pushes, deployments, threads, worktrees, new certificate, broader
authority, guard weakening, or budget increases occurred. The unrelated iOS
archive and `scripts/__pycache__/` remain untouched. Nothing needed from you right now.
