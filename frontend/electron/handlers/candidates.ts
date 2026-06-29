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

/** Strip Unicode punctuation chars that commonly appear in album titles:
 *  - U+2026 (…) horizontal ellipsis
 *  - U+3000 ideographic space
 *  - U+FEFF BOM
 *  These aren't covered by ASCII_PUNCTUATION_RE but cause false mismatches
 *  between MusicBrainz titles (which often append …) and file metadata. */
const UNICODE_PUNCT_RE = /[\u2026\u3000\uff00-\uffef\u2000-\u206f\u2010-\u2015\u2018-\u2019\u201c-\u201d\u3008-\u3011\u3014-\u3015]/g;

/** Strip combining diacritical marks (U+0300–U+036F) after NFKD decomposition. */
const COMBINING_DIACRITICAL_RE = /[\u0300-\u036f]/g;

/** Strip spaces between CJK ideographs (spaces in CJK titles are meaningless). */
const CJK_SPACE_RE = /([\u3400-\u9fff])\s+([\u3400-\u9fff])/g;

/**
 * Roman numeral map (I–XII) for standalone numeral conversion.
 * Longest-first regex alternation ensures `viii` matches before `vi` etc.
 */
const ROMAN_MAP = new Map([
  ["xii", "12"], ["xi", "11"], ["x", "10"],
  ["ix", "9"], ["viii", "8"], ["vii", "7"], ["vi", "6"],
  ["v", "5"], ["iv", "4"], ["iii", "3"], ["ii", "2"], ["i", "1"],
]);
const ROMAN_RE = /\b(xii|xi|x|ix|viii|vii|vi|v|iv|iii|ii|i)\b/gi;

/**
 * Normalize text for case/punctuation-insensitive comparison.
 *
 * Pipeline:
 *  1. NFKD decomposition (fullwidth→halfwidth, decomposes ligatures/diacritics)
 *  2. Lowercase
 *  3. Strip ASCII punctuation → space
 *  4. Strip Unicode punctuation → space
 *  5. Collapse whitespace → single space, trim
 *  6. Convert standalone Roman numerals (I–XII) → Arabic
 *  7. Strip combining diacritical marks (accented chars → base chars)
 *  8. Strip spaces between CJK ideographs
 *
 * Preserves CJK, Cyrillic, and other non-ASCII characters.
 */
export function normalizeLookupText(value: string | null): string {
  if (!value) return "";
  let result = value
    .normalize("NFKD")
    .toLowerCase()
    .replace(ASCII_PUNCTUATION_RE, " ")
    .replace(UNICODE_PUNCT_RE, " ")
    .replace(WHITESPACE_RE, " ")
    .trim();

  // Convert standalone Roman numerals to Arabic (longest-first regex prevents
  // `viii` matching as `v` + `iii` etc.).
  result = result.replace(ROMAN_RE, (m) => ROMAN_MAP.get(m.toLowerCase()) ?? m);

  // Strip combining diacritical marks (NFKD decomposes é → e + ´, then strip ´).
  result = result.replace(COMBINING_DIACRITICAL_RE, "");

  // Strip spaces between CJK ideographs (loop handles consecutive sequences
  // like "红 光 辉" → "红光辉").
  let prev: string;
  do {
    prev = result;
    result = result.replace(CJK_SPACE_RE, "$1$2");
  } while (result !== prev);

  return result;
}

export const ALBUM_TITLE_MATCH_THRESHOLD = 75;

const MIN_CJK_CONTAINMENT_LENGTH = 4;
const MIN_LATIN_TOKEN_LENGTH = 3;
const CJK_RE = /[\u3400-\u4dbf\u4e00-\u9fff]/;

function unique(values: string[]): string[] {
  return [...new Set(values.filter(Boolean))];
}

function compactLength(value: string): number {
  return value.replace(/\s+/g, "").length;
}

function hasUsefulLatinToken(value: string): boolean {
  return value.split(/\s+/).some((token) => token.length >= MIN_LATIN_TOKEN_LENGTH);
}

function canUseContainmentMatch(contained: string): boolean {
  if (!contained) return false;
  if (CJK_RE.test(contained)) {
    return compactLength(contained) >= MIN_CJK_CONTAINMENT_LENGTH;
  }
  return hasUsefulLatinToken(contained);
}

function hasCjk(value: string): boolean {
  return CJK_RE.test(value);
}

/**
 * Normalize text into equivalent forms for provider title matching.
 *
 * Keeps normalizeLookupText synchronous for existing dataset/cache paths, while
 * adding OpenCC Simplified/Traditional variants for remote CJK matching.
 */
export async function normalizedLookupForms(value: string | null): Promise<string[]> {
  const base = normalizeLookupText(value);
  if (!base) return [];
  const forms = [base];
  try {
    const mod = await import("opencc-js");
    const s2t = mod.Converter({ from: "cn", to: "tw" });
    const t2s = mod.Converter({ from: "tw", to: "cn" });
    forms.push(normalizeLookupText(s2t(value ?? "")));
    forms.push(normalizeLookupText(t2s(value ?? "")));
  } catch {
    // opencc-js is optional in some test/runtime shells.
  }
  return unique(forms);
}

export interface AlbumTitleMatchScoreOptions {
  localYear?: string | number | null;
  remoteYear?: string | number | null;
  artistMatches?: boolean;
  trackCountMatches?: boolean;
}

export interface AlbumTitleMatchScore {
  score: number;
  reason: "exact" | "remote-contains-local" | "local-contains-remote" | "fuzzy" | "none";
}

function yearPrefix(value: string | number | null | undefined): string | null {
  if (value == null) return null;
  const match = String(value).match(/\d{4}/);
  return match ? match[0] : null;
}

/**
 * Compute Longest Common Subsequence (LCS) length between two strings.
 * Used for fuzzy character-level similarity when exact and containment
 * matches fail (e.g. 记 vs 紀 variants).
 */
function lcsLength(a: string, b: string): number {
  if (a.length === 0 || b.length === 0) return 0;
  // Space-optimized: only keep previous and current row.
  let prev = new Uint16Array(b.length + 1);
  let curr = new Uint16Array(b.length + 1);
  for (let i = 1; i <= a.length; i++) {
    for (let j = 1; j <= b.length; j++) {
      if (a[i - 1] === b[j - 1]) {
        curr[j] = prev[j - 1] + 1;
      } else {
        curr[j] = Math.max(prev[j], curr[j - 1]);
      }
    }
    [prev, curr] = [curr, prev];
    curr.fill(0);
  }
  return prev[b.length];
}

/**
 * Fuzzy similarity ratio between two strings (0–100).
 * Uses 2 × LCS / (len(a) + len(b)) which equals the Sørensen–Dice
 * coefficient for character bigrams, generalized to any overlap metric.
 */
function fuzzySimilarity(a: string, b: string): number {
  const lcs = lcsLength(a, b);
  if (a.length + b.length === 0) return 100;
  return Math.round((200 * lcs) / (a.length + b.length));
}

const FUZZY_SIMILARITY_THRESHOLD = 80;

/**
 * Score a local album hint against a provider title.
 *
 * Exact normalized title matches score 100. Containment matches are accepted
 * only when the contained text is specific enough, so short CJK/Latin fragments
 * cannot accidentally match unrelated releases. Fuzzy character-level matching
 * handles CJK variant pairs (e.g. 记/纪) that opencc does not convert.
 */
export async function scoreAlbumTitleMatch(
  localHint: string | null,
  remoteTitle: string | null,
  options: AlbumTitleMatchScoreOptions = {},
): Promise<AlbumTitleMatchScore> {
  const localForms = await normalizedLookupForms(localHint);
  const remoteForms = await normalizedLookupForms(remoteTitle);
  if (localForms.length === 0 || remoteForms.length === 0) {
    return { score: 0, reason: "none" };
  }

  let score = 0;
  let reason: AlbumTitleMatchScore["reason"] = "none";

  for (const local of localForms) {
    for (const remote of remoteForms) {
      if (local === remote) {
        score = Math.max(score, 100);
        reason = "exact";
      } else if (remote.includes(local) && canUseContainmentMatch(local)) {
        if (score < 85) {
          score = 85;
          reason = "remote-contains-local";
        }
      } else if (local.includes(remote) && canUseContainmentMatch(remote)) {
        if (score < 70) {
          score = 70;
          reason = "local-contains-remote";
        }
      } else if (hasCjk(local) || hasCjk(remote)) {
        // Fuzzy fallback: character-level similarity for CJK variant pairs
        // (e.g. 记/纪) that opencc does not convert.
        const similarity = fuzzySimilarity(local, remote);
        if (similarity >= FUZZY_SIMILARITY_THRESHOLD) {
          const fuzzyScore = Math.round(similarity * 0.85); // cap below containment
          if (fuzzyScore > score) {
            score = fuzzyScore;
            reason = "fuzzy";
          }
        }
      }
    }
  }

  if (score > 0) {
    const localYear = yearPrefix(options.localYear);
    const remoteYear = yearPrefix(options.remoteYear);
    if (localYear && remoteYear && localYear === remoteYear) score += 10;
    if (options.artistMatches) score += 10;
    if (options.trackCountMatches) score += 10;
  }

  return { score, reason };
}

/**
 * Normalize CJK text by also trying Simplified/Traditional Chinese variants
 * via OpenCC-js. Returns an array of normalized forms (original + variants).
 */
async function addOpenCCNormalized(name: string): Promise<string[]> {
  const forms = [name];
  try {
    const mod = await import("opencc-js");
    const s2t = mod.Converter({ from: "cn", to: "tw" });
    const t2s = mod.Converter({ from: "tw", to: "cn" });
    const s2tNorm = normalizeLookupText(s2t(name));
    const t2sNorm = normalizeLookupText(t2s(name));
    if (s2tNorm !== name) forms.push(s2tNorm);
    if (t2sNorm !== name && t2sNorm !== s2tNorm) forms.push(t2sNorm);
  } catch {
    // opencc-js not available — fall back to original only
  }
  return forms;
}

/**
 * Compare a lookup hint against a candidate's album name.
 *
 * Returns:
 *   "match" — identical after normalization, or when there's no hint to compare
 *   "close" — one string contains the other
 *   "mismatch" — no match
 */
export async function verifyAlbumName(
  hint: string | null,
  candidate: AlbumCandidate,
): Promise<string> {
  if (!hint || !candidate.album) return "match";

  const hintNorm = normalizeLookupText(hint);
  const candNorm = normalizeLookupText(candidate.album);
  if (hintNorm === candNorm) return "match";

  // Try OpenCC variants for Simplified/Traditional Chinese
  const hintForms = await addOpenCCNormalized(hintNorm);
  const candForms = await addOpenCCNormalized(candNorm);
  const hasExactFormMatch = hintForms.some((hf) => candForms.includes(hf));
  if (hasExactFormMatch) return "match";

  // Substring and fuzzy similarity checks (also cross-variant).
  // Fuzzy is guarded by hasCjk to avoid false positives with Latin text.
  for (const hf of hintForms) {
    for (const cf of candForms) {
      if (hf.includes(cf) || cf.includes(hf)) return "close";
      if ((hasCjk(hf) || hasCjk(cf)) && fuzzySimilarity(hf, cf) >= FUZZY_SIMILARITY_THRESHOLD) {
        return "close";
      }
    }
  }

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
