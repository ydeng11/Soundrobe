# Offline MusicBrainz and Discogs fixtures

The standalone Node utility serves captured API JSON on loopback. It never
contacts upstream providers. Production Rust HTTP clients and parsers consume
the responses through the existing test endpoint injection; application settings
and provider discovery budgets are unchanged.

Import the raw, SHA-256-locked release details already present in the evaluation
candidate pools and the checked-in sanitized discovery bundle at
`test/fixtures/tauri/auto-tag-eval/provider-discovery.json`, then start the
service:

```sh
node scripts/mock-provider-service.cjs import-pools \
  test/fixtures/tauri/auto-tag-eval/candidate-pools.json \
  .planning/debug/provider-mock/manifest.json
node scripts/mock-provider-service.cjs serve \
  .planning/debug/provider-mock/manifest.json 18081
```

The importer verifies every source hash and writes 24 raw release-detail
records plus the locked discovery/search, artist-release, and representative
edition-detail records. Normalized candidate fixtures are rejected and are
never served as upstream responses.

Endpoints are `http://127.0.0.1:18081/musicbrainz/ws/2` and
`http://127.0.0.1:18081/discogs`. An omitted port selects a free port and prints
the endpoints as JSON. Stop with Ctrl-C.

## Captured search and browsing responses

Every page is an explicit fixture; query order is ignored, while query values,
pagination, and repeated parameters remain significant. Credential query fields
are ignored on incoming requests and forbidden in the manifest. To extend the
dataset, add sanitized locked records to `provider-discovery.json`; the normal
`import-pools` command merges them into the generated manifest.

```json
{
  "schemaVersion": 1,
  "records": [
    {
      "provider": "musicbrainz",
      "path": "/release",
      "query": {"artist": "ARTIST_MBID", "offset": "0", "limit": "100", "fmt": "json"},
      "bodyFile": "responses/artist-releases-page-0.json",
      "sha256": "SHA256_OF_CAPTURED_FILE"
    },
    {
      "provider": "discogs",
      "path": "/database/search",
      "query": {"artist": "Enya", "type": "release", "page": "1", "per_page": "25"},
      "bodyFile": "responses/search-page-1.json",
      "sha256": "SHA256_OF_CAPTURED_FILE"
    }
  ]
}
```

Query arrays of `[key, value]` pairs support repeated parameters. Inline `body`
JSON is supported for small synthetic fixtures. Optional `status` and
`headers` model errors and Retry-After/rate-limit headers. Missing routes return
501 `offline_fixture_missing`; they never fall through to the internet or
manufacture an empty successful result. Overlapping routes and hash mismatches
fail startup. Imported release details use the exact production query tuple
(`fmt` and `inc` for MusicBrainz, no query for Discogs); they do not simulate
arbitrary inclusion queries.

## Native evaluation

Start the service, then run the existing ignored native evaluation against it:

```sh
SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL=http://127.0.0.1:18081 \
SOUNDROBE_AUTO_TAG_EVAL_ARTISTS=Enya \
SOUNDROBE_AUTO_TAG_EVAL_PROFILE=assisted_without_ids \
just eval-auto-tag-live
```

Despite the recipe name, the mock URL selects offline provider fixtures, skips
user-config and environment-config loading and credential requirements, disables environment proxies and
redirects, and records `providerMode: offline_fixtures` per invocation. A forced
loopback proxy also rejects ancillary external HTTP/HTTPS artwork attempts rather
than allowing them onto the network. The URL
must be an HTTP origin on `127.0.0.1`. Synthetic media generation still requires
the runner's existing prerequisites and source hash/equivalence checks. The
native runner saves the final `GET /__fixtures` inventory as
`provider-inventory.json` and marks `command.log` as `incomplete` when the
service recorded any missing route; a passing test process does not hide
incomplete fixture coverage.

## Building reviewed evaluation cases

`GET /__fixtures` exports loaded payloads, source metadata and SHA-256 hashes,
plus request coverage and deduplicated missing requests. Authorization headers
and credential query values are never recorded. Use this inventory to identify
which pages/details need collection and to inspect complete provider tracklists.
Sanitize all captured bodies and source metadata before importing them.

Provider responses are **unreviewed evidence**, not expected answers. Keep
acceptable release IDs, complete track mappings, hard negatives and reviewer
decisions in the separate expectations ledger. Normalized `ProviderAlbum`
snapshots are explicitly skipped by the importer. No audio or source media is
written by the service.

## Checks

```sh
npx vitest run test/scripts/mock-provider-service.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --lib offline_
```
