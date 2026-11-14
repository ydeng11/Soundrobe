# Offline owner signing and independent signature audit

## Authorized outcome

Continue signing and auditing offline, with calls disabled. Retain the $5 allowance, estimate-based billing, existing caps/deadlines, default remote retention for public metadata and local evidence until review. Do not use the provisioned signer or real credentials, sign a real run, change the OS/accounts, commit, push or fetch. Preserve preceding uncommitted slices and frozen evidence.

## Design and ownership

The existing native test-build Journal is the scope authority. Add an explicitly called owner-side signing method that loads its own current frozen state, checks the owner handle and independently supplied expected checkpoint, and constructs the existing exact signed payload. Reject unfrozen, consumed, stopped, fixture, lost-context or mismatched-context runs before loading a signer. Reuse the existing claims checks for nonce, scope and bounded expiry. Load only the fixed `approval-signing-key.pk8` in a canonical owner-only directory, with existing bounded no-follow regular/single-link custody checks, and require its derived public key to match the native verifier. Accept OpenSSL PKCS8 v1 and ring PKCS8 v2. Return the signed approval in memory; publishing an owner file and launcher coordination remain separate. There is no new signer IPC, environment selector, automatic signing or activation switch.

The independent Python auditor receives the frozen report, approval, raw public verifier, independently supplied verifier digest and verification timestamp. Reconstruct the frozen checkpoint and signed scope, verify Ed25519 with OpenSSL rather than importing Rust signing logic, reject public fixture keys and malformed/expired/drifting evidence, and report signature verification without live authority, user-presence or model-quality claims. This is a component audit, not completed-workload acceptance. Existing mock success audits must continue to reject native-prepared evidence.

## Verifiable units

1. Write failing native controls for journal-bound signing, OpenSSL/ring compatibility and unsafe/mismatched key custody. Implement the smallest signer path and reuse claims validation.
2. Independently implement and test only the Python auditor and existing Node supervision test file in parallel. Cover valid random signatures, wrong pins/keys, byte mutations, rehashed scope/checkpoint drift, expiry, types and authority promotion.
3. Exercise the new issuer through the existing actual broker socket/owner-file tests, retaining HTTPS denial and zero credential lookups/reservations after valid approval. Independently audit actual native-produced public evidence using disposable keys; retain no private key bytes.
4. Run focused native, combined authority and Node checks, format, syntax, whitespace and typecheck. Review the incremental diff. Preserve red/green logs and freeze current source/executable correspondence under `.amp/in/artifacts/offline-owner-signing-20261114/`.
5. Update the execution decision artifact with exact tested behavior and remaining implementation. Do not relabel historical six-job runs or full gates as current-source executions. No fresh six-job workload is promised by component tests.

## Remaining integration after this slice

Trusted owner invocation and atomic owner-file publication within the unchanged supervision deadlines; launcher coordination without public fixture signing; non-fixture terminal response/publication and complete independent quality audit; a fresh source-frozen supervised rehearsal; pricing freshness and final concrete run-specific execution authorization. Hosted dispatch stays unconditionally closed.
