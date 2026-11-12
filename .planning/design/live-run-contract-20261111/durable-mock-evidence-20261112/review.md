# Review and dispositions

Independent subagent `TU-034w1rHiVoKS0elsBegHCr` repeatedly reviewed the evolving
mock boundary. These dispositions summarize its messages in this thread. They
are not signed attestations or a claim that every final native result was
independently rerun. No other thread or checkout was created.

| Finding | Correction and executed evidence |
| --- | --- |
| Fixture arrival was not response settlement | Separate durable Dispatch, Arrival and Complete facts; freeze/replacement require settled operations and participant quiescence. Process controls retain killed-before/after-Arrival cases with actual wires. |
| Stop did not govern native writes | Existing WriteQueue blocking guard captures the client and obtains authority admission; native contract covers sealing and stop. Already-admitted writes can finish; no rollback claim. |
| Paired invocation could not supply a global freeze | Maintained evaluator now has separate prepare-only and execute-only sessions; all six preparations precede one freeze/consume. Fresh integration tests the real split. |
| Stop/check-send and error-latch races | Durable Dispatch and Stop share immediate transaction ordering; the short admission mutex additionally serializes fallback error latching. Storage-fault controls deny later sends. Socket-byte cancellation is not claimed. |
| A released process lock allowed restart | Durable one-shot ClaimBroker plus exclusive advisory lock; process controls deny restart after death without registry-byte changes. |
| URL normalization changed admitted category | Validate final parsed path/query before reservation and send that parsed URL; native negatives include dot/encoded-dot/backslash paths. |
| Attachment/sealing and Finish were optional | Native provider/model, preparation completion, execution Finish and final publication require their stage's attachment/sealing/liveness invariants. Retained unattached-Finish red and fresh authority tests cover denial. |
| Prepared one-request plans could not restore | Validate count against recorded request events and role allowance; replay comparison count restarts, durable global allowance does not. Retained native run-01 and restore red precede correction. |
| Orphan/CONNECT/oversized/aborted ingress only denied locally | Registered fixture reports terminal denial; fixture admission is poisoned before notification. Real-broker controls check terminal authority and no later wire. |
| Failed denial notification let fixture resume | Required fatal callback abandons persistent fixture lease on ambiguous notification. `notification-red-01.log` reproduces the earlier failure; controls-04 checks the real-broker correction. |
| Missing/terminal final receipt could publish complete | Require owner release, clean exit and non-null complete receipt matching identity/checkpoint/membership/bindings/sessions/operations/counters. Post-publish-crash and altered-receipt controls deny completion. |
| Attachment hashed a different preparation read | Own one bounded preparation snapshot for hashing and parsing; native buffer test and execution bindings cover the correction. |

The reviewer's final narrow check independently reproduced a network-free fixture
control with stubbed IPC: initial 409, later 409, one fatal callback and only one
`ingress_denied` IPC. It found no remaining blocker in that path. It did not run
the full native selection or the final real-broker controls; those are separate
author-executed evidence. Earlier source-reviewed findings are not retrospectively
represented as independent runtime reproductions.

Author review inspected the tracked diff and new journal, broker and control
files for lifecycle bypasses, dead state, route drift and duplicated authority.
No application broker entry point or general live framework was added. Legacy
mock diagnostics remain separate and their compatibility tests pass. Known
limits and the historical label-binding failure are documented rather than
removed from the results.
