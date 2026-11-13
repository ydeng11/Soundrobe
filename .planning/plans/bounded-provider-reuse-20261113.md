# Bounded provider reuse

## Delivery authorization

The user authorized a local commit with reproducibility records and known limits after the offline verification below. No push, fetch, live activation or credential provisioning is authorized. The containing commit is the delivery identity; earlier uncommitted/no-commit statements describe the implementation checkpoint before this authorization.

## Goal and boundary

Reuse the existing native OpenRouter/OpenAI-compatible request interface for the bounded evaluation. Keep all evaluation traffic loopback-only, fake credentials only, and existing admission/receipt/publication authority intact. Do not read development secrets or change the configured model. No real calls, provisioning, commits or pushes are authorized.

## Design

The ordinary application client and the broker will share the JSON request encoder. A test-build-only bounded request entry point on that client will construct one request for the existing broker HTTP executor. It will reject non-loopback endpoints, non-OpenAI formats and invalid bounds, and explicitly disable reasoning, provider fallback and streaming. The broker retains single dispatch, nonrefundable admission before credential lookup/send, response-size limit, original timeout and receipt-before-interpretation. It must not use the ordinary completion loop, which permits retries and repair.

Reusing the whole completion loop would obscure exact raw response receipt and require broader changes to normal app semantics. Keeping a separate hand-built broker request would continue duplication. Shared encoding plus one prepared request preserves the narrower boundary.

## Sequence and checks

1. Add failing offline request contracts for unchanged DeepSeek model/messages/schema, bounded route rejection, and enforced request policy. Run focused Cargo tests and retain their failure.
2. Share ordinary request encoding and add the loopback-only prepared-request entry point; integrate the actual broker while preserving preparation hashes. Update independent JS preparation projections and their expected digests for explicit bounded fields.
3. Exercise actual local HTTP success, status, malformed/empty/oversize output, redirect and slow-body controls; retain one wire request and no escape/repair/fallback. Run existing broker authority controls.
4. Check normal client tests unchanged, server/desktop unit coverage as practical, script helper contracts, formatting, TypeScript and whitespace. Inspect the diff for scope and duplication. Update operator docs/changelog and record actual results and limitations.

## Acceptance

Existing provider-first eligibility, prompts and schema remain unchanged. The mock broker uses the shared request encoder and client request construction, not a second provider implementation. Real model selection remains a later frozen non-secret preflight input; the scripted fixture identity remains mock-only. No live-safety, real billing/privacy or tagging-quality claim follows from offline tests. A fresh full supervised six-job acceptance must be distinguished from targeted integration tests, not inherited from historical runs.

## Executed results

Implemented on local `artist-library-inventory` above `68df20ea7a6f82772df8214342f3034e7225f8cb`, uncommitted. Eight tracked files changed: the shared client, broker transport/policy and its integration test, two independent JS request projections, operator documentation and changelog. No configuration, credentials, original library, live calls, commits or pushes were used.

Evidence root: `.amp/in/artifacts/bounded-provider-reuse-20261113/`.

- `request-red.log`: new bounded request API missing before implementation. `request-green.log`: initial two request contracts passed. The later full suites include the third real-loopback DeepSeek request contract.
- `broker-red.log`: the old independent preparation digest rejects the changed request bytes. `broker-green.log`: updated independent projection passes the actual broker path. The original schema/hash admission is not weakened.
- `client-regression.log`: 16 passed / one credentialed live smoke ignored. `policy-regression.log`: three passed / one isolated subprocess entry ignored; the integration table exercises twenty accepted/denied cases, including empty/truncated replies, 429, redirect, invalid schema, oversize body, missing usage, wrong identity, expiry, revocation, replay and forged authority. Each dispatched case asserts one exact wire request, not a completion retry or repair loop.
- `process-controls.log` and `process-controls/results.json`: real competing model children produce one reservation, one wire operation and terminal/incomplete recovery with all 216 slots retained. This is a failure-side accounting control, not successful quality evidence.
- `server.log`: fresh offline server-feature library suite, 871 passed / 30 ignored, no failures. `desktop-client.log`: fresh desktop-feature client suite, 16 passed / one live smoke ignored. The full desktop suite, credentialed/display/packaged smokes and strict Clippy were not run for this slice; historical non-green Clippy remains unchanged.
- Fresh TypeScript and renderer checks passed: 858 renderer tests, twelve mock-provider contracts and ten review-tool contracts. `helpers.log`: seven Node helper contracts passed, no skips; negative-control stderr remains retained rather than hidden. Rust formatting, CJS syntax and whitespace checks passed.
- Retained `native-evaluator`, `source.patch`, `source-sha256.txt` and `runtime-sha256.txt` bind the fresh supervised `run/`; byte hashes rechecked after success. Launcher/receiver exit zero, audit `audit.json` passes all six cold jobs, 216 requested/observed slots, twelve sessions and 497 reserved/arrived/completed operations (472 provider, 24 inventory, one scripted model). Exactly one fake credential lookup and durable lookup admission. This is new execution, not reuse of run69.
- Preparation 221507 ms; supervisor exit observed at 578650368, acceptance 578650402, deadline 578796328. Original 300/600-second scopes and native witnessed-successful-exit contract unchanged. Storage audit records 320503200 growth bytes and 502190739 cumulative application/VFS-write reservations, including original terminal reserve, within original 512-MiB/1-GiB caps; no refunds or increases.
- Grounded assistance credit remains zero; user agreement remains 0/22. The single scripted answer is a wiring control, not a real-model result.

Diff review confirmed shared serialization leaves ordinary request policy/behavior unchanged. Bounded route/generation guards are at the request boundary; no new transport backend, fallback interface or secret/config loader was added. Send admission follows both fake lookup and request preparation, preserving a fresh revocation/deadline check. Request bytes are checked against reservation before the strict broker executor sends them.

Normal client header/body/deadline tests passed unchanged. No new slow-body fault was injected into the broker during this slice; its existing five-second outer timeout, byte cap and strict HTTP executor remain in place. The new request-preparation method exists only in test builds and cannot authorize live execution. Real approval/custody, exact configured-model preflight, hosting capabilities, disclosure/retention and billing remain separate unfinished live requirements. No new independent review, full adversarial fault matrix, physical-I/O/power-loss, malicious-host, anchor-loss-containment, fresh-clone replay or quality guarantee is claimed. Raw evidence and unrelated untracked archive/cache are retained.
