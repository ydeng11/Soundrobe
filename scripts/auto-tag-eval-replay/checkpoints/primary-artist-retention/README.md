# Candidate-scoped primary artist retention

This checkpoint retains raw Discogs primary canonical-name and ANV observations.
It does not accept them as aliases, establish artist equivalence, change donor
eligibility, rewrite display credits, or grant write authority. Only a concrete
release with matching positive raw ID and positive primary artist ID supplies
observations. Joint primary credits remain separate observations.

Both native candidate structs emit `discogsArtistNameVariants`, accept earlier
populated `discogs_artist_name_variants` cache rows, and default missing evidence
empty. Nested observation keys use camel case. Empty legacy cache rows cannot
recover previously discarded ANV without an ordinary fetch. There is no migration,
invalidation, forced fetch, additional discovery, or new eligibility consumer.

## Source and review boundaries

The motivating authentic body is the tracked Discogs release `37231524` fixture,
`artists[0]`, ID `2461941`, pairing `Crowd Lu` with `盧廣仲`. Its raw SHA-256 is
`1798a65076cb0ab25f376751bf30fbc42677702ec54b5ffe29f4139f0c939b1a`.
The [evidence receipt](evidence-summary.json) records its path, selector, source,
executable and tool hashes, and local validation bindings.

The master-route regression serves that authentic release body through the real
master call path as a synthetic resource-kind negative. It is not an authentic
master catalog assertion. Content-construction controls use synthetic one-track
candidates and do not certify either real release. Other controls distinguish
blank ANV, another entity, guest-only assertions, separate joint primaries,
homonym search results, successive requests, and candidate/cache ownership.

The tracked `test/fixtures/tauri/discogs-primary-artist-native-wire.json` contains
actual automatic and manual Rust candidate JSON, extracted from `VARIANT_WIRE:`
in the provider-boundary test. The maintained adapter test consumes that JSON
through unchanged mocked invoke. It is not a packaged Tauri/webview proof.
Delivery-time native serialization was compared with the fixture again.

The [independent specification re-review](independent-spec-review.md) found no
remaining retention-spec defects after the three corrections. It covers the
corrected implementation, not the later portable-tool packaging. It does not
establish artist equivalence or ordinary cache-to-originating-HTTP-receipt
association. That association remains unresolved.

Historical records are committed without rewriting their checkpoint claims:

- [Original diagnosis](../../../../.planning/research/crowd-lu-alias-eligibility-20261110.md).
- [Approved retention plan](../../../../.planning/plans/candidate-primary-artist-variants-20261110.md).
- [Initial retention report](../../../../.planning/research/candidate-primary-artist-variants-20261110.md).
- [Three-boundary correction report](../../../../.planning/research/primary-artist-retention-corrections-20261110.md).
- [Local delivery record](../../../../.planning/research/primary-artist-retention-delivery-20261110.md).

Their statements about being uncommitted or their historical helper paths refer
to those earlier checkpoints. The delivery record supersedes the delivery state,
not their source receipts, old oracles, or failed attempts. The proposed identity
policy remains unapproved, unimplemented, and outside this delivery.

## Portable contract checks

From the repository root, with normal dependencies installed:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features server --lib primary_variants -- --nocapture
npx vitest run test/shared/tauri-adapter.test.ts
node --test scripts/auto-tag-eval-replay/test_rehearsal.cjs
python3 -B scripts/auto-tag-eval-replay/checkpoints/primary-artist-retention/test_comparison.py
python3 -B scripts/auto-tag-eval-replay/test_evidence_export.py
python3 -B scripts/auto-tag-eval-replay/checkpoints/content-preservation/test_field_audit.py
```

These checks need no ignored planning helpers, sealed media, old executable,
provider credentials, or model access. The native tests use local mock HTTP.
The comparator's synthetic controls are tests of comparison rules, not ground
truth for catalog identity. They cover changed source binding, authority,
aliases, genre and numbering, plus plain/gzip/xz snapshot input.

## Reanalyse the retained paired replay

`compare.py` is the correction comparator moved into the maintained checkpoint.
Only its plain/compressed JSON reader, streaming hash function, case and profile
constants were made self-contained. Comparison predicates remain unchanged.
It imports the tracked non-authorizing exporter, not an ignored planning helper.
Delivery reanalysis produced the same complete comparison JSON as the historical
correction comparator. It did not execute new tagging or provider/model runs.

With the retained local evidence available, use an existing output parent and
an unused output filename:

```sh
python3 -B scripts/auto-tag-eval-replay/checkpoints/primary-artist-retention/compare.py \
  .amp/in/artifacts/primary-artist-variants-20261110/after-retry \
  .amp/in/artifacts/primary-artist-retention-corrections-20261110/replay \
  /absolute/fresh/comparison.json
```

This checks the six retained baseline evaluations against six correction-phase
evaluations, with 66 file observations per arm and eleven unique generated input
paths. Only the outer evidence-field spelling, temporary root UUID, elapsed
time, and compiled source hash differ. The pairs and their ownership stay equal.
Provider exchanges total 52 per arm, with zero model exchanges or fixture misses.
Requests, donor predicates/reasons, tags/readbacks, writes, IDs, mappings, caches,
payloads, provenance and review remain unchanged. Recovery still leaves eleven
genres empty per phase. Every export is insufficient, has `writableDelta: []`,
and has `acceptedCertificate: false`.

The independent source auditor retains four source-eligibility findings per arm.
Historical 84 findings remain unchanged. Missing durations, acoustic qualifier,
album/title/credit certification and edition gaps remain unresolved. The first
recovery artist rejection prevents later predicates from running; it does not
show those predicates pass. Overall safety remains non-clean and assistance
accuracy credit is zero.

Exact reanalysis is **not fresh-clone reproducible**. It requires retained local
executables, source archives, provider-pool manifest, full request/donor/evidence
snapshots, SQLite/cache records, source certificates, generated media bindings,
and execution plans. The committed receipt contains hashes, not these multi-GB
inputs. It does not supply a public download or reconstruct their availability.
Original diagnosis and initial-cycle commands also reference ignored local
helpers under `.planning/plans/`; those historical helpers remain local.

## Bounded scheduler and intentionally uncommitted preparation

`../../run-rehearsal.cjs` and `../../test_rehearsal.cjs` are the completed driver
and membership/budget controls used by these comparisons. They accept a locked
nonempty selection with exactly the three existing profiles, reject missing,
duplicate, extra or substituted jobs, disable assistance, and enforce a zero
model budget. Each job owns its process, state, cache and media. The one-case
`selection.json` is a byte-identical portable membership excerpt. It is not an
execution plan, oracle, new identity exception, or authorization to write.

The driver consumes a separately prepared `execution-plan.json` in a fresh run
directory and refuses to append to an existing `runs` directory. It requires a
bound executable, genuine provider fixtures, source/input/tool locks and generated
media. It does not construct that plan. Do not rewrite old plans to refer to new
commits or binaries. A future run needs fresh bindings and a fresh directory.

The five-album evaluation/preparation packet and rehearsal-preparation builders
under `.planning/plans/five-album-evaluation-20261110/` and
`.planning/plans/five-album-rehearsal-20261110/` remain local and uncommitted.
Their seals and raw data are unchanged. The unapproved correspondence proposal,
later policy review, and original ignored comparison/extraction helpers are also
not swept into this delivery. Raw media, binaries, caches, logs, credentials,
`ios-native-probe-0d104d95-source.tgz`, and `scripts/__pycache__/` stay outside Git.
