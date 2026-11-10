# Runtime receipt and diagnostic corrected-code reviews

Scope: current uncommitted source against `1c74c6c728f0a8ad87dc0a66de08dfd3fdabf5bd`, same `artist-library-inventory` checkout. These are independent read-only subagent reviews with different briefs, not a diverse-model vote. Neither reviewer changed source or ran tests, providers, models, tagging or services. Execution acceptance remains the coordinating agent's responsibility.

## Security review

Reviewer `TU-034vPSj2f6CKafZPwCUW7J` initially found unbounded persistent response bodies and unchecked candidate album-primary fields. Both were reproduced in `review-findings-red.log`. Corrections limit retained exact decoded receipts to 2 MiB without changing oversized metadata parsing, and check candidate `album_artist`/`album_artists` against the same bound singleton entity.

Its first re-review found that `ALBUM ARTISTS`, recognized by the existing reader, escaped completeness capture. `spaced-primary-red.log` reproduces false diagnostic support through actual file construction, reader, capture and predicate. Both plural spellings are now retained and classified as plural. Competing/multiple assertions are insufficient under the narrow policy.

Final fresh disposition: **“No remaining actionable security defect found in the current corrected diff.”** It covers the corrected eight-file source diff and diagnostic-only trust boundary, not identity-policy or release approval. It inspected `runtime-final-green.log` and its exit receipt: six passing targeted tests. It explicitly did not relabel the first failed broad run as passing.

The reviewer verified same-originating-buffer parsing/hashing; sanitized endpoints and no copied authentication headers; representative-only forwarding through candidate/content/cache conversions; optional legacy absence; resource/route/status/hash/raw pair/index checks; and absence of eligibility, alias, display, write or certificate consumers.

## Specification review

Reviewer `TU-034vPSj2xHEo4gayjYQH1t` initially found that lossy TrackData did not establish complete original primary scope, that competing homonymous bound entities were not considered, and that candidate album-primary presentation escaped checks. Reproductions are `review-findings-red.log`, `repeated-primary-red.log` and `competing-entities-red.log`. Corrections capture completeness-qualified original FLAC comments before writes, check all available candidate primary presentation, and downgrade individually supported same-input assertions for different Discogs IDs to insufficient without alias/entity union.

Its first re-review found both the spaced plural omission and leading ID3 tags outside Vorbis-only completeness. The latter is reproduced in `id3-primary-red-corrected-construction.log`; capture now rejects nonzero FLAC marker offsets instead of expanding into ID3 aggregation.

The initial ID3 construction did not reproduce the proposed diagnostic failure: Lofty exposed the last duplicate TPE2 rather than the reviewer's assumed first. `id3-primary-red-retry.log` retains that construction failure. Reversing the duplicate order produced matching visible DTO fields and exposed false support. The final reviewer explicitly corrected its earlier first-TPE2 characterization; neither attempt is omitted or called catalog evidence.

Final fresh disposition: **“No remaining specification blockers found in the inspected corrected diff, within the approved diagnostic-only scope.”** It is source-level specification approval, not confirmation that broad tests or replay completed. It verified corrected reader→capture→diagnostic controls, scoped original observations after profile application, candidate ownership, competing-entity downgrade, selected/content-only ID boundaries, legacy insufficiency, unknown decisions and no authority consumer. Existing raw guard call directions remain unchanged; artist rejection does not become a pass of unexecuted predicates.

## Limits attached to both dispositions

- Newly captured receipts are bounded individually, not by total cache quota. Existing transient HTTP/JSON allocations are not newly bounded. An externally replaced oversized cache row is not subject to the capture ceiling.
- Hashes establish consistency of stored decoded bytes and bindings, not source truth, freshness, provider-host attestation or resistance to coordinated complete receipt/candidate replacement.
- Completeness support is ordinary validated FLAC/Vorbis only. Leading ID3, unsupported formats/layouts, malformed/duplicate blocks and unavailable observations are insufficient. Raw capture hashes are not an atomic transaction with earlier DTO reads; evaluator media must remain isolated from concurrent mutation.
- Homonym aggregation concerns individually supported assertions available in this invocation's trace set. It is not exhaustive discovery or resolution of unbound, canonical-only, absent or cross-provider competitors.
- Original scoped Discogs primary IDs and applicable user-decision storage are unavailable. Unscoped/request/resolver-filled IDs and synthetic `Some(false)` probes do not substitute. Runtime aggregate decisions remain insufficient.
- Joint-primary mapping, broader normalization, acoustic/recording-version identity, duration/title/album/edition certification, collaborator completeness and write authority remain outside approval. Authentic bodies are catalog assertions; synthetic altered envelopes, comments, ID3 frames, completeness tuples and size padding are constructions.

All logs above are retained locally under `.amp/in/artifacts/runtime-artist-diagnostic-20261110/`. The approved runtime scope and frozen observations are in `.planning/plans/runtime-artist-diagnostic-20261110.md`. The historical retention re-review remains separate and unchanged.
