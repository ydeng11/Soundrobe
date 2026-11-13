# Final independent mock-safeguards review

The final same-model read-only reviewer found no blocking defect for the scoped, trusted-host, mock-only acceptance of source66 and run69. This is not live readiness or a general privacy or security certification.

This record transcribes the findings delivered by subagent `TU-034wiP6ls5K0jOxyhf3VNy`, titled "Review final source66 run69 evidence", in thread https://ampcode.com/threads/T-01a1be8d-794b-75ce-a0fb-4aca6f65d0db. It is an author-recorded review report, not a signed attestation. The reviewer performed read-only forensic inspection, including hashing, immutable SQLite queries, public-fixture signature verification and retained-byte analysis. The reviewer ran no workloads, gates, services, external calls, configuration or credential reads, and made no source edits.

## Independently checked bindings and completion

- HEAD was `727482d98fd2e30468a50ccd6750a59817f6d457`.
- All 116 source66 bindings, 134 run69 source/tool locks and 1,668 receipt source/tool entries matched. The receipt has 1,667 unique paths. Its duplicate is the same MusicBrainz fixture with an identical binding.
- Launcher, receiver, provisional receipt and published results hash links agreed. Provisional artifacts alone do not authorize successful completion.
- Both audit stderr files were zero bytes.
- The public test-fixture Ed25519 signature verified. Reconstructed checkpoint, approval scope, nonce, expiry and receipt agreed. Durable approval consumption was one.
- Native acceptance followed observed successful supervisor exit before the absolute deadline. Generation was `3ca093db-26f1-438f-8a7b-984bb8048d86`. Preparation took 216,811 ms. Supervisor exit was observed at 551007022, acceptance was at 551007055, and the deadline was 551162293. Headroom was 155,238 ms.
- Launcher and receiver exited zero without a signal, with actual exit observation. Recorded receiver, supervisor and execution groups were absent when checked. Later absence does not change earlier unknown containment observations.

## Independently checked requests and observations

The reviewer joined authority operations, fixture ingress and inventories. All 497 operation IDs were unique and settled. Preparation contributed 236 provider and twelve inventory operations. Execution contributed 236 provider, twelve inventory and one model operation. These are distinct populations, not additional model calls.

The 863-byte request matched frozen preparation, the independent request file, inventory and authority digest. The 296-byte seeded response matched inventory and receipt. Actual fake-supplier lookups and durable admissions were each one. The live Report's `actualFakeLookups=1` was the sole intended difference from the persisted receipt.

The reviewer recomputed 72 occurrences and 216 unique field slots across six executions and twelve sessions. Actual SQLite worker rows matched the exact declared projection for all six jobs and agreed with native JSONL. Static reasons were absent, `discovery_budget_limited`, or `ai_genre_fill` as appropriate.

## Independently checked retained privacy and storage

All 550 privacy71 hash and size bindings matched, including outer stdout and stderr. No forbidden fixture canary occurred outside the explicitly seeded input. Accepted values appeared only in three typed observation documents, one worker database and eleven disposable FLACs. The worker database's one raw occurrence matched the permitted logical evidence value. Independent FLAC parsing placed every accepted occurrence in a GENRE comment, not audio payload.

The exact broker TRACE and all twelve coordinator activation witnesses were present. Coordinator logs totaled 594,446 bytes. Lofty uses INFO while project and transport tracing remain enabled.

The reviewer replayed 2,511 storage records with 205 owners. Ordinary reservations were 318,398,108 growth bytes and 493,816,165 write bytes. Including terminal reserves, totals were 320,495,260 and 502,204,773 bytes. The stop latch was false, with no ambiguous ledger tail. These are nonrefundable prepaid application/VFS reservations, not measured physical disk writes.

## Resolved findings and retained limits

Earlier custody, approval-expiry, reporting, worker-projection and analysis-path findings were resolved in this scope. The final expiry check follows the post-journal seam, uses only verified durable approval expiry and stops on failure without refund. Broker-only model-send admission follows the supplier call. The maintained auditor checks actual worker rows without replacing the native completion gate. The privacy helper rejects canary keys, distinguishes JSON-pointer tokens and accepts only declared document wrappers. No remaining concrete false acceptance was found for the inspected retained sinks.

The reviewer inspected completed author-run helpers67 with twenty controls, faults68 with eleven controls and successful check55 output. These were not independent gate reruns. Ten faults establish group absence. `anchor-running-loss` retains unknown containment at return.

Each of four negative controls retains 216 requested slots, zero validated and 216 unavailable, without completion promotion. Privacy binding sets matched 366, 344, 343 and 343 files. Rejected proves actual downstream rejection without false service failure. Status, schema and cancel prove receipt of seeded responses and unchanged media, not downstream rejection or cancellation. Interrupted actual supplier counts and total wire attempts remain unavailable, not zero.

The original status hot journal was preserved. Earlier independent reconstruction applied its four page images in memory and matched the recovered forensic copy. Recovery of a copy supplies evidence, not execution authority.

Previous failed runs and failed analyses remain adverse history. No claim follows about power-loss durability, unavailable or unflushed bytes, interrupted-mid-write privacy, malicious-host containment, real approval trust, production credential custody, remote cancellation, billing cessation, provider identity or usage, pricing, AI quality, or legacy paired authority.

## Review anchors

| Record | SHA-256 |
|---|---|
| source66.json | 4012cf8927b4045926864b8fe428c627b17c1aaff2610f83ace9de747059e4b7 |
| server-tests43 | f8e7b3767c11d0d67c53ff5cfb171d56aca2dcc3e3882d4f721de723d858844f |
| audit70.json | 1863fcd5dfab5bb1ad631873de1d0a76783c09936994b42410e348518f59ee88 |
| privacy71.json | 6b7c5079bf3698671ed117e7801232f0749186c916e01285035db9becee2f7b8 |
| audit-privacy.py | b6e13e712484b126f6ce783ac159bd0e942bcbd6e739bddce24e416cc3677d1e |
| run69/source-and-tool-locks.json | 17a41f8afbdb4a73ad49833bbd1aa0b3ef45fcc6d8c85bd7326b4ac16b142faa |
| run69/launcher-outcome.json | e4317c8b37b70df3a45d0014dbb0c378cd44683aca9f4705d2b8da89bac1054d |
| run69/receiver-outcome.json | 92c183f489b93118662a0264744ee98c47121b46997e52cb281ad444ed2dc1bb |
| run69/authority/provisional.json | ac784098898e73ba3b20f543881f6c701aaabf5a577703639036b09cbfd4da4c |
| run69/results.json | 372e6f81a331c59cc3c446efd96790f41673824be3ae7b47cc20db817bea8325 |
