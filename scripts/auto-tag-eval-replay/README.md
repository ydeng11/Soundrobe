# Offline native auto-tag replay

`run-paired.cjs` compares two frozen native implementations on a small reviewed
selection. The older 51-case replay driver, expanded auditor, and independent
title rubric remain byte-identical checkpoint copies. Neither replay evaluates
the complete unified provider-and-agent workflow. Model assistance stays disabled.
The three input profiles describe available local metadata, not separate tagging
architectures.

Run commands from the repository root. Install the normal repository dependencies,
Python 3, `just`, and FFmpeg first. No original music, provider credentials, or
model credentials are needed. The replay writes only disposable silent FLACs
and uses frozen loopback HTTP responses. It does not test acoustic recognition
or establish local mastering identity.

## Compare a baseline and candidate offline

Use this procedure in the existing checkout. The default `paired-selection.json`
selects ordinary Bellyache and its Marian Hill remix from the maintained review.
Both are inspected development cases, not blind holdouts. Each implementation
runs two cases under `folder_filename`, `assisted_without_ids`, and
`tagged_recovery`, each cold and warm. That is twelve evaluations per implementation,
twenty-four in a pair. `assisted_without_ids` uses existing tags, not a model.

The local dependencies are Node with built-in `fetch`, Python 3, FFmpeg, the Rust
toolchain with cached Cargo dependencies, the retained provider fixtures, and
`.amp/in/eval-dummies/original/{corpus.json,media/}`. The CLI does not install or
download dependencies. Missing dependencies stop execution and remain in the
report. If Python itself is unavailable, stderr names the missing auditor and
`comparison-inputs.json` retains the requested membership instead of a scored report.
For absent dummy populations, use the existing dummy-generation procedure below,
never the original music library. Do not replace existing evidence to rerun setup.

Run from the repository root and choose new output-directory names. Freeze the
baseline before editing native source:

```sh
node scripts/auto-tag-eval-replay/run-paired.cjs freeze .amp/in/artifacts/NEW_BASELINE
```

After implementing and testing a candidate, freeze its executable too:

```sh
node scripts/auto-tag-eval-replay/run-paired.cjs freeze .amp/in/artifacts/NEW_CANDIDATE
```

Run the paired comparison with one command:

```sh
node scripts/auto-tag-eval-replay/run-paired.cjs compare \
    .amp/in/artifacts/NEW_PAIR \
    .amp/in/artifacts/NEW_BASELINE .amp/in/artifacts/NEW_CANDIDATE
```

An optional final argument selects another reviewed selection JSON with the same
schema. A case needs maintained scored membership, a verified match in its named
review manifest, local dummy bytes, and unambiguous provider credits. Unsupported
or missing setup remains unavailable in the full requested denominator. Expanding
the selection does not independently review new truth.

For a small multi-track/version safety expansion, pass
`scripts/auto-tag-eval-replay/paired-safety-selection.json` as that final argument.
It keeps the two Bellyache controls and adds Guitar Songs (two tracks), Storms In
Africa (Remix and LP Version siblings), and Paint The Sky With Stars (sixteen
tracks with unsupported filename remaster claims). This requests thirty
evaluations per implementation, sixty paired evaluations and 264 physical-file
slots. It uses the same dummy population and reviewed snapshots, not new truth.

```sh
node scripts/auto-tag-eval-replay/run-paired.cjs compare \
    .amp/in/artifacts/NEW_SAFETY_PAIR \
    .amp/in/artifacts/NEW_BASELINE .amp/in/artifacts/NEW_CANDIDATE \
    scripts/auto-tag-eval-replay/paired-safety-selection.json
```

The remaster case declares `"requireNoWrite": true`: compatible track content does
not establish its filename mastering claims. Any write is a wrong match and a
safety failure, even if independent content readback agrees. A completed unchanged
abstention passes this control but earns no recovery. Missing, skipped or failed
work cannot pass it. The report counts required and passed no-write controls
separately; its six controls per implementation remain in the full denominator.

`freeze` records the Git revision, dirty-source flag, source hashes, source archive,
diff, executable hash, and tool versions. `compare` copies both executables and
locks the shared selection, suite inputs, expectations, provider bodies, dummy
bytes, and comparison scripts. It rejects changed provider-client or album-discovery
sources between builds rather than silently comparing their different allowances.
The two arms have identical 600-second native folder limits, 1,300-second outer
process limits, and zero model requests. Configuration comes from native offline
defaults, with no inherited credentials or `.env.local` loading.

Every implementation, case, and profile has a new loopback service and a new native
cache. Warm reuses only its own cold cache and receives fresh disposable media.
Native request inventories, process exits, phase exports, and source-media hashes
remain beside the report. A timeout or missing fixture is incomplete work, not an
abstention or recovery. Never resume into an existing output directory.

Read `comparison.md` for the compact report and `comparison.json` for per-evaluation
findings. Both retain improvements, regressions, correct recoveries, wrong matches,
abstentions, missing fixtures, skips, and process failures. Counts of missing fixtures
and failed processes describe affected evaluation slots, not distinct requests.
Independent title, performer-credit, album, and physical-file checks determine
content recovery. Native readback, numbering, payload preservation, no-write
invariance, and cache isolation are separate checks. Reviewed representative and
layout counts do not prove acoustic or exact pressing identity.

Both written and no-write results require complete physical-file membership in
the before/after track lists and all three phase hash exports. Omitted, duplicated,
or substituted files cannot pass a no-write control. Missing or malformed ancillary
exports make execution unavailable, but the auditor still emits both reports with
every requested slot. An observed wrong write remains a wrong match even when its
process fails or fixtures are missing. Incomplete execution earns no recovery credit.
Null native readback remains unavailable without discarding known prohibited
writes. A partial-write error without native completion still grades readable
files whose full-file hashes changed; unchanged bad input tags are not new wrong
matches. Ancillary checks validate miss counts, request arrays, execution status,
and complete source-file sizes and SHA-256 values, not merely JSON syntax.
Missing, misbound, or incomplete readback is unavailable evidence, not itself an
observed wrong match. Only observed content/identity mismatches or established
prohibited writes count as wrong matches. Changed-file readback also rejects
reviewed hard-negative release IDs when native completion is missing. The optional
execution `skipped` status must be Boolean; invalid values retain incomplete reports.

The command exits zero only for a complete comparison with no new regressions,
wrong candidate matches, or candidate readback or safety failures. Abstentions
remain visible and do not by themselves fail the gate. These are local consistency
checks of trusted frozen builds, not executable attestation or a hostile-code
sandbox. Do not rerun the auditor into historical output directories.

`npm run test:review-eval` includes the paired-report and missing-dependency CLI
contracts. It does not build or run the native matrix. The older checkpoint
reproduction instructions below remain available for the larger historical suite.

## Check the disabled hosted HTTP transport offline

The test-only broker has a dormant sender for the native-prepared OpenRouter
request. Hosted HTTPS admission returns `live_dispatch_disabled` before a
reservation or credential lookup, including with public fixture approval.
Maintained bootstrap uses injection by default. A source-locked public hosted
fixture plan may add `"offlineHttp": true` to select a native-created, one-request
numeric-loopback fixture instead. False/nonboolean or context-free markers are
rejected. No caller-supplied endpoint is accepted. No launcher option, plan flag,
environment setting or application command enables external dispatch.

Run the component controls with generated public fake credentials:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
    --features server --lib http_ -- --test-threads=1
node --test scripts/auto-tag-eval-replay/test_model_preflight.cjs \
    scripts/auto-tag-eval-replay/test_supervision.cjs
```

The native control rewrites only the destination to numeric loopback HTTP.
It checks exact request bytes and a single send, using the existing no-proxy,
no-redirect, no-retry client and five-second attempt timeout. Request and
response limits remain 32 KiB and 64 KiB. Oversize, truncated, timed-out or
stopped delivery remains incomplete and cannot retry or refund its admission.
A fully received rejected response settles the attempt, not a valid answer.

HTTP receipts use `observation: native_http_response`. Their optional `http`
object records version, transport, request hash, validation and a bounded typed
projection. Rejected projections are null. Withheld projections suppress the
genre. Identity, pricing, usage and remote-arrival verification flags remain
false. Raw response bodies, headers and arbitrary remote errors do not enter
runtime Journal receipts or IPC. Set `SOUNDROBE_HTTP_CONTROL_EVIDENCE` to a new
local output file to retain the component table's public synthetic bodies for
cross-language checks. Export is optional and create-new; it cannot overwrite
an earlier checkpoint. The controls never read the development credential file.

The Python `http_response_audit` helper independently derives a response
projection from complete bytes and checks the exact receipt. Explicit offline
HTTP rehearsals also join the plan/binding marker, native loopback observation,
actual HTTP receipt, worker state and mock scores through the supervised auditor.
The local observation describes a validated request and prepared response bytes;
it is not remote-delivery authority. There are still 496 ordinary fixture arrivals
and one separately classified native observation, not an invented service arrival.
The accepted HTTP genre is retained only in declared typed projection/review/media
sinks. Journal creation and every write connection enable SQLite secure deletion,
so rewritten projections do not linger in freed cells or overflow pages. The
privacy checker rejects extra database-page copies rather than exempting them.
Raw bodies and private/rejected canaries have no new log or file exemption.
This checks completed retained bytes, not interrupted-write or physical erasure.

Historical injected-response audits reject HTTP substitution without the exact
mode marker. Mock scoring still grants zero grounded quality credit. Native
completion remains dependent on witnessed timely successful supervisor exit,
not response receipt or staging. TLS handshakes, external connectivity, real
credential compatibility, production approval custody and provider billing
remain untested. Component checks do not replace a fresh six-job rehearsal.

## Rehearse a different public response

An offline hosted HTTP plan can add `"offlineResponse": {"sha256": "<lowercase SHA-256>", "status": 200}`.
The exact public bytes must be in `response-fixture.json` beside the plan,
outside the new run directory. The driver adds this fixed sibling to the
immutable source locks. Native bootstrap rejects missing, changed, oversized or
symlinked input. The response remains bounded to 64 KiB. The option requires
both `offlineHosted` and `offlineHttp: true`; it cannot select an external URL.

The supervised auditor independently derives response validation, genre,
confidence and usage from those bytes. Accepted genre must match worker
provenance and media readback. Withheld results cannot invent assisted genre.
Rejected or incomplete responses cannot pass the completed-rehearsal audit.
Mock scores still grant zero grounded assistance credit. Historical fixed
response modes retain their existing audit rules.

A native-only `ApprovalVerifier` loader also checks an independently pinned,
owner-only, single-link 32-byte public-key file and rejects known public fixture
keys. A disposable random signer exercises Journal approval and replay denial.
The loader does not provision a signer or activate requests. Hosted dispatch
remains unconditionally disabled even after approval.

## Native prepared bootstrap and owner approval — still disabled

The test-build native broker recognizes a public `nativePrepared: true` selector.
It cannot carry a verifier, signing key, trust-directory override or activation.
It is incompatible with the public offline fixture options. Native bootstrap
reads the current account owner's home from the account database, not `HOME`,
then uses the fixed `.soundrobe/evaluation-approval/` directory. That canonical,
same-owner directory must be owner-only. Tests use a disposable directory and
random signing key; they do not access the provisioned private key.

The fixed `prepared-bootstrap.json` file contains exactly `version: 1`,
`planSha256`, `verifierSha256`, `credentialPath` and `pricing`. The plan digest
is SHA-256 of the native `serde_json::to_vec(RehearsalPlan)` serialization,
not a hash of arbitrary pretty-printed JSON. The verifier digest pins the fixed
`approval-verifier.ed25519`; known public fixture keys are rejected. `pricing`
uses the existing text-price evidence contract and the unchanged $5 allowance.
The canonical `.env.local` parent must exist, but bootstrap does not open the
credential file. No real bootstrap configuration or run approval is provisioned
by these tests. This file is configuration, not user execution approval.

Only a natively prepared broker can accept the exact owner command
`{"op":"approve_owner_file"}`. It reads the fixed `prepared-approval.json`,
then uses the maintained signature, scope, expiry, single-use and durable
approval checks. The command accepts no path or key; it never signs. Native
configuration, verifier and approval files are bounded, no-follow, same-owner,
single-link regular files with no group/other permissions. Invalid ingress
retains the existing terminal stop behavior. Custody assumes a trusted host;
it does not isolate malicious same-user workers or prove power-loss durability.

The public `testApproval` signer refuses native-prepared plans and disabled-hosted
contexts without explicit fixture trust. Mock receipt validation also refuses
native-prepared results. The independent mock auditor already rejects their
additional plan field; it cannot certify them as a completed mock rehearsal.
There is no automatic real-run signing path. A real owner signature can consume
approval, but pinned HTTPS still returns `live_dispatch_disabled` before a
reservation or credential read. Source bindings still record
`liveDispatchAvailable: false` and `readyForExecution: false`.

An explicit native owner-side `Journal::owner_approval` call now constructs the
payload from the current frozen Journal, not caller-supplied JSON. It checks the
owner handle, independently supplied expected checkpoint, native context, nonce
and bounded expiry before loading fixed `approval-signing-key.pk8`. Its derived
public key must match the separately pinned owner verifier. Bounded owner-only
custody applies to the signer too; OpenSSL PKCS8 v1 and ring PKCS8 v2 are covered.
Issuance returns a signature without changing the Journal or consuming approval.
Ordinary broker IPC and the native-prepared launcher do not invoke this signer.
The explicit owner-offline entry described in the
[owner-offline evaluation reference](../../docs/auto-tag-evaluation.md#owner-authorized-offline-evaluation)
can issue and publish an approval after consuming signed offline execution intent.

The separate `owner_approval_audit` Python helper takes a frozen report, approval,
raw public verifier, independently supplied verifier digest and verification
time. It reconstructs the checkpoint and scope and verifies Ed25519 with local
OpenSSL. It is not part of mock-success acceptance and does not establish human
presence, execution, remote delivery or quality credit. Signature verification
must not be treated as permission to enable dispatch.

A separate explicit `Journal::publish_owner_approval` verifies the current frozen
owner/context, checkpoint and already issued signature without opening a signer.
On macOS it writes a unique owner-only temporary sibling, syncs it, uses native
no-replace rename to fixed `prepared-approval.json`, syncs the directory and checks
exact readback. Existing targets remain untouched, even with racing publishers.
Pre-rename failure removes only its own temporary file. Post-rename failure is
publication-unknown: the target remains and must not be automatically retried.
Other platforms explicitly reject publication in this slice. Scoped evaluation
processes also reject this owner-side writer before file creation; it cannot
bypass their existing storage ledger. Owner-process publication is separately
bounded to 8 KiB per approval, not a new whole-run fit measurement. A visible
valid file is not proof that the publisher returned or that directory syncing
completed; power-loss durability is not established.

The exact owner-only `{"op":"owner_approval_status"}` operation checks the native
monitor, admission stop latch, frozen context, custody and signature. Only an
absent fixed file returns pending. Ready returns the current checkpoint and
canonical approval hash, never a signature or authority to execute. Errors stop
the run; consumed/stopped runs cannot remain ready. Status never signs or consumes.

`approveFrozen` keeps public fixture signing separate. Native-prepared coordination
checks the native remaining scope before each status observation and consumption,
waits only for pending, consumes the fixed file once, then joins the durable report
to the observed hash and unchanged frozen bindings/membership/preparations. It
requires zero credential/model activity and retains only
`owner-approval-coordination.json`, explicitly without success, quality, presence
or live authority. RPC failures and ambiguous consumption are never retried.
Without signed owner execution intent, the ordinary native-prepared launcher
stops at `native_prepared_execution_disabled` before pending executions.
It omits the scripted-model fallback for native mode. The explicit owner-offline
entry instead permits a disposable-key, fake-credential rehearsal; public fixture
behavior and historical mock-success rejection remain unchanged.

The offline proof includes disposable keys, atomic publication/no-overwrite,
actual broker pending/readiness/consumption and a Node-to-native coordinator
component control. The component uses an explicitly labelled local remaining-scope
seam, not a supervised native-clock or launcher-workload proof. No provisioned key,
real credential or real-run signature is used. Those are predecessor component
limits, not the scope of the owner-offline entry. Hosted credential compatibility,
a pricing estimate, real-provider response validation, and separately authorized
hosted activation remain unfinished. No prompt-preview requirement is reinstated
by this work.

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
