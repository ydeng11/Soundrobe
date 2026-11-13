# Reproduce the reviewed native checkpoint

These are byte-identical copies of the completed 51-case replay driver, expanded
auditor, and independent title rubric. They preserve the checkpoint method, not
the proposed unified provider-and-agent feedback loop. Model assistance is
disabled by default in the native evaluator. The three input profiles describe available
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

## Replay provider matching with scripted assistance

Use the same generated dummies and frozen provider responses to exercise the
existing coordinator's genre assistance. This diagnostic selects Ocean Eyes,
an already reviewed development case. It runs all three profiles cold and warm
with accepted, low-confidence, failed-service, and disabled model responses.
These are 24 evaluations of one physical track, not 24 independent recordings.
The script never loads user credentials or enables live model requests.

```sh
set -euo pipefail
just eval-auto-tag-suite
build="$PWD/.amp/in/assisted-build-$(date -u +%Y%m%dT%H%M%SZ).jsonl"
run="$PWD/.amp/in/artifacts/assisted-$(date -u +%Y%m%dT%H%M%SZ)"
cargo test --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --lib live_auto_tag_eval \
    --no-run --message-format=json > "$build"
node scripts/auto-tag-eval-replay/run-assisted.cjs "$run" "$build"
python3 -B scripts/auto-tag-eval-replay/audit-assisted.py "$run"
```

Keep the output directory new. `snapshot.json` locks source, input, oracle,
script, and executable hashes. Each native process starts with an empty cache.
Warm runs reuse only that process's cold cache and start from fresh media.
`inventory.json` retains model prompts, schemas, responses, request hashes,
provider source hashes, request counts, and unexpected requests, without headers.
The scripted model has two wire attempts per process. Unknown user payloads,
schema names, models, or excess calls fail as missing fixtures. Actual tokens and costs are
unavailable because no model executes.

Native phase records include original and readback fields, selected candidates,
payload checks, and worker evidence reopened from SQLite. The audit checks
physical-file binding, reviewed title and credits, retained recording identity,
genre authorization, and persistent review markers. It exits nonzero for a
non-clean result and writes `audit.json`. Do not treat the native process exit
or `Applied` alone as the verdict.

The genre is scripted inference, not independently grounded genre evidence.
The audit assigns it zero grounded-assistance accuracy credit, even when its
review marker is correct. This procedure tests provider and assistance
cooperation and failure isolation, not tool-using research, acoustic identity,
generalization, packaged UI integration, or restart-safe undo. The review dialog
is session-scoped; the field provenance and review flag persist in WorkerStore.
No reviewed oracle or provider receipt changes for this diagnostic.

## Run the broader offline safety set

Use the same dummy setup above. The fixed `safety-selection.json` contains
31 physical files in seven inspected development cases. Twelve scenarios across
three profiles and cold/warm phases request 72 evaluations and 246 physical-file
evaluations. These are not untouched holdouts or a live-model accuracy sample.

```sh
build="$PWD/.amp/in/safety-build-$(date -u +%Y%m%dT%H%M%SZ).jsonl"
run="$PWD/.amp/in/artifacts/safety-$(date -u +%Y%m%dT%H%M%SZ)"
just eval-auto-tag-suite
python3 -B scripts/auto-tag-eval-replay/test_safety_audit.py
cargo test --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --lib live_auto_tag_eval \
    --no-run --message-format=json > "$build"
node scripts/auto-tag-eval-replay/run-safety.cjs "$run" "$build"
python3 -B scripts/auto-tag-eval-replay/audit-safety.py "$run"
```

The driver rejects existing output directories. Do not edit locked sources or
inputs during a replay. Each run freezes source, executable, physical inputs,
raw provider evidence, request allowances and byte-identical expectation files.
`field-audit.json` retains original, proposed and readback fields, authorization,
source references, and per-file review obligations. Candidate and persisted
worker evidence accompany the ledger. A proposal is not source verification.

The native test-only safety input binds genres by filename on disposable media.
It rejects unknown files and requires explicit offline provider configuration.
Cancellation occurs after genre inference and before the first write, not during
HTTP or a write. Exports distinguish pending review flags from actionable held
worker rows after reopening SQLite. Scripted failure and rejection must not
discard a valid provider match or an explicit per-file genre choice.

Wrong-release discovery wrappers and `safety-negative-queries.json` are deliberate
counterfactual visibility controls. They are not authentic upstream captures.
`safety-model-inputs.json` binds metadata requests exactly, apart from the
disposable evaluator prefix. It supplies terminal metadata rejection for the
wrong-version cases and an adversarial high-confidence response for the
unsupported remaster case. No scripted answer receives grounded-assistance
accuracy credit. Unexpected queries or model payloads remain fixture failures.

The auditor exits **1** for a non-clean result, including missing fixtures,
unsafe writes or incomplete work. Read `audit.json` rather than treating a
successful native process as a safety verdict. The previous cycle's collaborator
omissions and unsupported remaster writes are fixed expectations, not accepted
outputs. Keep their failed baselines. The records are [broader safety
feedback](../../.planning/research/unified-safety-feedback-cycle-20261109.md) and
the [version and credit correction](../../.planning/research/version-credit-safety-feedback-cycle-20261109.md).

The field ledger retains provider donation receipts separately from selected
edition authority. A changed performer or genre attributed to another release
must agree with native readback. Neither a provenance receipt nor an acknowledged
review marker authorizes a contradictory write.

Run broad Cargo gates and matrix replays sequentially. If a local executable
stalls before entry, retain its logs and source snapshot, and reproduce the
launch with `ffmpeg -version` before interpreting native test failures as tagging
defects. Do not raise deadlines, alter fixed expectations, or report an incomplete
matrix as clean. A before-binary control distinguishes a new regression from a
failure that also occurs with the frozen build.

### Provider-first oracle revision

`safety-selection-provider-first-v2.json` records the user's explicit priority:
eligible provider genre before AI inference, while preserving per-file decisions
and cleanup. It preserves every title, performer and version expectation from
v1. The compatible nine-track Discogs 13893573 capture supplies Electronic, Pop,
Indie Pop, including the source-bound &burn guest credit. That supports content
genre, not exact local pressing. The filename-only collaborator control therefore
requires provider provenance and no AI request/marker. Unresolved content still
retains its persisted review obligation. V1 and historical reports are not edited.

Pass the oracle explicitly. A frozen run directory may replace the build receipt
to execute its exact binary and preserve its original source archive and locks.
The new harness has separate locks and an archive, so current scripts are never
misrepresented as the old binary's source. An optional comma-separated job list
runs affected controls with all three profiles and cold/warm isolation; omit it
for the complete 72-evaluation matrix.

```sh
oracle=scripts/auto-tag-eval-replay/safety-selection-provider-first-v2.json
node scripts/auto-tag-eval-replay/run-safety.cjs "$before" "$frozen_before" "$oracle"
node scripts/auto-tag-eval-replay/run-safety.cjs "$after" "$build" "$oracle"
python3 -B scripts/auto-tag-eval-replay/audit-safety.py "$before"
python3 -B scripts/auto-tag-eval-replay/audit-safety.py "$after"
python3 -B scripts/auto-tag-eval-replay/compare-safety.py "$before" "$after"
```

Set `before` and `after` to fresh output directories and `frozen_before` to the
preserved original condition. The before auditor is expected to fail when it
reproduces known unsafe writes. Run each command separately rather than using
`&&` to suppress the after condition. The comparator requires identical oracle,
input locks, budgets, manifests and harness. It exports changed provider/model
exchanges and fields to the after directory's `comparison.json`, then separately
requires a clean after audit. Only the user-authorized filename-only collaborator
genre substitution may remove the corresponding two scripted calls. Version
and collaborator safety expectations remain strict. Raw request hashes remain
in inventories; only disposable media roots are normalized for prompt comparison.

The v2 auditor also checks persisted field receipts, successful source requests
and raw capture hashes. These validate provenance, not acoustic identity or
every edition-dependent field. Scripted assistance still earns zero grounded
accuracy credit. See the provider-first cycle record under `.planning/research/`
for actual results and broad verification limits.

### Explicit provider-absent controls

The complete v2 run exposed a setup error: Guitar Songs has compatible Discogs
genre evidence, so its old scripted assistance controls did not establish
provider absence. `safety-selection-provider-first-v3.json` preserves all v2 field
expectations and marks six jobs `providerGenreAbsent`. Their Discogs responses
remove only genre and style fields, including discovery fields. Every other
response value remains unchanged. These are counterfactual controls, not new
authentic captures. Each modified response retains its original source location
and raw hash. The independent auditor checks the bounded transformation.

Use the v3 oracle for the common before/after safety matrix:

```sh
oracle=scripts/auto-tag-eval-replay/safety-selection-provider-first-v3.json
```

The same replay, audit, and comparison commands above apply. Both conditions
must use that oracle and identical provider controls. The five mixed jobs and
incomplete-duration job test assistance only in the explicit absence of provider
genre. Other jobs retain authentic genre evidence and user-choice controls.
Never add a production profile route or call AI to satisfy an old request count.
The failed v2 conditions and historical oracles remain unchanged. The full record
is [provider-first field authority](../../.planning/research/provider-first-authority-feedback-cycle-20261109.md).

## Export observations without tag authority

Offline native replay with `SOUNDROBE_AUTO_TAG_EVAL_DONOR_TRACE=1` also writes
`evidence-{phase}-{caseId}.json`. This test-only export binds the actual prewrite
lookup request, tag baseline, physical hashes, and separately observed tagged,
filename-derived, and file-count-derived positions. It retains four explicitly
named raw-source guard diagnostics and the existing parsed-candidate route
traces. A diagnostic is not proof that a source reached that production route.

Join one observation to the exact frozen manifest served by the loopback mock:

```sh
python3 -B scripts/auto-tag-eval-replay/export-evidence.py \
    "$run/evidence-cold-$caseId.json" "$provider_manifest" "$run/observations-cold-$caseId.json"
python3 -B scripts/auto-tag-eval-replay/test_evidence_export.py
```

The output path must be new. The exporter reads no media and makes no requests.
It verifies received receipt hashes, identities, queries, diagnostic arguments,
and prewrite/profile bindings. It records raw track and recording credits and
durations, occurrence alternatives, catalog support, qualifier applicability,
excluded packages, unresolved relevant assertions, and non-nested conflicts.
Missing Discogs track principals are not inherited from the album artist.

Every export has `writableDelta: []`, `acceptedCertificate: false`, and an
insufficient overall decision. No tagging code consumes it. Relapse's supported
catalog sequence and direct guest credits do not establish the local explicit
variant. Desperado's filename-only instrumental ambiguity remains unresolved.
Neither source can repair another release's missing duration certificate or
authorize genre, IDs, numbering, or performer writes through this export.

The bounded attempt records cumulative requests, receipts, failures, and consumed
wire calls, including warm-cache source lineage. Current telemetry does not
enumerate all scheduled, cancelled, or still-relevant sources or the separate
local-decision store. These limits remain explicit; process completion is not
exhaustive discovery. Source, policy, or input changes invalidate the observation.
The exporter is a diagnostic within the existing workflow, not a new certificate
framework or tagger. Its passing contracts do not erase historical safety findings
or earn scripted assistance grounded-accuracy credit.

## Run the cold selection with durable mock accounting

`run-bounded.cjs --durable-mock` uses a native test-only broker on Unix. The
application has no broker entry point or live-enable switch. This mode requires
the existing ignored generated inputs under `.amp/in/eval-dummies/`, the frozen
scorer-only label directory, and the matching native test executable. These
dependencies do not provide fresh-clone exact replay or executable attestation.

Build the server-library test executable without loading `.env.local`:

```sh
cargo test --offline --manifest-path src-tauri/Cargo.toml \
    --no-default-features --features server --lib --no-run
```

Use the executable path printed by Cargo, a new output directory, and the frozen
label directory. Never select the live evaluator or use original music as input.

```sh
node scripts/auto-tag-eval-replay/run-bounded.cjs \
    .amp/in/artifacts/NEW_RUN "$test_executable" \
    .planning/plans/crowd-lu-user-acceptance-v1-20261111 --durable-mock
```

The broker owns six cold jobs, 72 observations and 216 requested field slots
from preparation onward. Every preparation finishes and closes its native and
fixture leases before one checkpoint freezes. The driver signs a test-only
approval for that exact frozen scope. Its durable single-use consumption admits
execution on the same retained files with new caches and replacement fixture
sessions. Ordinary matching, assistance eligibility, WriteQueue ownership and
scorer-only user labels remain unchanged.

SQLite transactions serialize reservations, dispatch admission and stop. The
cumulative limits are 512 provider operations, 32 compact-inventory operations,
one conditional scripted model request and 576 operations in total. Arrival and
response settlement are separate facts. A reservation does not authorize a send.
A dispatch or atomic write admitted before stop may finish after stop. No new
admission follows stop, and no rollback or cancellation of socket bytes is promised.

The broker accepts only registered numeric-loopback routes. It never supplies
real credentials, reads account configuration or forwards upstream. Fixtures
stop admission on invalid ingress. If notification fails, they abandon their
persistent lease. Unknown outcomes, invalid handles, duplicate consumption,
storage errors and unexpected lease loss terminate the run without refund or
resumed execution. Existing registries permit read-only incomplete reporting,
not restart. The compiled verifier trusts an RFC 8032 public test fixture, never
a key supplied in a manifest or approval JSON. The signed scope binds the run,
checkpoint, sources, executable, requests, labels, disposable roots, policies and
limits. Expiry and revocation stop new grants without refund or resumed authority.
Test signatures and session handles assume trusted reviewed mock processes. They
are not real provisioned user approval or malicious-host containment.

The broker reconstructs exact approved genre bytes and owns the injected fake
credential supplier. Children receive only authenticated session operations and
bounded parsed results. They receive no model credential, target-selection,
header or general model-send interface. Model sends through the forwarding
gateway are denied. The separate loopback-only model client has no redirects,
proxy, fallback, retry or repair.
Its request, response and per-attempt limits are 32 KiB, 64 KiB and five seconds.
Returned identity, version, usage and pricing claims remain unverified.

General reports, events, inventories and receipts omit raw model replies, headers,
errors and parsed genre values. Accepted parsed values are allowed only in the
bound candidate, disposable media and staging, typed worker-review and scoring
sinks. Rejected values are discarded before IPC. Accepted-answer labels remain
scorer-only. Evidence remains retained locally; no deletion duration is user
consent. Real trust provisioning, credential custody, model/provider choice,
disclosure, retention and billing controls remain unresolved for live use.

`source-and-tool-locks.json`, `frozen-checkpoint.json`, the SQLite registry,
`wire-reconciliation.json` and `authority-receipt.json` bind the run. Final
publication requires a clean broker exit and a complete receipt matching the
checkpoint, membership, sources, preparations, observations and counters. Missing
or terminal receipts retain the full incomplete denominator. Per-job compact
fixture snapshots preserve completed inventory observations. An interrupted
native child can terminate the driver before those snapshots or log settlement.
Missing output is unavailable evidence, never zero arrivals or proof of no leak.
The registry's arrival and bounded response receipts remain distinct witnesses.
The 8-MiB compact-inventory ceiling is unchanged.

Run the offline process and crash controls with a completed run's membership:

```sh
node scripts/auto-tag-eval-replay/check-mock-authority.cjs \
    .amp/in/artifacts/NEW_CONTROLS "$test_executable" \
    .amp/in/artifacts/EXISTING_RUN/requested-membership.json
```

The controls compare actual loopback arrivals with reservations, race separate
children, replace fixture sessions and deny recovery after storage faults or
process crashes. Test-only bootstrap selectors cover reservation commit and
dispatch windows. SQLite process-restart observations do not prove power-loss
durability, physical flushing or platform-wide guarantees. Bare `--durable-mock`
does not install storage tickets or independent whole-run supervision. The
`run-supervised.cjs` entrypoint installs both before bootstrap, with unchanged
300-second preparation and 600-second execution scopes. See
[`docs/offline-provider-mock.md`](../../docs/offline-provider-mock.md) for that
mode. Neither entrypoint provisions real credential custody, live approval,
billing guarantees or a user-approved retention duration. Scripted responses
always earn zero grounded assistance accuracy credit.
