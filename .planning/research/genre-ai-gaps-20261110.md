# Controlled offline genre fallback validation

## Scope and evidence

This record continues the final 51-case content-preservation matrix without
changing its captures, expectations, certificates, or production sources.
The evidence directory is
`.amp/in/artifacts/genre-ai-gaps-20261110/`.
The preceding matrix is
`.amp/in/artifacts/reviewed-51-content-preservation-20261110/`.

The frozen selection covers every one of its 34 written genre-gap case/profile
jobs, comprising 68 cold/warm evaluations. Each job runs six scenarios through
the existing native coordinator, WriteQueue, and metadata reader. These are
controlled offline responses, not a separate tagging implementation or a test
of live-model genre accuracy. Scripted answers receive zero grounded accuracy
credit. No original-library media or real credentials are used.

`freeze.json` records the selection, source-derived expectations, exact request
payloads and responses, input locks, source archive, executable, branch, HEAD,
incoming Git status, and staged-diff hash. The source archive SHA-256 is
`ee38c6c50b9726a2a47e3cc3eaaaf07c1640086c17d2d6de9e77aab0c3ed00a4`.
The executable SHA-256 is
`69452eb3021d8c2f215a8e45e3b416ec0bd47a8d70221d7b95bc033ef5ac6388`.

The original full matrix remains 153 jobs/306 evaluations. This new experiment
is a bounded assistance selection, not a replacement full 51-case matrix.
It schedules 220 jobs/440 evaluations across 23 cases. All 34 gap jobs are
included without exclusions. The gap jobs cover twenty cases: ten filename-only
jobs, eight no-ID jobs, and sixteen recovery jobs. All jobs have separate output
directories and process-local mocks/caches. Warm evaluations reuse only their
own job's cold cache and materialize fresh profile-equivalent media.

## Frozen scenario contracts

| Scenario | Evaluations | Repeated physical files | Expected behavior |
|---|---:|---:|---|
| Accepted genre | 68 | 846 | Fill missing genres; persist inferred/unverified provenance and review |
| Rejected low-confidence genre | 68 | 846 | Keep missing genres; retain supported provider writes |
| Invalid schema/value | 68 | 846 | Keep missing genres; retain supported provider writes |
| Terminal HTTP 400 | 68 | 846 | Keep missing genres; persist failure diagnostic; no retry |
| Assistance disabled | 68 | 846 | No model request; retain provider-only behavior |
| Cancellation after inference | 68 | 846 | No write; preserve profile-baseline bytes and tags |
| Authentic provider-genre controls | 4 | 26 | No genre-model request, including provider disagreement |
| Mixed explicit local genre | 2 | 8 | Preserve explicit file genre; infer only missing values |
| All explicit local genres | 2 | 8 | Preserve every explicit value; no model request |
| Existing holds, disabled | 12 | 102 | No write or additional authority |
| Existing holds, scripted metadata | 12 | 102 | No write, despite ordinary-title/high-confidence responses |

The denominator is 258 distinct case/file inputs and 5,322 repeated file
evaluations if every scheduled job completes. The same physical input appears
under several profiles, scenarios, and phases. These are not 5,322 independent
tracks. The original 34 gap jobs contain 229 distinct inputs and 846 repeated
file observations across their 68 phases.

The actual native donor traces classify 25 gap jobs as having considered
genre-bearing donors that all fail eligibility, and nine as having no considered
genre-bearing candidate. Neither classification proves global source absence.
A fetched genre-bearing release is not automatically an eligible donor.
Independent certificate incompleteness does not decide production eligibility.
Authentic captures are unchanged. The local-genre controls explicitly patch
disposable inputs and are labeled separately from authentic profiles.

Accepted and rejected responses use `GenreFillResponse` with confidence 0.8 and
0.2 respectively. The invalid response has an empty genre and nonnumeric
confidence. Failure returns HTTP 400. The known Bellyache remix and Enya
compilation holds instead exercise the existing `TagCorrectionResponse` route.
Each isolated process permits at most two model requests, one per phase.

## Independent checks and preserved history

`audit.py` compares actual prewrite/readback values by physical file, all
non-genre fields, selected candidate/IDs/authority, provider attempts, nested
preservation receipts, payload hashes, cache state, exact model roles/payloads,
and candidate/provenance/review state after SQLite reopen. It compares supported
writes with the preceding final-source matrix. Cancelled and held evaluations
must instead match their own prewrite profile baseline.

`test-audit.py` supplies negative controls for changed collaborators, lost
persistent uncertainty, changed model request roles, and overwritten explicit
local genres. `supplement.py` independently checks AI status, warnings, and
persistent terminal-failure diagnostics. `exchange-summary.py` separates actual
per-phase increments from cumulative warm receipts. `paired-hashes.py` requires
exact full-file equality when genre is unchanged and bounds changed hash
consequences to expected genre changes with unchanged audio payload. Redacted
hashes remain independently unverified, not counted as equality.

The unchanged preceding field auditor also runs on every new record. Its
assistance-disabled assumptions remain visible as findings. The byte-identical
legacy auditor runs in nine partial views containing actual new records, with
no status rewrites or oracle changes. Its applied-only value branch does not
score assisted `needs_review` writes; the independent audit covers those writes.
Its partial-view counts must not be read as a fresh full 51-case run.

All 84 historical certification findings across twenty cases remain reportable.
The preceding legacy matrix also retains two title, 32 selected-credit,
twenty collaborator, and four numbering disagreements. None of those
expectations is rewritten to fit scripted output. The evidence-only exporter
must remain `insufficient`, with `writableDelta: []` and
`acceptedCertificate: false`, for every new evaluation.

## Uncertainty UI checks

The actual accepted Exile no-ID native record feeds the unchanged
`AutoTagReview` component. `review-data-binding.json` records the source and data
hashes. DOM checks cover four per-file unverified markers, pending and kept
states, retained assisted provenance, a disabled Keep button after acknowledgment,
and absence of horizontal document overflow at 1920×1080 and 390×844.

The screenshots show the warning that keeping changes acknowledges review,
not verification. Expanded diagnostics push file-level rows below the initial
viewport. With diagnostics collapsed, inspected pending/kept captures show
the per-file `AI-inferred · unverified` labels. Mobile ID columns remain crowded.
This task changes no UI appearance or unrelated layout.

Keep uses a component-only API stub. These checks do not establish packaged
native acknowledgment integration. Native pending provenance and review
persistence are separately checked after SQLite reopen. Existing review/undo
remains session-scoped.

## Reproducibility and limits

The retained driver is `run.cjs`; the frozen selection and exact model scripts
are in `freeze.json`. `analysis-tool-locks.json`, `final-tool-locks.json`,
`exchange-tool-lock.json`, and `paired-hashes-tool-lock-v2.json` bind the tools.
The earlier hash-classification version and its lock remain retained rather
than overwritten. Postflight streams one inventory at a time and verifies
source/input/tool/history locks, archive contents, unchanged Git/staging state,
actual provider exchanges, exports, and gate exits.

The pilot startup failure and initial checker failure remain retained. The
startup correction changes executable permission, not binary bytes. The checker
correction reconstructs the source-defined top-level metadata temperature
spelling `0.0` when verifying raw request hashes; parsed JSON records store `0`.
It does not alter prompts, expected results, production policy, or native output.

These tests use silent generated media. They establish behavior under exact
offline exchanges, not acoustic identity, genre truth, live-model quality, or
general performer applicability. Cleanup and duplicate/multi-disc boundaries
use fresh existing native controls rather than newly combined AI-enabled
fixtures. Changed full-file hashes with expected readback and unchanged audio
do not independently prove preservation of every unexported tag. Redacted
payload exports remain a visibility limit.

## Actual native coverage and values

All 220 jobs/440 evaluations completed, with zero process failures or fixture
misses. Actual denominators match the selection: 258 distinct physical input
paths and 5,322 repeated file observations. The six profile/phase counts are
65 filename-only, 55 no-ID, and 100 recovery evaluations in each phase.
`case-breakdown.md` contains all 44 case/profile/kind groups;
`case-profile-phase.csv` contains every evaluation and its findings.

There are 348 written evaluations and 4,272 repeated written files. These
include 278 `applied` evaluations and seventy written `needs_review`
evaluations. Another 24 evaluations remain held without writes. The 68
cancelled evaluations return the expected resolver cancellation error without
writes; they are completed driver tests, not successful tagging runs.

All 34 authentic gap jobs exercise all six scenarios cold and warm. Accepted
responses fill 846 missing per-file genres across 68 evaluations, all with
persisted inferred/unverified provenance and review. The mixed-local control
adds six inferred file values and retains two explicit values. Together with
the all-local control and authentic provider controls, the complete ledger is:

| Actual genre provenance | Repeated file observations |
|---|---:|
| AI-inferred, unverified | 852 |
| Provider-sourced, not newly certified | 26 |
| Retained explicit local value | 10 |
| Still missing | 4,434 |

The 272 rejected/invalid/failed/disabled gap evaluations retain useful
provider-supported writes but leave 3,384 repeated file genres missing.
Cancellation accounts for another 846 missing values, and held releases for
204. No result is described as all tracks truthfully tagged.

## Retained routing failures and actual exchanges

The frozen audit is **not clean**. It exits one with exactly two `model_count`
findings, cold and warm for
`hold-accepted-folder_filename-f7756354af37`. The frozen response template
expected two metadata-model requests per job, but the actual count is zero.
Both native phases and the preceding baseline report
`discovery_budget_limited`, with an incomplete Discogs attempt stating
`release detail limit reached`, no candidate/authority, zero writes, and
unchanged prewrite/readback media. The existing
`providers_confirmed_no_match` gate prevents metadata fallback after incomplete
discovery. This is a source-verified guarded skip, not a production regression
or permission to increase the budget.

That one held-control script was not exercised. The other five scripted held
jobs make ten metadata requests and reject the high-confidence ordinary-title
answers with `ai_version_conflict`. Across all held phases, ten report
`ai_not_configured`, ten `ai_version_conflict`, and four
`discovery_budget_limited`. All 24 retain their no-write/review obligations.
There are no excluded genre-gap jobs, but metadata-response coverage of the
held controls is explicitly partial.

The original `audit.py`, its failed output, frozen selection, and original
strict exchange checker remain unchanged. `exchange-summary-observed.py`
reports actual increments and the still-failed frozen expectation separately.
Its successful execution is not a claim that the frozen routing audit passed.

Actual exchanges comprise 10,529 ordered provider requests and 352 model
requests, with zero fixture misses. Every provider request record equals the
corresponding preceding-matrix record. Model roles are 342
`GenreFillResponse` requests and ten `TagCorrectionResponse` requests. There
are 284 HTTP 200 responses and 68 terminal HTTP 400 responses, without retries.
Cold/warm incremental receipts reconcile exactly with each cumulative final
inventory. No phase exceeds one model request or two per process.

There are 176 exercised script instances. Of 220 frozen response templates,
forty are not registered because assistance is disabled. Of the 180 registered
scripts, four are unused: two authentic provider-genre controls, the all-local
control, and the guarded Enya filename-only hold. Responses remain entirely
offline and receive zero grounded accuracy credit.

## Field, provenance, and checker verdicts

Apart from the two frozen routing-count findings, the primary audit reports no
unexpected per-file, mapping, payload, non-genre, selected-ID, authority,
candidate, nested receipt, cache, or persistent review/provenance differences.
The additional source-contract audit has zero findings. All four primary
negative controls pass.

The stricter construction comparison covers all 348 written evaluations with
zero unexplained differences. It compares selected mappings, progress,
kept-tag decisions, all non-configuration diagnostics, provider report events,
and persisted AI/review reasons. It recomputes the Rust source-defined
configuration fingerprint instead of exempting it. The previous matrix's
bounded runtime comparator handles only fully bound disposable-file
timestamps/inodes, retaining PCM, source-hash, and native/persisted equality
requirements. Eight additional negative controls detect changed decisions,
diagnostics, mappings, events, fingerprints, reasons, source hashes, and
native/persisted identities.

The initial construction checks fail because they omitted source-defined
optional AI fields/model-provider settings, expected a generic review event on
the early content return, and compared newly allocated dummy-file identities
as metadata. The red outputs, both original helper versions, and lock hashes
remain retained. The corrected checker uses the existing runtime comparator
and actual source branch contracts; no production behavior or independent
numbering/credit oracle changes.

The paired hash audit checks all 5,322 file observations. It finds 3,394 exact
unchanged-output byte matches, 1,046 exact no-write byte matches, 858 bounded
genre hash consequences, 24 independently redacted hashes, and zero unexpected
hash changes. Redacted values are not counted as equality. This does not prove
every unexported tag independently.

The unchanged field auditor retains 360 assistance-boundary findings, seventy
missing-genre-receipt findings, seventy unsupported-genre findings, and two
source-eligibility findings in this repeated selection. These are actual
findings under its assistance-disabled/source-certificate contract, not erased
as scripted genres appear. They are not 502 independent tracks or replacements
for the historical 84 certification findings.

Fresh `env RUST_TEST_THREADS=4 just --no-dotenv check` passes, rather than being
reused. It reports 856 renderer tests, 1,264 desktop Rust tests with 29 ignored,
730 server tests with seventeen ignored, fourteen audio integration tests with
twelve ignored, and five inventory CLI tests. Script suites pass nine and ten.
Ignored credentialed/original-media tests remain unrun. Overlapping targets are
not independent coverage. Fresh focused checks pass twelve genre, nine
content, ten preservation, sixteen UI/mock, and 26 exporter/safety tests.

## Export, legacy audit, and final bindings

All 440 evidence-only exports remain `insufficient`, with `writableDelta: []`,
`acceptedCertificate: false`, and zero grounded-assistance accuracy credit.
The export checker verifies 2,200 compiled-policy bindings. No script response
creates a performer certificate or additional write authority.

The nine unchanged legacy partial views contain all 440 actual records. Their
applied-only value branch scores 278 evaluations and reports 42 selected-credit
findings, ten collaborator findings, eight numbering findings, and zero title
findings. Integrity, cache isolation, cold/warm differences, and missing routes
are zero. These repeated-selection counts do not replace the historical full
matrix's two title, 32 selected-credit, twenty collaborator, four numbering,
or 84 source-certification findings.

The original strict postflight exits one on the same frozen routing discrepancy
as the primary and exchange audits. The separate integrity-only postflight
passes with `bindings_pass_with_retained_routing_failures` and explicitly
retains `frozenAuditPassed: false`. It verifies 3,691 source locks, 2,233 input
locks, 11,751 incoming/historical locks, all 3,361 preceding-matrix files,
220 inventories, the source archive and executable, tool hashes, branch/HEAD,
staging, and unrelated work. It streams one inventory at a time. This qualified
integrity result does not turn any failed audit green.

The retained execution records include `replay.exit`, `audit.exit`,
`exchange-summary.exit`, `exchange-summary-observed.exit`, `legacy-views.exit`,
`export-all.exit`, `postflight.exit`, `postflight-bindings.exit`, and
`check.exit`. The original failures and corrected checker versions remain
available. `seal-with-findings.py` seals closed evidence with the retained
routing failures explicit. The active preview log is excluded from that seal.

## Delivery and remaining limits

This validation phase changes no production, test, or maintained-tool source.
Its new paths are `.planning/plans/genre-ai-gaps-20261110.md`, this report, and
the local `.amp/in/artifacts/genre-ai-gaps-20261110/` evidence tree. Incoming
uncommitted work remains intact on `artist-library-inventory`. Nothing was
committed, pushed, deployed, or written to original-library media.

The experiment completes the bounded offline genre-routing coverage, not an
overall clean tagging-safety verdict. The two frozen routing findings, existing
certification gaps, legacy disagreements, and missing genres remain reportable.
Packaged acknowledgment integration, live-model accuracy, global provider-source
completeness, redacted independent hashes, and every unexported tag remain
outside the demonstrated evidence. The uncertainty UI has native-data binding
and component-stub acknowledgment evidence, not packaged acknowledgment proof.
Nothing needed from you right now.
