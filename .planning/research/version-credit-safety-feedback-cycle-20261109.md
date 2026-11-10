# Version and collaborator safety correction

## Scope and fixed expectations

This cycle continues the existing unified provider and assistance coordinator. It changes neither the tagging architecture nor the reviewed safety selection. All work stays in the same checkout on `artist-library-inventory`, above pushed base [d7659077](https://github.com/ydeng11/Soundrobe/commit/d7659077a5030c5ca627d99b6e967343d3597e28).

The immutable before condition is `.amp/in/artifacts/unified-safety-after-20261109/`, documented in [the previous safety cycle](unified-safety-feedback-cycle-20261109.md). Before editing, its 130 source locks and 1,614 input locks matched the checkout. The incoming diff and native source archive are retained in `.amp/in/artifacts/version-credit-cycle-20261109/`. Reviewed expectations, raw provider captures, scripted model inputs, negative visibility controls, and request budgets are not changed.

The set remains 31 unique physical files in seven inspected development cases, evaluated across three profiles with isolated cold and warm media. A complete condition has 72 evaluations and 246 repeated physical-file evaluations. These are not untouched holdouts or independent recordings. Scripted assistance receives zero grounded-assistance accuracy credit.

## Verified causes and bounded changes

The original metadata-assistance validator checked response structure and confidence but did not apply the existing local version guards. Enya's high-confidence scripted ordinary titles erased filename remaster claims after provider matching rejected them. The validator now applies the existing track matcher and recording-version contracts to each index-bound local file before accepting assistance. Conflicting remaster, live, and named-remix claims reject the candidate with `ai_version_conflict`. Ordinary title corrections remain possible.

Existing-tag dont smile at me discovery compared an apostrophized local album hint with apostrophe-free catalog titles. The fuller captured MusicBrainz editions were not requested. The selected Discogs release `37353822` actually omits Vince Staples, and the search fallback has only eight tracks. The fix removes apostrophes for identity comparison and admits that exact identity during catalog ranking. It does not add fixture responses or raise lookup budgets.

The coordinator now compares raw, strongly aligned provider performer credits. A richer compatible list can fill a missing guest without replacing the selected edition. Non-nested credits cause an explicit `provider_credit_conflict` hold. Local credits are not laundered into a supposed provider union. Provider-resolved primary aliases are equivalent identities, not extra collaborators. The existing Chinese-name alias regression remains a control.

Discovery can expose unresolved edition alternatives in the no-ID profile. The provisional content-only change borrows a provider genre when all admitted, strongly aligned genre donors agree. Conflicting admitted genres continue through the existing optional, unverified assistance policy. Explicit per-file genres still take precedence. Normal resolved-edition genre selection is unchanged. This new donation rule is not accepted as safe by the frozen cycle below.

Diagnostics bind donated credits and genres to provider, release, file index, and value. The field auditor now records those receipts instead of attributing every changed field to the selected edition's provider. A receipt that disagrees with readback fails the audit. This is a provenance claim checked against the separate reviewed field oracle, not blanket verification of every edition-dependent field.

## Failed attempts retained

- `version-credit-attempt1-20261109/` executed all 72 evaluations. The original version and collaborator findings disappeared, but the no-ID collaborator path made unsupported extra genre requests. The strict model fixture rejected them. The auditor reported two job-level findings, not a clean result.
- `version-credit-after-20261109/` retains a complete 72-evaluation intermediate attempt with 124 findings. `version-credit-final-20261109/` retains an interrupted intermediate attempt with 69 exported phase records. Broad genre donation bypassed fixed assistance or cancellation scenarios. These are not clean after conditions. Their source archives and binaries describe the executed code. Later source edits also invalidated their checkout locks. The latter run stalled and was stopped without deleting its evidence.
- Red native logs cover the original erased-version and missing-guest defects, lost content-only genre, primary-alias regression, local-credit union, and arbitrary conflicting-genre donation. `ledger-red.log` shows the former incorrect provider attribution. No expectation was broadened to accept these outputs.

## Verification record

The sealed replay driver exits **0** after all 72 evaluations and 246 physical-file evaluations. The separate safety auditor exits **1**, with 23 findings. The strict comparator also exits **1**. This is not a clean cycle and is not ready to ship.

Both original defects disappear in the executed before/after comparison. The four existing-tag Enya evaluations go from sixteen written files each to zero, retaining tags and full file bytes. All six collaborator evaluations retain Vince Staples. The explicit-ID profile retains Discogs edition `37353822`; the no-ID profiles leave edition identity unresolved. Non-nested guest credits, wrong titles, and wrong durations remain negative controls.

The new findings concern `collaborator-disagreement-folder_filename` only: eighteen genre checks across nine files cold and warm, two assisted-provenance checks, two written-review-outcome checks, and one job-level missing-request check. The new content donation writes `Electronic, Pop, Indie Pop` from captured Discogs release `13893573`. The frozen scenario requires a scripted `Pop` fill and its unverified assistance marker. A provider receipt does not override that fixed expectation. No oracle, model response, or visibility control was changed to make this pass.

The baseline has twelve findings across eight evaluations. The after condition has 23 findings across two evaluations plus the job-level request finding. These totals measure different failures and are not an aggregate safety improvement. Of the 72 after evaluations, 48 write files, eighteen hold without writes, and six cancel before writes. Two written evaluations have findings. There are 126 written physical-file evaluations, not 126 unique recordings.

All 1,614 locked inputs and expectations remain identical. All 130 final source locks match the checkout and frozen archive. Payload integrity, native write/readback, physical binding, cache isolation, and cold/warm equality have no findings. Job manifests and request allowances are unchanged. Unrelated tag fields remain identical in the independent comparison. The current, stricter auditor also reproduces the baseline's twelve findings without overwriting its frozen reports.

Provider requests change from 849 to 866. Only the two existing-tag collaborator jobs change exchanges: the no-ID job adds thirteen and removes four requests, while the explicit-ID job adds twelve and removes four. Newly admitted MusicBrainz release details supply the guest credit. `comparison.json` retains every query, source receipt, response status, and hash. Scripted requests change from 48 to 46 because the filename-only collaborator job skips its two genre calls. This difference remains a comparator failure. Grounded-assistance accuracy credit remains zero.

Focused native checks pass 130 tests with one ignored, plus the coordinator integration test's four controls. Six auditor contracts pass, including a receipt mismatch negative. The frozen-suite check passes 378 cases and 3,506 tracks, with 51 reviewed cases. Renderer tests pass 856 across 49 files. Diff whitespace checks pass. These results do not override the red safety verdict.

The broad `just --no-dotenv check` is not green. Overlapping native gates failed budget, acoustic, and subprocess contracts, then stalled. Obsolete runs and the final stalled gate were stopped, with logs retained. Both the preserved before binary and final binary fail the isolated duration/budget control on this host. The before binary also reports `ProcessTimedOut` instead of `OutputTooLarge`; the final binary passes that isolated process control. Sampling a replay child shows FFmpeg at `_dyld_start`, zero CPU, no output file, and a 96K physical footprint after several minutes. A direct `ffmpeg -version` launch likewise stalls before eventually succeeding. The replay process sample is blocked in fixture generation waiting for the child. These observations identify a pre-entry launch stall, not its underlying macOS cause. They do not prove that every broader failure is unrelated to the change. No deadlines or assertions were relaxed, and ignored live or native tests are not claimed as run.

The sealed run uses `.amp/in/artifacts/version-credit-sealed-20261109/`. Its command log is `version-credit-cycle-20261109/replay-sealed.log`, and its separate safety audit is `audit-sealed.log`. The native build receipt is `build-sealed.jsonl`.

Reproduce the fixed replay with the commands in `scripts/auto-tag-eval-replay/README.md`. Run the comparator from the repository root:

```sh
python3 -B .amp/in/artifacts/version-credit-cycle-20261109/compare.py \
    .amp/in/artifacts/unified-safety-after-20261109 \
    .amp/in/artifacts/version-credit-sealed-20261109
```

The comparator requires unchanged locked inputs, budgets, model exchanges, and job manifests. It permits provider-exchange changes only in the collaborator scenario, then exports every added and removed request. It exports failures before returning nonzero, so `comparison.json` distinguishes comparable inputs from a failed safety verdict. Unrelated readback tags must remain identical. Only the disposable metadata-request root is normalized, exactly as in the strict replay service. Raw request hashes remain retained.

## Decision before further tuning

The filename-only provider-genre donation and the frozen assistance control now disagree. Stop here rather than adding a profile-specific route or treating the provider receipt as permission to rewrite the oracle. Independently review that field-authority decision and define the next bounded cycle before tuning again. Preserve the current failed run, original expectations, version checks, credit corrections, and broader gate failures. No clean-safety claim or delivery approval follows from this checkpoint.

## Limits and delivery

Version guards check consistency with local claims. They do not prove acoustic or mastering identity, and a model echoing a version label is not independently grounded proof. This cycle does not evaluate such a response. The filename-only Enya hold also includes discovery-budget exhaustion, so it does not isolate the new version guard.

The existing safety limits remain: artist-concentrated development cases, incomplete duration evidence, scripted HTTP 400 failures rather than transport or retry recovery, and cancellation injected after inference but before writes. No packaged-app navigation, new visual capture, live model, acoustic test corpus, in-flight cancellation, crash recovery, or complete reviewed 51-case live matrix is claimed. Renderer review tests and reopened SQLite evidence cover their own boundaries only.

New changes in this task are confined to `src-tauri/src/commands/auto_tag.rs`, `auto_tag_collections_tests.rs`, `track_matcher.rs`, `scripts/auto-tag-eval-replay/audit-safety.py`, `test_safety_audit.py`, reproduction notes, and the changelog. The plan is `.planning/plans/2026-11-09-version-credit-safety.md`. Earlier uncommitted UI, evaluator, server, fixture-service, and first-cycle work is retained.

No original-library write, live or paid model call, new thread, worktree, commit, push, pull request, or deployment was performed. Frozen evidence, dummy media, failed attempts, the unrelated iOS archive, and `scripts/__pycache__/` remain local. Delivery is uncommitted and unpushed.
