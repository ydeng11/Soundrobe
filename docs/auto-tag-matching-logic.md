# Auto-Tag Matching and Write Logic

This document describes the current Electron auto-tag flow: how it chooses a
release, how it aligns local files to remote tracks, and how final tag values
are assembled for writing. It is meant to be the working map for improving the
logic without guessing at the code path.

Active implementation lives under `frontend/`. The legacy Python CLI is not
covered here.

## Main Objects

`LookupRequest`

- Built from the selected album path by `parseAlbumWithTags()`.
- Contains folder/tag hints: `artistHint`, `albumHint`, `yearHint`.
- Carries existing provider IDs found in local tags:
  `musicbrainzAlbumId`, `musicbrainzArtistId`, `discogsReleaseId`,
  `discogsArtistId`.
- Carries one `TrackCandidate` per local audio file, sorted by filename.

`AlbumCandidate`

- A proposed album-level and track-level metadata result.
- Sources include `musicbrainz`, `discogs`, `llm`, and `folder`.
- Provider candidates may contain album/release IDs and remote track lists.
- Fallback candidates usually contain weaker data derived from local tags,
  filenames, folder names, or model-assisted extraction of local context.

`TrackCandidate`

- A normalized per-track shape used both for local file hints and remote track
  metadata.
- Important fields: `title`, `artist`, `artists`, `trackNumber`,
  `trackTotal`, `discNumber`, `discTotal`, `musicbrainzTrackId`, `length`,
  `genre`.

`WriteFields`

- Final fields passed into `writeTags()`.
- Built by combining album-level fields and one track-level entry per sorted
  audio file.

## End-to-End Flow

### 1. Read Local Hints

`processAlbum()` starts by calling `parseAlbumWithTags(albumPath)`.

That function:

- parses folder hints from path segments;
- scans sorted audio files in the album directory;
- reads existing tags with `readTrackMetadata()`;
- falls back to filename-derived hints when tags are missing or unreadable;
- treats blank artists and bracketed domain watermarks such as `[example.com]`
  as untrusted: an explicit `Artist - Title` filename wins, otherwise a
  non-compilation album artist may fill the track artist;
- preserves meaningful per-track artists and never substitutes `Various
  Artists` as a track artist;
- captures existing MusicBrainz/Discogs IDs from local tags;
- creates `request.tracks` in sorted filename order.

For filename parsing, `trackHintFromFilename()` handles simple forms such as:

- `06. Title.flac` -> title `Title`, track number `6`;
- `Artist - Title.flac` -> artist `Artist`, title `Title`.

Important limitation: if the file tag itself says `TITLE=06.Title`, that value
is read as a tag title. The filename cleaner does not automatically rewrite the
tag title at this stage.

### 2. Resolve Provider Artist IDs

`resolveProviderArtistIds()` tries to attach MusicBrainz and Discogs artist IDs
to the request before release lookup.

These IDs improve release search because the provider clients can browse
releases for a known artist instead of doing only broad text search.

### 3. Read Cached Candidates

The lookup cache is checked with the full request hash.

If cached candidates exist, dataset candidates are discarded and the remaining
cached candidates are held behind candidates produced during the current run.
Fresh provider results therefore win equal-priority merge ties.

The cache stores fresh candidates before per-request track protection. Cached
candidates are not final: they pass through current filtering and track
protection again. A cache hit is not copied into the new cache payload, which
prevents duplicate growth. When provider lookups fail transiently, an existing
provider cache is retained instead of being replaced by folder fallback alone.

The lookup-cache key is versioned and includes the local per-track
`MUSICBRAINZ_TRACKID` values. This matters because an existing recording ID
changes deterministic track alignment and therefore changes which remote
title and artist fields are authoritative. A matcher/cache-key version change
causes older protected candidate payloads to miss instead of shadowing newer
matching behavior.

### 4. Direct Provider ID Lookup

`performDirectIdLookups()` uses existing IDs from the local files:

- `musicbrainzAlbumId` -> fetch MusicBrainz release directly;
- `musicbrainzArtistId` without album ID -> browse artist releases and match
  by album title;
- `discogsReleaseId` -> fetch Discogs release directly;
- `discogsArtistId` without release ID -> browse artist releases and match by
  album title.

Direct album/release ID lookup is the strongest release selection path.

Important distinction: an existing local `musicbrainzTrackId` is not used to
select the album. It is used later, after a MusicBrainz release candidate has
been selected, to align local files to remote tracks.

### 5. LLM Fallback Hints

`resolveTagsViaLLM()` may produce:

- a corrected request, if folder/tag hints look ambiguous;
- an `llm` fallback candidate.

For auto-tag matching, LLM output should be understood narrowly:

- for genre, it can act as an extra knowledge provider;
- for artist, album, and track names, it is model-assisted extraction from
  folder names, filenames, and existing local tags;
- LLM track names have the same weight as folder/filename fallback names;
- LLM output is not provider evidence;
- LLM output is not per-track write authority;
- LLM-as-judge behavior belongs to the audit flow, not this auto-tag matching
  flow.

That means LLM data can improve lookup hints and local fallback title forms, but
it should not be treated as stronger than provider release evidence.

### 6. Provider Search

MusicBrainz and Discogs searches run after direct lookup.

Each provider first tries an artist-scoped release browse when an artist ID and
album hint are available. If that does not return a match, it falls back to
search variants built from aliases and original hints.

Provider candidates are filtered by album-name verification before they are
allowed into the auto-apply pool.

Artist-scoped browsing builds a strict release shortlist:

- artist resolution scopes the provider browse;
- normalized album names must be very similar before a release enters the
  shortlist, such as exact match, Simplified/Traditional equivalent, or
  meaningful containment;
- local-contains-remote album-name containment can enter the shortlist when the
  contained provider title is meaningful;
- album normalization treats gendered Chinese `妳` and generic `你` as equivalent
  before containment is evaluated;
- the shortlist cap is a safety guard, not the filter itself;
- a cap of about three release details per provider is enough because strict
  album-name filtering should usually leave only two or three plausible albums;
- final choice among shortlisted release details should primarily use
  track-title name agreement, not album title alone, because every shortlisted
  release has already passed strict album-name similarity.
- shortlist winner ranking should prefer the release with the highest unique
  track-title coverage. Duration agreement, track count, year, album score, and
  provider priority are tie-breakers after title coverage.

### 7. Add Fallback Candidates

The candidate pool always gets:

- the LLM fallback candidate when available;
- a folder fallback candidate as the lowest-priority safety net.

These candidates can fill missing album-level fields, but they should not
overwrite stronger provider per-track evidence.

Folder and LLM fallback track names are both local fallback names. They can be
used as additional local title forms for matching against provider track names,
but they remain below provider evidence.

### 8. Protect Per-Track Fields

`protectCandidateTrackFieldsForAutoApply()` is the main safety gate for remote
track data.

For each provider candidate (`musicbrainz`, `discogs`, and legacy `dataset`),
it calls `matchRemoteCandidateTracks()` with:

- local request tracks from file tags/filenames;
- sorted audio filenames;
- remote candidate tracks;
- artist hints.

The output is still in local filename order. Unmatched local files keep their
local track data.

The matching direction should preserve one-file-to-one-provider-track alignment:
one local file can align to one API track only when the evidence identifies a
single provider track for that file. If a fallback title form matches multiple
provider tracks, it is ambiguous and should not align without another
deterministic signal.

## Release Confidence Direction

Release confidence is separate from per-track write authority.

For brand-new albums with no release IDs, artist-scoped provider browsing should
first resolve the artist, then use name agreement to find the album. Album name
agreement is the main release filter. Track title agreement is the main
tie-breaker among shortlisted releases.

Useful release-confidence signals:

- album name agreement between folder/manual/LLM-extracted hints and provider
  album title;
- track name agreement between local fallback title forms and provider track
  titles, measured by unique one-file-to-one-provider-track coverage;
- year agreement as supporting evidence;
- track count agreement as supporting evidence.

Signals that should not win by themselves:

- LLM self-reported confidence;
- LLM track matching;
- track count alone;
- position alone;
- broad artist match without album name agreement.

Release confidence can choose or reject a provider release. Per-track writes
still need deterministic track evidence. Auto-tag should not ask the LLM to map
local files to provider tracks in the main path; that would hide ambiguity
inside a prompt instead of producing explainable, testable alignment.

## Track Matching Evidence

`RemoteTrackMatcher` records why each local file matched a remote track. The
current evidence types are:

1. `musicbrainz-track-id`
2. `tag-title`
3. `filename-title`
4. `fallback-title`
5. `contained-title`
6. `position`

The evidence type controls how much remote data is trusted.

All title-based evidence should be unique against the provider tracklist. A
local title form, including a fallback track name, is not enough if it matches
more than one provider track.

For MusicBrainz, a release track can have a release-specific title and a
different recording title. The recording title is retained as match-only
evidence. A unique local match against that recording title aligns the file, but
the release-track title remains the provider value eligible for `TITLE`. For
example, `Top of the World（我站上全世界的屋頂）` can align the file while
`站在世界之巔` is written from the selected release.

### MusicBrainz Track ID Match

Used only for MusicBrainz candidates.

If the local track has `musicbrainzTrackId`, the matcher checks whether exactly
one remote track in the selected MusicBrainz release has the same recording ID.

When exactly one remote track matches:

- local file is aligned to that remote track;
- remote title is trusted;
- remote artist/artists are trusted;
- remote `musicbrainzTrackId` is preserved/written;
- full ordered match rules can still apply to track/disc numbers if all tracks
  align.

If more than one remote track has the same ID, the match is refused as
ambiguous.

Reliability: very high precision when the local ID is correct and the selected
release is correct. It does not help discover the release by itself.

### Tag Title Match

Generated from the local tag title.

The matcher:

- strips known annotation suffixes;
- creates Simplified/Traditional Chinese variants;
- exposes meaningful components of genuinely bilingual Latin/CJK titles, so an
  exact shared component can align translations with different word order;
- strips known artist suffixes like `Title-Artist`;
- normalizes punctuation and symbols for comparison.

If a unique remote title form matches, the local track aligns to that remote
track. If both local and remote durations exist, they must be close.

Bilingual component forms do not relax uniqueness. If a shared English or CJK
component identifies more than one remote track, the matcher refuses the match
unless duration uniquely disambiguates it.

For tag-title evidence:

- local title is usually preserved when it already matched;
- remote artist/artists are trusted for MusicBrainz/Discogs;
- MusicBrainz track ID is written when the remote matched track has one.

Reliability: good when local tags are clean. Weak when local tag titles contain
track-number pollution like `06.Title` because tag-title normalization does not
currently strip that prefix.

### Filename Title Match

Generated from the sorted filename.

The filename parser can strip:

- extension;
- leading track number at the beginning of the filename;
- common `Artist - Title` prefixes;
- known no-space artist prefixes when artist evidence matches.

If a unique remote title form matches, the local file aligns to that remote
track. Duration is used when available.

For filename-title evidence:

- cleaned filename title may be written;
- remote artist/artists are trusted for MusicBrainz/Discogs;
- MusicBrainz track ID is written when the remote matched track has one.

Reliability: good when filenames contain real titles. Weaker when filenames
contain multiple prefixes, inconsistent numbering, or no clean title.

### Fallback Track Name Match Direction

Folder-derived and LLM-derived fallback track names should be treated as
additional local title forms. They are useful when raw filenames are messy or
tags are empty, but they are still local-context extraction rather than provider
truth.

A fallback track name can match a provider track title when normalized names are
very similar, including meaningful containment in either direction. The match
must be unique: one local file should match exactly one provider track from the
API response.

When `fallback-title` evidence aligns a unique provider track, the provider
title and artist credits may be written. The LLM/folder value only established
alignment; it is never itself treated as provider write authority.

### Contained Title Match

Used for MusicBrainz and Discogs title cleanup.

This path handles suffix-polluted local titles where the provider title is
contained in the local title, such as:

- local `I Can't Go On(无限)(24bit-48Hz)`;
- provider `I Can't Go On`.

The helper requires containment plus meaningful extra normalized content before
replacing the title. This prevents punctuation-only differences from rewriting
tags.

For contained-title evidence:

- provider title is written;
- remote artist/artists are trusted for MusicBrainz/Discogs;
- MusicBrainz track ID is written when available.

Reliability: good for format/album suffix pollution. It intentionally does not
rewrite unrelated titles.

### Position Match

Position fallback activates only when:

- zero title matches succeeded;
- local and remote track counts are equal;
- there are at least two tracks.

It aligns local index `i` to remote index `i`.

For position evidence:

- local titles are preserved unless they are generic placeholders;
- non-empty local artists are preserved;
- blank local artists may be filled from remote;
- track/disc numbers can be written when the whole tracklist is aligned;
- MusicBrainz track IDs can be written because the remote track is aligned.

Reliability: useful for full albums with completely different scripts or
translations, but materially weaker than ID/title evidence. It can be wrong for
missing tracks, bonus tracks, reordered files, different editions, hidden
tracks, or multi-disc layouts.

## Candidate Merge Logic

After protection, `mergeAutoTagCandidateFields()` chooses a preferred candidate.

Priority:

1. candidates with provider release IDs;
2. `musicbrainz`;
3. `discogs`;
4. `llm`;
5. `folder`.

The merge fills missing album-level fields from later candidates:

- artist / artists;
- album;
- album artist / album artists;
- year;
- genre;
- provider IDs.

For tracks, the first preferred tracklist is kept, then later tracklists fill
gaps by position.

Track merge behavior:

- provider title cleanup can still improve stale cached provider titles;
- blank target artists can be filled;
- richer artist credits can replace target artists only when both candidates
  refer to the same provider release;
- LLM/folder candidates should not overwrite provider per-track artists.

Known risk: track merge is positional, so any future broadening here should be
careful around compilations and multi-disc albums.

## Final Write Construction

`applyCandidateTags()` builds final write jobs.

Album-level fields are built once:

- `album`;
- `albumArtist`;
- `albumArtists`;
- `year`;
- `genre`;
- `musicbrainzAlbumId`;
- `musicbrainzArtistId`;
- `discogsReleaseId`;
- `discogsArtistId`;
- optional cover data.

Track-level fields are built from `candidate.tracks` in order:

- `title`;
- `artist`;
- `artists`;
- `trackNumber`;
- `trackTotal`;
- `discNumber`;
- `discTotal`;
- `musicbrainzTrackId`.

For each sorted audio file:

1. take the album-level fields;
2. overlay that file's track-level fields by the same index;
3. optionally add local/downloaded lyrics;
4. optionally convert Chinese script;
5. send the result to `writeTags()` through the write queue.

The writer compares existing tags to requested fields and may report `skipped`
when the final requested values are identical to what is already on disk.

## How Specific Tags Get Their Final Values

### TITLE

Possible sources, strongest first:

1. MusicBrainz track-ID match release-track title.
2. MusicBrainz release-track title aligned through its recording title.
3. Provider title aligned through a unique fallback-title match.
4. Contained-title provider cleanup.
5. Placeholder local title replaced by provider title.
6. Cleaned filename title when filename evidence matched.
7. Existing local tag title preserved.
8. LLM/folder fallback title if no stronger provider tracklist won. These are
   local-context fallback names, not provider evidence.

### ARTIST / ARTISTS

Possible sources, strongest first:

1. Remote MusicBrainz/Discogs artist credits when evidence is
   `musicbrainz-track-id`, `tag-title`, `filename-title`, `fallback-title`, or
   `contained-title`.
2. Remote artist only fills blank local artist for `position` evidence.
3. Same-release provider merge can enrich stale cached provider artists.
4. Existing local artist is preserved for weak/position-only evidence.
5. An explicit `Artist - Title` filename replaces a blank or bracketed-domain
   watermark artist before fallback candidates are built.
6. A non-compilation album artist fills the same untrusted artist only when the
   filename has no artist.
7. LLM/folder fallback can provide artist when no provider data won. These are
   local-context fallback names, not provider evidence, and do not replace a
   meaningful existing per-track credit.

### MUSICBRAINZ_TRACKID

Possible sources:

1. Existing local tag ID, read into `request.tracks`.
2. Remote MusicBrainz recording ID from a matched remote track.

Writing a new MusicBrainz track ID requires local-to-remote alignment first.

Reliability by alignment evidence:

- existing exact ID match: very high precision;
- title/filename match with duration: good;
- contained-title match: good when containment is real;
- position-only match: weaker and edition-sensitive.

### Album-Level Provider IDs

Album-level MusicBrainz/Discogs IDs come from:

- existing local tags;
- direct provider lookup;
- provider search;
- candidate merge.

They are written as album-level fields to every file in the album.

## Known Weak Spots

- Existing `MUSICBRAINZ_TRACKID` is strong after a release is selected, but it
  does not currently select the release by itself.
- Filename-only `MUSICBRAINZ_TRACKID` writes are only as good as the matching
  evidence used to align the file.
- Tag titles with embedded track-number prefixes like `06.Title` are not fully
  normalized as titles.
- Position fallback can write IDs on full-count releases even when the edition
  differs.
- LLM fallback track data can be phrased too broadly; uniqueness against the
  provider tracklist is required before it can establish alignment.
- Release-detail cache entries can preserve older provider parsing behavior;
  provider cache namespaces should be bumped when provider parsing changes.
- Merge-by-position is useful but risky if tracklists are not actually aligned.

## Improvement Candidates

- Strip leading track-number prefixes from tag-title forms, not only filenames.
- Track and log match evidence per file so the UI/debug log can explain why a
  tag was written.
- Consider a policy that prevents position-only matches from writing
  `musicbrainzTrackId` unless extra evidence exists, such as matching duration
  for most tracks.
- Use local `musicbrainzTrackId` to recover or validate the release when
  `musicbrainzAlbumId` is missing.
- Reuse the same title normalization and unique-alignment rules for shortlist
  ranking and final track protection, but keep release scoring separate from
  protected write output. A score-only matcher mode is preferable to duplicating
  title comparison logic.
- Separate "safe to write IDs" from "safe to write title/artist" so each field
  can have its own evidence threshold.
- Keep the lookup-cache version and provider release-detail cache namespaces
  in sync with changes to matcher and provider parsing semantics.

## First Shortlist Implementation Tests

The first release-shortlist implementation should be test-driven around these
behaviors:

- artist-scoped browse fetches multiple shortlisted release details when
  multiple releases pass strict album-name agreement;
- track-title coverage beats album-score differences after releases have entered
  the shortlist;
- weak album-name matches do not enter the shortlist even if year or track count
  looks plausible;
- local-contains-remote album-name containment enters the shortlist when the
  contained provider title is meaningful;
- ambiguous track-title containment does not count toward coverage;
- MusicBrainz and Discogs artist-scoped browse both use the same shortlist
  behavior while generic search fallback remains unchanged.
