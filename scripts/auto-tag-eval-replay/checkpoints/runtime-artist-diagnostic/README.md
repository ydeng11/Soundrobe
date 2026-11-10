# Runtime receipt binding and diagnostic-only correspondence

This checkpoint binds concrete Discogs candidate observations to the decoded
response bytes that produced them. It adds a narrow offline primary-name
diagnostic, not an identity, alias, eligibility, display, selected-ID, write or
certificate consumer. All runtime aggregate diagnostics and performer exports
remain insufficient, with `writableDelta: []` and `acceptedCertificate: false`.
The applicable user-decision store is unavailable. A supported name subfinding
does not approve a field or pass native predicates skipped after artist rejection.

## Check the portable contracts

Run these commands from the repository root with normal dependencies installed.
They use tracked fixtures, disposable synthetic media and local HTTP mocks, not
provider credentials, original media, live providers or models.

```sh
cargo test --manifest-path src-tauri/Cargo.toml \
  --config 'profile.test.package.soundrobe.debug=0' \
  --config 'profile.test.package.soundrobe.incremental=false' \
  --no-default-features --features server --lib runtime_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml \
  --config 'profile.test.package.soundrobe.debug=0' \
  --config 'profile.test.package.soundrobe.incremental=false' \
  --no-default-features --features server --lib primary_variants -- --nocapture
npm run typecheck
python3 -B scripts/auto-tag-eval-replay/checkpoints/runtime-artist-diagnostic/test_comparison.py
python3 -B scripts/auto-tag-eval-replay/checkpoints/primary-artist-retention/test_comparison.py
python3 -B scripts/auto-tag-eval-replay/test_evidence_export.py
python3 -B scripts/auto-tag-eval-replay/checkpoints/content-preservation/test_field_audit.py
node --test scripts/auto-tag-eval-replay/test_rehearsal.cjs
```

`compare.py` imports only the tracked retention comparator and exporter.
`audit.py` additionally imports the tracked content-preservation field auditor.
Neither imports ignored planning helpers. The copied comparator changes only
repository-root discovery. The copied audit also fixes its baseline to
`1c74c6c728f0a8ad87dc0a66de08dfd3fdabf5bd`, requires that ancestor, and no longer
requires an empty index or that HEAD still equal the pre-delivery commit.
It compares source and incoming-file hashes, not staging state. Old helpers and
seals stay unchanged. Portable comparator controls are synthetic constructions,
not catalog identity truth or packaged bridge proof.

## Reanalyse the retained source-bound runs

Exact historical replay is **not fresh-clone reproducible**. These commands need
the existing local `.amp/in/artifacts/` tree, both sealed execution plans and
executables, genuine route pools, generated media bindings, full request and
evidence snapshots, caches, source archives and independent source certificates.
They do not fetch providers, invoke models or run tagging. Use unused output
filenames with existing parent directories.

```sh
python3 -B scripts/auto-tag-eval-replay/checkpoints/runtime-artist-diagnostic/compare.py \
  .amp/in/artifacts/primary-artist-retention-corrections-20261110/replay \
  .amp/in/artifacts/runtime-artist-diagnostic-20261110/final/replay \
  /absolute/existing-directory/new-comparison.json
python3 -B scripts/auto-tag-eval-replay/checkpoints/runtime-artist-diagnostic/audit.py \
  /absolute/existing-directory/new-audit.json
```

The maintained `../../run-rehearsal.cjs` consumes a separately bound execution
plan in a fresh run directory. It does not build that plan or reconstruct local
prerequisites. `selection.json` is the unchanged one-case membership excerpt,
not an oracle or execution plan. Never rewrite old plans or source snapshots to
refer to new commits. New executions need fresh source, tool and input bindings.

Delivery reanalysis must equal the full retained `final/comparison.json` and
`final/audit.json`. It compares six retained baseline evaluations with six
retained final evaluations, eleven unique generated inputs and 66 repeated
track observations per arm. Each arm has 52 provider exchanges, zero model
exchanges and no misses. Requests, eligibility reasons, mappings, tags,
readbacks, writes, IDs, payloads, cache behavior, provenance and review remain
unchanged. Only verified receipts, validated diagnostic observations, three
compiled hashes, elapsed time and disposable UUID paths are excluded.

Recovery still rejects the artist conflict and leaves eleven genres missing per
phase. Four source-eligibility findings per arm and historical 84 findings remain.
Overall safety is non-clean; grounded-assistance accuracy credit is zero.
This is assistance-disabled diagnostic evidence, not the unified workflow goal.

## Read the source-bound records

- [Runtime plan](../../../../.planning/plans/runtime-artist-diagnostic-20261110.md).
- [Executed report](../../../../.planning/research/runtime-artist-diagnostic-20261110.md).
- [Independent security and specification dispositions](../../../../.planning/research/runtime-artist-diagnostic-independent-review-20261110.md).
- [Compact source, receipt and validation bindings](evidence-summary.json).

The historical plan and report retain their earlier uncommitted-state wording
and local helper paths. This checkpoint supersedes delivery instructions only.
Their original bytes, source snapshots and failed attempts are not rewritten.
Independent reviewers found no remaining source-level blocker in the corrected
diagnostic scope, not general identity-policy or execution approval. They did
not review this later packaging. Reviewers used separate same-model subagents,
not a diverse-model vote.

`source-lock.json` and `guard-review.json` are exact copies of the historical
execution binding and source review. Their HEAD refers to the precommit state,
not a claim that the earlier commit contains the changed source. The compact
receipt binds the same source bytes to the implementation commit. Multi-GB local
inputs are represented by receipts, not copied into Git.

The source-bound prior gate records 738 server-library passes and 17 ignored
tests, 858 renderer passes, typecheck and targeted controls. Delivery reruns the
six native runtime tests using the exact sealed executable, typecheck, portable
comparison/export/field/scheduler controls and retained comparison/source audits.
This does not constitute a fresh default `just check`, default desktop build,
packaged bridge, acoustic or credentialed smoke. Mocked invoke is not packaged
integration. The compact receipt records exact reused-versus-fresh bindings.

The authentic positive is tracked release `37231524`, `artists[0]`, artist
`2461941`, canonical `Crowd Lu`, credited `盧廣仲`. Its exact body SHA-256 is
`1798a65076cb0ab25f376751bf30fbc42677702ec54b5ffe29f4139f0c939b1a`.
Native HTTP tests serve those bytes. Tampered receipts, distinct entity IDs,
joint arrays, roles, repeated/spaced FLAC comments, ID3 prefixes and size padding
are labeled synthetic controls, not genuine contradictory catalog assertions.

Retained red logs cover initial receipt absence and each corrected review
finding. The first ID3 construction assumed the wrong duplicate order and failed
to reproduce; the reversed construction reproduced false support. Compilation,
disk-full build/cache, subprocess-timeout and incomplete replay attempts remain
failed evidence. Only the final successful arm enters the comparison. Raw logs,
losslessly compressed snapshots, executables, caches and media remain local.

## Keep the diagnostic limits

Receipts cover captured concrete responses only and retain at most 2 MiB per
body. Oversized metadata still parses, but no receipt is retained or truncated.
Legacy or missing receipts remain insufficient without migration or refetch.
Hashes show consistency, not freshness, catalog truth, provider-host attestation
or resistance to coordinated receipt/candidate replacement. The cap is not a
total cache quota or a transient HTTP-allocation limit.

Primary completeness supports only the bounded validated FLAC/Vorbis layouts
and recognized fields. Leading ID3, ambiguous/repeated claims and unsupported
layouts remain insufficient. Capture is not atomic against concurrent mutation.
Homonym handling concerns available individually supported trace assertions,
not exhaustive discovery or cross-provider identity. Joint-primary mapping,
scoped original primary IDs, user decisions and broader normalization remain
unsupported. Missing durations, version/acoustic qualifiers, album/title/edition
certification and collaborator completeness remain unresolved.

Unrelated preparations and policy proposals, old seals, multi-GB evidence,
private configuration, credentials, the iOS probe archive and `scripts/__pycache__`
stay outside this delivery and in place. No original music is accessed.
