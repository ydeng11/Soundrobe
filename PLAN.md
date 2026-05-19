# Plan: Reduce Discogs Requests with Better Caching & Proxy Rotation

## Context

The Discogs API has strict rate limits (25 req/min unauthenticated, 60 req/min with token). The current code creates **5+ separate `DiscogsClient` instances per album** during a YOLO run, each making multiple HTTP requests (search with name variants, release detail GETs, image downloads). These requests are:

1. **Uncached at the HTTP level** — the only cache is `MatchCache` which caches the *final* `lookup_album()` result, but intermediate Discogs searches (same artist+album searched in `_enrich_genre_from_discogs`, `fetch_cover_art`, `_enrich_genre_from_lookup`) all hit the wire fresh.
2. **Unrecoverable on 429** — a rate-limit response immediately raises `DiscogsError` with no retry, backoff, or proxy fallback.
3. **No connection pooling** — raw `httpx.get()` calls without a shared `httpx.Client`.
4. **No proxy support** — all requests go directly from the runtime IP.

The user provided a Webshare proxy-list download URL that returns proxy addresses for round-robin rotation.

---

## Approach

### A. Add a shared `httpx.Client` with proxy support to `DiscogsClient`

Replace raw `httpx.get()` calls with a shared `httpx.Client` instance that:
- Uses connection pooling (reuse TCP connections).
- Loads a proxy pool by downloading the Webshare proxy-list URL (plain text, one `user:pass@host:port` per line).
- On 429 after local retries are exhausted (and no token configured), rotates to the next proxy in the pool.
- Thread-safe round-robin via `itertools.cycle` or an index counter.

### B. Add per-request Discogs response caching (in-memory)

Introduce an in-memory `DiscogsCache` dict (not SQLite — scoped to process lifetime) that caches individual API responses keyed by endpoint + params. This prevents repeated searches for the same artist+album across the multiple call sites.

### C. Add retry with exponential backoff on 429

When Discogs returns 429:
1. Wait with exponential backoff (0.5s, 1s, 2s) and retry up to 3 times.
2. If all retries exhausted **and no `discogs_token` is configured**, rotate to the next proxy in the pool and retry.
3. If a token is configured (60 req/min limit), skip proxy rotation — the higher limit is usually sufficient.
4. If all proxies exhausted (or no token), raise `DiscogsError` as before.

### D. Add rate limiting

Add a `RateLimiter` (reuse the pattern from `beets_client.py::RateLimiter`) to:
- Respect 25 req/min (or 60 req/min with token).
- Space requests across the proxy pool to respect per-proxy limits.

### E. De-duplicate DiscogsClient instances in `album.py`

`album.py` creates new `DiscogsClient()` in at least 5 places:
- `_fix_cover_art` → line 328
- `_enrich_genre_from_discogs` → line 852
- `_enrich_genre_fallback` → line 951
- `_enrich_genre_from_lookup` → line 994
- `_lookup_discogs` in `lookup.py` → line 189

Refactor to share one `DiscogsClient` instance per album workflow run.

---

## Files to Modify

| File | Changes |
|------|---------|
| `src/auto_tagger/integrations/discogs_client.py` | Major refactor: httpx.Client, proxy pool, retry+backoff, rate limiter, per-request cache |
| `src/auto_tagger/config/settings.py` | Add `discogs_proxy_url`, `discogs_cache_ttl`, and `discogs_image_cache_dir` settings |
| `src/auto_tagger/workflows/album.py` | De-duplicate DiscogsClient instances; share one per album run |
| `src/auto_tagger/integrations/lookup.py` | Accept existing DiscogsClient or allow sharing |
| `src/auto_tagger/workflows/artist.py` | Pass proxy config through to DiscogsClient |
| `.env.example` / `config.example.yaml` | Document new settings |
| `tests/test_discogs_client.py` | Add tests for retry, proxy rotation, caching |

---

## Reuse

- **RateLimiter** — already exists in `src/auto_tagger/integrations/beets_client.py:RateLimiter`. Reuse the same interval-based pattern.
- **Retry pattern** — already exists in `src/auto_tagger/llm/client.py:_post_with_retries` (retryable status codes + sleep backoff). Apply same pattern for 429 handling.
- **MatchCache schema** — already exists in `src/auto_tagger/integrations/cache.py`. The new `DiscogsCache` can use a separate table in the same SQLite DB (or a separate cache file).

---

## Steps

### Phase 1 — Refactor DiscogsClient internals ✅

- [x] 1.1 Replace raw `httpx.get()` with a shared `httpx.Client()` instance on the class.
- [x] 1.2 Add a `RateLimiter` instance (from beets_client pattern) to throttle requests.
- [x] 1.3 Add `_request()` method that wraps all HTTP calls: rate-limit wait → execute → 429 retry logic.
- [x] 1.4 Implement retry with exponential backoff (0.5s, 1s, 2s) for 429 responses.
- [x] 1.5 Add `_load_proxy_list()` that fetches the Webshare proxy list URL (plain text, one `user:pass@host:port` per line). Cache the proxy list in a module-level variable so it's fetched only once per process.
- [x] 1.6 Build a shared `itertools.cycle` iterator for round-robin proxy rotation.
- [x] 1.7 On 429 after all local retries exhausted (and no token), rotate to next proxy and retry.
- [x] 1.8 When all proxies are exhausted, raise `DiscogsError` as before.

### Phase 2 — Add per-request Discogs cache ✅

- [x] 2.1 Introduce an in-memory `DiscogsCache` class (dict[str, tuple[float, Any]] with TTL — timestamp + cached value).
- [x] 2.2 Cache keys: `{http_method}:{endpoint_path}:{sorted_query_params}` (with token excluded).
- [x] 2.3 Apply cache to `_search()`, `_get()`, and `_download_image_bytes()`.
- [x] 2.4 Add `cache_ttl_seconds` config (default 3600s / 1 hour).
- [x] 2.5 Wire into `DiscogsClient` — check cache before hitting network, store after.

### Phase 2b — Cache cover art & artist images on disk ✅

- [x] 2b.1 Add a `discogs_image_cache_dir` setting (default: `~/.cache/auto-tagger/discogs-images/`).
- [x] 2b.2 Cache downloaded image bytes keyed by the source URL hash (SHA-256, first 32 hex chars) + MIME sidecar.
- [x] 2b.3 Before downloading an image, check the disk cache; after downloading, write to disk cache.
- [x] 2b.4 Apply to both `fetch_cover_art` and `fetch_artist_image` download paths.

### Phase 3 — Config & plumbing ✅

- [x] 3.1 Add `discogs_proxy_url: str | None`, `discogs_cache_ttl: int`, and `discogs_image_cache_dir: Path` to `Settings`.
- [x] 3.2 Pass proxy URL, cache TTL, and image cache dir into `DiscogsClient.__init__()`.
- [x] 3.3 Update `.env.example` and `config.example.yaml` with new settings.

### Phase 4 — De-duplicate DiscogsClient instances in album workflow ✅

- [x] 4.1 In `AlbumWorkflow.__init__()`, add `_shared_discogs_client` lazy property.
- [x] 4.2 Convert `_fix_cover_art`, `_enrich_genre_from_discogs` (was static), `_enrich_genre_fallback`, `_enrich_genre_from_lookup` (was static) to use `self._shared_discogs_client`.
- [x] 4.3 In `LookupService._lookup_discogs()`, pass settings through to `DiscogsClient`.
- [x] 4.4 In `ArtistWorkflow`, pass proxy settings through to `DiscogsClient`.

### Phase 5 — Tests & verification ✅

- [x] 5.1 Update `test_discogs_client.py`: test retry on 429, cache hit/miss, TTL expiry, proxy rotation, graceful degradation.
- [x] 5.2 Update `test_discogs_cover.py`: fix tests to use mock httpx.Client and isolated image cache dir.
- [x] 5.3 Run full test suite: `pytest tests/` — **301 passed, 0 failed**.

---

## Verification

1. **Unit tests**: `pytest tests/test_discogs_client.py tests/test_cache.py -v`
2. **Integration**: Run `auto-tag --yolo ./test-album` with verbose logging and confirm:
   - Cache hit on repeated genre enrichment calls (same artist+album searched twice → second is cached).
   - Image disk cache hit on second run (no re-download of cover/artist art).
   - No 429 errors when proxy pool is configured and no token is set.
   - Only 1 `DiscogsClient` instance per album (log instance ID or count).
3. **Rate limit resilience**: Temporarily mock Discogs to return 429, verify retry → backoff → proxy rotation (when no token).
4. **Full suite**: `pytest tests/` — all existing tests must pass.
