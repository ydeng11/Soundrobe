# Auto-tag evidence and recovery

Monitoring's run history includes **Show evidence** for each album. New runs retain provider attempts, candidate rejections, per-file recording scores, effective nonsecret configuration and file-versus-release coverage in the worker database. Older runs show that evidence was not retained; the application does not reconstruct historical reasons from current tags.

A successful file write does not prove a complete album. Partial collections and unresolved editions remain in review. Strong recording recognition writes only verified title, credits and recording identity; unresolved album, year, release IDs and positions remain unchanged. A consensus across fully examined editions may support common album fields. Clear-tag settings do not clear unresolved fields in these partial writes. Named remixes, live performances, demos, instrumental/acapella recordings, and clean/explicit versions remain distinct.

Duplicate titles do not establish duplicate audio. Release matching can group compatible recording IDs only with matching acoustic evidence and duration, and expands the mapping back onto every physical file. It never deletes or moves copies.

Temporary transport, HTTP 429 and HTTP 503 failures schedule only affected albums. The worker retains due times and the originating run across restart. Automatic retries are limited to three deferred attempts, with exponential backoff and per-folder jitter; provider `Retry-After` deadlines are never shortened. Existing catalog/detail caches preserve completed discovery work, and cached details do not consume the next network budget. Budget-limited discovery, conflicting identities and ambiguous editions require evidence/review rather than automatic outage retries.

An HTTP 402 response opens the run's durable AI circuit. Subsequent tagging and genre AI requests in that run and its automatic retry lineage stop; provider recognition continues. Check service credits/settings before choosing an explicit retry. An explicit operator retry starts a new run and resets the deferred-attempt budget; it is the deliberate point at which AI may be tried again.

Configuration evidence records enabled/configured flags, the effective model and relevant behavior settings. Keys and endpoint URLs are neither stored nor hashed into the fingerprint. SQLite additions preserve existing worker rows and review flags. Evidence and terminal worker status commit together; failure to save diagnostics is surfaced as a failure rather than a healthy result.

The committed 66-folder/453-file manifest is a post-run structural regression fixture, not an identity oracle. Network-free copied-media safety checks establish preservation and repeat-run stability. Recognition correctness uses independently specified mocked provider identities; live recognition coverage requires a separately authorized provider run.
