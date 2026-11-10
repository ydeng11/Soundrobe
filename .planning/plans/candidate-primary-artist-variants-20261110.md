# Candidate primary-artist variant retention

Approved scope: retain Discogs release primary-artist canonical/credited name pairs as observations only. Do not change eligibility, aliases, display, requests, or authority. Work stays in this checkout, uncommitted.

Use a typed Discogs-only observation containing release ID, primary artist array index, artist ID, canonical name, and credited name. Require a matching positive raw release ID and a bound positive primary artist ID. Preserve joint primary entries separately. Skip absent/blank ANV or incomplete identity binding. The source selector is the raw release's `artists[index]`; existing captured HTTP receipts remain the capture provenance. Do not manufacture capture timestamps or cross-provider links.

Carry the optional serialized vector through `ProviderAlbum`, auto-tag and manual `AlbumCandidate`, existing candidate conversions, and caches. Empty/default observations are omitted. Old cache rows deserialize empty without invalidation or extra HTTP; they cannot recover discarded ANV until a normal refetch. Retain only the preferred candidate's own observations during candidate merge. No eligibility consumer reads them.

1. Add public Discogs HTTP-boundary red tests using frozen bodies. Check direct output, both candidate conversions, actual SQLite reopen, legacy rows, successive requests, blank/missing ANV, invalid binding, joint entries and guest/homonym isolation. Run before implementation and retain the failure.
2. Add the smallest fields/parser projection and conversion plumbing. Run the same tests green. Inspect every reader of the field and the complete production diff.
3. Run three isolated assistance-disabled Crowd Lu jobs, cold/warm, against the bound original executable and the rebuilt executable. Reuse only authorized captured/generated input, not original media. Use authentic strict-loopback captures, fresh per-job caches, and no upstream/model capability. Compare all unrelated fields and ordered exchanges, allowing only retained observations and new compiled-source hashes. Keep selected genre receipts, strict errors, physical mappings, readbacks, writes, payloads, and review identical.
4. Run provider, candidate/cache, adapter, field/exporter checks and fresh `just --no-dotenv check`. Preserve failures and ignored-test limits. Bind sources, tools, executable and inputs separately from old seals. Publish the results and report to the source thread.

The prior diagnosis, frozen five-album packet, historical 84 findings and four rehearsal source findings remain unchanged. No new certificate, donor authority or genre truth follows from retained name evidence. No user-visible tagging change is intended, so a changelog entry must not claim an eligibility or tagging improvement.
