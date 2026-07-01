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

const TITLE_POLLUTION_EXTRA_MIN_CHARS = 3;
const TITLE_POLLUTION_EXTRA_MIN_RATIO = 0.25;
const MEANINGFUL_VERSION_QUALIFIERS = new Set([
  "live",
  "remix",
  "demo",
  "acoustic",
  "instrumental",
  "karaoke",
  "edit",
  "version",
  "现场",
  "現場",
  "伴奏",
]);

const API_TITLE_CLEANUP_SOURCES = new Set(["musicbrainz", "discogs"]);

// ── Filename parsing ─────────────────────────────────────────────

const FILENAME_TRACK_PREFIX_RE = /^(\d+)[\s.\u2010\u2011\u2012\u2013\u2014\u2015\-]*/;
const FILENAME_ARTIST_PREFIX_RE =
  /^(.*?)[\s\-–—]+(?:[-–—]|[\u2013\u2014])\s*/;
const FILENAME_DIRECT_ARTIST_SEPARATOR_RE = /[-–—]/g;

// Match a leading CJK segment before any non-CJK character (space, Latin, etc.)
const LEADING_CJK_RE = /^([\u4e00-\u9fff\u3400-\u4dbf]+)/;

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

/** Normalize string for comparison: NFKC + lowercase + trim. */
function normArtist(s: string): string {
  return s.normalize("NFKC").trim().toLowerCase();
}

/** Get all OpenCC variants of a string (original + Simplified + Traditional). */
function getArtistVariants(name: string): string[] {
  const base = normArtist(name);
  if (!openCCInstance) return [base];
  return [base, normArtist(openCCInstance.s2t(name)), normArtist(openCCInstance.t2s(name))];
}

/** Check if two artist names match, including OpenCC Simplified/Traditional variants. */
function artistsMatch(a: string, b: string): boolean {
  const aVariants = getArtistVariants(a);
  const bVariants = getArtistVariants(b);
  return aVariants.some((av) => bVariants.includes(av));
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

    // Strip known artist suffix (e.g. "想念-林宥嘉" → "想念")
    const suffixStripped = stripKnownArtistSuffix(cleaned, knownArtists);
    if (suffixStripped && suffixStripped !== cleaned) {
      addForm(suffixStripped, "tag");
      if (oc) {
        addForm(oc.s2t(suffixStripped), "tag");
        addForm(oc.t2s(suffixStripped), "tag");
      }
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
 * Strip a known artist name from the END of a title string.
 * E.g. "想念-林宥嘉" with artist "林宥嘉" → "想念"
 * Handles separators: -, –, —, with optional surrounding spaces.
 * Returns null if no artist suffix is found.
 */
function stripKnownArtistSuffix(
  title: string,
  knownArtists: string[],
): string | null {
  const normalizedArtists = knownArtists
    .map((artist) => stripPunctuationAndSymbols(stripAnnotations(artist)))
    .filter((artist) => artist.length > 0);
  if (normalizedArtists.length === 0) return null;

  for (const match of title.matchAll(FILENAME_DIRECT_ARTIST_SEPARATOR_RE)) {
    const index = match.index;
    if (index == null) continue;
    const titlePart = title.slice(0, index).trim();
    const artistPart = title.slice(index + match[0].length).trim();
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
 * Extract the leading CJK segment from a bilingual title.
 * E.g. "想念 I Miss You" → "想念", "Fly My Way" → null
 * Used to match Chinese local titles against bilingual remote titles.
 */
function extractLeadingCjk(title: string): string | null {
  const match = LEADING_CJK_RE.exec(title);
  return match?.[1] ?? null;
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

/**
 * Decide whether an API title should replace a suffix-polluted local title.
 *
 * This is intentionally title-only evidence: the API title must already be
 * contained in the current title after normalization, and the current title
 * must have enough extra text to prove real pollution rather than harmless
 * punctuation/case differences.
 */
export function shouldReplacePollutedTitleWithApiTitle(
  currentTitle: string | null | undefined,
  apiTitle: string | null | undefined,
  apiTitleVariants: string[] = [],
): boolean {
  return replacementTitleForPollutedTitle(currentTitle, apiTitle, apiTitleVariants) !== null;
}

export function replacementTitleForPollutedTitle(
  currentTitle: string | null | undefined,
  apiTitle: string | null | undefined,
  apiTitleVariants: string[] = [],
): string | null {
  const currentCandidates = titleReplacementCurrentCandidates(currentTitle);
  const apiCandidates = [apiTitle ?? "", ...apiTitleVariants]
    .map((title, index) => ({
      raw: title,
      normalized: stripPunctuationAndSymbols(title),
      primary: index === 0,
    }))
    .filter((candidate, index, candidates) =>
      candidate.normalized.length > 0 &&
      candidates.findIndex((other) => other.normalized === candidate.normalized) === index,
    );
  if (currentCandidates.length === 0 || apiCandidates.length === 0) return null;
  const primaryApi = apiCandidates[0];

  for (const current of currentCandidates) {
    for (const api of apiCandidates) {
      if (current.normalized === api.normalized || api.normalized.length < 2) continue;
      if (shouldReplacePollutedNormalizedTitle(current.normalized, api.normalized)) {
        return current.coreNormalized === primaryApi.normalized
          ? current.core
          : apiTitle ?? api.raw;
      }
    }
  }

  return null;
}

function titleReplacementCurrentCandidates(
  currentTitle: string | null | undefined,
): Array<{ text: string; normalized: string; core: string; coreNormalized: string }> {
  const raw = (currentTitle ?? "").trim();
  if (!raw) return [];

  const candidates = [{ text: raw, core: raw.split(/[（(]/)[0]?.trim() ?? raw }];
  const beforeFirstParen = candidates[0].core;
  const coreAfterDisplayPrefix = beforeFirstParen
    .split(/\s[-–—]\s/)
    .at(-1)
    ?.trim();
  if (coreAfterDisplayPrefix && coreAfterDisplayPrefix !== beforeFirstParen) {
    const suffix = raw.slice(beforeFirstParen.length);
    candidates.push({
      text: `${coreAfterDisplayPrefix}${suffix}`,
      core: coreAfterDisplayPrefix,
    });
  }

  return candidates
    .map((candidate) => ({
      ...candidate,
      normalized: stripPunctuationAndSymbols(candidate.text),
      coreNormalized: stripPunctuationAndSymbols(candidate.core),
    }))
    .filter((candidate, index, candidates) =>
      candidate.normalized.length > 0 &&
      candidates.findIndex((other) => other.normalized === candidate.normalized) === index,
    );
}

function shouldReplacePollutedNormalizedTitle(current: string, api: string): boolean {
  const apiIndex = current.indexOf(api);
  if (apiIndex !== 0) return false;

  const before = current.slice(0, apiIndex).trim();
  const after = current.slice(apiIndex + api.length).trim();
  const extra = `${before} ${after}`.trim();
  if (!extra) return false;

  const extraTokens = extra.split(WHITESPACE_RE).filter(Boolean);
  if (extraTokens.some((token) => MEANINGFUL_VERSION_QUALIFIERS.has(token))) {
    return false;
  }

  const extraChars = current.length - api.length;
  const extraRatio = extraChars / current.length;
  return (
    extraChars >= TITLE_POLLUTION_EXTRA_MIN_CHARS ||
    extraRatio >= TITLE_POLLUTION_EXTRA_MIN_RATIO
  );
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
  const replacementOpenCC = await getOpenCC();
  const remoteMeta = remoteTracks.map((rt, i) => {
    const annotationStripped = stripAnnotations(rt.title ?? "");
    const titleVariants = replacementOpenCC
      ? [
        annotationStripped,
        replacementOpenCC.s2t(annotationStripped),
        replacementOpenCC.t2s(annotationStripped),
      ]
      : [annotationStripped];

    // Extract leading CJK segment from bilingual titles
    // E.g. "想念 I Miss You" → "想念"
    const leadingCjk = extractLeadingCjk(annotationStripped);
    if (leadingCjk && leadingCjk !== annotationStripped) {
      titleVariants.push(leadingCjk);
    }

    return {
      index: i,
      title: annotationStripped,
      titleVariants,
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

    // Also index CJK prefix variants (e.g. "想念" from "想念 I Miss You")
    for (const variant of rm.titleVariants) {
      const normalizedVariant = stripPunctuationAndSymbols(variant);
      if (normalizedVariant && normalizedVariant !== rm.normalized) {
        const variantExisting = remoteTitleIndex.get(normalizedVariant) ?? [];
        variantExisting.push(rm.index);
        remoteTitleIndex.set(normalizedVariant, variantExisting);
      }
    }
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

    if (!matched && API_TITLE_CLEANUP_SOURCES.has(source)) {
      const containedTitleMatches = remoteMeta
        .filter((rm) => !matchedRemote.has(rm.index))
        .filter((rm) =>
          shouldReplacePollutedTitleWithApiTitle(localTracks[i].title, rm.title, rm.titleVariants),
        )
        .filter((rm) => {
          if (localDuration == null || rm.duration == null) return true;
          return durationsMatch(localDuration, rm.duration);
        });

      if (containedTitleMatches.length === 1) {
        const ri = containedTitleMatches[0].index;
        matchedRemote.add(ri);
        matchedLocal[i] = ri;
        stats.matched++;
        matched = true;
      } else if (containedTitleMatches.length > 1) {
        stats.skipped.push({
          localIndex: i,
          localTitle: lf.tagTitle,
          reason: "duplicate_ambiguous",
        });
        matched = true;
      }
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

  // 5b. Positional fallback: when NO title matches succeeded but track counts match
  //     (and there are at least 2 tracks), assume the track listings are in the same
  //     order. This handles cases like Chinese albums on MusicBrainz where local titles
  //     (Chinese) differ completely from remote titles (English/Pinyin).
  //     Requires >=2 tracks because single-track matching is meaningless (only one
  //     position exists). Only activates when 0 title matches — if any matched,
  //     alignment is ambiguous.
  if (stats.matched === 0 && localTracks.length === remoteTracks.length && localTracks.length >= 2) {
    for (let i = 0; i < localTracks.length; i++) {
      matchedLocal[i] = i;
      matchedRemote.add(i);
    }
    stats.matched = localTracks.length;
    // Clear stale skip entries from failed title-match attempt
    stats.skipped = [];
  }

  const finalIsFullOrderedMatch =
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
    const titleReplacement = API_TITLE_CLEANUP_SOURCES.has(source)
      ? replacementTitleForPollutedTitle(
        localTrack.title,
        remoteTrack.title,
        remoteMeta[remoteIdx].titleVariants,
      )
      : null;

    if (
      API_TITLE_CLEANUP_SOURCES.has(source) &&
      titleReplacement
    ) {
      result.title = titleReplacement;
    } else if (anyTagFormMatched) {
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

    // ── Remote artist/artists ─────────────────────────────────
    // Prefer remote (MusicBrainz) artist credits when they are richer
    // than the local (e.g. "林俊傑 feat. MC HotDog" vs "林俊傑").
    // Transfer when:
    //   1. Local artist is blank, OR
    //   2. Remote artists contain the local primary artist AND add more
    //      (e.g. featured artist), using normalized comparison.
    // This preserves user-customized artists that differ from the remote.
    const localArtistBlank = !localTrack.artist || localTrack.artist.trim() === "";
    const localArtist = localTrack.artist;
    let remoteEnrichesLocal = false;

    if (!localArtistBlank && localArtist && remoteTrack.artist && remoteTrack.artist !== localArtist) {
      const remotePrimaryMatch = remoteTrack.artists.length > 0
        ? remoteTrack.artists.some((a) => artistsMatch(a, localArtist))
        : normArtist(remoteTrack.artist).startsWith(normArtist(localArtist));
      remoteEnrichesLocal = remotePrimaryMatch && remoteTrack.artists.length > localTrack.artists.length;
    }

    if ((localArtistBlank || remoteEnrichesLocal) && remoteTrack.artist) {
      if (localArtistBlank) {
        // No local artist — use remote as-is
        result.artist = remoteTrack.artist;
        result.artists = remoteTrack.artists.length > 0 ? [...remoteTrack.artists] : result.artists;
      } else if (remoteEnrichesLocal && localArtist) {
        // Remote enriches local — preserve local primary script, add remote extras
        const equivalentRemoteIdx = remoteTrack.artists.findIndex((r) => artistsMatch(r, localArtist));

        // Build enriched artists: local primary + remote extras (excluding equivalent)
        const remoteExtras = remoteTrack.artists.filter((_, idx) => idx !== equivalentRemoteIdx);
        result.artists = [localArtist, ...remoteExtras];

        // Build display artist: replace equivalent remote primary with local primary
        if (equivalentRemoteIdx >= 0) {
          const remotePrimaryName = remoteTrack.artists[equivalentRemoteIdx];
          result.artist = remoteTrack.artist.replace(remotePrimaryName, localArtist);
          // Fallback: if replacement didn't work, join artists
          if (result.artist === remoteTrack.artist) {
            result.artist = result.artists.join(" feat. ");
          }
        } else {
          result.artist = result.artists.join(" feat. ");
        }
      }
    }

    // ── Track/disc number fields: only for full ordered match ─
    if (finalIsFullOrderedMatch) {
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

  return { tracks: outputTracks, stats, isFullOrderedMatch: finalIsFullOrderedMatch };
}
