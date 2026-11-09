# Reproduce the reviewed native checkpoint

These are byte-identical copies of the completed 51-case replay driver, expanded
auditor, and independent title rubric. They preserve the checkpoint method, not
the proposed unified provider-and-agent feedback loop. Model assistance is
disabled by the native evaluator. The three input profiles describe available
local metadata, not separate tagging architectures.

Run commands from the repository root. Install the normal repository dependencies,
Python 3, `just`, and FFmpeg first. No original music, provider credentials, or
model credentials are needed. The replay writes only disposable silent FLACs
and uses frozen loopback HTTP responses. It does not test acoustic recognition
or establish local mastering identity.

## Restore the expected directory layout

The archived scripts deliberately retain their original relative paths. Do not
run them in this source directory or in a historical output directory. Each run
needs a fresh directory at exactly `.amp/in/artifacts/<run-name>/`.
Before creating outputs in a fresh clone, add `/.amp/in/` to the repository-local
exclude file reported by `git rev-parse --git-path info/exclude`. Keep existing
exclude rules. Dummy media, executables, caches, and runtime logs stay local.

Check the frozen membership and hashes before creating media:

```sh
just eval-auto-tag-suite
shasum -a 256 -c scripts/auto-tag-eval-replay/SHA256SUMS
```

For a fresh checkout, recreate the four dummy populations from captured metadata:

```sh
set -euo pipefail
fixtures="$PWD/test/fixtures/tauri/auto-tag-eval"
mkdir -p .amp/in/eval-dummies .amp/in/artifacts
for population in original artist-batch-extension curated-hard-cases library-sample; do
    corpus="$fixtures/$population/corpus.json"
    if [ "$population" = original ]; then corpus="$fixtures/corpus.json"; fi
    just eval-auto-tag-dummies "$corpus" "$PWD/.amp/in/eval-dummies/$population"
done
```

Dummy creation rejects existing destination directories. Preserve existing media
and run evidence rather than deleting them to rerun this setup. The four
populations contain 378 cases and 3,506 files. The driver selects only the 51
reviewed positives, totaling 539 files. Regenerated FLAC hashes are locked by
the new run and need not equal historical FFmpeg output hashes.

Copy the driver, auditor, and rubric into their expected locations:

```sh
set -euo pipefail
run="$PWD/.amp/in/artifacts/reviewed-51-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir "$run"
cp scripts/auto-tag-eval-replay/run-native.cjs "$run/run-native.cjs"
cp scripts/auto-tag-eval-replay/audit.py "$run/audit.py"
rubric_dir="$PWD/.amp/in/artifacts/album-content-comparison-20261107"
mkdir -p "$rubric_dir"
if [ -e "$rubric_dir/score-best-aligned.py" ]; then
    cmp scripts/auto-tag-eval-replay/score-best-aligned.py "$rubric_dir/score-best-aligned.py"
else
    cp scripts/auto-tag-eval-replay/score-best-aligned.py "$rubric_dir/score-best-aligned.py"
fi
```

Keep the same shell for the next commands so that `run` remains set.

## Build, replay, and inspect the audit

```sh
cargo test --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --lib live_auto_tag_eval \
    --no-run --message-format=json > "$run/build.jsonl"
node "$run/run-native.cjs" > "$run/replay.log" 2>&1
python3 -B "$run/audit.py" > "$run/audit.log"
```

The driver freezes the executable, source hashes, oracles, dummy hashes, and
provider inputs. It starts 153 fresh native processes: 51 cases across three
profiles. Each process runs cold and warm against separate media and a shared
per-process cache, producing 306 evaluations. Unexpected HTTP routes make the
driver fail. Do not bypass that failure or replace missing responses with empty
results. Capture authentic fixtures and rerun into a new output directory.

The auditor's exit status is not a clean-metadata verdict. Read `verification.json`
and `audit.log`, including `missingRoutes`, `integrityFailures`, `titleFailures`,
`unsupportedCreditEvidence`, `missingReviewedCollaborators`,
`coldWarmOutputDifferences`, `coldWarmClassificationDifferences`, and
`cacheIsolationFailures`. Keep abstentions in the denominator. Do not use the
rubric's legacy `--score` mode, which targets an older population.

The historical full run applied 294 of 306 evaluations, of which 288 had clean
independent metadata checks. Its unchanged machine-readable result summary is
preserved in `baseline-summary.json`, including the original gaps and findings.
Later fixture completion reran only the two
affected filename-only cases. Both still abstained without missing HTTP routes.
These are historical results, not promises for a new executable or a clean pass.
See [the handoff](../../.planning/handoffs/unified-tagging-feedback-loop-20261109.md)
for known findings and the unified feedback-loop priority.
