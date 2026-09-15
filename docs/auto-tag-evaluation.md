# Auto-tag evaluation

For HTTP replay through the production provider clients, use the
[offline MusicBrainz and Discogs fixture service](offline-provider-mock.md).
It serves locked raw snapshots and exports missing requests for evidence collection.

Soundrobe has a metadata-only evaluation corpus under `test/fixtures/tauri/auto-tag-eval/`. It freezes source-relative folders, filenames, observed tags, durations, numbering, provider IDs, and source hashes for Ariana Grande, Billie Eilish, Eagles, Doja Cat, Ellie Goulding, Eminem, and Enya. The corpus contains no audio or artwork.

Run the deterministic contracts with:

```sh
just eval-auto-tag
```

The three profiles are intentionally separate:

- `folder_filename` removes tags and provider IDs and measures clean discovery.
- `assisted_without_ids` keeps captured tags but removes every provider ID.
- `tagged_recovery` keeps captured IDs for direct lookup and idempotence.

The first five artists are provisional gold and require an independent provider tracklist/content/mapping review before they enter scored metrics. The reviewed Relapse With Bonus case is the one scored Eminem exception; the remaining Eminem and Enya cases remain diagnostic until reviewed. A complete compatible edition is acceptable when its content and mapping are proven; equal track counts or title matches alone never bless an edition. Selected-disc or explicitly allowed provider extras must be declared in the reviewed policy and are checked against exact provider positions. Unseen promising releases stay `oracle_review_required`.

The representative review ledger at `test/fixtures/tauri/auto-tag-eval/reviewed-subset.json` covers 30 cases across ordinary albums, deluxe/bonus editions, singles, box sets, title variants, and provider failures. Twenty cases have frozen provider payloads and complete local-to-provider mappings; selected-disc and explicitly allowed-extra cases declare their exact provider-track policy, while ten cases remain explicit unresolved holdouts outside scored metrics. Run `just eval-auto-tag-subset` to replay this review without contacting providers. It writes both `review.json` (the human-readable review result) and `expectations.json` (a schema-compatible ledger for the native evaluator). The raw Enya, Ariana, Doja Cat, Eagles, and Ellie Goulding payloads are also included in the frozen candidate pool and checked by `just eval-auto-tag-repro`.

## Matcher feedback loop

Use the generated reviewed ledger to measure the production resolver against the same locked cases through the offline provider service:

```sh
just eval-auto-tag-subset
node scripts/mock-provider-service.cjs import-pools \
  test/fixtures/tauri/auto-tag-eval/candidate-pools.json \
  .planning/debug/provider-mock/manifest.json
node scripts/mock-provider-service.cjs serve \
  .planning/debug/provider-mock/manifest.json 18081
```

In another shell, set `SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS` to the generated `expectations.json` and `SOUNDROBE_AUTO_TAG_EVAL_CASES` to the 20 `verified_match` case IDs from `review.json`, then run:

```sh
SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL=http://127.0.0.1:18081 \
SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT=/path/to/curated-library \
SOUNDROBE_AUTO_TAG_EVAL_PROFILE=folder_filename \
SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS=.planning/debug/auto-tag-eval/<run-id>/expectations.json \
SOUNDROBE_AUTO_TAG_EVAL_CASES='<verified case IDs>' \
just eval-auto-tag-live
```

Score the resulting `results.json` with the same generated expectations file:

```sh
SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS=.planning/debug/auto-tag-eval/<run-id>/expectations.json \
SOUNDROBE_AUTO_TAG_EVAL_RESULTS=.planning/debug/auto-tag-eval/<native-run>/results.json \
just eval-auto-tag-score
```

Only reviewed acceptable editions enter precision and coverage. Provider-unavailable rows, unresolved cases, cold/warm identity changes, and write/readback or payload failures remain separate diagnostics. A matcher change is attributable only when the same locked case changes from a safe failure to the reviewed acceptable edition without a new wrong match. The native replay uses synthetic silent FLAC copies, never writes source media, and requires the loopback mock URL; it does not contact MusicBrainz, Discogs, artwork hosts, or AI services.

The generated expectations ledger preserves a per-case `matcherAttribution` flag
when the reviewed mapping required a normalized title. Native scoring still
requires the selected provider candidate's track positions to match the full
reviewed local-to-provider mapping; release ID equality alone is not sufficient.
Readback also requires the candidate and on-disk track counts to agree before a
run can be reported as verified.

The exact Relapse Deluxe regression has a separate reviewed ledger at
`test/fixtures/tauri/relapse-deluxe/reviewed-truth.json`. It is the only reviewed
positive in that standalone Relapse ledger and records release `36441795`, the
alternate-bonus hard negative, and the reversed bonus-track mapping. Its
corresponding case is the first reviewed entry in
`test/fixtures/tauri/auto-tag-eval/expectations.json`; the other seven-artist cases
remain outside that broad corpus ledger's scored precision and coverage until they
receive the same review. This does not replace the separate 20-case representative
review ledger described above.

Audit the complete 321-case ledger offline with:

```sh
just eval-auto-tag-ground-truth
```

The audit requires one expectation entry per corpus case, checks source-relative
folders for root escapes, validates complete mappings for reviewed matches, and
keeps cases without independent provider review explicitly unscored. It writes
`ground-truth.json`, `ground-truth.md`, and a status log under the selected
`.planning/debug/auto-tag-eval/<run-id>/` directory. It exits nonzero while any
case remains unscored, so the current diagnostic inventory is intentionally not
a passing ground-truth benchmark.

The credentialed native replay is opt-in:

```sh
SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT=/path/to/curated-library \
SOUNDROBE_AUTO_TAG_EVAL_PROFILE=folder_filename \
SOUNDROBE_AUTO_TAG_EVAL_ARTISTS='Enya' \
just eval-auto-tag-live
```

Optional `SOUNDROBE_AUTO_TAG_EVAL_CASES` narrows by case ID. The runner reads each source folder with the production reader, then creates a fresh temporary folder containing one minimal silent FLAC per source track. It copies corpus-derived metadata, filenames, numbering, credits, and exact durations into those synthetic inputs; original audio payloads are never copied. IDs are cleared for discovery profiles through `WriteQueue`, AI and lyrics are disabled, one isolated cache is reused across cold and warm phases, a ten-minute folder and eight-hour run bound are enforced, and sanitized JSONL/results/report evidence is written under `.planning/debug/auto-tag-eval/<run-id>/`. Original files are hash-checked before and after the run and are never written.

The native runner overlays reviewed entries from `SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS` when present, while leaving unreviewed inventory cases diagnostic. The supplied expectations file is authoritative for native scoring: the checked-in broad ledger contains the standalone Relapse review, while the generated representative-subset ledger contains its separate 20 verified cases. The default points to the checked-in expectations ledger beside the corpus.

The synthetic-input equivalence gate compares production lookup requests from selected originals and their generated FLAC folders before the native replay. It must pass before broad evaluation is interpreted. The separate Enya audit remains the evidence for real-media payload preservation and malformed-layout behavior.

Audit the reproducibility inputs and retained replay with:

```sh
just eval-auto-tag-repro
```

This checks SHA-256 locks for every frozen candidate-pool fixture and provider
response, validates the saved production-reader equivalence cases, and lists
cold/warm identity drift. A drift is reported as failed verification and keeps
the replay separate from deterministic matcher results. The gate also requires
nonempty equivalence evidence and exactly one cold and one warm invocation for
each retained replay case; missing or duplicate phases fail the command.
The equivalence sample and native replay may have different case sets; native
phase completeness is evaluated from the cases present in the retained native
artifact, while identity drift still fails the replay gate.

The frozen pool manifest now covers eight edition and failure fixtures with
34 locked responses: Relapse Deluxe, the representative Enya, Ariana, Doja Cat,
Eagles, and Ellie Goulding
payloads, Enya maxi/box/disc and title negatives, the Enya CD/DVD group case,
and a separate Discogs edition fixture. The
targeted Shepherd Moons replay on 2026-09-14 used a fresh isolated cache and
selected the same MusicBrainz identity in both phases. The earlier broad
synthetic replay still retains one cross-provider drift as a historical
provider-ordering diagnostic; it remains failed verification in that artifact.

Report deterministic input coverage and retained native denominators for all
three profiles with:

```sh
SOUNDROBE_AUTO_TAG_PROFILE_RUN_ID=profile-baseline-2026-09-14 \
SOUNDROBE_AUTO_TAG_PROFILE_DIR=.planning/debug/auto-tag-eval/profile-baseline-2026-09-14 \
just eval-auto-tag-profiles
```

Native outcomes distinguish confirmed success, wrong match, safe abstention, unresolved, incomplete provider work, and failed verification. Provider recovery and a different-release selection are reported separately from matcher attribution. Ordinary CI runs only the deterministic contracts; live evaluation remains manual and credentialed.
When native result files are supplied, the profile audit exits nonzero unless
each corpus case has exactly one cold and one warm invocation. It still writes
the outcome, coverage, provider-unavailable, mapping, readback, and payload
failure counters for diagnosis.

When additional provider evidence is needed, use the resumable collector in
small batches:

```sh
SOUNDROBE_AUTO_TAG_EVIDENCE_DIR=.planning/debug/auto-tag-evidence \
SOUNDROBE_AUTO_TAG_EVIDENCE_BATCH_SIZE=4 \
just collect-auto-tag-evidence
```

Planning is offline. Set `SOUNDROBE_AUTO_TAG_EVIDENCE_RUN=1` to execute one
bounded batch through the existing ignored native test, or increase
`SOUNDROBE_AUTO_TAG_EVIDENCE_MAX_BATCHES` deliberately. The collector locks the
corpus and expectation digests, retains each batch artifact, and queues cases
without complete cold and warm provider records, including unavailable or
timed-out work. The checked-in reviewed subset is skipped by default; pass an
explicit case ID to revisit one of those rows. Re-running the same output
directory skips provider-complete cases and retries only the remaining queue.
The native runner still owns its cache, retry, rate-limit, ten-minute folder,
and eight-hour run budgets; clean discovery profiles continue to strip provider
IDs, and no release ID is placed into their lookup input.

Score retained native results offline with:

```sh
SOUNDROBE_AUTO_TAG_SCORE_RUN_ID=scored-2026-09-14 \\
SOUNDROBE_AUTO_TAG_SCORE_DIR=.planning/debug/enya-2026-09-13-followup/scored-2026-09-14 \\
just eval-auto-tag-score
```

The scorer records SHA-256 fingerprints for the corpus, expectations, native results, and reviewed ledger beside `score.json`, `score.md`, and `command.log`. Only `verified_match` and `verified_abstain` expectations contribute to precision and coverage; the current broad corpus is therefore diagnostic until its cases are independently reviewed. Provider-unavailable phases, warm recovery, cold/warm identity drift, and GuardedTitle attribution remain separate counts so a recovered provider or a different release cannot be reported as a matcher improvement.

The report also keeps all replay observations outside the scored denominator. For the retained 321-folder run, 293 folders encountered provider unavailability and one Enya folder selected Discogs release `2500754` cold but MusicBrainz release `4bf28c39-1b7e-4827-81ed-a306fa4a6b3b` warm; that historical cross-provider identity drift remains a failed-verification diagnostic rather than matcher credit. The profile baseline records 642/642 discovery invocations, 32/642 assisted invocations before provider latency stopped the run, and 0/642 broad recovery invocations; the assisted and recovery Relapse probes remain separately retained and successful.

The 2026-09-14 baseline measurements are profile-separated. The complete
`folder_filename` replay covered 321 cases and 642 invocations: 292 Incomplete,
28 Unresolved, and 1 Failed verification, with the single reviewed Relapse case
Incomplete (coverage 0%, because providers were unavailable). The assisted and
tagged-recovery profiles were run as reviewed Relapse probes after the broad
assisted replay encountered provider latency: assisted selected `36441795` after
one cold Incomplete and one warm Confirmed success; tagged recovery selected the
same release in both phases. Both probes had zero readback or payload failures.
The broad assisted replay is retained as an incomplete provider attempt (31
cold records); a full profile denominator remains pending provider recovery.

The improvement loop is replayed through the deterministic Rust contracts after
each narrow change. The retained failure ranking is provider unavailability
first, then no authoritative match, then cold/warm identity drift. The shipped
fixes address only evidence supported by fixtures: guarded `(Skit)`/diacritic
title comparison, edition and disc suffix identity normalization, Discogs CD/DVD
track-group scoping, and canonical FLAC Vorbis read precedence. The Relapse
regression moves from 16/22 strong title matches to 22/22 with six GuardedTitle
matches; hard negatives remain rejected and no unsafe positional fallback is
accepted.

The reviewed-subset replay provides the current deterministic measurement for
the provider-backed sample: exact-title matching alone maps 190/243 tracks and
completes 3/20 cases, while the guarded normalized-title comparison maps all
243/243 tracks and completes 20/20 cases. The 10 unresolved rows remain
explicit holdouts, so this improvement claim does not change the broad native
profile denominator or grant credit to provider-recovery runs.
