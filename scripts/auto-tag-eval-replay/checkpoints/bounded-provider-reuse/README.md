# Bounded provider reuse checkpoint

This checkpoint delivers local bounded reuse of the existing AI-provider interface above parent `68df20ea7a6f82772df8214342f3034e7225f8cb` on `artist-library-inventory`. The containing commit is the delivery identity. Delivery does not push, fetch, activate calls or inspect development credentials or configuration.

The commit contains eight implementation, test and operator-document paths, the scoped plan `.planning/plans/bounded-provider-reuse-20261113.md`, and nine compact checkpoint files. `bindings.json` records their source and retained-evidence relationships. No model setting, provider-first eligibility, prompt, schema, scoring label, media-write policy or user decision changes.

## Shared requests keep the mock boundary

The ordinary application client and the mock broker share the JSON request encoder in `OpenRouterClient`. The broker uses a test-build-only prepared-request method rather than the ordinary completion loop, which allows retries and schema repair. The prepared request permits only explicit-port numeric-loopback fixture URLs under the `/model` base path. It rejects alternate API formats and invalid request or output bounds. It explicitly disables reasoning, streaming and provider fallback and requires supported parameters.

The existing broker owns durable admission, fake credential lookup, a fresh send-admission check after lookup and preparation, one HTTP execution and exact response receipts before interpretation. The broker executor disables proxies, redirects and HTTP retries. Limits remain 32 KiB request, 64 KiB response, 1,024 output tokens and five seconds per attempt. Ordinary application behavior and development settings remain unchanged.

A local HTTP test preserves an existing DeepSeek identifier, messages and schema without contacting DeepSeek. The six-job replay deliberately retains the fake `scripted/genre-v1` identity. Neither test verifies the current private development configuration or a real provider's capabilities. No live mode, credential loader or real approval trust is added. Existing signed approvals use the deliberately public RFC 8032 fixture, not a private user trust root.

## Fresh implementation evidence

The retained source patch and executable under `.amp/in/artifacts/bounded-provider-reuse-20261113/` produced a new maintained-launcher run, not a reuse of run69. The read-only auditor passed six cold jobs, 216 requested and observed slots, twelve sessions and 497 settled operations. These comprise 472 provider, 24 inventory and one scripted model operation. Fake credential lookups and durable lookup admissions are each one.

Launcher and receiver exited zero. Native acceptance followed witnessed actual successful supervisor exit before the original deadline. Preparation took 221,507 ms within 300,000 ms. Supervisor exit was observed at 578650368, acceptance at 578650402 and deadline at 578796328. Execution remains bounded by 600,000 ms. Staging alone, eventual exit and lost recording cannot recover successful authority. Incomplete recovery retains requested slots and unknown effects without resume.

Storage records 2,509 grants and 205 owners. Reservations total 320,503,200 growth bytes and 502,190,739 cumulative application/VFS-write bytes, including the unchanged 2-MiB growth and 8-MiB write terminal reserves. Original caps remain 512 MiB and 1 GiB. Charges are not refunded. These are application reservations, not physical-device-write measurements.

Grounded assistance credit remains zero and user agreement remains 0/22. The scripted answer establishes request wiring, not real-model quality or useful completion of all slots.

Implementation checks passed 871 server-library tests with 30 ignored, 16 desktop client tests with one live smoke ignored, TypeScript, 858 renderer tests, twelve mock-provider tests, ten review-tool tests and seven Node helper tests with no skips. Client and broker tests also ran separately and overlap those suites. Twenty broker cases cover accepted and denied paths, including empty and truncated responses. A real competing-child control retained exactly one reservation and one wire, then terminal incomplete recovery with 216 unavailable slots. `request-red.log` records the missing API before implementation. `broker-red.log` records rejection of the old independent digest before its request projection was updated. Red and adverse evidence remain local.

## Delivery checks do not rerun workloads

Fresh delivery checks rerun TypeScript, twelve mock-provider tests, ten review-tool tests, seven Node helpers, Rust formatting, changed CJS syntax and whitespace checks. Fresh read-only verification rehashes the eight implementation paths, retained executable and patch, 134 runtime locks and 1,668 receipt bindings. It checks publication links, exact copied records and the expected branch and parent. The maintained re-audit is byte-identical to `audit.json` and replays all 2,509 storage records. It does not execute a model or media writer.

The prior source-bound server, desktop-client and renderer gates and six-job workload are reused for delivery. They are not described as fresh delivery matrices. Full desktop tests, strict Clippy and credentialed, display and packaged smokes were not run for this slice. Historical strict Clippy remains non-green. Normal client deadline tests passed, but no new slow-body fault was injected into the broker. There is no new independent review or complete adversarial fault matrix.

## Reanalysis needs retained local inputs

Five files are unchanged copies: `audit.json`, `source-and-tool-locks.json`, `receiver-outcome.json`, `launcher-outcome.json` and `process-controls.json`. `bindings.json` identifies copied and retained originals by path, size and SHA-256. `validation.json` separates fresh delivery checks from reused implementation gates and workload results. `SHA256SUMS` covers the eight implementation files, the scoped plan and every packet file except itself.

From this commit's source view, verify tracked bytes with:

```sh
shasum -a 256 -c scripts/auto-tag-eval-replay/checkpoints/bounded-provider-reuse/SHA256SUMS
```

Read-only reanalysis of the retained run uses:

```sh
python3 -B scripts/auto-tag-eval-replay/audit-supervised.py .amp/in/artifacts/bounded-provider-reuse-20261113/run
```

The second command requires ignored generated media, the matching retained executable, receipts, worker databases, ledger, fixtures and historical sealed auditor sources. `bindings.json` and the runtime manifests identify these dependencies. This packet does not provide fresh-clone exact replay, executable attestation or a restoration service. Rebuilding or regenerating media creates a new condition. For a new mock run, follow `docs/offline-provider-mock.md` with a fresh output directory. Never resume the retained run or promote its staging.

No binary, audio, database, cache, private configuration, credential or large raw log is committed. Raw and adverse evidence remains local. Unrelated untracked archive and Python cache remain untouched.

## Limits and remaining live decisions

This acceptance covers trusted mock code on this macOS checkout. It does not establish real hosting identity, schema enforcement, privacy, pricing, billing cessation, remote cancellation, secure OS credential custody, malicious-host containment, guaranteed anchor-loss containment, physical-I/O bounds or power-loss durability. Existing historical adverse gates and incomplete-control limits are not erased by this run.

The user chose to keep the existing DeepSeek interface and development setup. Real execution still needs an exact non-secret configuration freeze, compatible hosting and usage accounting, explicit disclosure and retention terms, a spending allowance and billing-risk decision, real approval and credential provisioning, and separate final execution approval. This commit authorizes none of those actions.
