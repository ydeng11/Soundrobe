# Owner publication and launcher coordination

This packet records offline owner-approval publication and Node/native coordination, not real-model quality or execution permission. All keys/signatures and bootstrap inputs used here are disposable test data. The provisioned signer, real credentials and original media were not opened. Calls remain disabled; no real run was signed.

## Final checks

The current native authority gate passes 112 tests, zero failures, with ten declared ignored; the Node harness passes 46 with no failures/skips. TypeScript, Rust formatting, script syntax and whitespace pass. Thirty actual native owner-file scenarios retain direct-ingress and separate readiness controls. Four Rust-produced signature witnesses independently verify, and two actual Node/native coordination receipts reconcile with the signed scopes and stopped zero-model/zero-credential snapshots. The scoped-writer red test demonstrates publication before the guard; the final green gate rejects it before creating a file and preserves the ledger.

The frozen source packet contains 323 bindings, twelve dirty tracked paths and eight incremental paths. Executable SHA-256 is `c38bce5320d534b69df1e6dba87b7e18e8eb2a9c5cc1285eaecd1f4763a16be5`. The retained executable and source are checked by `verify-checkpoint.py`; `final-check.py` rechecks source, signature/coordinator joins and outer evidence hashes without rerunning the native or Node matrices. This is author-run verification, not independent runtime review, a new full server-library gate or a fresh six-job workload.

## Reproduction

Run from the repository root with the existing local Cargo/Node dependencies. The exact native command and executable hash are captured in `source-manifest.json`.

- Native: `CARGO_INCREMENTAL=0 cargo test --offline --config 'profile.test.package.soundrobe.debug=0' --manifest-path src-tauri/Cargo.toml --no-default-features --features server --lib mock_run_authority:: -- --nocapture --test-threads=1`.
- Node: `node --test scripts/auto-tag-eval-replay/test_*.cjs`.
- Cross-language signature and actual coordinator joins: `python3 -B .amp/in/artifacts/owner-publication-20261114/check-native-signatures.py`.
- Frozen source/executable correspondence: `python3 -B .amp/in/artifacts/owner-publication-20261114/verify-checkpoint.py`.
- Launcher source ordering only: `node .amp/in/artifacts/owner-publication-20261114/check-launcher-order.cjs`.

The package-only no-debuginfo override reduces disposable build storage. Dependencies retain their build profile. No repository build configuration, runtime limit/reserve/deadline or OS setting changed. The executable is test-only, not a packaged application or attestation. Exact replay depends on retained inputs and the existing toolchain; this packet is not a fresh-clone replay claim.

## What the checks distinguish

The explicit owner publisher takes an already issued signature, rechecks the current frozen owner/checkpoint/native context and verifies the signature. It does not load a signing key or consume approval. macOS uses an owner-only unique temporary file, file sync, no-replace rename, directory sync and exact readback. Tests cover wrong owner/checkpoint/signature/context, unsafe directory, preservation of an existing file, exactly one winner between two racing publishers, mode/single-link/readback and temporary cleanup. Deleting the disposable signer before publication/readiness proves those paths do not reopen it.

The exact broker readiness operation checks native monitoring/admission stop state and returns pending only for a missing fixed file. Ready includes the canonical approval hash. Invalid custody, claims or callers fail through the stop latch. Actual broker tests preserve direct-ingress controls while separately exercising readiness controls, consumption/replay denial and the stopped/consumed states. Every case retains the 216-slot denominator and zero model reservations/credential lookups at the closed boundary.

Two actual Node coordinator invocations use real broker RPC for readiness, consumption and reports, joining observed approval hashes to current immutable scope. Their `scope` operation is an explicitly labelled local component seam because the component broker has no native supervisor monitor. These are not native-deadline or fresh launcher-workload proofs. `native-signature-audit.json` separately reconstructs four actual Rust signature witnesses across OpenSSL PKCS8 v1/ring PKCS8 v2 and joins the two Node receipts. Five synthetic approval snapshots and 160 rejected corruptions remain covered by the Node suite.

The launcher keeps fixture signing separate, omits the native scripted-model fallback and deliberately stops after coordination, before pending execution. Its source ordering is checked separately from permanent behavioral tests: `launcher-source-review.json` is not a runtime workload result. Node behavioral controls cover pending, scope exhaustion/invalidity, receipt drift, storage failure and nonretry of ambiguous consumption.

## Limits and retained history

Owner-side publication is separately bounded to 8 KiB per approval and refuses scoped evaluation writers; it cannot become an unaccounted in-process evaluation writer. There is no new whole-run storage-fit measurement. The original 512-MiB logical-growth / 1-GiB cumulative application/VFS-write caps, 2-MiB/8-MiB terminal reserve and 300/600-second supervision scopes are unchanged.

Publication after rename can be unknown if directory sync/readback fails. A visible valid file does not prove the publisher returned or directory sync completed. Do not retry an unknown publication or consumption. Existing targets are never automatically replaced or removed. No injected post-rename failure, power-loss, physical-I/O, interrupted-write erasure, malicious-host isolation, human presence, live routing/billing/retention or quality claim follows. Publication on other platforms is explicitly unsupported.

Compiler/setup failures and unfrozen intermediate passes remain in their own logs, not successful final gates. The storage boundary retains its actual pre-fix failure as well as the corrected result. The native missing-fault-barrier negative control intentionally emits a caught panic, and corrupt SQLite Node controls intentionally emit tracebacks; final exit/results determine their disposition. Historical broader ignored/adverse/Clippy/display/credential gates and previous six-job evidence are not relabelled as fresh on this source.

Trusted owner invocation, non-fixture terminal response publication/complete quality auditing, credential-source compatibility, a fresh source-bound supervised rehearsal and a fresh price/risk statement remain implementation work. Separate explicit one-request approval is still required later. Existing DeepSeek selection, $5 estimate-based allowance, custody, prompt-preview waiver and retention decisions are unchanged. Nothing is requested from the user for this offline slice.

No commit, staging, push, fetch, external request, OS/account change or unrelated cleanup was performed. Prior frozen packets, raw evidence/media and unrelated archive/cache remain intact. This work remains local and uncommitted above the recorded HEAD.
