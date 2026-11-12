# Durable mock authority evidence

This record covers the authorized mock-only slice above local commit
`5f8389e1afc966c3eb9bd7ba9b21684035cd6343`, on `artist-library-inventory` in
the existing Soundrobe checkout. It does not replace historical packets, seals
or results. The implementation remains uncommitted and unpushed.

## Boundary

The native test-only broker owns one SQLite journal, six cold jobs, 72 arm
observations and 216 requested title/ordered-artist/genre slots from preparation
onward. Every preparation completes and its participants close before one
checkpoint freezes. One fake checkpoint consumption admits execution on the
same retained generated files with fresh execution stores and fixture sessions.

Reservation, dispatch admission, fixture arrival and bounded response settlement
are separate journal facts. Short synchronous admission serialization includes
the storage-error latch; no SQLite transaction or admission mutex spans HTTP or
acknowledgment waits. The existing WriteQueue guard asks the authority before
native mutation admission. A previously admitted forward or atomic write may
finish after stop. No new admission, refund, retry, repair, rollback or execution
recovery is promised.

Provider, inventory and model counters remain separate and cumulative. Caps are
512, 32 and one respectively, with 576 total operations. Preparation records a
conditional request without sending it. Ordinary eligibility and provider/version,
collaborator-credit and user-decision safeguards are unchanged. User labels remain
scorer-only; a scripted response never earns grounded assistance credit.

## Completed native cycle

Fresh source-bound run-02 completed all six cold jobs. The read-only auditor
rederived 216 observed slots out of 216 requested, 30 labelable slots and 22
user-acceptance slots. User agreement remains **0/22**, with zero grounded AI
credit. Execution completion is not a clean-tagging or quality-improvement claim.

The final journal and twelve fixture snapshots reconcile **472 provider + 24
inventory + one model = 497 actual loopback arrivals**, all reserved, dispatched,
arrived and settled. Freeze captured 236 provider + 12 inventory + zero model
operations before consumption; execution added 236 + 12 + one. The split mode
has four inventory operations per job rather than the historical paired mode's
three. Its total must not be compared against historical 491 as an extra model
attempt. There were no misses or retries/repairs. Compact inventories retain all
1,533 routes each, measuring 969,410–999,453 bytes under the unchanged 8-MiB cap.

Both Enya arms genuinely held on `discovery_budget_limited` and retained their
baselines. Billie preserved Discogs release `37353822`, provider genre and ordered
Billie Eilish/Vince Staples credits. Crowd retained the 62.17-second Intro and full
Acoustic Ver. title. Only assistance-Crowd naturally reached the conditional
request. Its eleven scripted genre changes persist as inferred-unverified review,
not accepted user values. These cases do not cover every recording/content-only
branch or packaged highlights. Historical redacted Cookie Jar evidence remains
unavailable; nothing here awards it credit.

All 20 source-locked process/wire controls passed, including competing children,
simultaneous consume, service replacement without counter reset, stale/forged/
rebound handles, ingress/lease failures, failed notification, storage faults and
crash windows. Every incomplete control retains 216 requested slots. Native
authority checks passed 12 tests with the explicit broker entry ignored; paired
checks passed 23. Fixture compatibility passed 12 and portable scoring passed 19.
The final export passed 70 scorer-corruption controls and eight durable-audit
corruptions. `run-audit.json`, `projection-controls.json`, `audit-controls.json`
and `process-controls.json` hold the compact machine-readable results.

`CARGO_NET_OFFLINE=true just --no-dotenv check` exited zero. It passed typecheck,
858 renderer tests, 1,309 desktop-library tests (32 ignored), 12 fixture-script
tests, 10 review/suite-script tests, 778 server-library tests (21 ignored), 14
audio-CLI tests (12 ignored) and five recovery-CLI tests. Zero-test feature legs
and ignored tests are not claimed as executed coverage. Credentialed, display
and live smokes were not run. `validation.json` records the counts and log hash.
Four maintained CommonJS syntax checks and `git diff --check` also passed.
The separate historical label-reproduction error below is not hidden by that
successful default gate.

Fake approval and scoped handles assume trusted reviewed mock processes. They
are not authenticated user approval, malicious same-user containment or credential
custody. There is no real credential supplier, environment/home-secret fallback,
private account/config read, paid transport, external endpoint or live-enable
switch. Only local disposable generated media are used. Original music and the
unrelated iOS archive and Python cache remain untouched.

VFS/storage tickets, a full independent supervisor, hard whole-run deadlines,
physical-flush/power-loss proof, live approval, billing and privacy-retention
mechanisms remain deferred. SQLite process-crash observations prove neither
hardware durability nor general platform guarantees. Recovery reports stored
facts read-only and classifies the run as incomplete, even if a crash left the
stored lifecycle at `preparing` or `complete`; it never reopens dispatch.

## Reproduce the checks

Run from the repository root. Exact replay requires the retained ignored inputs
and matching executable; this is not a fresh-clone exact-replay claim. Raw local
evidence is under `.amp/in/artifacts/durable-mock-authority-20261112/`.

The source-locked build command was:

```sh
cargo test --offline --manifest-path src-tauri/Cargo.toml \
  --no-default-features --features server --lib --no-run
```

`final-build-receipt.json` records source and executable hashes and tool versions.
Its command field is absent because the original wrapper used an undefined
`spawnargs` result property. This record supplies the invocation without changing
that receipt. Cargo reused the up-to-date build in 0.55 seconds; it was not a
clean rebuild or executable attestation. `final-source.tar.gz` retains build
inputs and maintained tools. Run and controls freeze their own additional locks.

The completed process controls use a new destination each time:

```sh
node scripts/auto-tag-eval-replay/check-mock-authority.cjs \
  .amp/in/artifacts/NEW_CONTROLS "$test_executable" \
  .amp/in/artifacts/durable-mock-authority-20261112/run-02/requested-membership.json
```

The native six-job invocation uses a new output directory:

```sh
node scripts/auto-tag-eval-replay/run-bounded.cjs \
  .amp/in/artifacts/NEW_RUN "$test_executable" \
  .planning/plans/crowd-lu-user-acceptance-v1-20261111 --durable-mock
```

Read-only audit commands for a completed run are:

```sh
evidence=.planning/design/live-run-contract-20261111/durable-mock-evidence-20261112
run=.amp/in/artifacts/durable-mock-authority-20261112/run-02
python3 -B "$evidence/audit-durable.py" "$run"
python3 -B "$evidence/check-audit-controls.py" "$run"
python3 -B .planning/plans/cold-paired-mock-delivery-20261111/audit-projection-controls.py "$run"
```

The durable auditor reuses the sealed historical native auditor through one exact
in-memory preparation-path replacement. It does not alter that historical file.
It additionally checks the independently recomputed checkpoint, complete receipt,
source/tool/executable and observation digests, unchanged preparation sessions and
operations, compact fixture snapshots and global category/wire reconciliation.
Its negative controls change temporary metadata copies only and remove them.

## Historical label-check failure

`labels-green.log` retains its original, misleading filename but records an actual
failure: five frozen-label tests ran, four passed and `test_reproducible_packet`
errored. `freeze.derive()` differs from the sealed packet only in two source
bindings, for `auto_tag.rs` and `score-observations.py`. Both current hashes
already equal their bytes at starting HEAD `5f8389e1`; neither file was changed
in this slice. The packet's label values, identities and all other fields remain
identical. Both historical SHA256SUMS seals pass. Do not regenerate or weaken
the historical packet to hide source evolution. Current source/label/request
locks and actual scoring are checked separately by the new run and auditor.

## Retained attempts

Run-01 failed at assistance-Crowd restoration of a legitimate one-request plan:
183 observed slots plus 33 unavailable slots retained all 216 requested slots.
Its journal recorded 457 provider, 22 inventory and zero model reservations.
The executable and source hashes were frozen, but the exact pre-fix source bytes
were not separately archived. `restore-red.log` reproduces the narrow contract
failure. Restoration now validates the zero/one prepared request count against
request events, while execution starts with fresh comparison accounting and the
global wire budget never resets.

Earlier journal, broker, attachment, liveness, orphan-ingress, snapshot,
settlement and notification reds remain local. `snapshot-red.log` was a compile
failure; `snapshot-red-02.log` is the behavioral reproduction. Early malformed
orphan commands are not passing controls. Controls-02 and controls-03 are
superseded: controls-03 sources changed during execution and are not final
source-sealed evidence. Controls-04 is the final stable 20-scenario control run.
Zero-call synthetic lifecycle controls do not constitute native tagging evidence.
