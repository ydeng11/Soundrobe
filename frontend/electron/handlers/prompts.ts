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

// ── Helper ──────────────────────────────────────────────────────────

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
