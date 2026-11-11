# Bounded assistance preparation

This capability is test-only and mock-only. It does not activate a real model, authenticate a real approval, or provide a hard spending limit. Ordinary evaluation still defaults to disabled assistance and accepts only explicitly scripted loopback assistance. The disabled-only rehearsal runner is unchanged.

## Native boundary

`infra::bounded_eval` scopes the existing coordinator's OpenAI-format model transport. The separately ignored `bounded_mock_auto_tag_eval` entrypoint requires all ordinary evaluation inputs plus `SOUNDROBE_BOUNDED_EVAL_MANIFEST` and a separately supplied `SOUNDROBE_BOUNDED_EVAL_APPROVED_SHA256`. This digest is mock admission, not user approval. There is no supported external endpoint, real credential lookup, or live activation option.

The manifest uses camelCase keys and rejects unknown fields:

- `version: 1`, `mode: "mock"`, a unique alphanumeric/hyphen `runId`, and `expiresUnixMs`.
- Exact `provider: "openai"`, `model`, and `endpoint: "http://127.0.0.1:PORT/model"`. The effective destination appends `/chat/completions`. Redirects, proxies, alternate API formats, and provider-routing/fallback metadata are denied.
- `bindings`, a map of names to `{path, sha256}`. Paths are checkout-relative. Required names include `source`, `input`, `labels`, and `expectations`. Native integration additionally requires `executable`, `fixtures`, and `scoringContract`. A supplied safety input is consumed from its exact verified bytes.
- The native integration binds `source` to `src-tauri/src/commands/auto_tag.rs`, `input` to the actual corpus, `expectations` to the actual reviewed expectations file, and `executable` to the running test binary. Additional source, build, prompt, tool, label, and fixture bindings belong in the same map. A single core-source binding does not attest the whole build.
- `fixtures` is a frozen JSON snapshot of the strict mock's `/__fixtures` records. Preflight hashes and parses the same owned bytes, then compares the actual records, including bodies, routes, statuses, and receipts. It ignores mutable request history for this comparison. Actual provider clients must use the approved model endpoint's origin with `/model` removed. The local service remains trusted to serve its declared records; this is not host authentication.
- `jobs`, with exact IDs `caseId/profile/cold` and `caseId/profile/warm`, `inputSha256`, and `requests: [{role, bodySha256}]`. Selected membership must equal the manifest. Empty request lists allow provider-only controls. Allowed roles are `GenreFillResponse` and `TagCorrectionResponse`.
- A job input hash covers actual prewrite reader data and file/payload hashes. Only the disposable root is omitted from reader paths. A request hash covers the final serialized body after effective policy application, including model, role, schema, settings, and messages. Desired responses are not authorization inputs.
- `limits`: `attempts`, `retries`, `repairs: 0`, `reservedOutputTokens`, `requestBytes`, `responseBytes`, and `elapsedMs`. Positive ceilings are mandatory except retries, which may be zero. `responseBytes` also bounds the added inventory preflight read.
- `hardCostMicrounits: null` and `hardTotalBilledTokens: null`. Any requested hard billing contract is rejected as unsupported. Null means unsupported, not unlimited permission.

The retained `.amp/in/bounded-eval-mock-claims` directory uses exclusive claim creation. A crash consumes the mock run ID permanently. Deleting or replacing this directory defeats mock reuse protection; it is not an authenticated, tamper-resistant real approval store.

Model wire attempts reserve allowance before dispatch. Failures do not refund it. Retries and cold/warm jobs share one invocation ledger. A denied concurrent completion cannot finalize another attempt's receipt. Missing usage, wrong resolved model, oversized responses, cancellation, and repair attempts stop the scope. Output-token reservations bound the request parameter, not provider billing or hidden reasoning.

Scope health is checked at later job admission, review checkpoint, immediately before queued dispatch, and before publishing a staged replacement. Cancellation does not interrupt an atomic dispatcher already admitted. A local response and a timeout cannot prove that a remote provider stopped billing.

## Offline checks

From the checkout root, without dotenv or real credentials:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --offline --locked \
  --config 'profile.test.package.soundrobe.debug=0' \
  --config 'profile.test.package.soundrobe.incremental=false' \
  --no-default-features --features server --lib infra::bounded_eval -- --test-threads=1
python3 -B scripts/auto-tag-eval-replay/test_observation_scoring.py
python3 -B scripts/auto-tag-eval-replay/checkpoints/runtime-artist-diagnostic/test_comparison.py
```

These controls use injected fake credentials and local scripted HTTP. They earn zero grounded-model accuracy credit. Unit controls alone are not a full ignored-entrypoint run or a live-quality cycle. Historical evidence requires its own source-bound executable and documented evidence view; copying all historical checker files into one directory is not a portable replay.

`run-bounded.cjs NEW_OUTPUT TEST_EXECUTABLE` connects the maintained ignored evaluator to the frozen scorer. It uses generated media, authentic imported provider routes, fake credentials, and a strict loopback service with no upstream forwarding. Each selection first runs with assistance disabled to capture actual prewrite inputs. The runner then freezes its contract and mock manifest before the bounded execution. Final fixtures contain only authentic routes reached during preparation; their raw body hashes and acquisition records are unchanged. An unexpected route fails closed.

The diagnostic roles are provider-first, scripted genre fill, unknown model usage, cancellation after genre inference, and an incomplete/version-evidence hold. Cold and warm use fresh media and share one invocation allowance. A scripted genre is an instrumented test value, never a musical answer key. The model role must actually reach the wire. Unknown usage and cancellation must retain both phases without changing protected input tags.

The service records parsed HTTP and CONNECT arrivals, including denied methods and model bodies that abort before parsing. CONNECT is rejected locally. Counts include inventory polling and do not measure socket connection attempts or prove successful response delivery. The model ledger does not impose a global provider request quota. JavaScript inventory reads have independent byte and time ceilings; the native scope deadline is not a whole-driver deadline.

Before any media-content read, the driver validates canonical containment and exact visible regular-audio membership against the selected corpus. Native source hashing checks that membership on both its initial and final passes, and case materialization checks it again. These checks assume trusted anchor directories and no hostile concurrent filesystem replacement. They are not a race-proof filesystem sandbox.

The driver saves `requested-membership.json` before any evaluator child. It includes ten evaluations, 124 repeated physical observations, 372 requested field slots, 16 replicated labelable slots, and four unique selected occurrence-field labels. The full packet still contains six labels. Replication does not create independent musical examples.

`selection-status.json` is the authoritative selection status. A successful orchestration publishes it last with `executionStatus: "complete"`, `scoringStatus: "insufficient"`, and a reference to `results.json`. Each `score.provisional.json` and `result.provisional.json` remains provisional unless that complete status exists. Native exit, termination, fixture, model-role and stopped-diagnostic gates precede projection. Completion does not mean semantic-quality success.

Caught failure after membership freezing publishes an `incomplete` status and `failure-envelope.json` covering the full selection. Native exports are `exported_not_validated` or `unavailable`, never invented readbacks. The envelope projects zero slots and retains all requested and labelable denominators. An abruptly terminated driver, an unreadable status, or failed filesystem writes cannot provide a complete result. Missing status never permits partial score acceptance.

The separate orchestration limits are 600,000 ms per native child, 60,000 ms per Python child, and 15,000 ms plus 128 MiB per JavaScript inventory fetch. Native children use a POSIX process group so watchdog termination reaches ordinary inherited descendants. This is macOS orchestration, not a portable process sandbox. Native postflight and donor inventory JSON reads remain unrestricted. No hard whole-evaluator response-size or elapsed-time bound is claimed.

## Frozen labels and observation scoring

The source-reviewed packet is `.planning/research/bounded-assistance-label-review-20261111.json`. Its exact machine-value addendum is `.planning/research/bounded-assistance-machine-labels-20261111.json`. The original packet remains unchanged. Both must be explicitly bound in any later run manifest.

There are six labelable title/artist occurrence-field pairs across Wine Pon You, Crowd Lu's final acoustic-qualified catalog title, and &burn. Perfect Duet and partial Hot Space support inventory-preservation diagnostics only. All samples are exposed, not holdouts. Catalog positions do not authorize numbering changes. Catalog titles do not authenticate acoustic, mastering, or clean/explicit audio. There are no true-genre accuracy labels.

`score-observations.py CONTRACT SHA256 OBSERVATIONS NEW_OUTPUT` compares frozen field slots with exactly one observation per slot. A slot has `id`, `original`, and either exact `acceptedValues` or null for unscorable truth. An observation has the same `id`, explicit `readbackAvailable`, `readback` when available, `serviceFailure`, and `decisionVisibility: "unavailable"`.

The comparator reports correct catalog fills, correct catalog changes, wrong catalog changes, missing fields, unchanged unknown decisions, unavailable observations, and unscorable fields. It records value transition separately. It never infers intentional retention or an appropriate hold from unchanged values. Explicit retention/hold scoring is unsupported until native decisions and independently specified guard labels exist.

`freeze-native SPEC PREPARATION_RECORDS NEW_CONTRACT` freezes complete requested job, phase, physical-file and field membership from the bound corpus and actual prewrite observations. It verifies the exact input string digest, full-file and payload hash records, relative paths, source inventory, and complete one-to-one captured/generated mapping. Independent label selectors must bind the captured case, folder, occurrence and hash. Unlabeled fields remain unscorable. Unique labels and replicated cold/warm slots are counted separately.

`project-native CONTRACT SHA256 NATIVE_RECORDS MANIFEST NEW_SCORE` validates the frozen source, label, baseline, request, invocation and resource bindings before scoring actual native readbacks. Omitted or duplicate jobs, changed baselines, wrong paths and mismatched source labels reject. Available tracks need explicit values and exact physical membership. Reader fallbacks are not observations; unavailable readbacks retain all slots as `observation_unknown`. Service failures remain visible even when provider metadata is usable. `project_native(..., manifest=None)` is only a portable comparator API, not manifest-bound invocation verification.

The native `requestJson` is the portable lookup request, not the final model HTTP body. Final model-body authorization belongs to the native transport gate. Matching hashes establish consistency, not authenticated native provenance or true labels. Prewrite capture chronology is established by retained runner sequencing, not by the comparator alone. A source-reviewed catalog label does not establish source eligibility, a justified native decision, or causal model usefulness.

Every export remains insufficient, with `writableDelta: []`, `acceptedCertificate: false`, and zero grounded-assistance accuracy credit. Catalog-value agreement does not prove eligibility, user permission, safe writes, or causal model usefulness. Historical 84 certification findings and legacy disagreements remain unresolved.

## Retained execution evidence

The first complete mock integration is retained under `.amp/in/artifacts/bounded-entrypoint-attempt-5-20261111/`. It exercised all five roles through the maintained native entrypoints, coordinator, model transport, WriteQueue, readback, frozen projector and scorer. Attempt 6 repeated the integration before the final publication-order and final-source-membership corrections. Neither earlier run is final-source proof.

The final-source run and its independent reviews are recorded in the [bounded assistance preparation report](https://ampcode.com/artifacts/bounded-assistance-preparation-and-independent-label-review-1f106f53-0592-4dd5-abe8-cbe9c111ec5a). Red and green controls remain in `.amp/in/artifacts/bounded-assistance-preparation-20261111/`. They include rejection of extra audio membership, malformed source folders, corrupted native exports, unavailable readbacks, and a child that exports both phases before exiting with code 42. That last control uses replayed observations to test orchestration only, not native provenance.

Final Attempt 7 completed all ten evaluations under `.amp/in/artifacts/bounded-entrypoint-attempt-7-20261111/`. Its authoritative status reports complete orchestration and insufficient scoring. `verification.json` confirms 372 observed slots, 356 unscorable outcomes, sixteen unchanged-unknown outcomes, unchanged payloads and source media, and persisted actionable genre review after restart. Scripted genre filled 22 repeated observations without changing their titles or artist lists. Unknown usage and cancellation preserved all baseline bytes and denied warm admission. The hold wrote nothing. `projection-controls.json` records forty passing corruption and unavailable-readback controls against the real native exports. The executable SHA-256 is `f68d7b0f967ab6149312fd80d4fae22d1d5d7d2d2c0b0996ed915103486bd333`.

Final service telemetry recorded 209 provider-fixture requests, four scripted model requests, sixteen locally denied CONNECTs and twenty inventory requests, totaling 249 parsed arrivals with zero fixture misses. These counts describe final execution services only, not preparation traffic. Grounded-assistance accuracy credit remains zero.

The diagnostic selection uses Amala with existing tags but no IDs, Crowd Lu recovery in genre, unknown-usage and cancellation scenarios, and Paint the Sky filename-only hold. All five run cold and warm. This is three physical cases and 40 unique dummy files, not 124 independent tracks. Title and artist labels cover only Wine Pon You and Crowd Lu's final occurrence in this selection. Genre is unscorable.

The retained optimized test build uses `profile.test.package.soundrobe.debug=0`, `profile.test.package.soundrobe.incremental=false`, and `opt-level=3` for `sha2`, `regex`, `regex-syntax`, and `regex-automata`. These settings are part of the build record. No production deadline or allowance was increased to obtain the result.

The older broad native run remains incomplete. Its terminated executable reported 731 passes, 17 failures, 18 ignored tests, and one unfinished fixture-server join. The mapped executable differed from the replaced pathname's later executable. A fresh frozen executable subsequently passed all 757 server-library tests at default parallelism and again at the older four-thread setting, with 18 tests ignored each time. The original failure causes remain unassigned. These passes do not make the historical run pass.

The missing-request fixture join was independently reproduced and corrected using the existing stoppable test server. The regression sends only the first lookup, then requires teardown without the remaining requests. The complete integration still requires all five requests. This changes test fixtures only, not tagging, transport allowances, or write authority. The corrected source passed a fresh complete server gate: 758 library tests with 18 ignored, five CLI tests, 14 audio integration tests with 12 ignored, and zero-test binary and documentation targets. The explicitly selected offline background smoke also passed.

The first offline desktop build stopped before compilation on the missing locked `alloc-no-stdlib` archive. Standard locked Cargo fetching then restored 184 public package archives. Every restored checksum matches Cargo.lock. Manifests, lockfile, configuration, tools, and the corrected native source remained unchanged. No archive was fabricated, dependency upgraded, or registry changed.

The fresh default-desktop gate then passed offline with `default,desktop` features and default parallelism: 1,292 library tests passed, zero failed, 30 were explicitly ignored, and zero were filtered out. The three additional desktop targets and doctests each contained zero tests and exited successfully. Server-only CLI and audio integration tests are covered by the separate server gate, not counted as desktop passes. Desktop execution used a retained macOS sandbox policy allowing only loopback outbound connections. No native suites overlapped, no deadline changed, and the outer watchdog never fired.

The desktop executable SHA-256 is `bc45c42a538cfba82d5e833ee65d281f81db20327642c87127e2d97e345315b2`. Full commands, source and configuration hashes, complete output and test membership, archive checksums, and ignored-test reasons remain under `.amp/in/artifacts/native-gate-discrepancies-20261111/`. `desktop-verification.json` and `original-discrepancy-matrix-desktop-completed.json` add the desktop results without overwriting the earlier receipts or assigning causes to the historical failures. The preparation report summarizes both gates.

Attempt 7 remains evidence for its sealed source and executable, not a fresh mock execution after this test-only correction. Credentialed and real-display gates remain unselected. Persisted actionable review obligations do not establish new per-field UI highlights.

A [compact reproducibility checkpoint](../scripts/auto-tag-eval-replay/checkpoints/bounded-assistance/README.md) retains portable tools, source-only labels, mock manifest receipts, gate bindings, ignored membership and the fixture failure matrix. Its delivery index identifies normalized metadata copies separately from immutable local originals. Exact integration replay still needs local generated media, authentic route bodies and matching source-bound binaries; this is not a fresh-clone replay bundle.

## Proposed next live-run scope — not approved

The smallest useful next diagnostic selection is four case/profile jobs, each cold and warm:

- Amala `0c9e8b505ea7`, `assisted_without_ids`: provider-first control, no model role allowed.
- Crowd Lu `036e0a00e45b`, `tagged_recovery`: only `GenreFillResponse`, if existing eligibility checks reach it.
- Paint the Sky `f7756354af37`, `folder_filename`: incomplete-discovery hold, no model role allowed.
- Paint the Sky `f7756354af37`, `tagged_recovery`: only `TagCorrectionResponse`, after complete provider no-match and with existing version guards.

This proposal is eight evaluations and 112 repeated physical-file observations, with at most four model dispatches, no retries or repairs, and concurrency one. These counts are proposed scope, not approved allowance. The exact provider, version-pinned model, endpoint, generation settings, token limits, deadlines, pricing revision, hard currency limit, credential contract, approval trust root, and prompt privacy policy remain unresolved.

Hard total billed tokens, dollars, provider-internal retries, and remote cancellation cannot currently be guaranteed. Provider-wide run caps are also not implemented by this model-only ledger. External execution therefore stays unavailable. A future native transport and approval review must resolve these limits before activation, even if someone approves a model name.

Meaningful genre accuracy additionally needs a frozen taxonomy, multilabel alternatives, and qualified human or scoped authoritative occurrence-bound labels. The proposed missing-genre job can inspect behavior but cannot establish musical accuracy from today's packet. It must not be sold as a semantic-quality result.
