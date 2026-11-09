# Auto-tag evaluation corpus

`corpus.json` is a metadata-only, frozen inventory of the seven-artist input set. It contains no audio, artwork, lyrics bodies, secrets, or run logs. The source-relative folder names, filenames, captured tags, durations, numbering, provider IDs, and full-file SHA-256 values are retained for auditability.

All 321 inventoried physical folders are represented as individual cases (2,841 tracks). The three input profiles are materialized per case: `folder_filename` is clean discovery, `assisted_without_ids` preserves observed tags while removing provider IDs, and `tagged_recovery` preserves captured IDs for direct lookup/idempotence checks. Expectations remain `unverified` until a provider-backed reviewer records acceptable editions, hard negatives, and complete mappings; the reviewed Relapse case is the first exception.

Refreshes require a reviewed diff. Generate with `python3 scripts/generate_auto_tag_eval_corpus.py --source-root /path/to/curated-library` from the repository root (or set `SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT`); the script reads the curated source and writes metadata only. Run `just eval-auto-tag-ground-truth` to reconcile all cases against the expectation ledger, and `just eval-auto-tag-repro` to verify frozen response digests and saved synthetic-input equivalence.

The five non-diagnostic artists are marked `provisional_gold_unverified` in the corpus until an independent provider content and mapping review records complete acceptable-edition mappings. The Relapse With Bonus case is the reviewed Eminem exception; the remaining Eminem and all Enya cases remain diagnostic and are outside scored metrics. `expectations.json` is the separate reviewed-answer ledger; `candidate-pools.json` and its response snapshots provide deterministic edition replay across Relapse, Enya edition/disc/title and CD/DVD cases, plus a separate Discogs edition fixture. `reviewed-subset.json` records a provider-backed representative review of 30 cases (20 complete matches and 10 explicit unresolved holdouts), with raw Enya, Ariana, Doja Cat, Eagles, and Ellie Goulding payloads frozen under `provider-snapshots/`. Candidate-pool entries carry SHA-256 locks so a changed local fixture or provider response fails the reproducibility audit.

`artist-batch-extension/` adds a separate, unverified metadata-only inventory of
33 folders and 418 tracks from 张学友, 卢广仲, Kenny G, and seven instrumental
compilations. `curated-hard-cases/` adds ten folders and 73 tracks from the current
Curated Beyoncé population; `library-sample/` adds 14 folders and 174 tracks from
the external music volume. Their `selection.json` files record exact membership,
difficulty labels and sampling reasons, not independently reviewed identities.
`batch-suite.json` locks all four inventories and provider evidence,
with 15 artist batches and 12 independent compilation-album batches.
Run `just eval-auto-tag-suite` to check its hashes and membership. The combined
suite has 378 cases and 3,506 tracks.
See [expanded inventory limits and regeneration](../../../../docs/auto-tag-evaluation.md#expanded-artist-batch-inventory).

The reviewed suite now has **51 positives with 539 mapped tracks, 22 unresolved
cases, and 305 unverified cases**. Eight positives belong to the retrospective
Billie Eilish holdout; this is not a blind benchmark.

The [full 51-case replay](../../../../docs/auto-tag-evaluation.md#full-reviewed-coldwarm-replay-2026-11-09)
executes all three profiles cold and warm on one locked binary: 153 fresh
processes and 306 evaluations, with no copied results. Each profile applies
49/51 cases per phase. The audit accepts 49 filename-only, 48 tags-without-IDs
and 47 ID-assisted written cases per phase after retaining title and credit findings.
All 3,132 written-track readbacks preserve payloads and numbering, and cold/warm
outputs agree. Utada passes all six evaluations. Bellyache's Marian Hill remix
and Paint the Sky with Stars abstain in every profile; filename-only discovery
also exposes three uncaptured routes. Existing-tag `&burn` omits Vince Staples,
and ID-assisted Book of Days retains a Gaelic qualifier outside the frozen
title oracle. These findings remain open; this is not a clean verification pass.
Evidence is in `.amp/in/artifacts/reviewed-51-cold-warm-20261108/verification.json`
and `verification-summary.json`. Provider responses and reviewed answers remain unchanged.

The later [fixture-completion rerun](../../../../docs/auto-tag-evaluation.md#full-reviewed-fixture-completion-2026-11-09)
adds 72 genuine provider responses without replacing earlier captures or answers.
The imported mock now has 1,533 routes. Both affected filename-only cases ran
cold and warm on the same binary, with zero missing routes across 155 HTTP
requests. Both still abstain and write nothing. Bellyache rejects title mappings;
Paint the Sky with Stars exhausts its Discogs release-detail allowance.
There are no payload or cold/warm differences. These are application findings,
not remaining fixture gaps. The other profiles were not rerun in this follow-up.
Final evidence is in
`.amp/in/artifacts/reviewed-51-fixture-replay-3-20261109/verification-partial.json`.

The [remaining-abstention investigation](../../../../docs/auto-tag-evaluation.md#remaining-abstention-investigation-2026-11-09)
reproduces both failures across all profiles in 43 controlled native probes.
Billie's standalone `(2017)` pollutes the named-remix comparison. Enya's
fourteen `Remastered 1997` filenames require evidence absent from the frozen
representative. No version check, provider response or reviewed answer changed.
Counterfactual successes are diagnostic only, not additional tagging passes.

The second 2026-11-08 expansion adds 22 positives with 262 complete mappings
through `reviewed-expansion-2-20261108.json`,
`reviewed-cjk-artist-2-20261108.json`, `reviewed-library-2-20261108.json`, and
`reviewed-curated-2-20261108.json`. It covers eleven Enya physical cases, two
Billie Eilish cases, Amala deluxe, six 盧廣仲 albums, 宇多田光's Single Collection
Vol.2, and Beyoncé's 7/11. Nineteen additional raw MusicBrainz responses are frozen
under `provider-snapshots/review-expansion-2-20261108/` with request provenance.
All new positive occurrences have measured compatible durations. Earlier answers
and title/version/duration checks are unchanged.

Eleven more cases remain explicitly unresolved: eight 张学友 cases need title
alias review, with duration conflicts in some; a 盧廣仲 single needs subtitle
review; Grown Woman needs Studio Version evidence; and Brother has conflicting
13-versus-15-track associations. These are not verified no-matches.

After blocker fixes, the eight affected cases have 48 fixture-complete native
cold/warm records across all three profiles. Filename-only application improves
from 0/8 to 7/8; both existing-tag profiles improve from 5/8 to 8/8, identically
cold and warm. All three expanded Enya albums complete with edition IDs unset.
Ten genuine provider responses preserve the 1,440 earlier records and reviewed
answers. All 600 written-track readbacks preserve payloads, full selected-provider
credits and local-order numbering, with no cold/warm differences. Utada still
abstains with filenames, and its non-ID profile retains the two previously
flagged Passion remaster-title findings. The suite now locks 1,507 inputs.
See [blocker-fix results](../../../../docs/auto-tag-evaluation.md#blocker-fix-replay-2026-11-08)
and `.amp/in/artifacts/reviewed-blocker-replay-4-20261108/verification.json`.
This subset rerun does not clear the other cases' earlier findings.

The [Utada evidence follow-up](../../../../docs/auto-tag-evaluation.md#utada-evidence-follow-up-2026-11-08)
confirms that the reviewed digital release is the 2014 remaster and that the
romanized Hymne subtitle has official catalog support. Nineteen controlled
native scenarios isolate title handling and an existing version-label gap.
They do not change fixtures, reviewed answers, scores or the cold/warm results,
and they do not authenticate the local audio's mastering.

The subsequent [Utada evidence fix and replay](../../../../docs/auto-tag-evaluation.md#utada-evidence-fix-and-replay-2026-11-08)
retains release remaster context, gives valid tag titles precedence over
filenames, and rejects explicit version conflicts even when recording IDs agree.
Three fresh native processes produce six cold/warm records. Both tagged
profiles apply all 18 tracks in each phase; all 72 written readbacks pass strict
title and credit checks, clearing the preceding Passion findings. Filename-only
Hymne still abstains without a provider-supplied romanized subtitle alias.
There are no missing routes, payload changes or cold/warm differences. No
fixtures or reviewed answers changed. Final evidence is in
`.amp/in/artifacts/utada-evidence-cold-warm-2-20261108/verification-partial.json`.
This Utada-only replay does not clear findings on other cases.

The later [alias capture and filename-only replay](../../../../docs/auto-tag-evaluation.md#utada-alias-capture-and-filename-only-replay-2026-11-08)
freezes Apple's English and Japanese titles under the same song and album IDs,
paired official catalog pages, and MusicBrainz's empty recording-alias response.
These are separate review evidence, not fabricated native provider fixtures.
The capture-only replay still abstained cold and warm with zero writes or
missing routes because the provider path did not consume those aliases.

The subsequent [provenance-backed ingestion replay](../../../../docs/auto-tag-evaluation.md#provenance-backed-alias-ingestion-and-utada-replay-2026-11-08)
uses the separate bundled catalog `data/provider-title-aliases.json`, with
hash-checked raw receipts and an exact reviewed MusicBrainz occurrence binding.
Version checks remain mandatory, aliases do not replace canonical display
titles, and no upstream response or reviewed answer changes. One fresh native
process applies filename-only Utada's 18 tracks cold and warm. All 36 written
titles and credits pass the unchanged audit, with zero missing routes, payload
changes, numbering failures or cold/warm differences. Other profiles are not
rerun. Evidence is in
`.amp/in/artifacts/utada-alias-ingested-filename-replay-20261108/verification-partial.json`.
Synthetic audio does not establish original recording or mastering identity.

**The preceding 22-case baseline has complete fixture coverage for 132 native
cold/warm evaluations, with open tagging findings.** Eight capture/replay rounds
added 478 genuine HTTP 200 responses under
`provider-snapshots/reviewed-22-completion-20261108/`, preserving all 962 earlier
records and reviewed answers. The 54 affected case/profile pairs were rerun;
12 already-complete baseline pairs were retained. Applied cases per phase are
14/22 for filenames, 19/22 for tags without IDs, and 19/22 with IDs retained.
There are zero missing routes. All 1,176 native written-track readbacks preserve
audio payloads and local-order numbering, with no cold/warm output differences.

In that baseline, the three expanded Taiwanese Enya cases abstain in every
profile; five other cases abstain only with filenames. &burn retains only Billie Eilish in
both existing-tag profiles: selected Discogs 37353822 omits
Vince Staples, while reviewed Discogs 13898510 credits him as `Guest [With]`.
Strict title checks flag the Gaelic-only Book of Days qualifier and Utada's
filename-derived 2014-remaster wording on Passion, each cold and warm. No answer
or title rule was relaxed. All 478 new routes pass byte-exact HTTP probes;
the suite now locks 1,497 inputs. See the detailed
[second expansion replay](../../../../docs/auto-tag-evaluation.md#second-provider-review-expansion-2026-11-08)
and `.amp/in/artifacts/reviewed-22-fixtures-replay-8-20261108/verification.json`.

The first 2026-11-08 review expansion added nine positives with 34 mapped tracks through
`reviewed-billie-20261108.json`, `reviewed-cjk-artist-20261108.json`, and
`reviewed-cjk-library-20261108.json`, bringing the preceding total to 29 positives
with 277 mapped tracks. Three CJK
positives use explicitly reviewed simplified/traditional artist-name pairs.
Ordinary *bellyache* and its Marian Hill remix are reciprocal version/duration
hard negatives. The *終極一班2* soundtrack remains unresolved because its frozen
tracklist lacks durations and needs occurrence-level version and extra-track review.

Ten raw responses and their request provenance are frozen under
`provider-snapshots/review-expansion-20261108/` and `provider-discovery.json`.
`just eval-auto-tag-suite` checks every review manifest together and rejects
conflicting answers. Original inventories and reviewed answers remain unchanged.
These are metadata-content reviews, not acoustic or source-pressing proof.
The first native replay, before subsequent fixes, added 173 real provider
responses and completed all 54
invocations with zero missing routes. Both phases apply 3/9 folder-only cases,
9/9 existing-tag cases without IDs, and 9/9 ID-assisted cases. Independent basic
tag checks accept 2/9, 9/9, and 8/9 respectively. Folder-only *醜奴兒* retains an
artist prefix in `Intro` and uses an artist spelling outside the reviewed aliases.
ID-assisted *Lo Vas A Olvidar* duplicates its artist credit. Readback and payload
checks pass, with all source dummies unchanged. Exact-edition scoring remains
separate. Subsequent fixes and the full 29-case rerun supersede those historical
failures; all 174 case/profile/cache evaluations passed the full-credit and
local-order audit. See the [review expansion results](../../../../docs/auto-tag-evaluation.md#second-provider-review-expansion-2026-11-08).

Use `just eval-auto-tag-dummies <corpus.json> <new-output-directory>` to create
full-duration silent FLACs from frozen metadata without access to original audio.
Native writes use `WriteQueue`; each output has its own runtime corpus, expectations
and `media/` tree. Originals are not copied. Same-stem rows, conflicting tags,
ordered credits and provider IDs remain separate. Dummy audio cannot prove
recording identity, duplicates or original-codec behavior. See the
[dummy replay instructions](../../../../docs/auto-tag-evaluation.md#dummy-media-without-original-tracks).
