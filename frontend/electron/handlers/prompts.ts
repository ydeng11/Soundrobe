/**
 * Prompt builders for LLM tagging decisions.
 * Ported from Python auto_tagger.llm.prompts.
 */

import type {
  AlbumCandidate,
  LookupRequest,
  TrackCandidate,
} from "./candidates";

// ── Selection ───────────────────────────────────────────────────────

export function buildSelectionMessages(
  request: LookupRequest,
  candidates: AlbumCandidate[],
  maxCandidates = 5,
): Array<{ role: string; content: string }> {
  const payload = {
    artist_hint: request.artistHint,
    album_hint: request.albumHint,
    track_count: request.tracks.length,
    track_titles: request.tracks.map((t) => t.title).filter(Boolean),
    candidates: candidates.slice(0, maxCandidates).map((c, i) => candidateSummary(i, c)),
  };

  return [
    {
      role: "system",
      content:
        "Select the best album candidate for audio tagging. " +
        "Return only JSON with selectedIndex, confidence, and reason. " +
        "Use selectedIndex null when all candidates are poor.",
    },
    {
      role: "user",
      content: JSON.stringify(payload, null, 2),
    },
  ];
}

// ── Fallback Tag Generation ─────────────────────────────────────────

export function buildFallbackMessages(
  request: LookupRequest,
  folderCandidate: AlbumCandidate,
  currentTracks: Array<{
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    trackNumber?: number | null;
  }>,
): Array<{ role: string; content: string }> {
  const payload = {
    artist_hint: request.artistHint,
    album_hint: request.albumHint,
    folder_candidate: candidateSummary(0, folderCandidate),
    current_tracks: currentTracks,
  };

  return [
    {
      role: "system",
      content:
        "Generate conservative fallback music tags as JSON. " +
        "Do not invent MusicBrainz IDs. Leave uncertain fields empty. " +
        "Return artist, artists, album, albumArtist, albumArtists, " +
        "tracks, genre, confidence, and reason. " +
        "For genre, use Discogs-style comma-separated tags " +
        "(e.g. 'Electronic, House, Deep House' or 'Rock, Alternative, Indie' " +
        "or 'Stage & Screen, Score, Contemporary Classical'). " +
        "Only include genre if you are confident in the classification; " +
        "leave null otherwise.",
    },
    {
      role: "user",
      content: JSON.stringify(payload, null, 2),
    },
  ];
}

// ── Folder Extraction ───────────────────────────────────────────────

export function buildFolderExtractionMessages(
  folderName: string,
  parentName: string | null,
): Array<{ role: string; content: string }> {
  const payload: Record<string, string | null> = {
    folder_name: folderName,
  };
  if (parentName) {
    payload.parent_name = parentName;
  }

  return [
    {
      role: "system",
      content:
        "Extract music metadata from a folder name. " +
        "Return JSON with artist (the primary performing artist), " +
        "album (the release title), year (4-digit release year), " +
        "and disc (disc number if present). " +
        "Leave fields null if uncertain. " +
        "Do not invent information.",
    },
    {
      role: "user",
      content: JSON.stringify(payload, null, 2),
    },
  ];
}

// ── Tag Correction ──────────────────────────────────────────────────

/**
 * Build a prompt asking the LLM to correct/resolve album and track tags
 * by comparing folder names, existing file metadata, and the basic parser's hints.
 *
 * The LLM returns:
 *   - artist / albumArtist / album / year / genre — corrected album-level fields
 *   - tracks — per-track corrections (title, artist)
 *   - confidence — how confident the LLM is
 *
 * This output is used for:
 *   1. Corrected search params fed to MusicBrainz/Discogs API queries
 *   2. Fallback candidate when all API lookups return nothing
 */
/**
 * Extra context the caller may provide to help the LLM make better decisions.
 * All fields are optional — when absent the payload omits them.
 */
export interface TagCorrectionContext {
  /** Full directory path (gives the LLM structural context). */
  fullPath?: string;
  /** List of audio filenames in the directory. */
  filenames?: string[];
  /** Unique album tags found in the audio files. */
  existingAlbumTags?: string[];
  /** Unique artist tags found in the audio files. */
  existingArtistTags?: string[];
}

export function buildTagCorrectionMessages(
  folderName: string,
  parentName: string | null,
  parsedArtistHint: string | null,
  parsedAlbumHint: string | null,
  parsedYearHint: string | null,
  currentTracks: Array<{
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    trackNumber?: number | null;
    genre?: string | null;
  }>,
  context?: TagCorrectionContext,
): Array<{ role: string; content: string }> {
  const payload: Record<string, unknown> = {
    folder_name: folderName,
    parsed_hints: {
      artist: parsedArtistHint,
      album: parsedAlbumHint,
      year: parsedYearHint,
    },
  };
  if (parentName) {
    payload.parent_name = parentName;
  }
  if (context?.fullPath) {
    payload.full_path = context.fullPath;
  }
  if (context?.filenames && context.filenames.length > 0) {
    payload.filenames = context.filenames;
  }
  if (context?.existingAlbumTags && context.existingAlbumTags.length > 0) {
    payload.existing_album_tags = context.existingAlbumTags;
  }
  if (context?.existingArtistTags && context.existingArtistTags.length > 0) {
    payload.existing_artist_tags = context.existingArtistTags;
  }
  if (currentTracks.length > 0) {
    payload.current_tracks = currentTracks.map((t, i) => ({
      index: i,
      title: t.title ?? null,
      artist: t.artist ?? null,
      album: t.album ?? null,
      track_number: t.trackNumber ?? null,
      genre: t.genre ?? null,
    }));
  }

  return [
    {
      role: "system",
      content:
        "Resolve correct music metadata by analyzing the folder name, " +
        "parent folder name, basic parser hints, and existing file tags.\n\n" +
        "CRITICAL: Your entire response must be ONLY a single JSON object. " +
        "Do NOT include any explanatory text, reasoning, analysis, " +
        "markdown formatting, or code fences before or after the JSON.\n\n" +
        "Rules:\n" +
        "1. Return JSON with artist, albumArtist, album, year, genre, tracks, and confidence.\n" +
        "2. The folder_name is the album directory (may include year prefix like '2009-', " +
        "format suffix like '[flac]', or other annotations). Strip these.\n" +
        "3. The parent_name (if present) is usually the artist or collection folder.\n" +
        "4. parsed_hints are from the basic folder-name parser — use them as a starting point.\n" +
        "5. current_tracks show existing metadata tags from the audio files — " +
        "use them to verify/correct artist, album, track titles.\n" +
        "6. If current_tracks have a consistent genre across all tracks, use it.\n" +
        "7. For genre, use Discogs-style comma-separated tags " +
        "(e.g. 'Hip Hop' or 'Electronic, House, Deep House').\n" +
        "8. Leave genre null if you are not confident.\n" +
        "9. For albumArtist: if all tracks share the same artist, use that. " +
        "If tracks have different artists (compilation/mixtape), use 'Various Artists'.\n" +
        "10. Per-track corrections: only include title/artist fields that differ " +
        "from the album defaults. Leave null for fields that match.\n" +
        "11. Do not invent MusicBrainz IDs. Leave uncertain fields null.\n" +
        "12. Set confidence (0.0-1.0) based on how sure you are about the correction.\n" +
        "13. parsed_hints may be WRONG — especially when the folder name contains a year " +
        "prefix followed by a number that looks like a month (e.g. '2009-100天' was " +
        "misparsed as '0天'). Always derive the album from folder_name by stripping " +
        "ONLY the 4-digit year+separator prefix. Never consume following album digits " +
        "as part of the date.\n" +
        "14. When folder_name uses 'Year - Artist - Album' or 'Year - Artist - Album " +
        "(format)' patterns, extract only the Album portion. Common separators: dash " +
        "(-), space, comma (,), underscore (_). Common format suffixes: (Lossless), " +
        "[FLAC], (24bit), (24bit-48Hz)(WAV).",
    },
    {
      role: "user",
      content: JSON.stringify(payload, null, 2),
    },
  ];
}

// ── Audit ──────────────────────────────────────────────────────────

export interface AuditContext {
  /** Discogs lookup alias (English search name for a Chinese artist). */
  discogsAlias?: string | null;
  reviewTargets?: Array<{
    index: number;
    field: string;
    current: string;
    expected?: string;
    evidence: string;
    reason: string;
  }>;
}

export function buildAuditMessages(
  albumArtistHint: string | null,
  albumHint: string | null,
  tracks: Array<{
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    albumArtist?: string | null;
    albumArtists?: string[];
    artists?: string[];
    year?: string | null;
    genre?: string | null;
    trackNumber?: number | null;
    trackTotal?: number | null;
    discNumber?: number | null;
    discTotal?: number | null;
    path?: string;
  }>,
  filenames: string[],
  context?: AuditContext,
): Array<{ role: string; content: string }> {
  const trackData = tracks.map((meta, i) => ({
    index: i,
    path: filenames[i] ?? "",
    title: meta.title ?? "",
    artist: meta.artist ?? "",
    album: meta.album ?? "",
    album_artist: meta.albumArtist ?? "",
    album_artists: (meta.albumArtists ?? []).join(", "),
    artists: (meta.artists ?? []).join(", "),
    year: meta.year ?? "",
    genre: meta.genre ?? "",
    track_number: meta.trackNumber ?? null,
    track_total: meta.trackTotal ?? null,
    disc_number: meta.discNumber ?? null,
    disc_total: meta.discTotal ?? null,
  }));

  const payload: Record<string, unknown> = {
    album_folder: albumHint ?? "",
    artist_folder: albumArtistHint ?? "",
    preferred_artist_name: albumArtistHint ?? "",
    tracks: trackData,
    review_targets: context?.reviewTargets ?? [],
  };

  if (context?.discogsAlias) {
    payload.discogs_lookup_alias = context.discogsAlias;
  }

  return [
    {
      role: "system",
      content:
        "You audit music track metadata. Deterministic code has already checked " +
        "obvious path/tag mismatches, so you only review `review_targets`.\n\n" +
        "Primary principle: file path evidence must match metadata. The album " +
        "folder is evidence for album/year, the parent artist folder is evidence " +
        "for album artist, and each filename is evidence for title, artist, and " +
        "track/disc numbers.\n\n" +
        "Rules:\n" +
        "1. Review only fields listed in `review_targets`; do not invent findings for other fields.\n" +
        "2. Return one field at a time. Each result's `corrected` object may contain only that result's field.\n" +
        "3. Include `confidence` from 0.0 to 1.0 for every warning or error.\n" +
        "4. 'error' means the correction is clearly right; 'warning' means manual review is needed.\n" +
        "5. If evidence is ambiguous, return warning with no `corrected` value.\n" +
        "6. For genre, use Discogs-style comma-separated terms only when confident; otherwise warn.\n" +
        "7. **Chinese name preference**: When the `preferred_artist_name` " +
        "(from folder/tags) is in Chinese and a `discogs_lookup_alias` is " +
        "provided, prefer the Chinese name in corrected metadata. The English " +
        "alias is only a search aid, not a replacement tag value.\n" +
        "8. For Chinese tracks, judge Simplified vs Traditional script from the filename.\n\n" +
        "Output examples:\n" +
        '{ "index": 0, "field": "genre", "status": "warning", ' +
        '"message": "Genre is missing and cannot be inferred confidently from path evidence", ' +
        '"confidence": 0.4 }\n' +
        '{ "index": 1, "field": "title", "status": "error", ' +
        '"message": "Title has a clear typo compared with filename", ' +
        '"suggestion": "Karma Police", "confidence": 0.95, ' +
        '"corrected": { "title": "Karma Police" } }\n\n' +
        "Return only valid JSON. No markdown, no code fences, no extra text.",
    },
    {
      role: "user",
      content: JSON.stringify(payload),
    },
  ];
}

// ── Genre Fill ───────────────────────────────────────────────────────

/**
 * Build a prompt asking the LLM to provide genre for an album that already
 * has all other metadata resolved (artist, album, tracks from MusicBrainz/Discogs).
 * Only called when no source (Discogs, LLM enhance) provided genre.
 *
 * Returns JSON with just: { genre: string | null, confidence: number }
 */
export function buildGenreFillMessages(
  artist: string | null,
  album: string | null,
  trackTitles: string[],
): Array<{ role: string; content: string }> {
  const payload = {
    artist,
    album,
    track_titles: trackTitles,
  };

  return [
    {
      role: "system",
      content:
        "Classify the musical genre of this album. " +
        "Return only JSON with genre (string or null) and confidence (0.0-1.0). " +
        "Use Discogs-style comma-separated tags " +
        "(e.g. 'Electronic, House, Deep House' or 'Rock, Alternative, Indie' " +
        "or 'Stage & Screen, Score, Contemporary Classical'). " +
        "Leave genre null if you are not confident. " +
        "Do not invent information or guess from artist name alone.",
    },
    {
      role: "user",
      content: JSON.stringify(payload, null, 2),
    },
  ];
}

function candidateSummary(
  index: number,
  candidate: AlbumCandidate,
): Record<string, unknown> {
  return {
    index,
    artist: candidate.artist,
    album: candidate.album,
    year: candidate.year,
    genre: candidate.genre,
    track_count: candidate.tracks.length,
    track_titles: candidate.tracks.map((t) => t.title).filter(Boolean),
    source: candidate.source,
    musicbrainz_albumid: candidate.musicbrainzAlbumId,
    musicbrainz_artistid: candidate.musicbrainzArtistId,
  };
}
