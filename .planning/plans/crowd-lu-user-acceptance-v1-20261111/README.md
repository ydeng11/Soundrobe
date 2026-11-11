# Crowd Lu user-acceptance freeze v1

This packet freezes exactly three alternative genre strings for all eleven Crowd Lu occurrences and the album-level answer: `Pop`, `Mandopop`, and `Pop, Mandopop`. It measures agreement with the user's acceptance, **not independently established musical/acoustic truth**. It authorizes no execution or writes.

## Authority and preservation

The user's clarification is: “Accept Pop or Mandopop individually, and also accept the combined value Pop, Mandopop.” [Annotation artifact revision 63](https://ampcode.com/artifacts/crowd-lu-genre-annotations-8054842d-554b-48e3-94e1-766e1a36dbe2) records that confirmation for every row and the common album-level answer. `annotations-rev63.md` preserves the artifact-tool Markdown, including original notes, the user's stated `both` basis, typos and unfilled reviewer/date and vocabulary placeholders. The snapshot adds only a terminal newline; it is not the underlying rich-document serialization or an authenticated user signature. The editable artifact remains untouched. Later edits require a new version, not replacement of this freeze. The checksum seal detects drift against retained hashes; it is not authenticated approval or filesystem write-protection.

The explicit confirmation controls the scoring alternatives; older “Pop or Mandopop” row wording and template instructions remain historical context. No extra spelling, case, spacing, order, delimiter or synonym acceptance is inferred. In particular, `Mandopop, Pop`, `Pop,Mandopop`, `pop` and a value with trailing whitespace do not match. Each combined value is one string, not a comma-split set. JSON escaping that decodes to the same string is not a new spelling rule.

The user's `both` entries and artist-based rationale are preserved as statements, not independently verified expertise or listening. Intro explicitly uses an album fallback. The reviewer/date and vocabulary placeholders do not override the later exact acceptance confirmation and have not been completed on the user's behalf. There are no untouched holdouts.

## Bound scope

`labels.json`, version `crowd-lu-user-acceptance-v1`, is a scoring-only sidecar to the unchanged historical sample. It binds case `036e0a00e45b`, `tagged_recovery`, eleven unique relative file paths in original order, full titles, captured durations, inventory/sample/provider selectors, captured media-hash claims and recording-ID claims. It retains the 62.170-second Intro and full final movie-theme `[Acoustic Ver.]` qualifier. These claims do not authenticate original audio. Original genre missingness remains unobserved; historical null genre observations concern generated dummy media only.

Historical packet files, expectations, annotations, seals and raw logs remain unchanged. The old sample still correctly records the labels available at its own freeze time. This sidecar does not merge new labels into it, rescore Attempt7, change Billie/Enya controls, add a holdout or grant catalog-source eligibility. Eleven unique occurrence labels and one common answer do not become twelve independently judged audio examples. They are not independent AI outcomes or proof of improvement.

## Checks and scoring separation

Run from the repository root:

```sh
python3 -B .planning/plans/crowd-lu-user-acceptance-v1-20261111/freeze.py
python3 -B .planning/plans/crowd-lu-user-acceptance-v1-20261111/test_freeze.py
shasum -a 256 -c .planning/plans/crowd-lu-user-acceptance-v1-20261111/SHA256SUMS
```

`freeze.py --derive` reproduces the JSON from metadata and the preserved annotations. It never opens media or starts the app. The tests cover exact three-value membership on every row and the album, mutation rejection, and a synthetic equality probe through the existing scorer. That scorer's legacy `catalogValueAgreement`/`correct_catalog_fill` names do not change this packet's user-acceptance basis. Its native projection does not ingest these genre labels today; no integration is claimed or changed. Synthetic readbacks are test data, not new tagging results. Grounded assistance accuracy credit stays zero.

The genre prompt source currently uses candidate artist, album and track titles plus generic instructions, not this answer sheet. A source-only check verifies those payload keys and the absence of expected-answer inputs/labels in that prompt construction; source hashes bind the inspected version. This packet remains scorer-only. No request has been constructed or dispatched. No runtime contamination/admission guarantee is claimed: exact future serialized requests, builds and dataflow still need independent review before any separately authorized execution. Labels, notes and taxonomy hints must never be injected as expected answers.

## Limits after this freeze

User-acceptance scoring labels are now available; qualified acoustic truth, source eligibility, native live enforcement, authenticated execution approval, privacy/cost bounds, actual prewrite/readback integrity and paired disabled-versus-assistance evidence remain separate requirements. Provider-first behavior, stronger evidence, version guards, collaborators and user decisions remain protected. This freeze changes none of them and establishes no AI improvement.

Only offline metadata review and scorer probes are in scope. No model/provider/tagging calls, original-library access, media writes, eligibility/runtime changes, credentials, commits, pushes, deployments, new threads or worktrees are authorized or performed by the freeze tools.
