# Independent retained-evidence review 113

## Verdict

The completed evidence supports the reviewed mock supervision controls and an operational source/binary correspondence chain. It does not establish final-source six-job acceptance. Run-110 is incomplete with an unexplained preparation failure. Strict default and server Clippy both failed. No new runtime source defect was established by this evidence review.

HEAD: `8287b6b0cb47365522ac13eab94e1c8595e90a06`. No builds, tests, evaluations, network access, or diagnostic processes were launched. Only retained evidence and source/hash data were read; this requested report is the only file added. Diagnostic-111 was not evaluated or promoted to acceptance.

## Source and binary correspondence

- Independently rechecked all 19 entries in `source-100/SHA256SUMS`: all match the current files, including the five native supervision modules, launcher/driver/harness/fixture/auditor and retained executable.
- Independently verified `source-100/archive.sha256`: `source.tgz` matches `20aaa21a69a18d77a021e20125922917886a5cd16e4fd3eb79dda49c84da9dbc`.
- Read the archive without extracting or executing it. All 225 non-AppleDouble regular files match current file bytes. The other 225 entries are macOS `._` metadata, not source drift.
- All 103 entries in `faults-101/source-and-tool-locks.json` match current bytes. This includes the retained executable, launcher and synthetic finalization fixture.
- `build-107.jsonl` reports a successful Cargo test-artifact build with `features:["server"]`, executable `src-tauri/target/debug/deps/soundrobe_lib-063309b8ec43ac15`, and `fresh:true`. Independently hashing that artifact gives the same SHA-256 as `source-100/native-evaluator`: `1f585b9822d0aa8e3846a58e3bd29cd6b56b3ff1f54245e6138a5148165b0ce3`.
- `source-100/final-bindings-107.log` records matching manifest entries; the checks above independently corroborate it.

This is a consistent local Cargo-cache/artifact/hash chain, not an independent clean rebuild, reproducible-build proof, or cryptographic attestation that an arbitrary executable was derived from the archive. Authority-99 names the same Cargo artifact path; its historical invocation is supported by the retained log, not independently re-executed here.

## Controls closed by completed evidence

`authority-99.log`: **45 passed, 0 failed, 3 ignored**, including managed finalization and receiver acceptance tests. The reviewed test bodies require the selected reached markers, expected outcome-file behavior, and managed group absence; an earlier bootstrap failure cannot satisfy them.

- Managed synthetic positive: all twelve real broker sessions/leases, preparation/freeze/consume, staging, joint retirement and timely receiver acceptance. Its receipt is control-only, not tagging evidence.
- Managed negatives: file-sync-entry stall, provisional rename before directory-sync acknowledgment, one retirement request before the other, and retirement intent before supervisor reap.
- Receiver negatives: blocked verification, recording after rename before directory-sync acknowledgment, and post-acceptance-CAS delay; also complete-witness sender suspension, malformed/trailing framing, and late/missing/nonzero/unknown-exit cases.

`faults-101/results.json` and `faults-101.log`: **10 controls passed**. Nine report actual execution-group absence and incomplete receiver termination, retaining 216 slots. Recorded quiescence ranges from about 18 ms to 1965 ms; these are observed control timings, not general latency guarantees.

- Broker/supervisor/receiver loss, blocked Node, stalled FIFO storage, held SQLite journal, supervisor/broker suspension, and pre-exec anchor loss completed.
- Running-anchor loss reports launcher exit 1, receiver exit 78, containment unknown at return, and later finite-orphan absence under the reviewed harness. Launcher return is approximately 7131 ms. It does not mislabel a surviving payload group as contained.
- Selected logs include lease-loss and stopped-broker error/panic output. Those are failure-path diagnostics, not positive-completion evidence and not an explanation of run-110.

These close the previously missing schedule/fault controls compositionally. Receiver-only fault controls still use a synthetic sender; not every receiver fault traverses all twelve managed sessions. Park/sleep seams at I/O boundaries are not physical kernel-sync, uninterruptible-I/O, or power-loss proofs.

## Broader checks and outstanding gates

`combined-check-102.log` ends successfully: TypeScript typecheck; 49 renderer test files / 858 tests; default Rust 1311 passed / 32 ignored; server Rust 811 passed / 23 ignored; additional script and integration suites also report success. Both finalization and receiver acceptance tests appear as passed. This supplies a successful broader rerun after failed combined-check-66; ignored tests remain explicitly outside the completed coverage.

`clippy-108.log` and `clippy-server-109.log` both **fail** strict Clippy. Diagnostics report 11 lib-test errors, including regex compilation in a loop, needless borrows/clones, `manual_inspect`, and a while-let suggestion. Locations are `commands/auto_tag.rs`, `commands/auto_tag_eval.rs`, and `infra/bounded_eval.rs`; none are in the five supervision modules. A current `git diff HEAD` for those three files is empty; `clippy-unchanged-109.diff` is also empty. Thus these are not newly introduced supervision-diff findings, but the strict gates are not green. Resolve or explicitly disposition them; any source fix needs a new freeze and relevant evidence.

`run-110.log` and `run-110/launcher-outcome.json` record incomplete status, receiver 78, null signal and 216 requested slots. No receiver outcome or execution-scope file was found. The supplied two-preparation / 111-completed-operation observation does not establish a cause, timely execution acceptance, or success. Do not attribute it to startup load, a lease, or a deadline without diagnostic evidence.

No completed final-source maintained-launcher success plus audit is established by the evidence reviewed here. Run-90/audit-92 remain earlier-source evidence. Diagnostic-111, regardless of outcome, uses regular-file stdout through the native entry rather than the maintained launcher and cannot alone replace that final gate.

## Remaining requirement

Retain a source-100-bound maintained-launcher six-job result and independent audit covering all 216 slots, with native witnessed acceptance and actual supervisor zero exit observed before the unchanged deadline. Preserve run-110 as incomplete and explain or bound the failure before claiming reliable completion. Later staging files, eventual exit, or lost recording must not recover authority.

Nothing needed from the user during the already-running serial validation work.

## Final addendum — run-114 / audit-115 / controls-116

This addendum supersedes the earlier statement that no completed final-source maintained-launcher result was available. It does not erase the failed run or strict-Clippy results.

### Narrow disposition

**Accept the demonstrated trusted, mock-only supervision slice for run-114 on source-100.** The maintained launcher records actual exit zero; the receiver records timely native acceptance after observed supervisor zero exit; the independent retained audit passes all 216 slots. No new runtime blocker was found. This is not an all-gates-green or repeatability declaration. The corruption lever has one concrete proof defect described below; do not claim eight independently isolated semantic negatives.

### Final binding and successful-run evidence

- `run-114.log` explicitly records actual launcher exit 0. The matching launcher outcome records receiver exit 0, null signal, actual exit observation and `liveAuthority:false`.
- Generation is consistently `ee1d123d-9bb8-4fab-99fa-da528ab89265`. Receiver PID 22602, supervisor group/PID 22603 and execution group/anchor PID 22604 agree across identity and outcome files; the receiver group is separately 22599. `controlOnly:false`, with empty native control selection.
- Preparation uses 210911 ms of 300000 ms. Execution transition is at 473927434; supervisor zero exit is observed at 474364940; acceptance is at 474364973, before deadline 474527434. Acceptance consumes 437539 ms of execution, leaving 162461 ms. The receiver's exit observation precedes acceptance by 33 ms. These are retained native-clock observations, not a new live measurement.
- Independently verified launcher-to-receiver, receiver-to-provisional-evidence and receiver-to-results SHA-256 links. The Node provisional mirror equals the broker provisional report.
- Requested membership equals the compiled membership fixture. Six cold jobs retain slots 33, 27, 48, 27, 48, 33 in frozen order: disabled-crowd, disabled-billie, disabled-enya, assistance-billie, assistance-enya, assistance-crowd. Requested, published and summed observed slots are all 216.
- All twelve sessions are closed and finished, with both native and fixture leases false. Owner lease is false, consumptions 1, denials 0. All 497 operations are dispatched, arrived and completed. Counts reconcile as provider 472, inventory 24, model 1; reserved/arrived/completed are each 497. Execution alone accounts for provider 236, model 1 and 249 wire arrivals; aggregate counts include preparation and are not conflicting totals.
- Independently compared each job's JSONL with `records.json` and each published execution score with its score file: all agree. `audit-115.json` reports independent scoring/native checks passed, 216 observed slots and twelve sessions; `audit-115.err` is empty. The audit was inspected, not rerun here.
- All 128 run-start source/tool locks and all 1662 provisional-receipt source/tool bindings match current bytes. All source-100 manifest entries still match, and overlapping receipt/manifest hashes agree. The selected executable and receipt executable digest are the previously reviewed source-100 binary `1f585b9822d0aa8e3846a58e3bd29cd6b56b3ff1f54245e6138a5148165b0ce3`.
- Completed follow-ups were available without waiting: authority-117 reports 45 passed / 3 ignored; driver-118 reports 3 passed; final-bindings-119 reports every source-100 manifest entry OK. These are retained results, not independently launched gates.

### Corruption lever: seven isolated negatives, one confounded negative

`audit-controls-116/results.json` reports one passing positive copy and eight rejected corruptions. The positive-copy stderr is empty. Seven stderr traces identify the intended semantic rejection: projection/JSONL mismatch, published-score mismatch, late exit ordering, unsuccessful receiver, generation mismatch, denominator mismatch, and staging-only state. The lever rebinds outer hashes, so these are not merely stale-file-hash rejections. They test auditing of disposable evidence copies, not live cancellation or kernel wait behavior.

**Proof defect:** `check-audit-103.py:44–55` writes the wire-accounting mirror before its common block reserializes results and updates `evidence.resultSha256`. The resulting two provisional reports differ only in `resultSha256`; both contain the corrupted model count. `wire-accounting.stderr` confirms rejection at `assert receipt == stored`, before counter reconciliation. Thus this case proves mirror inconsistency is rejected, not that independently consistent false counters are rejected.

Smallest correction to this evidence-only lever: write `authority-provisional.json` after the common digest update for wire-accounting, so both reports remain identical, and require the retained rejection to reach counter-versus-wire reconciliation. This does not require changing source-100 runtime sources or rerunning tagging. Until then, retain controls-116 unchanged and qualify its wire-accounting claim. The valid run's actual 497-operation reconciliation and reviewed audit checks remain evidence; this confounded negative does not invalidate run-114.

### Boundaries of acceptance and remaining failures

- **Run-110:** retain as an unexplained incomplete preparation result with all 216 requested slots. Run-114 does not explain it or prove reliable/repeatable completion. Do not retroactively attribute it to host load, leases or deadlines. Diagnostic-111's native zero exit is supporting diagnostic evidence only, not maintained-launcher acceptance and not a substitute for run-114.
- **Clippy-108/109:** both remain failed strict gates in unchanged, out-of-slice files. Disposition as documented baseline findings for this narrow review, not new supervision regressions and not waived/passed gates. Repository-wide strict-Clippy or merge acceptance still needs remediation or an explicit authorized exception; a source change would require renewed bindings and appropriate checks.
- **Tagging quality:** this is scripted mock wiring and supervision evidence, not grounded model/provider accuracy. The audit reports grounded assistance accuracy credit 0 and user-acceptance agreement 0 of 22 requested occurrences/fields. It does not authorize production writes or live paid calls.
- **Trust:** native live acceptance is established by the reviewed trusted process chain plus retained evidence. Editable JSON/hashes do not attest against a malicious host or coordinated evidence forgery; retrospective audit cannot create authority.
- **Containment/storage:** unexpected anchor loss remains containment-unknown. I/O-boundary park/sleep controls are schedule tests, not proof of physical kernel-sync interruption, power-loss durability, remote cessation or rollback. Lost recording remains incomplete; a staged or renamed file and eventual exit never substitute for witnessed timely acceptance.

No tests, evaluations or builds were launched for this addendum. Only this review document was modified. Nothing needed from the user to substantiate the narrow run-114 result; the wire-accounting evidence correction and strict-gate disposition remain explicit follow-ups.

## Evidence-only correction disposition — controls-121

**The wire-accounting proof finding is resolved. No remaining concrete blocker was found to the narrow trusted mock-only run-114 conclusion.** This supersedes the unresolved corruption-lever follow-up above, not its historical description of controls-116.

- Read `check-audit-120.py`, `audit-controls-120-red.log`, and fresh controls-121 results/stderr. The retained red fails at the explicit mirror-equality prerequisite, exposing the old confound before invoking the auditor for that case. The corrected lever writes the mirror after result-digest rebinding, asserts receipt equality, and checks the first native-auditor traceback location against the counter-reconciliation assertion's derived line number.
- Independently inspected controls-121's wire-accounting copy: both provisional reports are equal; versus valid run-114, receipt changes are only the model counter (1 to 0) and the rebound result digest. Results are semantically unchanged; membership, operations, twelve sessions and source bindings are unchanged. One fully dispatched/arrived/completed model operation remains, and retained wire reconciliation still reports the valid original counts.
- Independently verified all three digest links: results to both receipt/outcome, provisional evidence to outcome, and outcome to launcher. Thus rejection is not caused by stale digests or inconsistent mirrors.
- The first native traceback is line **78**, matching the actual counter-versus-wire assertion after adaptation. It is no longer the earlier receipt-equality assertion. Controls-121 reports unchanged-copy success and all eight corruptions rejected; positive-copy stderr is empty. The other seven cases retain their previously reviewed semantic targets.
- Rechecked every source-100 manifest entry and all 1662 run-114 receipt source/tool bindings against current bytes: no mismatches. No runtime source, retained valid run or original lever was edited in this review. Controls-116 still contains its old mirror inconsistency and remains historical, not silently replaced.

This resolves an evidence-isolation defect; it does not add live authority, malicious-host authentication, physical-storage guarantees or a repeatability claim. The corruption tests are retained executions on independent evidence copies; no audit/test/build/evaluation was launched for this verification.

Strict Clippy108/109 remain documented, unchanged, out-of-scope baseline failures: **non-green, no waiver, no merge or lint-clean claim**. Run110 and run74 remain unexplained incomplete attempts; run-114 does not explain them and no reliability claim is made. Lost recording remains incomplete. The acceptance remains only the previously specified trusted mock-only run-114 supervision slice, not tagging-quality or live-provider approval.

Only this review document was updated. Nothing needed from you right now.
