# Owner-file publication and launcher coordination

## Scope

Continue offline integration only. Keep all calls disabled, the $5 estimate-based allowance, existing caps/reserves and 300/600-second supervision scopes unchanged. No provisioned signer or real credentials, real-run signature, OS/account changes, commits/push/fetch, original-media operations or cleanup of prior evidence. Preserve preceding uncommitted work and frozen packets.

## Design

Add an explicit owner-side Journal publisher for an already signed approval, separate from signing and unavailable through IPC. Refuse scoped evaluation processes before creating any file so this separate, at-most-8-KiB owner operation cannot bypass their shared storage ledger; it is not new whole-run fit evidence. Reuse current frozen owner/context checks and verify the signature/checkpoint before writing. Write at most8192 bytes to a unique owner-only temporary sibling, sync the file, atomically rename without replacement to fixed `prepared-approval.json`, then sync the directory and read back exact bytes. Existing targets, unsafe custody and stale/wrong approvals fail; do not overwrite or delete existing owner files. macOS uses native RENAME_EXCL; other platforms explicitly remain unsupported for publication in this slice. Failed pre-rename temporary files are removed; failure after rename remains publication-unknown with the file retained and no automatic retry.

An exact owner-only `owner_approval_status` broker operation checks the current frozen native context, returns pending only for a missing fixed approval file, and returns ready plus checkpoint/approval hash only after custody and signature verification. It neither signs nor consumes approval. Unsafe/stale/invalid evidence fails through the existing stop latch. Check the monitor/admission stop state before status and existing consumption commands.

The launcher retains the frozen report, keeps fixture signing unchanged for fixture runs, and uses separate native-prepared coordination. Query the unchanged native remaining scope while waiting; pending is not an error/retry of a mutating operation. Consume only once with `approve_owner_file`, then join durable consumption to the observed approval hash/run/checkpoint and zero model/credential activity. Retain only a public coordination receipt, never a fixture signature for native mode. Native-prepared main execution deliberately stops after coordination; no native success scoring/publication or call activation is enabled.

## Verification

1. Red/green native file publication and pending/readiness controls through actual broker sockets with disposable keys. Check atomic/no-overwrite/mode/readback, wrong scope/signature/context, missing versus unsafe files, consumed/stopped runs, and no signer/key loading by status/launcher.
2. In parallel, TDD the JS coordinator and launcher branch in the existing Node files. Cover pending→ready, native remaining scope exhaustion/unknown state, strict markers, changed hashes, one consume attempt, errors and fixture compatibility.
3. Invoke the actual JS coordinator against the native broker in a component control, with an explicitly labelled local test scope seam if no supervised monitor exists. Keep HTTPS denial/zero reservations/lookups and216 slots. This is not a fresh six-job workload or native-clock proof.
4. Run combined authority and Node checks, formatting/type/syntax/whitespace and incremental source review. Preserve failures and avoid duplicating tests/builds. Reuse no prior gate as a new-source workload. Disk remains tight; only this turn's disposable build outputs may be removed if needed.
5. Freeze source/executable/evidence correspondence in `.amp/in/artifacts/owner-publication-20261114/` and update the execution decision artifact. Report remaining implementation without reopening settled user decisions.
