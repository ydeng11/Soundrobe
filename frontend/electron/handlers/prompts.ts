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
        "12. Set confidence (0.0-1.0) based on how sure you are about the correction.",
    },
    {
      role: "user",
      content: JSON.stringify(payload, null, 2),
    },
  ];
}

// ── Audit ──────────────────────────────────────────────────────────

export function buildAuditMessages(
  albumArtistHint: string | null,
  albumHint: string | null,
  tracks: Array<{
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    albumArtist?: string | null;
    artists?: string[];
    path?: string;
  }>,
  filenames: string[],
): Array<{ role: string; content: string }> {
  const trackData = tracks.map((meta, i) => ({
    index: i,
    path: filenames[i] ?? "",
    title: meta.title ?? "",
    artist: meta.artist ?? "",
    album: meta.album ?? "",
    album_artist: meta.albumArtist ?? "",
    artists: (meta.artists ?? []).join(", "),
  }));

  const payload = {
    album_folder: albumHint ?? "",
    artist_folder: albumArtistHint ?? "",
    tracks: trackData,
  };

  return [
    {
      role: "system",
      content:
        "You audit music track metadata. The **primary principle** is that " +
        "the file path must match the metadata: the album folder name should " +
        "match the `album` field, the parent artist folder should match the " +
        "`album_artist` field, and each filename should match its `title` field.\n\n" +
        "The input provides:\n" +
        "- `album_folder`: the album directory name (authoritative source for album name)\n" +
        "- `artist_folder`: the parent directory name (authoritative source for album_artist)\n" +
        "- `tracks[].path`: the filename (authoritative source for track title)\n" +
        "- `tracks[].album`, `tracks[].album_artist`, `tracks[].title`: current metadata values\n\n" +
        "Rules:\n" +
        "1. 'error' means a field is clearly wrong or missing.\n" +
        "2. 'warning' means the field might be wrong (typo, inconsistent capitalization, " +
        "mismatched artist/album_artist convention).\n" +
        "3. 'correct' means the field looks right.\n" +
        "4. **Compare `tracks[].album` with `album_folder`.** If the album folder suggests " +
        "a different album name than what is tagged, flag the `album` field. " +
        "The folder name is authoritative.\n" +
        "5. **Compare `tracks[].album_artist` with `artist_folder`.** If the parent " +
        "artist directory suggests a different artist than what is tagged, " +
        "flag the `album_artist` field. The folder name is authoritative.\n" +
        "6. **Compare `tracks[].title` with `tracks[].path` (filename).** " +
        "If the filename suggests a different title (after stripping track numbers, " +
        "separators, and extensions), flag the `title` field.\n" +
        "7. If artist != album_artist and artists is empty, flag artists as warning.\n" +
        "8. Don't flag empty album_artist on single-artist albums.\n" +
        "9. Be conservative — only flag when you have reasonable confidence.\n" +
        "10. Title casing variations ('Come Together' vs 'come together') are warnings, " +
        "not errors.\n" +
        "11. For Chinese tracks: judge the correct character script " +
        "(Simplified vs Traditional) based on the filename. The filename " +
        "is the authoritative source for which script to use.\n" +
        "12. **For every track with a warning or error, provide the complete " +
        "corrected metadata in the `corrected` field.** Populate `corrected` " +
        "with all the metadata fields that the track SHOULD have — title, " +
        "artist, artists, album, album_artist, year, genre. Only include " +
        "fields that are relevant (the code will merge your corrected values " +
        "with the existing metadata).\n" +
        "13. The `suggestion` field is used for per-field display " +
        "but `corrected` is what gets written to the file.\n\n" +
        "Examples of path-metadata matches (no issues):\n" +
        "- album_folder='OK Computer', album='OK Computer' → correct\n" +
        "- artist_folder='Radiohead', album_artist='Radiohead' → correct\n" +
        "- path='01. Karma Police.flac', title='Karma Police' → correct\n" +
        "\n" +
        "Examples of path-metadata mismatches (with corrected metadata):\n" +
        "- album_folder='OK Computer', album='OK Computer 1997' (wrong) → " +
        '{ "index": 0, "field": "album", "status": "error", ' +
        '"message": "Album tag \'OK Computer 1997\' does not match folder name \'OK Computer\'", ' +
        '"suggestion": "OK Computer", ' +
        '"corrected": { "album": "OK Computer" } }\n' +
        "- artist_folder='Pink Floyd', album_artist='Pink Floyd' but artist='David Gilmour' → " +
        '{ "index": 0, "field": "artist", "status": "warning", ' +
        '"message": "Track artist \'David Gilmour\' differs from album_artist " +' +
        '"\'Pink Floyd\', which matches the artist folder", ' +
        '"suggestion": "Pink Floyd", ' +
        '"corrected": { "artist": "Pink Floyd" } }\n' +
        "- path='01. 我爱的人.flac', title='I Love You' (English, not matching filename) → " +
        '{ "index": 0, "field": "title", "status": "error", ' +
        '"message": "Title \'I Love You\' does not match filename \'01. 我爱的人.flac\'", ' +
        '"suggestion": "我爱的人", ' +
        '"corrected": { "title": "我爱的人" } }\n' +
        "- filename='03 - Bohemian Rhapsody.flac', title='Bohemian Rhapsody (Remastered 2011)' → " +
        '{ "index": 0, "field": "title", "status": "warning", ' +
        '"message": "Title \'Bohemian Rhapsody (Remastered 2011)\' may have extra suffix not in filename", ' +
        '"suggestion": "Bohemian Rhapsody", ' +
        '"corrected": { "title": "Bohemian Rhapsody" } }\n\n' +
        "Other common patterns (preserved from existing logic):\n" +
        '- artist=\'Beatles\' (missing \'The\') → ' +
        '{ "index": 0, "field": "artist", "status": "warning", ' +
        '"message": "Artist may be missing \'The\'", "suggestion": "The Beatles" }\n' +
        "- year='20' (truncated) → " +
        '{ "index": 0, "field": "year", "status": "error", ' +
        '"message": "Year truncated to 2 digits", "suggestion": "2020" }\n' +
        "- title is placeholder string → " +
        '{ "index": 0, "field": "title", "status": "error", ' +
        '"message": "Title is placeholder text, not real track name" }\n\n' +
        "Return only valid JSON. No markdown, no code fences, no extra text.",
    },
    {
      role: "user",
      content: JSON.stringify(payload),
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
