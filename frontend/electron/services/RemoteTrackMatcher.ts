/**
 * Remote Track Matcher — deterministic per-file matching for auto-tag.
 *
 * Replaces the coarse positional guard with title+duration alignment.
 * Pure business logic: no Electron APIs, testable in plain Node.js.
 */

import type { TrackCandidate } from "../handlers/candidates";

// ── Annotation-stripping constants ────────────────────────────────

const ANNOTATION_KEYWORDS = new Set([
  "live",
  "remaster",
  "version",
  "karaoke",
  "instrumental",
  "伴奏",
  "现场",
  "現場",
  "片头",
  "片頭",
  "片尾",
  "theme",
]);

// Build annotation-stripping patterns using character-alternative groups
// instead of negated character classes for Unicode safety.
// Each pattern matches a trailing suffix containing an annotation keyword.
const annotationKwPattern = [...ANNOTATION_KEYWORDS].map(escapeRegex).join("|");

// Pattern: (text keyword text) or (text keyword text) — fullwidth parens
const ANNOTATION_PAREN_RE = new RegExp(
  "[(（][\\s\\S]*?(?:" + annotationKwPattern + ")[\\s\\S]*?[)）]\\s*$",
  "i",
);

// Pattern: [text keyword text]
const ANNOTATION_BRACKET_RE = new RegExp(
  "\\[[\\s\\S]*?(?:" + annotationKwPattern + ")[\\s\\S]*?\\]\\s*$",
  "i",
);

// Pattern: trailing standalone keyword preceded by space/dash
const ANNOTATION_TRAILING_RE = new RegExp(
  "[-\\u2013\\u2014\\s]+(?:" + annotationKwPattern + ")\\s*$",
  "i",
);

const ANNOTATION_PATTERNS = [
  ANNOTATION_PAREN_RE,
  ANNOTATION_BRACKET_RE,
  ANNOTATION_TRAILING_RE,
];

// ── Filename parsing ─────────────────────────────────────────────

const FILENAME_TRACK_PREFIX_RE = /^(\d+)[\s.\u2010\u2011\u2012\u2013\u2014\u2015\-]*/;
const FILENAME_ARTIST_PREFIX_RE =
  /^(.*?)[\s\-–—]+(?:[-–—]|[\u2013\u2014])\s*/;
const FILENAME_DIRECT_ARTIST_SEPARATOR_RE = /[-–—]/g;

// ── Unicode punctuation/symbol regex ──────────────────────────────

const UNICODE_PUNCT_SYMBOL_RE = /[\p{P}\p{S}]+/gu;
const WHITESPACE_RE = /\s+/g;

// ── OpenCC lazy loader ───────────────────────────────────────────

let openCCInstance: {
  s2t: (s: string) => string;
  t2s: (s: string) => string;
} | null = null;

async function getOpenCC(): Promise<{
  s2t: (s: string) => string;
  t2s: (s: string) => string;
} | null> {
  if (openCCInstance) return openCCInstance;
  try {
    const mod = await import("opencc-js");
    const s2t = mod.Converter({ from: "cn", to: "tw" });
    const t2s = mod.Converter({ from: "tw", to: "cn" });
    openCCInstance = { s2t, t2s };
    return openCCInstance;
  } catch {
    return null;
  }
}

// ── Helpers ──────────────────────────────────────────────────────

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Strip trailing annotation suffixes from a title.
 * Applied BEFORE generalized punctuation stripping so bracket structure is intact.
 */
function stripAnnotations(title: string): string {
  // NFKC normalizes fullwidth/Latin variants (e.g. Ａ→A, （→(, Ｌｉｖｅ→Live)
  // so annotation keyword matching works across script widths.
  let result = title.normalize("NFKC").trim();
  for (const pattern of ANNOTATION_PATTERNS) {
    result = result.replace(pattern, "");
  }
  return result.trim();
}

/**
 * Normalize punctuation/symbols in a title.
 * Assumes annotations have already been stripped.
 */
function stripPunctuationAndSymbols(title: string): string {
  return title
    .normalize("NFKC")
    .toLowerCase()
    .replace(UNICODE_PUNCT_SYMBOL_RE, " ")
    .replace(WHITESPACE_RE, " ")
    .trim();
}

// ── Exported title form generation ───────────────────────────────

export interface NormalizedTitleForm {
  text: string;
  source: "tag" | "filename";
}

export interface RemoteTrackMatcherOptions {
  artistHints?: string[];
}

/**
 * Generate all normalized title forms for a local track.
 *
 * From the tag title:
 *   1. NFKC + lowercase + strip annotations → strip punctuation/symbol
 *   2. Same as above but Simplified → Traditional Chinese
 *   3. Same as above but Traditional → Simplified Chinese
 *
 * From the filename stem:
 *   4. Strip extension + leading track number + Artist - prefix, then same normalization
 *   5–6. Same SC/TC variants as above
 *
 * Empty forms are filtered out.
 */
export async function generateTitleForms(
  tagTitle: string | null,
  filename: string,
  knownArtists: string[] = [],
): Promise<NormalizedTitleForm[]> {
  const forms: NormalizedTitleForm[] = [];
  const oc = await getOpenCC();

  const addForm = (text: string, source: "tag" | "filename") => {
    if (!text) return;
    const normalized = stripPunctuationAndSymbols(text);
    if (!normalized) return;

    // Deduplicate within the same source
    const exists = forms.some(
      (f) => f.source === source && f.text === normalized,
    );
    if (!exists) forms.push({ text: normalized, source });
  };

  // ── Tag title forms ─────────────────────────────────────────
  if (tagTitle && tagTitle.trim()) {
    const cleaned = stripAnnotations(tagTitle.trim());
    addForm(cleaned, "tag");

    if (oc) {
      addForm(oc.s2t(cleaned), "tag");
      addForm(oc.t2s(cleaned), "tag");
    }
  }

  // ── Filename-derived forms ──────────────────────────────────
  const stem = extractFilenameStem(filename, knownArtists);
  if (stem) {
    addForm(stem, "filename");

    if (oc) {
      addForm(oc.s2t(stem), "filename");
      addForm(oc.t2s(stem), "filename");
    }
  }

  return forms;
}

/**
 * Extract a cleaned title from a filename stem.
 * Used when writing back the filename-derived title for matched tracks.
 */
export function cleanFilenameTitle(
  filename: string,
  knownArtists: string[] = [],
): string | null {
  const stem = extractFilenameStem(filename, knownArtists);
  if (!stem) return null;
  return stripAnnotations(stem).trim() || null;
}

/**
 * Extract a cleaned, unnormalized filename stem.
 * Strip extension, leading track number, common Artist - prefix.
 */
function extractFilenameStem(
  filename: string,
  knownArtists: string[] = [],
): string | null {
  if (!filename) return null;
  // Strip extension
  const dotIdx = filename.lastIndexOf(".");
  const base = dotIdx > 0 ? filename.slice(0, dotIdx) : filename;
  if (!base.trim()) return null;

  // Strip leading track number
  let stripped = base.replace(FILENAME_TRACK_PREFIX_RE, "").trim();
  if (!stripped) return null;

  // Strip common Artist - Title prefix pattern
  const artistPrefixMatch = stripped.match(FILENAME_ARTIST_PREFIX_RE);
  if (artistPrefixMatch) {
    const artistPart = artistPrefixMatch[1].trim();
    const titlePart = stripped.slice(artistPrefixMatch[0].length).trim();
    if (artistPart.length > 0 && titlePart.length > 0) {
      stripped = titlePart;
    }
  } else {
    const titlePart = stripKnownArtistPrefix(stripped, knownArtists);
    if (titlePart) stripped = titlePart;
  }

  return stripped.trim() || null;
}

function stripKnownArtistPrefix(
  stem: string,
  knownArtists: string[],
): string | null {
  const normalizedArtists = knownArtists
    .map((artist) => stripPunctuationAndSymbols(stripAnnotations(artist)))
    .filter((artist) => artist.length > 0);
  if (normalizedArtists.length === 0) return null;

  for (const match of stem.matchAll(FILENAME_DIRECT_ARTIST_SEPARATOR_RE)) {
    const index = match.index;
    if (index == null || index <= 0) continue;
    const artistPart = stem.slice(0, index).trim();
    const titlePart = stem.slice(index + match[0].length).trim();
    if (!artistPart || !titlePart) continue;
    const normalizedArtistPart = stripPunctuationAndSymbols(
      stripAnnotations(artistPart),
    );
    if (normalizedArtists.includes(normalizedArtistPart)) {
      return titlePart;
    }
  }

  return null;
}

/**
 * Normalize a track length to seconds.
 * MusicBrainz returns milliseconds (>1000), other sources return seconds.
 */
export function normalizeDurationSeconds(
  value: number | null | undefined,
  source: string,
): number | null {
  if (value == null || value <= 0) return null;
  // MusicBrainz durations are in milliseconds
  if (source === "musicbrainz" && value > 1000) {
    return value / 1000;
  }
  // All other sources (or values already <= 1000) are seconds
  return value;
}

/**
 * Check if two durations are close enough to be considered a match.
 * Requires closeness within max(5 seconds, 3% of local duration).
 */
export function durationsMatch(
  localDuration: number | null,
  remoteDuration: number | null,
): boolean {
  if (localDuration == null || remoteDuration == null) return false;
  const diff = Math.abs(localDuration - remoteDuration);
  const threshold = Math.max(5, localDuration * 0.03);
  return diff <= threshold;
}

// ── Matcher stats ───────────────────────────────────────────────

export interface MatchStats {
  matched: number;
  local: number;
  remote: number;
  skipped: SkipReason[];
}

export interface SkipReason {
  localIndex: number;
  localTitle: string | null;
  reason: "no_title_match" | "duration_mismatch" | "duplicate_ambiguous" | "no_local_evidence";
}

// ── Main matcher ───────────────────────────────────────────────

export interface MatchedCandidate {
  /** Tracks aligned to local file order, with safe per-track fields applied. */
  tracks: TrackCandidate[];
  /** Match statistics for observability. */
  stats: MatchStats;
  /** Whether every local track has a unique remote match and counts align. */
  isFullOrderedMatch: boolean;
}

/**
 * Match remote candidate tracks against local tracks by title + duration.
 *
 * Input:
 *   localTracks — TrackCandidate[] in sorted filesystem order
 *   filenames — sorted filenames (from dir listing) in the same order
 *   remoteTracks — TrackCandidate[] from the remote candidate
 *   source — remote source name ("musicbrainz", "discogs", "dataset")
 *
 * Output:
 *   tracks — aligned to local file order with safe per-track fields
 *   stats — match/skip counts
 *   isFullOrderedMatch — whether all local tracks matched uniquely
 */
export async function matchRemoteCandidateTracks(
  localTracks: TrackCandidate[],
  filenames: string[],
  remoteTracks: TrackCandidate[],
  source: string,
  options: RemoteTrackMatcherOptions = {},
): Promise<MatchedCandidate> {
  const stats: MatchStats = {
    matched: 0,
    local: localTracks.length,
    remote: remoteTracks.length,
    skipped: [],
  };

  if (localTracks.length === 0 || remoteTracks.length === 0) {
    return {
      tracks: localTracks.map((t) => ({ ...t })),
      stats,
      isFullOrderedMatch: false,
    };
  }

  // 1. Build title forms for each local track
  const localForms: Array<{
    tagForms: string[];
    filenameForms: string[];
    tagTitle: string | null;
    filename: string;
  }> = [];

  for (let i = 0; i < localTracks.length; i++) {
    const lt = localTracks[i];
    const fname = filenames[i] ?? "";
    const knownArtists = [
      ...(options.artistHints ?? []),
      ...(lt.artist ? [lt.artist] : []),
      ...lt.artists,
    ];
    const forms = await generateTitleForms(lt.title, fname, knownArtists);
    localForms.push({
      tagForms: forms.filter((f) => f.source === "tag").map((f) => f.text),
      filenameForms: forms.filter((f) => f.source === "filename").map((f) => f.text),
      tagTitle: lt.title,
      filename: fname,
    });
  }

  // 2. Normalize remote titles and durations
  //    Strip annotations first, THEN punctuation/symbols
  const remoteMeta = remoteTracks.map((rt, i) => {
    const annotationStripped = stripAnnotations(rt.title ?? "");
    return {
      index: i,
      title: annotationStripped,
      normalized: stripPunctuationAndSymbols(annotationStripped),
      duration: normalizeDurationSeconds(rt.length, source),
      track: rt,
    };
  });

  // 3. Build title-form index for remote tracks
  //    Maps normalized title → array of remote indices
  const remoteTitleIndex = new Map<string, number[]>();
  for (const rm of remoteMeta) {
    if (!rm.normalized) continue;
    const existing = remoteTitleIndex.get(rm.normalized) ?? [];
    existing.push(rm.index);
    remoteTitleIndex.set(rm.normalized, existing);
  }

  // 4. For each local track, try to find a unique remote match
  const matchedRemote = new Set<number>();
  const matchedLocal = new Array<number | null>(localTracks.length).fill(null);

  for (let i = 0; i < localTracks.length; i++) {
    const lf = localForms[i];
    const localDuration = normalizeDurationSeconds(localTracks[i].length, "local");

    // Try tag forms first (more reliable), then filename forms
    const allForms = [
      ...lf.tagForms.map((f) => ({ form: f, source: "tag" as const })),
      ...lf.filenameForms.map((f) => ({ form: f, source: "filename" as const })),
    ];

    let matched = false;

    for (const { form } of allForms) {
      if (!form) continue;
      const candidates = remoteTitleIndex.get(form);
      if (!candidates || candidates.length === 0) continue;

      // Filter to unassigned remote tracks
      const unassigned = candidates.filter(
        (ri) => !matchedRemote.has(ri),
      );
      if (unassigned.length === 0) continue;

      if (unassigned.length === 1) {
        const ri = unassigned[0];
        const rm = remoteMeta[ri];

        // Both durations exist → must be close
        if (localDuration != null && rm.duration != null) {
          if (durationsMatch(localDuration, rm.duration)) {
            matchedRemote.add(ri);
            matchedLocal[i] = ri;
            stats.matched++;
            matched = true;
            break;
          }
          // Duration mismatch
          stats.skipped.push({
            localIndex: i,
            localTitle: lf.tagTitle,
            reason: "duration_mismatch",
          });
          continue;
        }

        // Duration missing on one or both sides → allow if exactly one
        // unassigned candidate (already guaranteed by filter above)
        matchedRemote.add(ri);
        matchedLocal[i] = ri;
        stats.matched++;
        matched = true;
        break;
      }

      // Multiple unassigned remote tracks share this title form.
      // Try to resolve by unique duration match.
      const localDur = localDuration;
      const withDurations = unassigned
        .map((ri) => ({ ri, dur: remoteMeta[ri].duration }))
        .filter(
          (x) =>
            x.dur != null &&
            localDur != null &&
            durationsMatch(localDur, x.dur),
        );

      if (withDurations.length === 1) {
        const ri = withDurations[0].ri;
        matchedRemote.add(ri);
        matchedLocal[i] = ri;
        stats.matched++;
        matched = true;
        break;
      }

      // If no duration on either side, check if exactly one unassigned
      // candidate exists.
      if (localDur == null) {
        if (unassigned.length === 1) {
          const ri = unassigned[0];
          matchedRemote.add(ri);
          matchedLocal[i] = ri;
          stats.matched++;
          matched = true;
          break;
        }
        // Duration missing, multiple candidates → skip (ambiguous)
        stats.skipped.push({
          localIndex: i,
          localTitle: lf.tagTitle,
          reason: "duplicate_ambiguous",
        });
        continue;
      }

      // Duration mismatch or ambiguous duplicates
      stats.skipped.push({
        localIndex: i,
        localTitle: lf.tagTitle,
        reason: "duplicate_ambiguous",
      });
    }

    if (!matched) {
      const reason: SkipReason["reason"] =
        allForms.every((f) => !f.form)
          ? "no_local_evidence"
          : "no_title_match";
      stats.skipped.push({
        localIndex: i,
        localTitle: lf.tagTitle,
        reason,
      });
    }
  }

  // 5. Determine if we have a full ordered match
  const isFullOrderedMatch =
    stats.matched === localTracks.length &&
    localTracks.length === remoteTracks.length;

  // 6. Build output tracks in local file order
  const outputTracks: TrackCandidate[] = localTracks.map((localTrack, i) => {
    const remoteIdx = matchedLocal[i];
    if (remoteIdx === null) {
      // Unmatched: return local track as-is (no remote per-track fields)
      return { ...localTrack };
    }

    const remoteTrack = remoteTracks[remoteIdx];
    const lf = localForms[i];

    // Check if any tag form matched (not just filename-derived)
    const anyTagFormMatched = lf.tagForms.some(
      (tf) => tf === remoteMeta[remoteIdx].normalized,
    );

    const result: TrackCandidate = { ...localTrack };

    // ── Title ─────────────────────────────────────────────────
    if (anyTagFormMatched) {
      // Local tag title matched — preserve original local title
      result.title = localTrack.title;
    } else {
      // Only filename-derived title matched — write cleaned filename title
      const knownArtists = [
        ...(options.artistHints ?? []),
        ...(localTrack.artist ? [localTrack.artist] : []),
        ...localTrack.artists,
      ];
      const cleaned = cleanFilenameTitle(lf.filename, knownArtists);
      result.title = cleaned || localTrack.title;
    }

    // ── musicbrainzTrackId always allowed for matched tracks ──
    if (remoteTrack.musicbrainzTrackId) {
      result.musicbrainzTrackId = remoteTrack.musicbrainzTrackId;
    }

    // ── Remote artist/artists only when local is blank ────────
    const localArtistBlank =
      !localTrack.artist || localTrack.artist.trim() === "";
    if (localArtistBlank && remoteTrack.artist) {
      result.artist = remoteTrack.artist;
      result.artists =
        remoteTrack.artists.length > 0
          ? [...remoteTrack.artists]
          : result.artists;
    }

    // ── Track/disc number fields: only for full ordered match ─
    if (isFullOrderedMatch) {
      if (remoteTrack.trackNumber != null)
        result.trackNumber = remoteTrack.trackNumber;
      if (remoteTrack.trackTotal != null)
        result.trackTotal = remoteTrack.trackTotal;
      if (remoteTrack.discNumber != null)
        result.discNumber = remoteTrack.discNumber;
      if (remoteTrack.discTotal != null)
        result.discTotal = remoteTrack.discTotal;
    }

    return result;
  });

  return { tracks: outputTracks, stats, isFullOrderedMatch };
}
