# Offline owner-approval integration delivery

This commit delivers public-response rehearsals, native bootstrap, owner signing, independent signature auditing, and owner-file publication and coordination. Calls remain disabled. No real run was signed, no real credential or provisioned signer was opened, and no original media was used.

The containing commit on `artist-library-inventory` has exact parent `8b4087b1ebb63b60450a8128b04e31ab44f8371a`. `bindings.json` identifies twelve implementation paths, four plans, twenty unchanged copied records, and retained witness provenance. `SHA256SUMS` covers every other packet file. No executable, raw media, large log, private configuration, credential, or private-key file is included. The four native signing witnesses contain public disposable test data, not production approval trust.

## Delivered behavior

Different public responses can pass through the native loopback worker, typed provenance, readback, scoring, and privacy checks. Native-prepared bootstrap rejects fixture trust. Explicit owner-signing controls bind a signature to the frozen checkpoint and independently held owner verifier. The independent Python audit reconstructs the checkpoint and signed scope.

The macOS publisher accepts an already issued signature, not a signer or caller-selected path. It writes at most 8 KiB to a unique owner-only sibling, syncs, renames with no replacement, syncs the directory, and checks readback. Scoped evaluation writers are refused before publication, preventing an uncharged write. Existing targets are never overwritten. Other platforms reject publication explicitly.

Readiness distinguishes a genuinely missing fixed file from invalid custody or approval. The Node coordinator checks scope before each observation and consumption, consumes once, and joins the report to the observed approval hash and immutable scope. Ambiguous mutations are never retried. The launcher deliberately stops with `native_prepared_execution_disabled` before pending execution and omits the native scripted fallback. Hosted HTTPS still rejects before reservation or credential access, including after valid approval.

## Verification and retained history

`validation.json` separates fresh delivery checks from earlier source-bound workloads. Fresh delivery checks pass 46 Node tests without skips, 22 fixture/review/provider tests, TypeScript, formatting, syntax, and whitespace. Fresh read-only checks independently verify the four disposable Rust signature witnesses and two actual Node/native coordination receipts. No new native matrix, full server gate, or six-job workload was run for delivery.

The latest retained native authority gate passes 112 tests with zero failures and ten ignored. Thirty actual broker scenarios cover owner-file controls. The native scope regression first demonstrated an unaccounted publication, then passed after the guard refused the write without changing the ledger. Five accepted synthetic signature snapshots and 160 rejected corruptions are covered by the Node tests. Initial compiler/setup failures and adverse intermediate gates remain retained.

Two actual Node/native coordination receipts use native broker RPC for readiness, consumption, and reports. Their `scope` operation is a labelled local component seam, not proof of the native supervisor clock. Launcher ordering is source-reviewed, not an executed launcher workload on the final source. No post-rename failure was injected.

The earlier different-response six-job rehearsal accounts for 216 requested/observed slots, twelve sessions, 497 settled operations, 496 fixture arrivals, and one separate native HTTP observation. It reserved 457,476,559 logical-growth bytes and 774,170,785 cumulative application/VFS-write bytes, including terminal grants. Preparation took 219,028 ms and execution through witnessed acceptance took 456,479 ms. Its 4,058 ledger records and successful completion belong to that earlier source, not the latest bootstrap/signing/publication source.

The original limits remain 512 MiB logical growth, 1 GiB cumulative application/VFS writes, 2 MiB/8 MiB terminal reserve, and 300/600-second preparation/execution scopes. There are no refunds or increases. These numbers do not bound physical I/O.

Delivery verification checks all 1,913 archived rehearsal source bindings, 1,672 receipt bindings with 1,671 unique pairs, 563 privacy bindings, and current 323 source bindings. Historical source receipts are checked against their archive, not silently rebound to current files. Bootstrap/signing retained source copies and outer evidence hashes are checked separately.

A delivery-time attempt to rerun today's full auditor against the old rehearsal failed with `ValueError: changed source binding`. All six scoring contracts pin the earlier `run-bounded.cjs`. This is an expected source-lock refusal, not a fresh successful audit or a reason to relax the contract. The original successful audit is preserved unchanged. The retained privacy scan passes again and reproduces every result except the provenance hash identifying today's auditor.

## Known limits and remaining work

Grounded assistance credit remains zero and user agreement remains 0/22. No real-model quality result follows from public scripted replies. Historical strict Clippy failures and ignored or unselected display, credential, packaged-app, renderer, and broader native gates remain explicit. Earlier full server gates of 882/31 ignored and 884/31 ignored are historical, not final-source gates.

These checks assume a trusted host. They do not establish human presence, malicious-host containment, physical-I/O bounds, power-loss durability, interrupted-write erasure/privacy, live billing or retention assurance, executable attestation, or fresh-clone exact replay. A failure after rename can leave publication outcome unknown. A visible valid file does not prove directory sync or publisher return. The target must remain intact and no automatic retry is permitted.

Trusted owner invocation, nonfixture terminal response publication and full quality auditing, credential-source compatibility, a fresh source-bound supervised integration, and a fresh price/risk proposal remain implementation work. Separate frozen one-request execution approval remains necessary later. The existing DeepSeek selection, $5 estimate-based allowance, owner-only custody, preview waiver, and retention decisions are unchanged. Nothing is requested from the user for this delivery.

## Reproducibility

The four `notes-*.md` files, copied JSON records, and plans are unchanged historical snapshots. Statements in those snapshots about being uncommitted or lacking later controls describe their checkpoints. This delivery README supersedes those status statements without changing historical bytes.

Offline checks use the existing local dependencies:

```sh
node --test scripts/auto-tag-eval-replay/test_*.cjs
npm run typecheck
npx vitest run test/scripts/mock-provider-service.test.ts test/scripts/review-auto-tag-subset.test.ts test/scripts/build-auto-tag-eval-suite.test.ts
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

The retained final native authority command is recorded in `sources-publication.json`:

```sh
CARGO_INCREMENTAL=0 cargo test --offline --config 'profile.test.package.soundrobe.debug=0' --manifest-path src-tauri/Cargo.toml --no-default-features --features server --lib mock_run_authority:: -- --nocapture --test-threads=1
```

The package-only no-debuginfo override reduces disposable build storage. It changes no repository configuration, runtime allowance, or OS setting. Rebuilding can change the executable hash and is not an exact-replay promise.

Exact historical replay needs the retained source archive, matching executables, public fixture inputs, generated media, databases, receipts, ledgers, and sealed auditors under `.amp/in/` and `.planning/`. Never substitute the development `.env.local` for a fake public fixture. The latest retained executable SHA-256 is `c38bce5320d534b69df1e6dba87b7e18e8eb2a9c5cc1285eaecd1f4763a16be5`. Raw evidence is not included in Git.

The original `final-check.py` and checkpoint verifiers intentionally assert their pre-delivery HEAD and empty index. They remain unchanged. A separate read-only delivery verifier checks the containing commit's parent, exact scope/blobs, checksums, source copies/archive, receipts, privacy, signatures, and coordination without changing those historical expectations:

```sh
python3 -B .amp/in/artifacts/owner-publication-20261114/delivery/verify-delivery.py committed
```

That verifier and raw delivery logs remain local until result review. A checkout without retained ignored evidence cannot run it. No push, fetch, external activation, real-run approval/signing, OS/account change, or unrelated cleanup is part of this delivery.
