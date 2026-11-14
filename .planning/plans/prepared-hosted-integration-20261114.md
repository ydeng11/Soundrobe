# Prepared hosted integration plan

## Authorized outcome

Complete remaining integration offline, keep external calls disabled, and return only concrete approval decisions. Reuse the existing request interface and evaluator. Do not read real credentials/private configuration, provision real signing material, change account/OS settings, access original media, relax caps/deadlines, commit, push or activate calls.

## Design and choice

Extend the existing native capability and supervised rehearsal rather than adding a parallel evaluator or a live environment switch. Public fixture plans remain non-authoritative. A native-only owner-file verifier loader can prepare an independently pinned approval context, but does not generate keys or enable HTTPS. File custody is trusted-host only. Existing public fixture signers must be rejected for this context. Credential compatibility is exercised with disposable files only.

An optional `offlineResponse` object (`sha256`, `status`) is allowed only with `offlineHosted` and `offlineHttp:true`. Its bytes live at the fixed sibling `response-fixture.json`, outside the run root, and must be source-locked and <=65536 bytes. This is public synthetic data, never an arbitrary credential path or external URL. The native fixture, driver and independent auditor bind the same bytes/status. Absent this option, old request/response receipts remain unchanged.

The full model join derives accepted/withheld/rejected projections independently instead of assuming the accepted canary. Accepted projection provenance and readback must agree; withheld results must not invent assisted metadata. Rejected and incomplete responses remain unavailable/failure evidence, never successful model fills. Existing requested membership, source/storage/publication/completion and mock-only zero-credit rules remain.

## Verifiable units

1. Add a native owner-file trust loader with failing tests first. Validate pinned digest, exact key bytes, owner-only regular single-link/no-follow bounded reads, and reject every known public fixture key. Exercise authentic disposable signatures through the Journal and keep pinned HTTPS closed.
2. Add public non-fixed response plan/native/driver support with failing tests first. Preserve historical behavior and immutable locks. Test missing/hash-drift/oversize/wrong-mode inputs without touching real configuration.
3. Independently extend `audit-supervised.py` and `test_supervision.cjs` for exact source-locked non-fixed bytes, all preparation/worker/readback/scoring joins, and semantic corruptions. Extend retained historical adaptations only with counted changes; do not modify sealed sources.
4. Run combined Node/native/format/type checks; inspect all substantial diffs. Rehearse one fresh six-job non-fixed-response workload using fake credentials, original caps/deadlines, and a new output root. Re-audit completion, storage, privacy and frozen sources/executable. Preserve failures without resuming/promoting prior roots.
5. Document a concrete approval-storage proposal and a final risk/run-approval sequence. Record remaining implementation truthfully. No live request, provisioning, commit or push is authorized by this plan.

## Ownership and evidence

The main thread owns native trust, transport/plan/bootstrap, driver and final integration. A bounded parallel agent owns only the supervised Python auditor and its CJS test file. New evidence goes under `.amp/in/artifacts/prepared-hosted-integration-20261114/`. Exact replay retains ignored local inputs; mock proof is not live identity, billing, power-loss, malicious-host or general AI-quality assurance.
