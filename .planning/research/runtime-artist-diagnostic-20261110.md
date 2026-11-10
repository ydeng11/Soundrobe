# Runtime Discogs receipts and diagnostic-only artist correspondence

The implementation binds retained primary-artist observations to the concrete Discogs response that produced them. The offline evaluator can explain a narrow single-primary name correspondence. It does not change requests, aliases, donor eligibility, display, selected IDs, certificates or write authority. Recovery still rejects the same artist-name conflict. Overall tagging safety remains non-clean.

This work is uncommitted on `artist-library-inventory` above `1c74c6c728f0a8ad87dc0a66de08dfd3fdabf5bd`. No commit, push, remote mutation, deployment, new thread or worktree occurred. The historical retention implementation is `49ab739bf77617d4dc948c397697e6fedd28e98f`. Earlier planning and seals remain historical records, not rewritten claims about the new source.

## Origin is captured where the response arrives

`DiscogsClient::release_metadata_result` obtains bytes through the existing retry path. It parses and hashes the same successful decoded response buffer. The optional receipt contains GET method, concrete release route and ID, sanitized request and response URLs, HTTP status, exact UTF-8 body and SHA-256. No authentication headers, query parameters, userinfo or capture time are retained. Matching positive raw release identity is required. Master, search-only and ambiguous origins cannot produce a concrete-release receipt.

Only exact bodies of at most 2 MiB are retained. Larger responses still follow normal metadata parsing, but receipt-dependent diagnostics are insufficient. There is no truncation. This ceiling does not limit total cache growth, pre-existing transient response allocations, or arbitrary externally replaced cache rows.

Provider albums and both native candidate types carry the optional receipt. Merge retains only the preferred candidate's receipt and observations. Album-content conversion retains the chosen representative's origin while leaving selected edition and recording IDs suppressed. SQLite detail and lookup serialization preserve that same object through reopen. Missing old lineage stays missing. Cache namespaces, lookup policy, discovery limits and migration behavior do not change. No legacy refetch is forced.

The originating runtime object establishes an association with the actual fetched bytes. The replay comparator also checks those bytes against the routed authentic capture. Equality with that fixture is a consistency check, not the source of the runtime association. Hashes do not establish freshness, catalog truth, provider-host attestation, acoustic identity, or protection from a coordinated replacement of both receipt and candidate.

## The identity decision is diagnostic-only

The predicate lives under the test-only offline evaluator. It validates the receipt, exact primary array and selector, positive artist ID, raw canonical and credited-name pair, and candidate ownership. Both artist and album-primary presentation must belong to that bound singleton entity. Content-only provenance does not become the local selected edition.

Original per-file `albumArtist` observations come from this invocation after profile application and before resolver writes. A collapsed request hint or first-track guest is not album-primary evidence. The new capture uses the existing bounded FLAC layout validator and Lofty to enumerate recognized primary comment tuples. It recognizes singular and plural spellings, including spaced aliases. Multiple or inconsistent claims are insufficient. Leading ID3 and unsupported or rewrite-required layouts remain unavailable. The normal reader and writer behavior are unchanged.

Comparison permits whitespace collapse and ASCII case folding only. It does not strip punctuation or entity suffixes, transliterate, simplify Chinese, or infer cross-provider identity. Source joints, guest-only assertions, unknown primary scope and unscoped local Discogs IDs remain insufficient. Request or resolver-filled IDs do not become original scoped IDs. MusicBrainz IDs stay separate evidence. Different individually supported artist IDs for the same visible local claim make the available correspondence insufficient, without merging identities or names.

Synthetic probes exercise an explicit rejection and a provided no-rejection argument. Runtime export has no readable applicable decision store and supplies unknown decisions. Thus a supported name-correspondence subfinding still has an aggregate outcome of insufficient. No invented empty decision set supplies approval. Secondary unmatched folder spellings remain visible without becoming entity conflicts.

Completeness is limited to accepted FLAC/Vorbis layouts and recognized fields. Hashes bracket the raw capture but do not make the earlier DTO read, baseline hashes and resolver run atomic. Inputs here are isolated generated files. Homonym handling covers available, individually supported assertions in this trace set, not exhaustive discovery, unbound or canonical-only assertions, or cross-provider adjudication. Joint-primary policy, original scoped primary IDs and real user-decision access remain unsupported.

## Boundary TDD and corrected-code reviews

All logs and exit receipts below are under `.amp/in/artifacts/runtime-artist-diagnostic-20261110/`.

| Boundary | Retained failing evidence | Final correction |
| --- | --- | --- |
| Originating HTTP, parser, candidate and SQLite | `receipt-red.log` | Capture and parse one buffer; carry optional receipt through native conversions and cache reopen. |
| Original primary scope and diagnostic export | `diagnostic-red-retry.log` | Use actual prewrite original scope, not collapsed guest hints or stripped-profile reviewer material. |
| Receipt size and candidate album-primary tampering | `review-findings-red.log` | Limit retained bodies and validate available album-primary presentation. |
| Repeated standard primary comments | `repeated-primary-red.log` | Do not treat a lossy first-value DTO as complete primary evidence. |
| Competing bound homonymous entities | `competing-entities-red.log` | Downgrade incompatible supported entity bindings to insufficient in invocation-local aggregation. |
| Preferred plural spelling hides a spaced alias | `spaced-primary-red.log` | Enumerate and classify both plural spellings. |
| Leading ID3 fields outside Vorbis completeness | `id3-primary-red-corrected-construction.log` | Return unavailable for a nonzero FLAC marker offset. |

The initial ID3 construction exposed the last duplicate TPE2, contrary to the reviewer's first-value assumption. `id3-primary-red-retry.log` records that failed construction. Reversing the order reproduced false support through the real reader and diagnostic. An earlier diagnostic attempt had an owned-argument compilation error, and one ID3 test build failed for lack of disk space. These failures remain retained.

Final `runtime-final-green.log` records six passes. Three are new receipt or diagnostic tests, and three are existing tests selected by the `runtime_` filter. `retention-final-green.log` records five existing retention-contract passes, including actual content construction, resource namespaces, candidate ownership, cache and legacy compatibility. The final source preserves camelCase candidate wire fields and earlier populated snake-case observation rows. Native serialization is tested; the existing invoke test is mocked, not packaged Tauri integration.

The genuine positive uses the tracked `37231524` response, SHA-256 `1798a65076cb0ab25f376751bf30fbc42677702ec54b5ffe29f4139f0c939b1a`, served through the HTTP boundary. Other controls use labeled synthetic modified receipts, IDs, indices, requests, primary arrays, roles, cache rows, duplicate comments, ID3 frames, size padding and local primary snapshots. They are constructions, not catalog contradictions. Existing guest, homonym, joint-primary, cross-candidate, successive-request, profile-isolation, source-display and content-only-ID controls remain.

The final independent security re-review reports no remaining actionable security defect in the corrected diff. The final independent specification re-review reports no remaining specification blocker within diagnostic-only scope. These are fresh corrected-code dispositions after reproduced findings, not reuse of earlier standards or policy reviews. They do not certify execution, identity policy or clean tagging. Their scopes and limitations are recorded in `runtime-artist-diagnostic-independent-review-20261110.md` beside this report.

## Final paired Crowd Lu replay

The final run uses three assistance-disabled jobs, cold and warm, on eleven unique generated files. That is six fresh evaluations and 66 repeated track observations. The comparator uses six retained baseline evaluations, not six other fresh evaluations. The strict loopback route pool and all job membership, input and budget constraints are unchanged. All 1,783 final execution locks and the executable hash pass after execution.

| Profile | New exchanges, cold + warm | Written per phase | Missing genre per phase | Name correspondence | Aggregate identity |
| --- | --- | --- | --- | --- | --- |
| Filename-only | 16 + 0 | 11 | 0 | Insufficient, original primary scope stripped | Insufficient |
| No-ID tags | 16 + 0 | 11 | 0 | Supported for bound Crowd Lu primary pairs | Insufficient, decisions unavailable |
| Recovery | 15 + 5 | 11 | 11 | Supported for bound Crowd Lu primary pairs | Insufficient, decisions unavailable |

The fresh run has 52 provider exchanges, zero model exchanges and zero fixture misses. Filename-only still selects genre from `37231524`; no-ID tags still select `37230261`. Recovery still rejects the relevant donors for artist conflict. Supporting the name subfinding does not execute or pass the later native predicates.

The tag-bearing profiles each report fourteen candidate diagnostic rows per phase. Four concrete releases have supported name correspondence: `25560652`, `37230261`, `37230264` and `37231524`. Repeated trace appearances are not independent evidence. Filename-only has ten rows per phase and no supported correspondence. Every aggregate diagnostic remains insufficient.

`final/comparison.json` passes exact ordered provider exchanges and budgets, requests, native eligibility reasons and scores, raw guard observations, selected donors, mappings, proposed tags, readbacks, writes and holds, selected IDs, content-only suppression, cache behavior, payload/full hashes, persisted provenance and review. Retained primary-name variants and ownership are identical. Allowed differences are only verified receipt fields, validated diagnostic observations, three declared compiled source hashes, elapsed time and disposable UUID roots.

All twelve compared performer exports remain insufficient, with `writableDelta: []` and `acceptedCertificate: false`. The unchanged field auditor still reports four source-eligibility findings per arm. It uses the unchanged independent source-certificate inventory. Historical 84 findings, rehearsal findings, old contracts and certificates remain unchanged. Missing durations, the final acoustic qualifier, independent album/title certification, collaborator completeness and edition questions are not resolved. There is no model-accuracy credit or claim of the complete unified tagging workflow.

## Gates and failed attempts

Fresh renderer and tool gates pass TypeScript, 858 renderer tests, nine provider-mock tests, ten evaluation-tool tests, four scheduler controls, eighteen exporter tests, four retained comparator controls, four new comparator controls, eight safety controls, four field checks, five preservation checks and ten frozen-packet checks. Formatting and whitespace checks pass.

The final-source broad server-library retry exits 0 with 738 passes, zero failures and seventeen explicitly ignored tests. Its source-bound executable is the one used by the successful replay. The prior failed run recorded 736 passes, two failures and seventeen ignored tests. One failure was SQLite cache initialization during disk exhaustion. The other was `ProcessTimedOut` instead of `OutputTooLarge` in an existing 200 ms subprocess-bound test under load. Neither failure is relabeled passing. No unrelated test or timeout was changed.

The first replay attempt generated all six phase records, but two jobs failed while writing the final provider inventory because the disk was full. Only one job exited successfully. Its final driver binding check also detected source changes from the later review corrections. That arm is not accepted as final-source proof. The fresh final arm completes all three jobs and all six evaluations. Together the two attempts start six jobs and perform 104 loopback provider exchanges, with zero model exchanges or misses. Only the final arm enters the new paired comparison.

A compression watcher initially started before the driver created its `runs` directory. A separate watcher then compressed the closed cold snapshots, and finalization compressed the closed warm snapshots. Both compression rounds streamed decompressed bytes back and checked SHA-256 before deleting redundant new raw copies. No input or historical evidence was removed. The first arm's six snapshots are also retained losslessly. Compiler cleanup selected recorded task-born closed failed-build objects or incremental sessions, not old evidence or unrelated caches. The paths, byte counts and object-file selection cutoff remain in the cleanup receipts.

The compact build disables root-package test debug information and incremental compilation to fit available disk. It does not change dependency versions or application source. The final executable SHA-256 is `3e73a31fc0d8ab8790c6ba5433d1e216807e17ca16a9f93256ae5376332558a8`. The retained baseline executable is `6fdbf49d0e16c662920581577432db3e6588a201997ddfce28ac374651b60d30`. The first new attempt used `1062726396a31e6b0e218abf4d0179427615a4abfc88fd8bc389956a7337f6c4`. Each historical execution plan retains its own source and executable hashes. No old output was rewritten to use a new source or commit.

The earlier retention delivery's desktop gate is historical, not a fresh check of this diff. No fresh default desktop, packaged bridge, acoustic, original-library, credentialed or live-service smoke is claimed. The server tests exercise the shared native logic and both candidate types.

## Changed paths and local reproduction

The eight tracked changes are:

- `src-tauri/src/state/providers.rs`
- `src-tauri/src/commands/auto_tag.rs`
- `src-tauri/src/commands/album_search.rs`
- `src-tauri/src/commands/auto_tag_eval.rs`
- `src-tauri/src/commands/auto_tag_evidence.rs`
- `src-tauri/src/commands/mutations.rs`
- `src/shared/desktop-api.ts`
- `docs/CHANGELOG.md`

New local working records are the runtime plan, this report, the independent-review record, and `compare.py`, `test_comparison.py` and `audit.py` under `.planning/plans/runtime-artist-diagnostic-20261110/`. Raw logs, executables, snapshots, compression receipts and cache data remain under `.amp/in/artifacts/runtime-artist-diagnostic-20261110/`.

These commands recheck retained comparisons without tagging or network calls. Output filenames must be unused:

```sh
python3 -B .planning/plans/runtime-artist-diagnostic-20261110/test_comparison.py
python3 -B .planning/plans/runtime-artist-diagnostic-20261110/compare.py \
  .amp/in/artifacts/primary-artist-retention-corrections-20261110/replay \
  .amp/in/artifacts/runtime-artist-diagnostic-20261110/final/replay \
  /absolute/existing-directory/new-comparison.json
python3 -B .planning/plans/runtime-artist-diagnostic-20261110/audit.py \
  /absolute/existing-directory/new-audit.json
```

Portable native contract checks use tracked genuine fixtures and disposable media with normal dependencies. The resource-conscious build command used here is:

```sh
cargo test --manifest-path src-tauri/Cargo.toml \
  --config 'profile.test.package.soundrobe.debug=0' \
  --config 'profile.test.package.soundrobe.incremental=false' \
  --no-default-features --features server --lib runtime_ -- --nocapture
```

Exact paired replay is local-only. It needs the retained baseline executable, genuine route pool, generated inputs, independent source certificates, execution plans and sealed evidence. The new comparison and audit helpers themselves remain ignored local planning files under this authorization. A fresh clone alone cannot reproduce these exact historical arms. Intermediate failed-arm source hashes are retained, but there is no full intermediate source archive or exact-reconstruction claim for that failed arm. The maintained replay driver and prior portable retention checks remain unchanged.

The source audit verifies seven critical request, alias and eligibility function sections byte-for-byte against HEAD, and 85 other native files remain unchanged. Preservation checks cover 3,874 incoming files, with only the eight authorized tracked paths changed. The index remains empty. Existing local tracking refs show five commits ahead and zero behind; no remote state was refreshed or changed. Incoming preparation and policy records, raw evidence, `ios-native-probe-0d104d95-source.tgz` and `scripts/__pycache__/` remain in place. No original music was read or written.

The new `evidence-manifest.json` under the evidence root binds this report, the corrected-code review, planning tools, source/input locks, current source archive, exact executable, successful and failed logs, and retained runtime evidence. It does not replace an old seal. `final/source-archive-check.json` verifies 95 archived current source files against the checkout. The HEAD source archive remains separate from the uncommitted source archive. There is no implementation blocker within the authorized scope. Broader identity policy, a readable decision store, other primary-tag layouts and any eligibility consumer remain separate work.
