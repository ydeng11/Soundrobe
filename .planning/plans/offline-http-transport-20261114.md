# Bounded hosted transport and response auditing

## Approved outcome

Implement the remaining one-attempt hosted transport and response auditing, using offline tests only. Keep maintained bootstrap and launcher unable to select external transport. Do not read real credentials, provision approval keys, activate calls, change accounts, touch original media or commit/push. Existing model, limits, allowance and retention decisions remain unchanged.

## Design

Add a dormant native HTTP sender in the existing broker transport. It accepts only the native-prepared POST to the pinned OpenRouter HTTPS endpoint, with exact bounded request bytes and no child-supplied executor or endpoint. The broker's existing no-proxy/no-retry/no-redirect client and five-second attempt deadline cover actual send and body reads. A private native local-control constructor rewrites only the pinned URL to a numeric loopback listener. Maintained bootstrap never constructs this sender. An activation flag or new public-plan mode would expand authority and is excluded.

Keep injectable-response and actual HTTP paths distinct. Complete HTTP responses retain exact hashes/status/length plus a typed, bounded validation projection in the existing Journal receipt. Rejected/withheld content, arbitrary remote errors/headers and raw bodies never enter general persisted evidence or IPC. A complete rejected response is a settled attempt, not a valid answer. Truncation, timeout and oversize delivery remain incomplete, nonrefundable and nonretryable. HTTP receipts never assert remote arrival, identity, usage or pricing verification.

The independent Python helper reconstructs that projection from public local-control response bytes and rejects altered receipt fields. It does not promote the old scripted/mock completion auditor to live authority or quality scoring. Full live bootstrap, actual trust provisioning, billing-risk acceptance and final execution approval remain separate.

## Verifiable units

1. Add native broker controls first. Exercise exact request/auth, accepted and withheld values, non-200/schema/identity/usage rejection, refusal/tools/reasoning, redirects, slow/truncated/oversize bodies, replay, revocation and default-disabled dispatch. Retain red evidence before implementation.
2. Add dormant pinned sender and optional typed HTTP evidence on ModelReceipt. Preserve historical receipt bytes when absent, durable admission ordering, one attempt and sanitized failure handling. Run native controls and formatting.
3. In parallel, extend only audit-supervised.py and test_supervision.cjs with an independent per-response audit and semantic corruptions. Keep old completion semantics unchanged.
4. Join the independent auditor to actual local HTTP broker evidence, inspect the substantial combined diff, run relevant regressions, freeze source/binary/evidence hashes and update the execution decision record. Report component scope, unrun gates and remaining live prerequisites explicitly.

## Evidence

Retain controls and delivery evidence under `.amp/in/artifacts/offline-http-transport-20261114/`. Generated media, if needed, must be disposable copies. Existing evidence, unrelated archive and Python cache remain untouched.

## Implemented component checkpoint

The native sender now uses the broker's bounded HTTP client and request encoder. Its pinned HTTPS constructor rejects admission before reservation or credential lookup. Only a private native control accepts numeric loopback HTTP. Maintained bootstrap still uses response injection. All evaluation modules remain test-build-only. No external activation or real approval/credential provisioning was added.

Complete HTTP observations retain digest, size, status and typed accepted/withheld/rejected projections. Rejected projections are null. Withheld genres are empty. Identity, pricing, usage and remote-arrival claims remain unverified. Incomplete delivery does not settle or restore an attempt. The shared parser also rejects refusals, function calls, nonempty tool calls and reasoning content. Existing model/request settings and limits are unchanged.

The new Python helper independently reconstructs complete HTTP response evidence. It is not a full HTTP supervised-run auditor or a live-quality scorer. All seven historical auditor functions remain structurally unchanged. Existing historical rehearsal audits reject HTTP observation substitution.

### Fresh checks and corrections

- `native-01-red.log` and `pinned-03-red.log` retain the missing sender/API and missing pinned-admission guard failures.
- `native-12.log` passes six tests with no skips, including the 24-mode actual broker HTTP table. Sixteen complete responses produce retained synthetic request/response bytes and actual Journal receipts. Eight other modes cover incomplete or unattempted outcomes. The table checks one send or zero as appropriate, replay denial, in-flight stop, input drift, default/pinned closure, privacy canaries and zero invented arrivals.
- `authority-05.log` passes 106 authority tests with ten explicitly ignored, before the final three HTTP table modes. Runtime code is unchanged by those later test-only additions.
- `helpers-11.log` passes 22 Node tests with no skips, including 147 independent HTTP audit cases. Expected negative-recovery tracebacks remain visible in the passing log.
- `join-13.json` joins all sixteen actual complete native HTTP receipts and rejects 144 semantic corruptions. `join-native-http.py` is the read-only repeatable check, not another workload.
- Diff review found Python accepted duplicate answer keys, overflowing diagnostic numbers and lexical negative-zero usage that Rust rejects. `json-10-red.log` reproduces the duplicate-key mismatch. New native controls confirm duplicate answer, number overflow and negative-zero rejection; the independent decoder now agrees. The five additional auditor cases also cover duplicate confidence and overflowing integer diagnostics.
- `typecheck-07.log` passes TypeScript. Rust formatting, Node syntax, Python compilation, historical-function comparison and whitespace checks pass.
- `server-08.log` is the broader serial server-library gate, still running on shell handle 60490 at this checkpoint. It has reported a failure in unchanged `commands::auto_tag::collection_tests::exact_flac_copies_reach_shorter_artist_release_and_keep_conflicts_in_review`; buffered failure details/final counts are not yet available. It is non-green, not waived. It started before the final three native table modes; runtime code is unchanged by those later test additions.
- A focused current-executable rerun of that test remains running on shell handle 65550, with output in `flac-copy-14.log`. `ffmpeg-startup-15.sample` samples its actual generated-media child after 40 seconds alive: all 794 sampled stacks are `_dyld_start`, before FFmpeg main. This proves that sampled startup delay only, not the cause of the broad failure or an OS diagnosis. A read-only capacity check observed 1.4 GiB available on the 1.8 TiB data volume, reported 100% used. No causal storage claim, cleanup or OS change follows.
- `http-controls-native12.json` preserves the sixteen final-table receipts separately. The earlier running broad test binary may overwrite the table's default `http-controls.json` with its earlier thirteen receipts. The repeatable join reads the preserved native12 copy. Neither file is supervised completion authority. No new matrix was launched after observing the startup sample.

### Limits and next integration

No TLS handshake, external connectivity, actual provider/model response, real credential read, production trust provisioning, fresh supervised six-job HTTP rehearsal, full desktop/renderer gate, strict Clippy, packaged/display smoke or independent runtime review is claimed. Historical adverse/ignored gates are not erased. The local HTTP control verifies the request sender and body handling, not the deployed service or remote billing. Grounded quality credit remains zero, with historical user agreement 0/22.

Remaining live work includes trusted real approval provisioning, authorized credential-source compatibility, deliberately enabled run-specific bootstrap, HTTP observation integration into full supervised worker/scoring audits, a concrete-policy offline rehearsal and frozen execution evidence. Applicable billing exposure and final one-request execution approval remain separate. Prior model/$5 allowance/retention decisions are preserved and do not enable calls.

The current eight tracked edits remain uncommitted above `a0e9a2255ca7397da9f31241991be38ae8157c85`. No commit, push, fetch, OS/account changes, real credentials/private configuration, original media, live request or unrelated cleanup was performed. Evidence stays local until result review.
