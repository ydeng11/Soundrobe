# Native prepared bootstrap and approval wiring

## Authorized outcome

Continue integration offline with hosted calls unconditionally disabled. Estimate-based billing and a $5 allowance are approved, as is owner-only local signing-key custody. No real-run signature, live request, account/OS change, commit or push is authorized. Existing provisioned keys are not used by this work.

## Design

Extend the maintained test-build native broker. A strict public `nativePrepared: true` selector is not authority and cannot carry key paths, verifier bytes, pricing authority or activation. Native bootstrap loads `~/.soundrobe/evaluation-approval/prepared-bootstrap.json` from the current owner's account-home directory, not HOME or a child-selected path. The owner-only file binds the exact canonical public plan hash, independently pinned verifier digest, text pricing evidence, and the native credential-file path. It contains no signing key. File trust is trusted-host custody, not malicious-host isolation.

The owner directory and bounded files must be canonical, same-owner, owner-only, regular/single-link and no-follow. The fixed verifier filename is already provisioned, but no real bootstrap configuration is written in this slice. Public fixture keys and fixture options remain rejected. Native construction keeps the pinned HTTPS sender closed before credentials/reservations.

A new exact broker-only `approve_owner_file` IPC operation carries no path or signature. It reads the fixed owner-side `prepared-approval.json`, then invokes the existing durable signature/scope/expiry/single-use approval verifier. It never signs. Ordinary fixture mode cannot use this operation; a public fixture signature cannot approve native-prepared execution. Invalid requests retain the existing stop latch. Public launcher automation must fail explicitly for native-prepared plans instead of silently substituting its fixture signer.

## Verification units

1. Red/green disposable-file bootstrap controls: correct context and no credential read; wrong plan/pin/pricing, fixture flags, unsafe files/directories and missing configuration fail.
2. Red/green actual broker socket controls: disposable owner signature consumption, public/wrong signatures, scope/expiry/replay/path injection/missing file rejected. Pinned HTTPS denied with zero model reservations and credential lookups.
3. Native process bootstrap uses the same tested constructor and fixed current-owner directory; old fixture constructors stay unchanged. No real files are created or signed during tests.
4. Focused native suite, combined authority suite, Node supervision helpers, formatting, whitespace and source-diff review. Preserve red and green logs under `.amp/in/artifacts/native-prepared-bootstrap-20261114/`.
5. Record exactly what is and is not integrated. Existing frozen six-job evidence predates these changes and is not a current-source workload. Run-specific approval issuance/activation and live-response acceptance remain separate, disabled work.

## Delivery

Local uncommitted source and reproducibility records only. The ten existing modified paths from the preceding accepted offline slice are preserved; its source-bound workload evidence is not relabelled. No original media, real credential bytes or provisioned private signing-key bytes are accessed.
