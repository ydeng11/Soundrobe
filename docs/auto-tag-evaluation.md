# Auto-tag evaluation

## Goal and planned offline feedback loop

The goal is a repeatable feedback loop for changes to auto-tagging capabilities. Each comparison must show what improved, what regressed, and whether the app still writes correct metadata safely. The eval set supplies reviewed inputs and expected outcomes; expanding or hardening that set is supporting work, not the final deliverable.

The bounded provider-only loop is implemented by `scripts/auto-tag-eval-replay/run-paired.cjs`. Its `freeze` operation captures each native executable and source identity. Its `compare` operation runs both builds on identical locked inputs and emits `comparison.md` and `comparison.json`. The default selection contains two reviewed development cases, ordinary Bellyache and its Marian Hill remix. See the [paired command procedure](../scripts/auto-tag-eval-replay/README.md#compare-a-baseline-and-candidate-offline) for dependencies, commands, limits, and exit behavior. The larger unified provider-and-agent loop remains incomplete.

The broader loop's acceptance contract is below. The bounded command implements provider replay and content/readback checks; it does not yet score every field or exercise assisted research.

1. Freeze both executable identities, the reviewed suite, provider responses, configuration, scoring rules, and equal request budgets. Compare the same eligible cases across the three profiles, with separate cold and warm caches.
2. Run both implementations through the native auto-tagging path using loopback provider responses and disposable media. Hosted dispatch remains disabled. Missing fixtures, process failures, and skipped cases stay visible rather than disappearing from the comparison.
3. Compare correct recoveries, regressions, wrong matches, and abstentions by case and profile. Score core album content, parent-album identity, exact edition, per-track mappings, and artist credits separately. Unreviewed cases cannot contribute to accuracy.
4. Check independent tag readback, audio-payload preservation, numbering, write authorization, and request-budget accounting. A write or successful process exit alone is not a correct recovery.
5. Inspect failures and classify product defects, fixture gaps, and uncertain truth separately. Turn confirmed product failures into regression cases without weakening reviewed expectations, then rerun after a targeted change.

The report must name both executable revisions and input locks, retain the full requested case denominator, and show improvements alongside regressions and unresolved failures. A gain in recovery count cannot hide a new wrong assignment or safety violation. Comparisons with different inputs or incomplete execution must be reported as incomplete, not as evidence of improvement.

The current inventory contains 378 cases and 3,506 tracks. The maintained checkpoint scores 51 metadata positives with 539 track mappings. Two opt-in review manifests, `reviewed-selected-media-20261115.json` and `reviewed-explicit-extras-20261116.json`, extend that to 53 metadata positives with 575 mappings. The remaining 45 unresolved and 280 unverified cases stay unscored. The additions do not replace the maintained checkpoint or authorize exact-edition writes from core-content evidence.

Offline v1 does not require labeling all 325 unscored cases, collecting new provider evidence, or proving acoustic or genuine hosted-model quality. If all 53 positives are eligible for every profile, each implementation requires 318 case evaluations across three profiles and cold/warm phases; a baseline/candidate pair requires 636. These are planned counts, not measured runtimes. Further general receipt-hardening passes are out of scope unless a concrete defect blocks this loop.

The initial selection requests twelve evaluations per implementation: two cases, three profiles, and cold/warm phases. The command retains every slot when setup, a process, or a fixture fails. Independent content checks and native readback/write-safety checks are reported separately. Exact-edition and acoustic correctness, model quality, assisted review highlights, and the broader 53-case matrix are not established by this bounded loop. No hosted execution is enabled.

The optional `paired-safety-selection.json` keeps those controls and adds two-track Guitar Songs, two-track Storms In Africa with distinct Remix/LP Version mappings, and sixteen-track Paint The Sky With Stars. Five cases and twenty-two physical files request thirty evaluations per implementation, sixty paired evaluations and 264 physical-file slots. The remaster case declares `requireNoWrite`: its compatible reviewed content does not establish the filename mastering claims. An unchanged completed abstention passes that control without earning recovery credit; any write fails it. Unavailable execution cannot pass a no-write control.

The 2026-11-16 local pair compared clean main at `3d041679` with a source-frozen dirty candidate on that base. All twenty-four evaluations completed. Correct recoveries rose from six to twelve, abstentions fell from six to zero, and the report found six improvements with no regressions or wrong matches.

Readback and safety failures were independently zero, as were missing fixtures, skips, and process failures. The targeted fix ignores a separately bracketed bare year without erasing meaningful remix/remaster years and writes the confirmed canonical title. The first candidate run exposed four polluted-title outputs and exited nonzero; that evidence was preserved before the final paired rerun.

Local reports are `paired-remix-fix-20261116/comparison.md` and `paired-remix-final-20261116/comparison.md` under `.amp/in/artifacts/`. These two inspected single-track cases do not establish generalization to the larger suite.

The expanded safety pair, `paired-safety-20261116/comparison.md`, reuses the same frozen executables. All sixty requested evaluations completed with 18/30 correct baseline recoveries and 24/30 candidate recoveries. Each profile improved from 6/10 to 8/10. Both builds recovered the two multi-track cases in all six combinations and passed all six required no-write controls. The comparison found six improvements and zero regressions, wrong matches, unavailable evaluations, missing fixtures, skips, process failures, readback failures, or safety failures.

The expanded run retained all 132 physical-file slots per implementation, including 96 unchanged no-write slots. All 1,622 input locks still matched after execution, and model-request counts stayed zero. `validation-summary.json` records the physical-file denominator and per-case results. Eight separate auditor probes accepted the two unchanged multi-track exports and rejected swapped titles, swapped file rows, and omitted tracks. These probes are checker controls, not additional native evaluations. No further native matching change was needed for this expansion. Changes remain local and uncommitted. Multi-disc, duplicate-file, acoustic, and model-assisted coverage remain outside this set.

Review found four defects outside that clean execution: nested live-performance years could collapse, partial exports could pass no-write controls, truncated ancillary JSON could prevent reporting, and process failures could hide observed wrong matches. Regression tests reproduced each defect before correction. Only top-level bare-year annotations are now ignored. Both write and no-write results require complete physical-file bindings. Malformed ancillary exports retain unavailable rows and non-clean reports. Observed wrong matches remain counted independently of execution availability, without recovery credit for incomplete work.

The corrected candidate is frozen in `paired-candidate-reviewed-20261117/`, with executable SHA-256 `edd052b0b68e9efbabd2815e237aef714ef368e760d4671c2cb73e784700db3e`. The fresh `paired-safety-reviewed-20261117/comparison.md` uses the original baseline, unchanged five-case selection, fixtures, and budgets. All sixty evaluations completed, retaining 18/30 baseline and 24/30 candidate recoveries, six improvements, and zero regressions, wrong matches, unavailable evaluations, missing fixtures, skips, process failures, readback failures, or safety failures. Both builds passed all six no-write controls. The nested-year conflict is a separate focused native regression, not an additional case in that paired denominator.

Correction evidence is under `.amp/in/artifacts/paired-review-fixes-20261117/`. The full server library passed 907 tests with 34 explicitly ignored. All 47 matcher tests passed in each desktop and server configuration. Ten Python report contracts, two Node CLI contracts, ten current-source review/suite tests, and twelve current-source mock-provider tests passed. Copies of the original review probes now emit incomplete reports, retain wrong-write counts alongside process failures, and reject the omitted remaster export. The changed Rust files pass formatting. Repository-wide formatting still flags three unchanged locations in `auto_tag.rs`; those were left outside this correction. No commit, push, deployment, or hosted execution occurred.

Follow-up regressions cover three further reporting gaps: native-supported null readback, independently observed wrong output after a partial-write error with no native completion, and malformed nested ancillary fields. Null readback now retains known prohibited writes and both reports. Partial errors grade only readable, physically bound files whose full-file hashes changed; unchanged bad input is not a new wrong match, and incomplete work earns no recovery credit. Inventory counts and arrays, execution status, and source-file membership, sizes, and SHA-256 values are validated before aggregation. Fourteen Python report contracts and both CLI controls pass, including a zero-write claim with changed media. Fresh copies of all four review probes retain every requested row and the expected non-clean outcomes. Evidence is under `.amp/in/artifacts/three-gap-fixes-20261117/`; historical probes are unchanged. No native matching change or rebuild was needed for these corrections.

The final follow-up pair, `paired-safety-three-gaps-final-20261117/`, reuses the same frozen executables, selection, fixtures, and budgets with the corrected auditor archived in `harness.tgz`. All thirty processes and sixty evaluations completed. Baseline retained 18/30 recoveries and twelve abstentions; candidate retained 24/30 and six. There were six improvements and no regressions or wrong matches. Missing fixtures, skips, process failures, independent readback failures, and write-safety failures were independently zero. Both builds passed all six no-write controls. `validation-summary.json` confirms all 132 physical-file slots per build, 96 unchanged no-write slots per build, equal inputs, isolated cold/warm caches, all 1,622 locks, zero model requests, and protected-file bytes and modes. No required local dependency was missing. The superseded `paired-safety-three-gaps-20261117/` was correctly rejected after the auditor changed during a zero-write regression correction; its incomplete reports remain preserved and are excluded from the final result. The failure probes use native-shaped exports rather than induced filesystem faults. No commit, push, deployment, original-library write, or hosted/model execution occurred.

The next three review findings are covered by failing-then-passing CLI regressions. The independent audit now rejects reviewed hard-negative MusicBrainz and Discogs release IDs in changed-file readback even when native completion is absent. It separates observed content mismatches from unavailable, incomplete-count, and misbound readback diagnostics; unknown content earns neither wrong-match nor recovery credit. Known prohibited writes and observed bad content remain wrong matches, including when other evidence is incomplete. The optional execution `skipped` field is validated as Boolean rather than coerced by truthiness. Seventeen Python report contracts and both CLI controls pass, including unchanged bad-ID inputs, allowed IDs, positive prohibited-write controls, incomplete native counts, and both valid skip states. Fresh copies of all five review controls now emit the expected reports and counts. Correction evidence is `.amp/in/artifacts/report-classification-fixes-20261117/`; no native source or executable changed.

The fresh `paired-safety-classification-20261117/` comparison completed all thirty processes and sixty evaluations with the unchanged frozen baseline and candidate, five-case selection, provider fixtures, and budgets. Baseline retained 18/30 recoveries and twelve abstentions; candidate retained 24/30 and six. There were six improvements and no regressions, wrong matches, unavailable evaluations, missing fixtures, skips, process failures, metadata-readback failures, or write-safety failures. Both builds passed all six required no-write controls. Its `validation-summary.json` confirms 132 physical-file slots and 96 unchanged no-write slots per build, identical initial inputs, isolated cold/warm caches, all 1,622 input locks, zero model requests, and unchanged protected-file bytes and modes. No required local dependency was missing. The review/evaluation command also passed ten current-source Vitest contracts; the preserved detached checkout contributed ten duplicate passes, not additional coverage. Native source was unchanged, so the full Rust suites were not repeated. Failure controls use native-shaped exports and deliberately malformed ancillary inputs, not induced filesystem faults. Historical receipts, reports, and backups remain intact. All changes remain local and uncommitted; no push, deployment, original-library write, or hosted/model execution occurred.

Historical evidence remains preserved under Git-ignored `.amp/in/artifacts/`; it is not included in a fresh clone. The [offline eval handoff](../.planning/handoffs/offline-eval-v1-20261116.md) records the preceding fixture and owner-component work. Its statement that the paired command is planned predates this implementation.

## Owner-authorized offline evaluation

The test-only owner entry runs one supervised six-job rehearsal with disposable
signing keys, a fixed fake credential, loopback fixtures, and disposable media.
It does not enable hosted dispatch or award real-model quality credit.
This is a separate component of the broader feedback loop, not the provider-only
baseline/candidate comparison command described above.

The explicit ignored native test is
`infra::mock_run_authority::supervisor::ownership::owner_launch::native_owner_process`,
built with `--no-default-features --features server` on macOS.
`SOUNDROBE_OFFLINE_OWNER_REQUEST` selects a bounded public JSON request with
`offlineCustody`, `disposableRunRoot`, `proposal`, `intent`, `planFile`, and
`bindings`. The run root must be fresh. Proposal generation does not start a run.
The proposal binds the evaluator, six-job membership, public native plan,
request, verifier, pricing context, and ordered source locks.

The owner supplies explicit signed `offlineRehearsal` intent before launch.
Native code claims its nonce once, checks disposable custody, then starts the
receiver and independent supervision. At the frozen checkpoint, owner code
checks the current checkpoint against that intent before issuing and publishing
approval. Approval work cannot suspend the heartbeat loop or extend the native
deadline. Ordinary native-prepared runs without this intent still stop before
execution. Hosted intent is rejected by this offline launcher.

Public `response-fixture.json` and `response-wire.json` siblings can select a
locked response or a premature-body-disconnect control. A withheld answer can
finish an observation with no accepted genre and no actionable genre review.
A disconnected response is incomplete. Neither outcome becomes a successful tag
fill or real-model quality evidence. The native owner archives completion only
after observing successful exit of its owned receiver child and validating its
receipt. A public PID and claimed exit code cannot supply that observation.
Missing or ambiguous completion evidence cannot be resumed or promoted.

The independent Python API is
`owner_offline_completed_audit(run, *, proposal, intent, approval, public_key,
expected_verifier_sha256, verified_at, trusted_executable)` in
`scripts/auto-tag-eval-replay/audit-supervised.py`. This is a function, not a CLI
mode. Its caller supplies duplicate-free parsed public JSON, the raw 32-byte
public key, an independently pinned verifier SHA-256, integer verification time,
and the independently selected canonical evaluator path. The auditor checks
signatures, actual source bytes, bootstrap and consumed approval bindings before
completion, storage, readback, and scoring checks. An execute bit or signed
source declaration cannot make a private file eligible for reading.
Signatures do not prove human presence, private nonce custody, or live authority.

## Existing replay tools and checkpoints

For HTTP replay through the production provider clients, use the
[offline MusicBrainz and Discogs fixture service](offline-provider-mock.md).
It serves locked raw snapshots and exports missing requests for evidence collection.

For the completed 51-case, three-profile native checkpoint, use the
[archived replay procedure](../scripts/auto-tag-eval-replay/README.md).
It recreates silent media from tracked metadata and preserves the exact driver,
expanded auditor, and independent title rubric. This model-disabled replay is
regression evidence, not validation of the complete provider-and-agent workflow.

For the subsequent provider-first assistance cycles, preservation fixes, and
remaining findings, read the [current tagging feedback record](tagging-feedback-status.md).
The [checkpoint reproduction notes](../scripts/auto-tag-eval-replay/checkpoints/README.md)
distinguish portable tests from reanalysis that needs retained local evidence.

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

## Basic album tags and edition accuracy

Auto-tagging keeps confirmed editions first. If edition matching cannot finish,
it can use the best fully aligned candidate already fetched in that invocation
for basic album tags. This fallback ignores edition country, edition year and
provider-ID consensus, but retains album/artist identity, strong per-file title
mapping, the matcher's duration/version checks, selected-disc scope and distinct
provider occurrences for distinct audio groups. Proven exact copies retain the
existing shared-occurrence handling. Provider extras may remain; missing local
content may not.
Unavailable or incomplete discovery blocks fallback writes, except when unrelated
Discogs work reaches only its release-detail limit. In that case, a fully aligned
candidate from a completed provider remains usable. Retries, outages, other
incomplete discovery, and conflicting mappings still block this fallback.

Both basic and edition-backed auto-tag output preserve the matched track's
display credit and ordered artist list, including collaborators. Album Artist
remains shared; a missing track credit falls back to the album artist. Basic
output retains a validated Chinese album-artist display name for solo credits.
Coherent captured collaborator displays and lists retain their spelling, order,
and compound entries when the completed performer set is unchanged or a subset.
Incompatible credits stop before writes or metadata fallback.
Complete album auto-tag writes retain each physical file's captured track/disc
positions and totals. Missing positive track positions and totals fall back to
local file order and file count. Captured tags are observations, not proof of
an exact edition or a verified user lock. Manual selected-track edits and
recording-only recovery do not acquire this fallback numbering policy.
Basic output prefers the folder year and does not assert release or recording
IDs. Independently resolved album-artist IDs are
retained for subsequent lookups; they do not identify an edition. The basic
output's date policy remains unchanged. Captured disc totals and per-file genres
are retained unless cleanup explicitly removes them. Eligible provider genre
precedes optional AI inference. Genre disagreement is not absence. An AI fill
retains provider identity, records field provenance, and requires review. Keeping
changes acknowledges review without making the genre provider-verified.
Explicit cleanup of year, genre, or disc total is honored. Cleanup of required basic fields,
such as title or artist, stops this fallback before any write and requires review.
Run details report `completionBasis: album_content`, `completeBasicTags: true`,
and `completeAlbum: false`. This is metadata alignment, not an acoustic proof,
an exact pressing, mastering or clean/explicit identification.

Score this separately from the edition ledger. A basic-tag result can have a
successful write/readback and remain `unresolved` for exact-edition accuracy.
Native replay exports `readbackTracks` for an independent per-file title audit;
the write/readback check compares with the accepted candidate, not reviewed
edition numbering. This applies to both edition-backed and basic-tag writes.
Exact-edition scoring separately requires the reviewed identity, track mapping,
and provider-track policy. Preserved or fallback positions can remain unresolved
for strict edition-layout scoring even when the release ID is retained and all
titles and credits are correct. Do not change reviewed provider positions to
make local output numbering count as provider evidence. Artist-credit recovery
must be audited separately from the historical album-artist-only uniformity
metric; Artist and Artists are intentionally not album-wide uniform fields.
Preserve meaningful version labels in the title oracle, and
join truth by `(caseId, localTrack)` where `localTrack` is one-based file order.
Do not substitute embedded track numbers or a prototype's own normalization.

## Expanded artist-batch inventory

`artist-batch-extension/` adds 33 physical folders and 418 tracks without changing
the original corpus or its reviewed answers:

| Population | Folders | Tracks |
|---|---:|---:|
| 张学友 | 11 | 141 |
| 卢广仲 (captured credits also use 盧廣仲) | 10 | 86 |
| Kenny G | 5 | 70 |
| Accuphase multi-artist instrumental compilations | 7 | 121 |

These are observed inputs, not reviewed truth. Existing tags and provider IDs
remain evidence to investigate, never an automatic promotion to a match.
Compilation cases use `Various Artists` for the container identity and preserve
each track's captured artist and album-artist fields separately.

`batch-suite.json` combines all four inventories: 378 cases, 3,506 tracks, 15 artist
batches, and 12 independent compilation-album batches. Artist batches share
catalog discovery. Compilation batches require album-only discovery, not a
Various Artists catalog browse. Physical disc cases stay in their release family.
The manifest locks 1,497 input files, including the corpora, ledgers, selection
manifests, reviewed provider snapshots, candidate pools, and captured discovery responses.

Check the frozen membership and hashes without regenerating them:

```sh
just eval-auto-tag-suite
```

The check replays every review manifest and rejects conflicting overlapping
answers instead of letting the last manifest replace earlier truth.
Relapse appears in both original ledgers but counts once. There are 51 scored
positive cases with 539 mapped tracks, 22 unresolved cases, and 305 unverified
cases. The original corpus, broad ledger, and 30-case review manifest remain unchanged.

The deterministic partition keeps entire artist batches and release families
together: 340 development cases and 38 holdout cases.
This is a retrospective partition, not an untouched blind benchmark. There are
43 reviewed positives in development and eight in the Billie Eilish holdout.
Earlier replays covered the preceding 29 positives and the latest 22 positives
separately. The [full reviewed replay](#full-reviewed-coldwarm-replay-2026-11-09)
now covers all 51 cases on the same binary. The targeted fixture-completion
rerun below closes its HTTP gaps. Abstentions and metadata findings still
prevent a clean tagging pass.
Basic-tag results remain separate from exact-edition accuracy.
The manifest records requirements for equal HTTP-attempt budgets,
retry accounting, isolated cold and warm runs, and disposable media copies.
It does not run the coordinator or enforce those budgets itself.

To capture another metadata-only extension, repeat `--artist` and
`--compilation-container` with the existing generator. Set `--output-dir` to a
separate directory and assign `--corpus-version`; generation replaces that
directory's corpus and expectation files, so review the diff before reuse.
`scripts/build_auto_tag_eval_suite.py --help` lists the suite inputs. To propose
a new freeze, pass the arguments from the `eval-auto-tag-suite` recipe without
`--check`, then review the changed locks and membership.

### Full reviewed cold/warm replay, 2026-11-09

All 51 reviewed positives and 539 source dummies ran across three profiles,
cold and warm: 153 fresh native processes and 306 evaluations. No earlier
evaluation was copied. Frozen-suite membership and all earlier reviewed answers
remain unchanged. The replay locks 2,156 source/input hashes, the bundled alias
catalog and 1,461 provider responses.

| Profile | Applied cases, cold | Applied cases, warm | Clean written cases per phase |
| --- | ---: | ---: | ---: |
| Folder and filenames | 49/51 | 49/51 | 49/51 |
| Existing tags without IDs | 49/51 | 49/51 | 48/51 |
| Existing tags with IDs | 49/51 | 49/51 | 47/51 |

Applied writes are not automatically clean metadata or edition verification.
The independent audit accepts 288 written evaluations out of 306 requested.
All 153 native processes exit zero, but the driver exits nonzero for three
uncaptured filename-only routes: the MusicBrainz Marian Hill remix search,
the MusicBrainz Paint the Sky with Stars search and Discogs master 27623 versions.

*Bellyache (Marian Hill Remix)* and *Paint the Sky with Stars* abstain in all
profiles. Their existing-tag profiles have complete HTTP fixtures but reject
provider title mappings, so filling the three routes alone does not clear them.
The `ai_not_configured` terminal code follows provider rejection; it does not
establish AI configuration as the cause.

Four existing-tag readbacks of `&burn` omit Vince Staples. They match selected
Discogs 37353822, but reviewed Discogs 13898510 explicitly credits him.
Two ID-assisted readbacks write `Book of Days (Gaelic only version)` where the
frozen oracle accepts `Book of Days`. This remains a strict-title finding for
adjudication, not proof of a different recording or permission to strip wording.

All 3,132 written-track readbacks preserve payloads and local-order numbering.
The audit reports no unsupported selected-provider credits, cache-isolation
failures or cold/warm output/classification differences. Thirty per-file payload
exports are redacted; native unsanitized verification passes. Utada applies all
18 tracks in each of its six evaluations, with accepted titles and credits.

Evidence is in `.amp/in/artifacts/reviewed-51-cold-warm-20261108/`, including
`verification.json`, `verification-summary.json`, the frozen executable and all
153 execution records. `audit-expanded.py` adds support for the original
normalized Relapse receipt schema while preserving the title rubric and checks.
The initial locked auditor and its schema error remain available. No native
code, provider responses or reviewed answers changed during this rerun. No live
providers or original music were accessed. Silent FLACs do not prove acoustic
identity or mastering, and the retrospective partition is not a blind benchmark.

### Full-reviewed fixture completion, 2026-11-09

The three missing filename-only routes led to 69 further requests during
replay. All 72 responses are now frozen with exact official URLs, capture times,
HTTP statuses and SHA-256 digests. Every response is a genuine HTTP 200 body.
The mock replays all 72 byte-for-byte. The 1,450 earlier discovery records,
reviewed answers and suite membership remain unchanged. The imported mock now
contains 1,533 routes, and the suite locks 1,579 inputs.

Only filename-only `0e7c8a578cea` and `f7756354af37` reached these missing routes
in the full replay's 153 HTTP inventories. Three capture/replay rounds ran six
fresh native processes. The final round has four cold/warm evaluations on the
same locked executable, with no copied results and zero missing routes across
155 HTTP requests. This is a targeted rerun, not another full 51-case matrix.

Both cases still abstain in each phase and write nothing. Bellyache's Marian
Hill remix has no credible title mapping. Paint the Sky with Stars reports
`discovery_budget_limited`, with 40 Discogs candidates cold and 80 warm but
no credible candidate. Fixture completion does not clear either application
finding. Title/version checks, discovery limits and production code are unchanged.

The independent audit reports no payload, cache-isolation or cold/warm
output/classification differences. No writes means no new written-title or
credit verification. The other full-matrix findings remain open. All 17 mock
and review-tool tests and `just eval-auto-tag-suite` pass.

Final evidence is in
`.amp/in/artifacts/reviewed-51-fixture-replay-3-20261109/verification-partial.json`.
Capture receipts, before-snapshots and byte-exact validation are in
`.amp/in/artifacts/reviewed-51-fixture-completion-20261109/`. The earlier full
replay remains unchanged. Live providers were accessed only for read-only
fixture capture. Original music was not accessed.

### Remaining abstention investigation, 2026-11-09

Forty-three controlled probes through the unchanged native credibility and
track-protection functions reproduce both failures in all three profiles.
Requests come from the production reader on disposable copies of the silent
FLACs. Production clients parse three frozen responses over loopback. All
controls meet their declared expectations. This is not a new cold/warm resolver
replay, and counterfactual acceptance is not tagging credit.

**Bellyache has a standalone-year parsing defect.** Its filename ends with
`(Marian Hill Remix) (2017)`. The version check compares the entire bracketed
qualifier list when either side contains a performance qualifier, treating
`2017` as part of the remix comparison. Title normalization also retains that
standalone year. Removing only the artist prefix does not help. Removing only
the year from every filename-derived channel permits the reviewed remix in all
three profiles, with the named remix intact. The studio hard negative, a
different remix at the same duration, and a duration-conflict control still
reject. Clean existing title tags do not avoid the filename's qualifier veto.

**Paint the Sky with Stars lacks captured remaster evidence.** Discogs 636215
was successfully fetched in all three profiles. Fourteen filenames assert
`Remastered 1997`, which its unqualified tracklist and release metadata do not
establish. Its `Mastered By` credit is not `Remastered By` evidence. All 16
durations are compatible. Removing artist prefixes leaves the same 14 failures.
Removing the remaster claims permits complete acceptance, but that is a
diagnostic counterfactual, not an authorized repair. Synthetic matching provider
context also permits acceptance; wrong-year context, a contradictory track
remaster and incompatible duration still reject. No synthetic context became
a provider fixture or reviewed answer.

The safe next change is to parse Billie's bare year without discarding its
named remix. Enya must retain its version check until source-bound remaster
evidence is established. The frozen receipts do not determine whether the real
1997 edition was remastered. Neither a larger discovery budget nor AI
configuration addresses the demonstrated first rejections.

Inputs, results, source locks and archived probe source are in
`.amp/in/artifacts/remaining-abstentions-20261109/`. No production code, provider
fixture, reviewed answer or score changed. No upstream provider or original
music was accessed. Temporary media copies were removed.

### Second provider review expansion, 2026-11-08

Four further manifests add 22 positives with 262 complete one-to-one mappings
and explicitly leave 11 more cases unresolved. They preserve all earlier answers.

| Manifest | Corpus | New positives | Mapped tracks | Unresolved |
| --- | --- | --- | --- | --- |
| `reviewed-expansion-2-20261108.json` | `corpus.json` | 14 | 175 | 0 |
| `reviewed-cjk-artist-2-20261108.json` | `artist-batch-extension/corpus.json` | 6 | 68 | 9 |
| `reviewed-library-2-20261108.json` | `library-sample/corpus.json` | 1 | 18 | 1 |
| `reviewed-curated-2-20261108.json` | `curated-hard-cases/corpus.json` | 1 | 1 | 1 |

The positives cover eleven Enya physical cases, a second physical *Lo Vas A
Olvidar*, the nine-track *dont smile at me*, *Amala* deluxe, six 盧廣仲 albums,
宇多田光's 18-track *Single Collection Vol.2*, and Beyoncé's *7/11*.
Repeated physical cases do not establish independent recordings or editions.
All mapped occurrences have measured compatible durations; the largest delta
is 4.013 seconds. Version qualifiers and distinct occurrences remain intact.
Case-scoped artist aliases are explicit; no generic transliteration, subtitle
stripping, or duration-threshold change was introduced.

Nineteen additional raw MusicBrainz responses are frozen under
`provider-snapshots/review-expansion-2-20261108/`, with hashes and request
provenance in `provider-discovery.json`. Content hard negatives include an
eight-track *dont smile at me* without *&burn*, the 13-track *Amala* without
the three deluxe additions, and *A Day Without Rain* substituting *The First
Of Autumn* for *Isobella*. Pressing differences alone are not hard negatives.

The Shepherd Moons case `872fee627bfd` uses an unqualified release-track title
whose recording disambiguation and duration support Gaelic-only *Book of Days*.
The captured local release's explicitly qualified title remains unsupported by
the unchanged mechanical reviewer. This is a compatible content representative,
not acoustic proof of language or a proven source pressing.

Eight 张学友 cases still need title-script or lexical-alias adjudication; some
also have incompatible durations. A 盧廣仲 single needs subtitle review.
Beyoncé's *Grown Woman (Studio Version)* needs version evidence. 久石让's
*Brother* has 13 local versus 15 provider tracks and conflicting title/duration
associations; captured IDs or similar timings cannot resolve those conflicts.
None is a verified no-match.

#### Initial replay before fixture completion

The initial replay completed 22 cases × three profiles × cold/warm, using the same
executable as the preceding 29-case run. All 66 native processes exited zero,
but the outer replay gate failed because 205 requests lacked fixtures across
132 distinct routes. Missing responses are not evidence of no provider match.

| Profile | Applied cold / warm | Abstained cold / warm | Written tracks per phase |
| --- | --- | --- | --- |
| Folder and filenames | 10 / 10 | 12 / 12 | 119 |
| Existing tags without IDs | 12 / 12 | 10 / 10 | 144 |
| Existing tags with IDs | 16 / 16 | 6 / 6 | 173 |

All 872 written-track readbacks preserve payloads, local-order numbering and
uniform shared album fields. Cold/warm outputs agree, caches are isolated, and
all 262 source dummies remain unchanged. Twelve exported payload hashes are
redacted by the existing filename sanitizer; native payload checks pass for them.

Two strict title checks reject `Book of Days (Gaelic only version)` in the
ID-assisted Shepherd Moons case because its frozen representative uses the
unqualified title. The selected raw release explicitly supplies that qualifier;
this is the disclosed review-boundary mismatch, not proof of a wrong recording.
Neither the title rubric nor the reviewed answers was changed to pass it.

All written artist credits match their selected frozen responses. However,
ID-assisted *&burn* remains Billie Eilish-only in both phases: selected Discogs
37353822 omits Vince Staples, while reviewed Discogs 13898510 credits him as
`Guest [With]`. Complete known collaborator recovery is therefore not established.
Filename-only Amala also issues artist searches for `Albums`, exposing an input
parsing problem, although missing responses still confound its final outcome.

The frozen Utada representative actually has one medium with 18 tracks; the
review notes' two-media description is inaccurate. Native output uses local
track numbers 1–18 and disc 1, with no source disc tags to preserve. This replay
does not prove two-disc reconstruction. Frozen review inputs were left unchanged.

Evidence is in `.amp/in/artifacts/reviewed-22-native-20261108/`, including the
locked executable, all 132 records, exact missing routes and the independent
`audit.py` / `verification.json`. No production behavior or provider fixtures
changed in this replay. Complete missing fixtures before scoring the abstentions.

#### Fixture-complete replay, 2026-11-08

Eight capture/replay rounds added 478 genuine HTTP 200 responses: 58 MusicBrainz
and 420 Discogs responses. Exact queries, credential-free URLs, capture times,
statuses and raw-byte hashes are frozen under
`provider-snapshots/reviewed-22-completion-20261108/` and in
`provider-discovery.json`. All 962 earlier records and all reviewed answers,
membership and partitions remain unchanged. The imported mock has 1,451 routes;
all 478 added routes pass direct HTTP status and byte-hash probes.

The 54 initially affected case/profile pairs were rerun cold and warm. Twelve
fixture-complete baseline pairs were retained, and each later round reran only
pairs that still lacked responses. The eight rounds executed 144 new native
processes, all exiting zero, with the unchanged executable and production
request limits. Final accounting covers 66 pairs and 132 cache-phase records,
with zero missing routes and 2,352 recorded HTTP 200 requests.

| Profile | Applied cold / warm | Abstained cold / warm | Written tracks per phase |
| --- | --- | --- | --- |
| Folder and filenames | 14 / 14 | 8 / 8 | 156 |
| Existing tags without IDs | 19 / 19 | 3 / 3 | 216 |
| Existing tags with IDs | 19 / 19 | 3 / 3 | 216 |

All 1,176 native readbacks preserve payloads, local-order numbering and uniform
shared album fields. Outputs and classifications agree cold versus warm, cache
isolation passes, and all 262 source dummies remain unchanged. Independent
exported payload checks cover 1,164 readbacks; the sanitizer redacts the other 12.

The three expanded Taiwanese Enya cases still abstain in all profiles. Their
tag-assisted runs report `discovery_budget_limited`; filename-only runs reach
`ai_not_configured`. Filename-only Amala, Dark Sky Island deluxe, Watermark
Misprint, Utada and 7/11 also abstain. These outcomes now have complete fixture
coverage, but neither reason code proves a no-match or establishes the root cause.
Amala's observed artist search for `Albums` remains an input-parsing finding.

Strict title checks accept 1,172 of 1,176 readbacks. Two failures retain the
provider's Gaelic-only Book of Days qualifier. Two more occur in Utada's
non-ID profile, which writes `Passion (Single Version _ 2014 Remastered)`.
That wording exists in the captured filename, but the reviewed title is
`Passion (Single Version)` and the selected representative names it `Passion`.
The replay does not verify the filename's remaster claim. No title rule or
reviewed answer was changed to accept either discrepancy.

All written credits match their selected responses. However, &burn lacks the
independently reviewed Vince Staples credit in both existing-tag profiles,
cold and warm. Selected Discogs 37353822 omits him; reviewed 13898510 credits him
as `Guest [With]`. Selected-response preservation is not complete collaborator recovery.

Final evidence is in `.amp/in/artifacts/reviewed-22-fixtures-replay-8-20261108/`;
capture provenance, round logs and direct replay validation are in
`.amp/in/artifacts/reviewed-22-fixture-completion-20261108/`. Each round's
`run-plan.json` and `executions.json` distinguish reruns from reused evidence.
The final audit checks 1,859 source/input locks. All 95 script/package tests,
the frozen-suite check and capture validation pass. Production code, discovery
budgets, title rules, reviewed answers and original music are unchanged.

#### Blocker-fix replay, 2026-11-08

The eight affected cases were rerun with a freshly built, source-locked native
evaluator across all three profiles, cold and warm. Folder parsing now traverses
explicit nested containers and separates catalog-labelled Deluxe and Misprint
suffixes from album names. Empty CJK artist-name searches can use an exact
provider alias, and validated aliases can remove a folder's artist prefix.
Guarded title comparison accepts unique, duration-supported word-spacing
differences without discarding performance qualifiers or changing display titles.

Content-only selection can use a completed provider's evidence when unrelated
Discogs discovery exhausts its release-detail allowance. Other incomplete
discovery, outages, retries and conflicting mappings still block this fallback.
Discovery budgets and global year matching are unchanged. Content-only writes
leave release and recording IDs unset and do not establish edition provenance.

| Profile | Applied before, cold / warm | Applied after, cold / warm | Written tracks per phase after |
| --- | --- | --- | --- |
| Folder and filenames | 0 / 0 | 7 / 7 | 88 |
| Existing tags without IDs | 5 / 5 | 8 / 8 | 106 |
| Existing tags with IDs | 5 / 5 | 8 / 8 | 106 |

These are eight-case subset results, not a new full 51-case accuracy run. All
three expanded Enya albums now complete in every profile. Amala deluxe, Dark
Sky Island deluxe, Watermark Misprint and Beyoncé's 7/11 also recover on the
filename path. All previous applied outcomes in this subset remain applied.
Only four of the 48 records confirm a reviewed edition; basic-tag completion
is not an exact-edition or acoustic identity claim.

Utada's filename-only case still abstains. The corrected lookup reaches the
reviewed representative, but track 17's romanized subtitle and one-sided
`2014 Remastered` qualifier do not establish equivalence with the Japanese
provider title. Both provider attempts finish without fixture errors. Its
existing-tag, non-ID profile still writes the previously flagged
`Passion (Single Version _ 2014 Remastered)`, producing two strict-title
findings, cold and warm. Neither the reviewed answer nor the title rubric
was changed to accept that remaster claim.

Ten genuine HTTP 200 responses, two MusicBrainz and eight Discogs, fill routes
reached by the corrected lookups. They retain exact request and raw-byte
provenance under `provider-snapshots/reviewed-blocker-completion-20261108/`.
All 1,440 preceding records, reviewed answers, membership, partitions and
non-discovery input hashes remain unchanged. Four replay rounds execute 30
native processes; the final accounting retains 24 pairs and 48 cache-phase
records with 1,356 HTTP requests and zero missing routes. Later rounds rerun
only pairs with missing responses and retain each pair's execution provenance.

All 600 written-track readbacks preserve payloads, local-order numbering and
uniform shared album fields. All 600 selected-provider credits are supported,
with no missing independently reviewed collaborators in this subset. Strict
title checks accept 598 of 600 readbacks, with only the two Utada findings above.
All 106 source dummies remain unchanged. Outputs and classifications agree
cold versus warm, and cache isolation passes. Independent exported payload
checks cover 594 readbacks; the sanitizer redacts six, whose native checks pass.

The final audit checks 1,713 source/input locks and 1,461 frozen provider
responses. All ten new routes pass byte-exact HTTP replay probes. The native
server suite passes 701 tests with 17 explicitly gated tests ignored; all 95
script/package tests, the frozen-suite check and scoped formatting checks pass.
The earlier Gaelic-only title and Vince Staples findings outside this subset
are not cleared by this replay. Original music was neither read nor changed.

Evidence is in `.amp/in/artifacts/reviewed-blocker-replay-4-20261108/`, including
`verification.json`, source locks, the executable and execution provenance.
Capture manifests and closure logs are in
`.amp/in/artifacts/reviewed-blocker-fixtures-20261108/`; regression red/green,
native-suite and script logs are in
`.amp/in/artifacts/reviewed-blocker-fixes-20261108/`. Changes remain local and
uncommitted; nothing was pushed, merged or deployed.

### Utada evidence follow-up, 2026-11-08

The reviewed MusicBrainz release `8c3d0f30-247e-4b56-809c-79f795f66d4e`
is the **2014 remaster**, not the 2010 CD. Its frozen response already contains
`2014 Remastered, 96kHz/24bit`, the date `2014-12-09`, one 18-track digital medium,
and a link to the exact mora product. The
[contemporary Universal announcement](https://www.universal-music.co.jp/utada-hikaru/news/2014-12-09/)
names Vol.2 (2014 Remastered) and links
[that product](https://mora.jp/package/43000006/00600406550866/).
The store confirms the date, 18 tracks, 96kHz/24bit FLAC and Ted Jensen's remaster.
This corrects the earlier interpretation that the reviewed catalog lacked
remaster support. It does not authenticate the local audio's mastering.

The official artist catalog identifies track 17 as
[Hymne à l'amour ～Ai no Anthem～ in English](https://www.en.utadahikaru.jp/music/8ccemsh22)
and [Hymne à l'amour ～愛のアンセム～ in Japanese](https://www.utadahikaru.jp/music/8ccemsh22).
[Apple's song entry](https://music.apple.com/us/song/hymne-a-lamour-ai-no-anthem/1440748484)
also supplies the ASCII title `Hymne A L'Amour - Ai No Anthem`.
Its displayed date is inconsistent with the official release evidence and is
not used as date evidence here. These sources establish the subtitle's catalog
identity without dropping version qualifiers or proving acoustic identity.

Nineteen controlled 18-track scenarios call the unchanged public native
credibility and track-protection functions. No HTTP, tag writes or original
media are involved. Removing only Hymne's remaster qualifier still abstains;
using its Japanese subtitle while retaining that qualifier completes.
Adding the sourced ASCII alias in memory also completes, against both the
2014 digital and 2010 CD representatives. The accented English alias alone
does not complete. These are diagnostic controls, not new provider fixtures.

The non-ID replay selects the separate 2010 CD `1d231286-61e1-45b4-8d83-a83638088cb4`.
Its bare Passion title does not match the local wave-delimited Single Version
tag through the same path as the reviewed digital title. The matcher instead
uses the filename and retains its remaster wording. Changing only the filename
to remove that wording, or only the local tag to parenthesized Single Version,
produces a clean Single Version title. The provider parser also drops the
release-level remaster disambiguation. These findings explain why catalog
support and the selected result must be assessed separately.

A synthetic negative control exposes an existing version-label gap: consistent
local tag and filename `Passion (After The Battle)` complete against provider
`Passion (Single Version)` when durations are held compatible. A numeric
filename behaves the same way; an incompatible-duration control rejects.
This is not an observed wrong-version audio assignment. The
[official single catalog](https://www.universal-music.co.jp/utada-hikaru/products/toct-5004/)
names both versions separately. Adding aliases alone would not fix this gap.

No production code, reviewed answers, provider fixtures or scores changed in
this investigation. The earlier cold/warm results and two strict-title findings
remain recorded. A future fix needs version-conflict controls, retained
release-level version evidence and title precedence that does not promote
filename claims. It must not infer local mastering from catalog support.

Evidence is in `.amp/in/artifacts/utada-evidence-investigation-20261108/`.
`verification.json` records the controls; source locks cover 93 native and frozen
inputs. `native-probe-source.tgz` preserves the diagnostic source and locked
dependencies. This is not another end-to-end cold/warm run.

### Utada evidence fix and replay, 2026-11-08

The native parser now retains MusicBrainz release disambiguation as matching
evidence. Wave-delimited version tags compare with parenthesized versions
without changing their display text. Passion now matches its existing tag
before filename fallback, so the output does not acquire the filename's
`2014 Remastered` wording.

Version conflicts reject before recording-ID, title, containment and positional
matching. Local remaster claims require provider support, including the year
when specified. Release context cannot override a contradictory track-level
remaster year. These checks do not authenticate the local audio's mastering.
Old query and MusicBrainz detail cache namespaces are retired so cached parsed
responses cannot silently omit the retained evidence.

Three fresh native processes replay Utada's 18 tracks in all profiles, each
with separate cold and warm media and a shared cache between phases.

| Profile | Cold | Warm | Written tracks per phase |
| --- | --- | --- | --- |
| Folder and filenames | Needs review | Needs review | 0 |
| Existing tags without IDs | Applied, edition unresolved | Applied, edition unresolved | 18 |
| Existing tags with IDs | Applied, reviewed edition confirmed | Applied, reviewed edition confirmed | 18 |

Both tagged profiles select the reviewed 2014 digital representative. The
non-ID profile leaves release and recording IDs unset. All 72 written-track
readbacks pass the unchanged strict title and credit checks, clearing the two
preceding Passion findings. Payloads, local-order numbering and shared album
fields are preserved. There are no missing routes, missing reviewed collaborators,
cache-isolation failures, or cold/warm output or classification differences.

Filename-only Hymne still lacks a provider-supplied romanized subtitle alias.
The reviewed release reaches track 17 but supplies only positional evidence,
which is insufficient for a write. No alias was fabricated, qualifier dropped,
reviewed answer changed, or provider fixture added. This is a six-record Utada
rerun, not a new full-suite accuracy result or acoustic identity proof.

The final native server suite passes 704 tests with 17 gated tests ignored.
The first broader attempt had subprocess deadline failures and one outdated
remaster expectation. After correcting that expectation, the full suite passes
with four test workers. Typecheck, scoped Rust formatting and diff checks pass.

Final replay evidence is in
`.amp/in/artifacts/utada-evidence-cold-warm-2-20261108/verification-partial.json`.
The unchanged partial auditor retains eight-case selection metadata, but only
six evaluations and three fresh executions ran. It validates 1,713 locks and
1,461 frozen responses. Regression and suite logs are in
`.amp/in/artifacts/utada-evidence-fix-20261108/`. No original music was accessed.
Changes remain local and uncommitted.

### Utada alias capture and filename-only replay, 2026-11-08

Five raw HTTP 200 captures now document the title localization independently
of the native provider responses. Apple lookup responses for the US and Japan
use the same track ID `1440748484`, album ID `1440747007`, artist ID `18756224`,
disc 2 track 4 and duration 389.497 seconds. Their titles are respectively
`Hymne A L'Amour - Ai No Anthem` and `Hymne a l'amour ～愛のアンセム～`.
Both identify the album as `2014 Remastered`. The local captured duration is
389.947 seconds, a difference of 0.450 seconds.

Raw English and Japanese official artist pages corroborate the paired titles
at the same catalog entry and link the same mora product. MusicBrainz's live
recording response for `331ad074-f703-4816-b7e1-3ed225e6b225` explicitly returns
an empty alias list. These captures establish catalog title correspondence,
not local recording or mastering identity. Apple's inconsistent 2010 album and
2004 song dates are not treated as evidence for the 2014 edition date.

A fresh native process reruns filename-only Utada cold and warm using the
previously verified binary and unchanged MusicBrainz/Discogs fixtures. Both
phases still need review, with zero writes and zero missing routes. The reviewed
representative reaches track 17 but has only positional evidence. Payloads and
cache isolation pass, with no cold/warm output or classification differences.
No written-title checks execute because nothing is written.

At this capture-only stage, the native provider path did not consume Apple or
official-site alias evidence. The captures remained separate from MusicBrainz
payloads and reviewed answers. Version checks, application code and scored
membership were unchanged; that rerun did not improve the accuracy result.

Capture URLs, timestamps, raw bytes and SHA-256 hashes are in
`.amp/in/artifacts/utada-alias-evidence-20261108/capture-manifest.json`, with
the checked correspondence in `verification.json`. Fresh native execution and
audit evidence are in
`.amp/in/artifacts/utada-alias-filename-replay-20261108/verification-partial.json`.
Its inherited eight-case selection metadata does not change the executed scope
of one case, one profile and two cache phases. No original music was accessed.

### Provenance-backed alias ingestion and Utada replay, 2026-11-08

The native provider parser now consumes the reviewed localization receipt in
`data/provider-title-aliases.json`. This bundled catalog contains verbatim Apple
US and Japanese response bodies, request URLs, timestamps and SHA-256 hashes.
It does not add an Apple search provider or change MusicBrainz responses.

The explicit binding targets MusicBrainz release
`8c3d0f30-247e-4b56-809c-79f795f66d4e`, recording
`331ad074-f703-4816-b7e1-3ed225e6b225`. Enrichment requires the reviewed release,
artist, remaster disambiguation, canonical titles, unique recording occurrence,
position, duration, track count and matching artist credits. Both Apple receipts
must agree on artist, album, song, position and duration. Corrupt receipts fail
validation. These reviewed bindings establish catalog localization, not local
recording or mastering identity.

Aliases enter comparison titles, not primary display titles. Candidate evidence
retains the receipt ID and source hashes through alignment and cache reuse.
The shared matcher still checks performance, mix and remaster claims before
matching. Alias-bearing tracks additionally require finite compatible durations.
Query and parsed MusicBrainz detail cache namespaces change to retire entries
that omit alias evidence. No reviewed answer or scoring rubric changed.

One fresh native process reruns only Utada `folder_filename` cold and warm.
Both phases apply all 18 tracks using the reviewed 2014 representative. The
unchanged independent audit accepts all 36 written titles and artist credits,
including `Hymne à l'amour 〜愛のアンセム〜` and `Passion (Single Version)`.
Readback, payload preservation, local-order numbering and cache isolation pass.
There are zero missing routes and no cold/warm output or classification differences.
The other two profiles and seven inherited selection cases are not rerun here.

Evidence is in
`.amp/in/artifacts/utada-alias-ingested-filename-replay-20261108/verification-partial.json`.
The runner locks the bundled catalog as well as native source, input media and
unchanged provider responses. Earlier capture-only abstentions remain historical
results. This replay uses synthetic FLACs and cannot authenticate original audio.

### Provider review expansion, 2026-11-08

The first expansion added three manifests reviewing ten existing physical cases:
nine positives with 34 complete, one-to-one title mappings and one unresolved
soundtrack. Ten raw MusicBrainz responses retain their capture timestamps,
request parameters, and SHA-256 values in `provider-discovery.json`.

| Manifest | Corpus | New positives | Mapped tracks | Unresolved |
| --- | --- | --- | --- | --- |
| `reviewed-billie-20261108.json` | `corpus.json` | 6 | 7 | 0 |
| `reviewed-cjk-artist-20261108.json` | `artist-batch-extension/corpus.json` | 2 | 15 | 0 |
| `reviewed-cjk-library-20261108.json` | `library-sample/corpus.json` | 1 | 12 | 1 |

The Billie Eilish cases cover *Guitar Songs*, *Lo Vas A Olvidar*, *my future*,
*Ocean Eyes*, ordinary *bellyache*, and its Marian Hill remix. The two *bellyache*
versions are reciprocal hard negatives with different qualifiers and durations.
The CJK positives cover 盧廣仲's *100種生活* and *幾分之幾*, and 草東沒有派對's
*醜奴兒*. Their manifests explicitly review simplified/traditional artist-name
pairs through `artistAliases`. The reviewer accepts only the exact listed provider
spelling for that case. This does not enable generic transliteration or relax
track-title, version, duration, or occurrence checks.

All 34 new mappings have provider duration evidence. The largest delta is
2.021 seconds. For *my future*, the release-track duration is null, but the frozen
recording supplies 210 seconds. The reviewer uses that recording duration only
when a release-track duration is absent. Unknown durations remain unknown.

The *終極一班2* soundtrack remains unscored. Its frozen release has ten tracks
versus nine local files, different order, simplified/traditional title forms,
several version-specific titles, and no track or recording durations. Promotion
needs explicit occurrence-level version review, duration evidence, and an allowed
extra-track policy for `數不盡的星空`. This is not a verified no-match.

These reviews establish metadata-content compatibility, not acoustic identity,
mastering, clean/explicit content, or the source pressing. The original
collaboration review permitted album-artist-only output without establishing
per-track credit recovery. Current credit preservation needs its own audit
against the frozen provider credits, separate from that historical title review.
Read-only provider capture used existing captured IDs as investigation leads,
not as discovery input or sufficient truth. No original media was accessed.

The first native replay exposed 77 missing requests. Filling those responses
revealed more dependent requests. Seven capture/replay passes added 173 genuine
HTTP 200 responses, comprising 59 MusicBrainz and 114 Discogs routes. No existing
response or reviewed answer changed. The final run used the same native binary
for all nine positives, three profiles, and cold and warm phases: 54 invocations,
560 HTTP 200 responses, and zero missing routes.

| Profile | Applied cases per phase | Reviewed basic-tag completions per phase |
| --- | --- | --- |
| Folder and filename | 3/9 | 2/9 |
| Existing tags without provider IDs | 9/9 | 9/9 |
| Existing tags with provider IDs | 9/9 | 8/9 |

Existing-tag profiles retain observed titles, so their results are not proof of
title discovery from filenames. The ID profile reports 34 written files per
phase, but only 22 change bytes. The 12 files in *100種生活* remain unchanged
reapplications. All applied titles agree with reviewed content after removing
one known artist prefix, but that prefix remains a title-format defect.

Folder-only *醜奴兒* writes `草東沒有派對 - Intro` instead of `Intro` and uses
`No Party For Cao Dong`, which is outside the reviewed artist-name aliases.
ID-assisted *Lo Vas A Olvidar* now applies through Discogs, but its artist reads
`Billie Eilish & Rosalía & Billie Eilish | ROSALÍA`. The duplicate credit fails
the clean basic-tag check. Six folder-only cases still abstain with no credible
accepted release. These are application findings, not missing test responses.

Readback and payload checks passed throughout. All 34 source dummies remain
unchanged, and cold and warm outcomes agree. Edition-oriented classification
still reports unresolved cases separately from basic-tag completion. Final
evidence is in `.amp/in/artifacts/new-nine-completed-7-20261108/verification.json`;
capture provenance and validation are in `.amp/in/artifacts/new-nine-fixtures-20261108/`.

To export one new manifest through the existing subset recipe, set
`SOUNDROBE_AUTO_TAG_EVAL_CORPUS` and `SOUNDROBE_AUTO_TAG_SUBSET_MANIFEST` to the
corresponding paths in the tables. Set `SOUNDROBE_AUTO_TAG_SUBSET_DIR` to a new
output directory, then run `just eval-auto-tag-subset`. The suite recipe checks
all eight review manifests together. The `--reviewed-manifest` flag is repeatable.

Next evidence work is independent provider review of the new populations and
the holdout artists. Reviewed TV OST compilations, true no-match cases, and
real-audio recognition remain gaps. Synthetic replay cannot establish acoustic
accuracy, and this inventory expansion demonstrates no tagging improvement.

## Difficult albums and external-library sample

Two bounded selections add 24 folders and 247 tracks. Each `selection.json`
records the exact source-relative folders, difficulty labels, and reasons.
This is a purposeful structural sample, not a random or representative accuracy sample.

| Population | Folders | Tracks | Coverage |
|---|---:|---:|---|
| `curated-hard-cases/` | 10 | 73 | Current Curated Beyoncé conflicts, versions and collaborator credits |
| `library-sample/` | 14 | 174 | External-volume soundtracks, partial Queen editions, CJK aliases and disc numbering |

The Curated selection follows the [retained investigation](https://ampcode.com/threads/T-01a1ad54-f1b8-75a1-8f82-48e86f2644ac).
It includes `4 (Deluxe Edition)`, `B'day`, `Beyoncé`, `I AM...SASHA FIERCE`,
and six single/remix folders. Counts reflect current membership after earlier
quarantines, not the historical 87-folder census. Conflicting Schoolin' Life
titles/positions, explicit version words, HTML entities, and same-stem physical
rows remain intact. Ordered plural credits preserve The Isley Brothers as one act.
Earlier acoustic hypotheses and completed runs do not supply expected answers.

The external selection includes three TV OSTs, `友情岁月 3CD`, `Slam Dunk Best
Collection`, five partial Queen edition/live folders, 宇多田光, 草东没有派对,
and two differently titled 久石让/Brother soundtracks. `友情岁月` has 51 files
across observed discs 1–3 and 50 unique disc/track positions. Missing disc tags
in the Slam Dunk and 宇多田光 inputs remain missing, not inferred from counts.
All new expectations are `unverified` with empty provider mappings.

To refresh one selection, use the source library read-only:

```sh
python3 -B scripts/generate_auto_tag_eval_corpus.py \
  --source-root "$HOME/Downloads/Music/Curated" \
  --selection test/fixtures/tauri/auto-tag-eval/curated-hard-cases/selection.json \
  --output-dir test/fixtures/tauri/auto-tag-eval/curated-hard-cases \
  --corpus-version 2026-11-06.curated-hard-1
```

For the external sample, use `/Volumes/downloads/music`, `library-sample/`, and
version `2026-11-06.library-sample-1`. Review any changed observations before
updating suite locks. Never regenerate a reviewed ledger over existing answers.

## Dummy media without original tracks

The checked-in fixtures contain metadata only. Generate disposable silent FLACs
directly from any frozen corpus; no original media is needed:

```sh
mkdir -p .amp/in/eval-dummies
just eval-auto-tag-dummies \
  test/fixtures/tauri/auto-tag-eval/curated-hard-cases/corpus.json \
  .amp/in/eval-dummies/curated-hard-cases
```

Repeat for `library-sample/`, `artist-batch-extension/`, and the root corpus,
using a different output directory for each. Requires FFmpeg and the Rust server
test dependencies. The output directory must not already exist; a failed or
completed generation is never overwritten. Remove only your disposable output
or choose a fresh directory when retrying.

Each output contains `media/`, a runtime `corpus.json`, and `expectations.json`.
Captured tags and IDs are written through the native `WriteQueue`. Original
filenames, formats and source hashes remain in separate provenance fields;
runtime paths and hashes identify the dummy files. Same-stem different-codec
files remain separate using zero-width filename suffixes. Compilation paths
receive the artist component required by the native evaluator.

Captured positive durations are preserved. Two legacy rows have a captured
duration of zero; they receive one-second placeholders with `originalDuration: 0`
and `durationIsPlaceholder: true`. Their runtime duration describes the dummy,
not the original recording. Exclude these rows from duration-fidelity metrics.
No source duration is guessed or written back into the frozen corpus.

For native replay, set `SOUNDROBE_AUTO_TAG_EVAL_CORPUS` and
`SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS` to those generated files, and
`SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT` to the generated `media/` directory.
Choose a profile explicitly and use the existing offline provider service.
Creation itself performs no provider requests. The original checked-in corpus
and reviewed answers remain unchanged.

Dummy files support metadata, mapping, duration, credit, write/readback, and
profile-isolation checks. They do **not** establish acoustic recognition,
duplicate-recording identity, original-codec behavior, or tagging accuracy.
Every dummy contains silence; accidental equal audio payloads are not source
duplicate evidence. Do not use this population for acoustic or duplicate-detection
metrics. Unreviewed inputs remain outside scored results even when a run completes.

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

Recording-only readback checks the permitted title, artist, ordered credits and
recording-ID changes against the candidate. Every other exposed readback field must match
the pre-write snapshot, and the write count must match eligible recording patches.
File size and container-derived bitrate may change when tags change; audio payloads
are checked separately. An unresolved edition stays unresolved after successful
recording-only verification. Non-applied results without writes must remain byte-identical.

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
