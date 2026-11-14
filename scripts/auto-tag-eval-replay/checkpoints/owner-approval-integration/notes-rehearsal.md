# Prepared hosted integration

This is an offline implementation and verification record. Calls remain disabled.
No commit, push, fetch, OS/account change, real signing-key provisioning or
original-library mutation is authorized by this checkpoint.

## Source and inputs

Base: local `8b4087b1ebb63b60450a8128b04e31ab44f8371a` on
`artist-library-inventory`. Ten tracked implementation/test/operator paths are
modified, with an empty index. The pre-existing archive and Python cache are
unrelated and remain untouched.

`source-manifest.json` binds 1,913 source/input entries, the binary and patch.
`source.tar.gz` retains those exact entries. `native-test-executable` is an APFS
clone of the compiled test binary, SHA-256
`736efd5174a8dc1994f6938e2cf57978ac5f3e584dbfcc7b0c639542d6d532f8`.
This is source/executable correspondence, not executable attestation or
fresh-clone exact replay. Retained ignored dummies, labels and historical auditor
inputs are still required.

The plan uses the historical public DeepSeek alias/target/provider snapshots.
Those snapshots are not fresh availability, price, retention or identity claims.
The new public response is `OFFLINE_PUBLIC_RESPONSE_GENRE`, confidence 0.9,
usage 17 prompt / 6 completion / 23 total tokens. Missing reasoning usage
normalizes to zero. All of this is synthetic; no model generated the answer.
The only credential supplied is the deliberately public fake sibling file.

## Implementation boundary

- `ApprovalVerifier` loads independently pinned owner-only, same-owner,
  single-link, non-symlink exact 32-byte verifier material. Known public fixture
  keys reject. A disposable random signer exercises frozen Journal approval and
  replay denial. This does not generate or provision real signing authority.
- Optional source-locked `offlineResponse` metadata selects only the bounded
  fixed sibling `response-fixture.json`, with exact bytes/status through native
  numeric-loopback HTTP, the worker and independent audit.
- Accepted and withheld results have distinct provenance/readback rules.
  Rejected and incomplete responses cannot pass a completed-rehearsal audit or
  gain quality credit. Default historical audit sources remain pinned.
- Hosted HTTPS admission remains unconditionally disabled before credential
  lookup or request reservation. There is no external activation switch.

## Verification record

Fresh completed component results: authority suite 109 passed / zero failed /
10 ignored; all Node contracts 35 passed / zero failed or skipped; TypeScript
passed. Native HTTP fixture controls are included in the authority suite.
Provider/review fixtures passed eighteen tests. Retained-byte controls pass two
positive cases and reject seven corruptions, including unclassified page bytes.
`trust-red.log`, `response-native-red.log`, driver red/green and auditor red/green
logs retain development failures separately from the passing checks.

Fresh full server-library gate passed 882 tests, zero failed, with 31 declared
ignored tests. The six-job `run-01` completed with receiver/launcher exit zero
after witnessed actual successful supervisor exit. Its full audit reconciles
216 requested/observed slots, twelve sessions and 497 settled operations:
472 provider, 24 inventory and one native HTTP observation. There are 496
ordinary fixture arrivals, not 497. Grounded credit remains zero and agreement
is 0/22 for the deliberately non-answer synthetic genre.

Preparation took 219,028/300,000 ms. Execution through acceptance took
456,479/600,000 ms. Nonrefundable reservations including unchanged terminal
grants were 457,476,559 logical-growth bytes and 774,170,785 cumulative
application/VFS-write bytes, under the original 512-MiB/1-GiB caps. The terminal
grants remain 2 MiB/8 MiB. The audit replayed 4,058 storage ledger records.
These are reservations, not physical-device-write measurements.

Retained-byte privacy passes all 563 file bindings, including outer logs, with
zero forbidden/rejected disclosures. Two counted changes extend the previous
seven-change privacy checker chain for the new public sentinel and exact
source-locked response hash. Typed sinks and strict raw Journal-page counts
remain. Fifteen semantic corruptions of actual completed-run JSON reject in
memory, without rewriting the run. These are model joins, not native fault
scheduling or full recovery-matrix tests.

`verification.json` matches all 1,913 frozen source/archive bindings, the
matching built/retained executable, 1,672 receipt-binding occurrences / 1,671
unique bindings and all 563 privacy bindings. Five recorded actors and three
groups are absent. The workload and serial native gate ran concurrently in
separate processes/output roots. No isolated-host or performance claim follows.
All results are fresh for this source; no new desktop/renderer gate, strict
Clippy, credential/display/packaged smoke or independent runtime review ran.

The frozen scratch verifier has one incorrect expected field: it expects absent
reasoning usage as null. `verify-result.py` makes one counted correction to zero,
then executes the same frozen checks and requires the new full native gate.
Neither run evidence nor runtime/auditor source is changed by that correction.

## Concrete decisions for later live preparation

Already settled: existing DeepSeek interface/model and `.env.local` source,
US$5 experiment allowance, waived prompt-preview review, documented provider
retention defaults for scoped public metadata, and local evidence until review.

Recommended approval custody: authorize creation of a new owner-only
`~/.soundrobe/evaluation-approval/` directory (0700) with a separate random
Ed25519 signing key and verifier (0600). Never overwrite existing material or
put the signing key in the checkout, logs, manifests or child IPC. Present the
public fingerprint in the final run-specific approval. Keep the private key
outside retained result evidence until separately authorized cleanup. This
trusted-host file proposal is not OS-backed custody, human-presence proof,
malicious-worker isolation or malicious-host protection. No provisioning has
occurred, and no Keychain/OS/account change is proposed.

Billing: choose a verified provider-side US$5 ceiling, or review a fresh quantified
estimate and explicitly accept its residual billing uncertainty. The historical
text illustration is not a current quote or enforceable ceiling. A single paid
attempt can be charged after timeout or rejected response. No retry, account
purchase or top-up is authorized. The US$5 allowance alone does not accept
unknown or unbounded exposure.

After those decisions, real native bootstrap/credential compatibility,
non-fixture approval wiring and applicable hosted-response publication/auditing
still need completion and offline verification. Only then freeze the exact run,
request, endpoint/pricing/terms and executable, and ask for separate execution
approval. Provisioning or risk acceptance does not authorize the call.

## Known limits

Trusted-host/public-fixture proof only. No real-model quality measurement, real
credential compatibility, live TLS/connectivity, remote-identity assurance,
provider-billing guarantee, physical-device-I/O bound, power-loss durability,
interrupted-write privacy, malicious-host containment, guaranteed anchor-loss
containment, packaged-app smoke or independent runtime review is established.
Historical failed/ignored gates remain historical. No cap, reserve, deadline,
scoring label, original media or sealed auditor is changed. Grounded assistance
credit remains zero; a successful synthetic fill is not an accuracy result.
