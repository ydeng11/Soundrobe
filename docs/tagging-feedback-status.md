# Tagging feedback checkpoint, 10 November 2026

Provider-first genre completion, guarded metadata assistance, and protected
physical-file numbering and collaborator presentation are implemented. The
evidence-only exporter grants no write authority. Bounded routing and preservation
checks pass, but the overall tagging-safety verdict is not clean.

This record supersedes delivery-state statements in the historical reports.
Their source snapshots, failed checks, and precommit Git-state receipts remain
unchanged. The branch is `artist-library-inventory`, above pushed checkpoint
`d7659077a5030c5ca627d99b6e967343d3597e28`. Delivery is by local commits only.
No push, PR, release, deployment, live model call, or original-media write is
part of this checkpoint.

## Behavior and evidence boundaries

The existing coordinator uses eligible provider genre before optional genre
inference. It retains explicit per-file genres and cleanup. Accepted inference
has field provenance and remains unverified after acknowledgment. Review flags
and evidence persist after reopening the worker store. Content-only writes still
suppress edition and recording IDs.

AI metadata cannot erase meaningful live, remix, or remaster evidence.
Incompatible provider/local collaborators stop before writes or authorizing
fallback. Coherent captured credits retain spelling, order, and compound list
entries. Captured physical-file track/disc positions and totals survive enrichment.
Missing values keep the existing fallback behavior. Captured metadata is not an
automatically verified user lock.

The Lo Vas A Olvidar regression exercised the actual composite prewrite
`Billie Eilish | ROSALÍA` representation through resolver, queued writes, and
readback. Completion had restored the credits, but later album-content
construction replaced them with album-artist presentation. The construction
condition now counts performer identities rather than list entries. All 28
nested preservation receipts are fulfilled in the final full matrix.

No performer certificate was introduced. Exported catalog support and raw
guard diagnostics remain observations, not authorization. All exports retain
`writableDelta: []`, `acceptedCertificate: false`, and an insufficient decision.

## Executed results and unresolved findings

| Evidence | Native execution coverage | Independent verdict |
| --- | --- | --- |
| Final content-preservation matrix | 153 jobs, 306 evaluations, 539 unique physical paths, 3,234 repeated file observations | 294 written, twelve holds, all 28 preservation receipts fulfilled; 84 source-certification findings remain |
| Offline genre gaps | 220 jobs, 440 evaluations, 258 unique physical paths, 5,322 repeated file observations | 348 written, 24 held, 68 no-write cancellations; original routing contract has two Enya expectation failures |
| Enya v2 correction | Reanalysis of the same 440 outputs; zero new native evaluations or requests | Corrected primary, strict exchange, and postflight checks pass; original failed verdicts remain immutable |

The full matrix consumed 5,068 provider requests and zero model requests.
Assistance was disabled. It still has 68 written genre-incomplete evaluations.
The unchanged legacy audit reports two title, 32 selected-credit, twenty
collaborator, and four numbering disagreements. Some disagreements reflect the
authorized preservation policy, but the old oracles were not rewritten. The
source-certification findings are evidence gaps, not 84 proven bad writes.

The genre experiment covers all 34 identified gap case/profile jobs across
accepted, rejected, invalid, terminal failure, disabled, and cancellation
scenarios, plus provider-present, local-choice, and held-release controls.
It uses authentic provider captures without manufacturing absence. A fetched
genre donor is not automatically eligible. Its 352 scripted model requests
include 342 genre and ten metadata calls, alongside 10,529 provider requests.
No fixture misses or extra retries occur. Scripted answers receive zero grounded
accuracy credit.

The resulting genre observations are 852 AI-inferred/unverified, 26 provider,
ten explicit local, and 4,434 missing. Legitimate provider-supported writes are
useful, but these counts do not mean all tracks are tagged. The disabled-only
field auditor still reports 360 assistance-boundary, seventy missing-receipt,
seventy unsupported-genre, and two source-eligibility findings. Legacy results
remain 42 selected-credit, ten collaborator, eight numbering, and zero title
findings across 278 applied rows. Original 84-finding certification records remain.

Only the Enya `hold-accepted-folder_filename-f7756354af37` expected request count
changes under the separately authorized v2 contract. An incomplete forty-detail
Discogs search is not confirmed no-match. `providers_confirmed_no_match` therefore
prevents fallback. Zero metadata calls and zero writes are source-defined, not
an expectation fitted to the observed output. Its original registered script
and two-request allowance are unchanged. Five complete-discovery no-match
controls still reach assistance. The original `frozenAuditPassed: false` remains.

## Verification and reproduction

The [maintained replay instructions](../scripts/auto-tag-eval-replay/README.md)
cover fresh dummy generation, native 51-case replay, and the first 24- and
72-evaluation controlled assistance cycles. The
[checkpoint notes](../scripts/auto-tag-eval-replay/checkpoints/README.md) identify
portable tests, exact small checker copies, versioned contracts, local data
dependencies, reanalysis commands, and expected non-clean findings.

The passing broad gate is reused, not rerun during commit preparation. Its
runtime source and executed tool bytes match the frozen final source. The
delivery-only documentation correction in `docs/auto-tag-evaluation.md` does not
change executable behavior. The prior command was
`env RUST_TEST_THREADS=4 just --no-dotenv check`. It passed typechecking,
856 renderer tests, 1,264 desktop tests with 29 ignored, 730 server tests with
seventeen ignored, fourteen audio tests with twelve ignored, five CLI tests,
nine mock-provider tests, and ten review/evaluation tests. Ignored tests are
not counted as passes. Strict Clippy, credentialed smokes, and packaged UI
integration are not newly verified.

Commit preparation freshly passes ten automatic-preservation, nine content-filter,
and nine genre-filter test executions. These filters overlap and are not
independent populations. Sixteen renderer/mock tests, eighteen exporter tests,
eight safety-auditor tests, four portable field contracts, five portable
preservation contracts, four original negative controls, and six routing controls
also pass. Fixture locks, Rust formatting, Python/Node/JSON syntax, archived
driver hashes, and whitespace checks pass.

Fresh read-only reanalysis reproduces the original two Enya findings and both
original exit-1 checks. Under v2 it reproduces all five frozen JSON results
exactly: primary audit, strict exchanges, construction comparison, paired hashes,
and supplemental audit. No native evaluation, provider request, or model request
is added by that reanalysis. The delivery log directory is
`.amp/in/delivery-20261110/`. It is not part of an old seal. The first integrity
attempt correctly rejected the deliberate documentation update; a separately
hash-bound documentation allowance retains that failed log without changing
the old source locks. No runtime-source allowance was added.

The frozen final native source archive SHA-256 is
`ee38c6c50b9726a2a47e3cc3eaaaf07c1640086c17d2d6de9e77aab0c3ed00a4`.
Its executable SHA-256 is
`69452eb3021d8c2f215a8e45e3b416ec0bd47a8d70221d7b95bc033ef5ac6388`.
The sealed local matrix and genre evidence are deliberately not committed.
Their exact restoration is not available from a fresh clone. Source locks,
large phase records, private/local input bindings, binaries, and caches remain
local. A recompiled binary or regenerated FFmpeg file needs new locks, not an
old historical hash.

Twenty-four independent genre-experiment payload hashes remain redacted.
Reader-exported fields do not establish preservation of every unexported tag.
Native persistence and visible uncertainty were inspected, but Keep used a
component stub with native review data. That is not packaged acknowledgment
or restart-safe undo verification. No live-agent semantic-quality claim is made.

## Local commit groups

1. `acd501e59060e6434017dc7364241dc37ce3b865` contains the coordinated behavior,
   native observation support, review UI, strict mock, regressions, and Unreleased
   entries. Keeping the overlapping coordinator changes together avoids an
   artificial intermediate implementation.
2. `e2a9c126a1269f9ac4286b7972dd44424c918927` contains maintained assistance replay,
   versioned provider-first controls, independent audits, comparison, exporter,
   checker tests, and replay instructions.
3. The documentation/checkpoint commit contains this current record, corrected
   operator documentation, original decision reports and proposal, exact small
   checker copies, portable original selection, and separate Enya correction
   provenance. Its own ID is available from `git log` after creation.

Only local commits are authorized. The unrelated
`ios-native-probe-0d104d95-source.tgz` and `scripts/__pycache__/` stay untracked
and untouched. Generated media, binaries, caches, raw evidence, and private
configuration stay outside the commits. No clean-worktree claim is required.

## Historical decision and review records

These reports preserve their original dates, failures, limits, and delivery
statements. They are records of those executions, not current Git status:

- [First unified genre cycle](../.planning/research/unified-genre-feedback-cycle-20261109.md)
- [Broader safety cycle](../.planning/research/unified-safety-feedback-cycle-20261109.md)
- [Version and credit guards](../.planning/research/version-credit-safety-feedback-cycle-20261109.md)
- [Provider-first authority and v3 counterfactual controls](../.planning/research/provider-first-authority-feedback-cycle-20261109.md)
- [Full provider-first matrix](../.planning/research/reviewed-51-provider-first-20261109.md)
- [Donor-eligibility diagnostics](../.planning/research/donor-eligibility-genre-gaps-20261109.md)
- [Independent Desperado and Relapse review](../.planning/research/desperado-relapse-independent-review-20261109.md)
- [Unimplemented field-scoped certificate proposal](../.planning/design/field-scoped-performer-certificate-20261109.md)
- [Non-authorizing exporter](../.planning/research/performer-evidence-export-20261109.md)
- [Numbering and collaborator preservation matrix](../.planning/research/reviewed-51-preservation-20261110.md)
- [Composite-credit construction fix and final full matrix](../.planning/research/content-credit-preservation-20261110.md)
- [Original offline genre experiment](../.planning/research/genre-ai-gaps-20261110.md)
- [Separately authorized Enya routing correction](../.planning/research/genre-ai-routing-contract-v2-20261110.md)

## Remaining work

Source applicability and certification policy remain unresolved for the existing
84 findings. Desperado's instrumental ambiguity and Relapse's content/edition
distinction need evidence review before any new certificate authority. Bellyache
and Book of Days holds remain. Further genre coverage must distinguish absent
eligible evidence from disagreement and incomplete discovery.

Live-agent quality, independent acoustic/mastering identity, complete source
discovery, packaged acknowledgment, and restart-safe undo are not established
by these offline scripts. Preserve the fixed oracles and failed baselines for
the next bounded feedback cycle. No user action is needed for this local delivery.
