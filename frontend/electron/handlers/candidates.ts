/**
 * Normalized lookup candidate types, ported from Python auto_tagger.integrations.candidates.
 *
 * Used by every handler in the auto-tag chain to pass album/track metadata
 * between cache, dataset, MusicBrainz, Discogs, LLM, and fallback steps.
 */

import { createHash } from "node:crypto";

// ── Enums ───────────────────────────────────────────────────────────

export type LookupSource = "beets" | "dataset" | "discogs" | "folder" | "llm" | "musicbrainz";

// ── TrackCandidate ──────────────────────────────────────────────────

export interface TrackCandidate {
  title: string | null;
  artist: string | null;
  artists: string[];
  trackNumber: number | null;
  trackTotal: number | null;
  discNumber: number | null;
  discTotal: number | null;
  musicbrainzTrackId: string | null;
  length: number | null;
  genre: string | null;
}

export function makeTrackCandidate(
  overrides?: Partial<TrackCandidate>,
): TrackCandidate {
  return {
    title: null,
    artist: null,
    artists: [],
    trackNumber: null,
    trackTotal: null,
    discNumber: null,
    discTotal: null,
    musicbrainzTrackId: null,
    length: null,
    genre: null,
    ...overrides,
  };
}

export function trackCandidateToJson(t: TrackCandidate): Record<string, unknown> {
  return {
    title: t.title,
    artist: t.artist,
    artists: t.artists,
    track_number: t.trackNumber,
    track_total: t.trackTotal,
    disc_number: t.discNumber,
    disc_total: t.discTotal,
    musicbrainz_trackid: t.musicbrainzTrackId,
    length: t.length,
    genre: t.genre,
  };
}

export function trackCandidateFromJson(data: Record<string, unknown>): TrackCandidate {
  return makeTrackCandidate({
    title: (data.title as string) ?? null,
    artist: (data.artist as string) ?? null,
    artists: (data.artists as string[]) ?? [],
    trackNumber: (data.track_number as number) ?? null,
    trackTotal: (data.track_total as number) ?? null,
    discNumber: (data.disc_number as number) ?? null,
    discTotal: (data.disc_total as number) ?? null,
    musicbrainzTrackId: (data.musicbrainz_trackid as string) ?? null,
    length: (data.length as number) ?? null,
    genre: (data.genre as string) ?? null,
  });
}

// ── AlbumCandidate ──────────────────────────────────────────────────

export interface AlbumCandidate {
  artist: string | null;
  artists: string[];
  album: string | null;
  albumArtist: string | null;
  albumArtists: string[];
  year: string | null;
  genre: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  discogsArtistId: string | null;
  discogsReleaseId: string | null;
  tracks: TrackCandidate[];
  distance: number | null;
  source: LookupSource;
  verification: string | null;
}

export function makeAlbumCandidate(
  overrides?: Partial<AlbumCandidate>,
): AlbumCandidate {
  return {
    artist: null,
    artists: [],
    album: null,
    albumArtist: null,
    albumArtists: [],
    year: null,
    genre: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    discogsArtistId: null,
    discogsReleaseId: null,
    tracks: [],
    distance: null,
    source: "beets",
    verification: null,
    ...overrides,
  };
}

export function albumCandidateToJson(c: AlbumCandidate): Record<string, unknown> {
  return {
    artist: c.artist,
    artists: c.artists,
    album: c.album,
    album_artist: c.albumArtist,
    album_artists: c.albumArtists,
    year: c.year,
    genre: c.genre,
    musicbrainz_albumid: c.musicbrainzAlbumId,
    musicbrainz_artistid: c.musicbrainzArtistId,
    discogs_artist_id: c.discogsArtistId,
    discogs_release_id: c.discogsReleaseId,
    tracks: c.tracks.map(trackCandidateToJson),
    distance: c.distance,
    source: c.source,
    verification: c.verification,
  };
}

export function albumCandidateFromJson(data: Record<string, unknown>): AlbumCandidate {
  return makeAlbumCandidate({
    artist: (data.artist as string) ?? null,
    artists: (data.artists as string[]) ?? [],
    album: (data.album as string) ?? null,
    albumArtist: (data.album_artist as string) ?? null,
    albumArtists: (data.album_artists as string[]) ?? [],
    year: (data.year as string) ?? null,
    genre: (data.genre as string) ?? null,
    musicbrainzAlbumId: (data.musicbrainz_albumid as string) ?? null,
    musicbrainzArtistId: (data.musicbrainz_artistid as string) ?? null,
    discogsArtistId: (data.discogs_artist_id as string) ?? null,
    discogsReleaseId: (data.discogs_release_id as string) ?? null,
    tracks: ((data.tracks as Record<string, unknown>[]) ?? []).map(trackCandidateFromJson),
    distance: (data.distance as number) ?? null,
    source: (data.source as LookupSource) ?? "beets",
    verification: (data.verification as string) ?? null,
  });
}

export function candidatesToJson(candidates: AlbumCandidate[]): string {
  return JSON.stringify(candidates.map(albumCandidateToJson));
}

export function candidatesFromJson(payload: string): AlbumCandidate[] {
  return (JSON.parse(payload) as Record<string, unknown>[]).map(albumCandidateFromJson);
}

// ── LookupRequest ───────────────────────────────────────────────────

export interface LookupRequest {
  path: string;
  artistHint: string | null;
  albumHint: string | null;
  yearHint: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  discogsReleaseId: string | null;
  discogsArtistId: string | null;
  tracks: TrackCandidate[];
}

export function makeLookupRequest(
  overrides?: Partial<LookupRequest>,
): LookupRequest {
  return {
    path: "",
    artistHint: null,
    albumHint: null,
    yearHint: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    discogsReleaseId: null,
    discogsArtistId: null,
    tracks: [],
    ...overrides,
  };
}

export function lookupRequestToJson(r: LookupRequest): Record<string, unknown> {
  return {
    path: r.path,
    artist_hint: r.artistHint,
    album_hint: r.albumHint,
    year_hint: r.yearHint,
    musicbrainz_album_id: r.musicbrainzAlbumId,
    musicbrainz_artist_id: r.musicbrainzArtistId,
    discogs_release_id: r.discogsReleaseId,
    discogs_artist_id: r.discogsArtistId,
    tracks: r.tracks.map(trackCandidateToJson),
  };
}

export function lookupRequestFromJson(data: Record<string, unknown>): LookupRequest {
  return makeLookupRequest({
    path: (data.path as string) ?? "",
    artistHint: (data.artist_hint as string) ?? null,
    albumHint: (data.album_hint as string) ?? null,
    yearHint: (data.year_hint as string) ?? null,
    musicbrainzAlbumId: (data.musicbrainz_album_id as string) ?? null,
    musicbrainzArtistId: (data.musicbrainz_artist_id as string) ?? null,
    discogsReleaseId: (data.discogs_release_id as string) ?? null,
    discogsArtistId: (data.discogs_artist_id as string) ?? null,
    tracks: ((data.tracks as Record<string, unknown>[]) ?? []).map(trackCandidateFromJson),
  });
}

/**
 * Stable hash for cache keys.
 * Mirrors Python's query_hash() — uses sorted JSON keys for stability.
 */
export function queryHash(request: LookupRequest): string {
  const query: Record<string, unknown> = {
    artist_hint: request.artistHint,
    album_hint: request.albumHint,
    musicbrainz_album_id: request.musicbrainzAlbumId,
    musicbrainz_artist_id: request.musicbrainzArtistId,
    discogs_release_id: request.discogsReleaseId,
    discogs_artist_id: request.discogsArtistId,
    tracks: request.tracks.map((t) => ({
      title: t.title,
      track_number: t.trackNumber,
      disc_number: t.discNumber,
    })),
    track_count: request.tracks.length,
  };
  const payload = JSON.stringify(query, Object.keys(query).sort());
  return createHash("sha256").update(payload).digest("hex");
}

// ── Text normalization ──────────────────────────────────────────────

const WHITESPACE_RE = /\s+/g;

/**
 * Strip ASCII punctuation characters (leaves CJK and other scripts intact).
 * Unlike `[^\w\s]` we explicitly list the ASCII punctuation to avoid
 * stripping CJK ideographs which are not matched by `\w` in JavaScript.
 */
const ASCII_PUNCTUATION_RE = /[!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~]/g;

/**
 * Normalize text for case/punctuation-insensitive comparison.
 * Applies NFKC normalization, case-folds, strips ASCII punctuation, trims whitespace.
 * Preserves CJK, Cyrillic, and other non-ASCII characters.
 */
export function normalizeLookupText(value: string | null): string {
  if (!value) return "";
  return value
    .normalize("NFKC")
    .toLowerCase()
    .replace(ASCII_PUNCTUATION_RE, " ")
    .replace(WHITESPACE_RE, " ")
    .trim();
}

/**
 * Compare a lookup hint against a candidate's album name.
 *
 * Returns:
 *   "match" — identical after normalization, or when there's no hint to compare
 *   "close" — one string contains the other
 *   "mismatch" — no match
 */
export function verifyAlbumName(
  hint: string | null,
  candidate: AlbumCandidate,
): string {
  if (!hint || !candidate.album) return "match";

  const hintNorm = normalizeLookupText(hint);
  const candNorm = normalizeLookupText(candidate.album);
  if (hintNorm === candNorm) return "match";

  // Substring check
  if (hintNorm.includes(candNorm) || candNorm.includes(hintNorm)) return "close";

  return "mismatch";
}

// ── Artist normalization ───────────────────────────────────────────

const MULTI_ARTIST_RE =
  /\s+(?:feat\.?|ft\.?|featuring)\s+|\s*[&/;,]\s*|\s*[＋+\uFF0B]\s*|\s*[、，；]\s*|\s*[·‧\u00B7]\s*|(?<=[\u4e00-\u9fff\u3400-\u4dbf])\.(?=[\u4e00-\u9fff\u3400-\u4dbf])/i;

export function splitArtistNames(values: Array<string | null | undefined>): string[] {
  const result: string[] = [];
  const seen = new Set<string>();

  for (const value of values) {
    if (!value) continue;
    for (const part of value.split(MULTI_ARTIST_RE)) {
      const artist = part.trim();
      const key = artist.toLocaleLowerCase();
      if (!artist || seen.has(key)) continue;
      seen.add(key);
      result.push(artist);
    }
  }

  return result;
}

export function artistDisplayName(artists: string[], fallback: string | null): string | null {
  const normalized = artists.length > 0 ? artists : splitArtistNames([fallback]);
  if (normalized.length > 0) return normalized.join(" & ");
  return fallback;
}

export function buildLookupVariantPairs(
  artist: string | null | undefined,
  album: string | null | undefined,
): Array<[string, string]> {
  const artistText = artist ?? "";
  const albumText = album ?? "";
  const pairs: Array<[string, string]> = [];
  const addPair = (nextArtist: string, nextAlbum: string) => {
    if (!pairs.some(([a, b]) => a === nextArtist && b === nextAlbum)) {
      pairs.push([nextArtist, nextAlbum]);
    }
  };

  try {
    const { Converter } = require("opencc-js");
    const s2t = Converter({ from: "cn", to: "tw" });
    const t2s = Converter({ from: "tw", to: "cn" });
    addPair(t2s(artistText), t2s(albumText));
    addPair(s2t(artistText), s2t(albumText));
  } catch {
    // opencc-js is optional in some test/dev installs.
  }

  addPair(artistText, albumText);
  return pairs;
}
