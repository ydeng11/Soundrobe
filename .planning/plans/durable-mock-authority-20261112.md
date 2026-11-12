# Durable mock run authority

## Authorized slice

Continue the approved protocol in the existing checkout above cold-only commit `5f8389e1afc966c3eb9bd7ba9b21684035cd6343`. Implement a native test-only SQLite authority, not live approval or a general transport framework. No commits, pushes, fetches, real credentials or original media operations are authorized.

## Ownership decisions

Use a single native broker with an isolated fixed `registry.sqlite` filename and exclusive process lock in the declared local test registry. Driver and child handles are opaque mock capabilities, not authenticated-user approval or protection against malicious same-user code. A recovered incomplete registry permits bounded read-only reporting, never resumed dispatch. SQLite transactions use immediate locking and synchronous journaling. Successful process-restart checks do not establish power-loss durability.

The broker forwards only the existing loopback provider, compact-inventory and scripted genre routes. It commits a reservation before forwarding and requires a separately journaled fixture acknowledgment before response delivery. Arrival and bounded response settlement are separate durable facts; freeze and service replacement also require native-process quiescence. No SQLite lock spans a network wait. A stop revokes admission before waiting for admitted forwards to settle; stop acknowledgment is not issued while forwards remain active. A previously admitted network/write operation can race revocation; no new admission follows stop. The existing native WriteQueue guard obtains broker admission immediately before writes, including its blocking-task checks. Missing or ambiguous authority fails closed. Control messages use bounded local Unix IPC, not the provider HTTP accounting category. Native clients and driver carry scoped handles; services acknowledge with registered replacement-specific handles and actual method/route/body identity. No general URL, external endpoint or credential-supplier operation exists.

One identity retains all six jobs, 72 observations and 216 slots from preparation onward. Prepare every job before the single freeze and exclusive fake consumption, then execute on retained files with fresh stores. Bind each preparation export, exact request and existing source/tool/label receipts into the frozen checkpoint. Reject changed/stale/replayed handles, unresolved operations, duplicate consumption and mismatched arrivals. Never import service arrays as authority. Preserve provider ≤512, inventory ≤32, model ≤1 and total ≤576 counters without refund, including failures. Natural zero-model holds remain valid.

A direct shared SQLite file for every JavaScript/native caller was rejected because it duplicates lifecycle rules across languages. Merely counting fixture ingress was rejected because it cannot reserve before forwarding. Broker forwarding concentrates the mock send boundary without changing ordinary desktop provider APIs or eligibility. Legacy diagnostics remain outside this explicitly selected authority mode.

## TDD and verification sequence

1. Add journal-contract tests for membership, exclusive freeze/consume, category counters, unknown operations, stale/rebound capabilities and terminal denials. Observe failures before implementing transactions.
2. Add multiprocess contention, journal-lock/unavailable-storage and crash-window controls. Preserve their raw receipts and assert actual loopback arrivals, not counters alone.
3. Integrate the narrow broker, scoped fixture acknowledgment and native handles at existing HTTP ownership boundaries. Keep no-retry/no-repair transport and WriteQueue ownership.
4. Factor maintained preparation/execution into prepare-only (cannot fall through) and execute-only (cannot materialize, profile or prepare). Validate paired baselines before freeze; execute on the exact retained physical files with new execution stores and newly registered services. Extend the maintained driver and incomplete envelope, not another evaluator or scorer.
5. Run a fresh source-bound six-job mock selection where practical, with complete operation reconciliation and all 216 requested slots on failure. Independently review concurrency/durability/admission changes and resolve reproduced findings before combined checks.
6. Update the existing artifact with actual evidence and limits. Preserve historical labels/seals and raw failures. Report uncommitted delivery state to the source thread.

## Explicit limits

No VFS/storage tickets, physical-flush guarantees, independent supervisor, hard whole-run deadline, malicious-host containment, real credential custody, live approval, billing or privacy-retention assurance is implemented here. A write already admitted by WriteQueue can race stop; no rollback is promised. Unexpected broker/service death or ambiguous admission is terminal. If the broker cannot prove a safe transition with these minimum dependencies, retain the full incomplete denominator and report the narrower blocker rather than invent completion.

For this authorized mock slice, the fake verifier and these explicit exclusions take precedence over the older live design's unresolved approval/credential prerequisites and mandatory VFS/supervisor gates. Those older proposals remain unchanged and are not runtime proofs.
