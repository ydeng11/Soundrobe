# Real-model preflight implementation

## Goal and authorization

Implement the agreed preflight and offline preparation toward one real-model quality evaluation. Keep external inference disabled. The user selected existing `.env.local` credentials/model, a $5 allowance (USD assumed), documented provider-default retention for public catalog metadata, and local evidence retention until result review. Prompt-preview review is waived. No inference, account mutation, credential migration, signer provisioning, commit or push is authorized in this turn.

## Observed boundary

HEAD starts at 5f84f9b3c0b73e4cc0f61f526b57f16bc28d19ea. Current native broker, Journal, receipt and public fixture approval remain explicitly mock-only. Ordinary app retries cannot be used for this experiment. The existing bounded request method accepts loopback URLs only. The graph is stale and references retired Electron files; current Rust source is authoritative.

Authorized secret-safe file inspection found populated LLM_API_KEY, exact LLM_MODEL `~deepseek/deepseek-v4-flash-latest`, and absent LLM_PROVIDER/LLM_BASE_URL. Only credential presence was output. No private application YAML was read. For this file-only preflight, existing endpoint resolution defaults to OpenRouter; this does not establish effective app configuration after other sources.

Public unauthenticated catalog GETs confirm the exact tilde alias and current alias_target `deepseek/deepseek-v4-flash-0731`. Alias endpoint enumeration is empty; concrete-target endpoint enumeration is populated. This does not imply the alias cannot serve. Preserve exact alias in request; target is observed metadata, not weight attestation or permission to substitute.

## Design and alternatives

Use the existing shared native request encoder and strict mock executor. Add a no-network file-only preflight command producing a sanitized non-executable report with snapshot hashes, exact model/endpoint, candidate host compatibility, allowance and explicit blockers. Native preparation uses the shared encoder, preserves the alias, and freezes a candidate one-shot body for an exact host route without credentials or dispatch authority. Keep the existing mock boundary unchanged.

Rejected alternatives: ordinary completion bypasses one-shot accounting; relaxing the mock broker's loopback/fixture trust would misrepresent historical mock authority as live approval. No new provider backend or general signer service is needed for preflight. A successful plan is not a successful run or permission to send.

## Sequence

1. Add failing Node contracts for exact tilde preservation, no credential disclosure, missing/unsafe configuration, supplied metadata matching, endpoint parameter filtering, limits/allowance, stale/changed inputs and disabled activation. Implement the no-network command using Node's standard parseEnv, not shell sourcing or dotenv environment mutation.
2. Add a failing native contract for preparing the configured alias and exact host route without completion/retry or dispatch. Reuse native shared encoding; preserve existing bounded loopback behavior and apply request byte/generation guards to both paths.
3. Run fresh focused Node and Rust tests, existing supervision helper tests, TypeScript, formatting and whitespace. Review all changed logic for boundary mistakes. Exercise the maintained command against authorized `.env.local` and retained public snapshots, never actual inference.
4. Record actual endpoint terms, price/usage uncertainties, implementation limits and remaining activation gaps in docs and the existing decision artifact. Do not call snapshot availability proof of actual routing, pricing enforcement or runtime safety.

## Acceptance and limits

No network import or inference command in the preflight tool. No key values or hashes of credentials/private whole files in output. Errors are static codes, not arbitrary input echo. Snapshot provenance/time is operator evidence, not attestation. Non-secret configuration/snapshot fingerprints detect changes but are not approval. Reports and native plans always carry liveAuthority=false, readyForExecution=false and no executable approval token. Real signer trust, native-only secret supply, durable real-run integration and full final-source acceptance must not be claimed from preparation tests.

## Executed checkpoint

- Maintained file-only CLI parsed authorized `.env.local` and public snapshots, preserving `~deepseek/deepseek-v4-flash-latest`. Only credential presence is retained. No credential hash or private whole-file hash is exported, and no evaluated child received the credential.
- Candidate `wafer/us` preserves the alias and freezes exact host restrictions in a non-executable plan. The independently recomputed body SHA-256 is `5923deaf5e154a6deb436d079edf3b17912c011e979690a46398852f2b7bb7b8`. Observed target is metadata, not a replacement model.
- Seven new Node contracts plus seven existing supervision contracts passed, zero skips. Red/green logs cover missing implementation, empty route segments, and unknown or higher cached-input prices. Self-review of the substantial diff found the latter two preparation defects; both now fail closed. No unrelated source was changed.
- Actual native preparation passed, with four further native processes rejecting changed authority, changed allowance, forbidden input fields and a credential-bearing endpoint. The retained verifier independently compares the model, host, body/input/report hashes and public prompt data and checks local evidence for actual credential disclosure without emitting the credential.
- Focused native client gate passed 17 with one live smoke ignored before adding the explicit preparation entry. The existing twenty-case broker integration table passed separately. Final native source passed the full serial offline server-library gate: 872 passed, zero failed, 31 explicitly ignored, 819.05 seconds. This is a regression gate, not a supervised six-job experiment or a relaxation of its 300/600-second scopes.
- TypeScript, Rust formatting, CJS syntax and whitespace passed. No full desktop suite, fresh six-job run, strict Clippy, live or display/packaged smoke was run. Historical non-green gates remain historical.

Evidence is retained under `.amp/in/artifacts/real-model-preflight-20261113/`, including public catalog snapshots, sanitized preflight/plan, controls, red/green logs and final regression output. Nothing is staged or committed for this checkpoint. No inference, authenticated account query, top-up, configuration mutation, OS change, original-library operation or remote Git operation occurred.

## Public terms and unresolved enforcement

OpenRouter data-collection documentation says content logging/product-use opt-ins default off, with metadata retention and safety/CSAM/legal exceptions. Actual account settings are unqueried. Wafer's November 3 security page and current serverless ZDR documentation describe mandatory serverless ZDR and metadata retained about thirteen months. Its older June 22 privacy policy describes conditional ZDR, up-to-thirty-day non-ZDR content retention and internal draft-model training. Prefer the newer operational serverless statement while retaining the conflict; no route-specific contract is verified and no new zero-retention requirement is imposed.

Sources: https://openrouter.ai/docs/guides/privacy/data-collection ; https://www.wafer.ai/security ; https://docs.wafer.ai/serverless/zero-data-retention ; https://www.wafer.ai/privacy-policy . Public reads only. The selected route's token prices yield US$0.0006627328 for the declared 8,192-input/1,024-output illustration. This is not actual tokenization or an all-in spending bound. Public OpenRouter limits documentation does not establish this account's budget/hold coverage or an overshoot-free $5 ceiling. No authenticated `/key` request or other account lookup occurred.

## Remaining implementation

- Add real approval verification and native-only credential supply without weakening the loopback/public-fixture mock boundary or provisioning real secrets without authorization.
- Integrate durable one-attempt admission, real receipts, response/usage failure accounting and downstream readback/scoring. Rehearse with fake credentials and no external dispatch.
- Bind effective evaluation configuration, exact request and route, billing risk, executable/source, inputs/labels, allowed transformations and evidence locations. Run fresh final-source offline acceptance before separate run-specific execution approval.

This checkpoint completes file-only preflight and request preparation, not the remaining live integration or real-model quality evaluation. Nothing is needed from the user for this checkpoint.
