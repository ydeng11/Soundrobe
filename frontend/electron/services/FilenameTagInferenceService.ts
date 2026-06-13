import path from "node:path";
import { splitArtistNames } from "../handlers/candidates";
import type { WriteFields } from "../handlers/writer";
import { prettifyTag } from "./TagPrettifyService";

export interface FilenameTagInference {
  trackPath: string;
  fields: WriteFields;
  reason: string;
}

export interface FilenameTagInferenceOptions {
  title?: boolean;
  artist?: boolean;
  artists?: boolean;
  /** When true, inferred tag values are prettified (underscores → title case). */
  prettify?: boolean;
}

const LEADING_TRACK_NUMBER_RE = /^\s*(?:disc\s*)?\d{1,3}(?:[._ -]+|\s+)/i;
const ARTIST_TITLE_SEPARATOR_RE = /\s[-–—]\s/;

function cleanStem(stem: string): string {
  return stem
    .replace(LEADING_TRACK_NUMBER_RE, "")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * Check whether the original stem had a leading track number.
 * Used to decide whether a compact (no-space) dash is likely an
 * artist-title separator vs. part of the title.
 */
function hadLeadingTrackNumber(stem: string): boolean {
  return LEADING_TRACK_NUMBER_RE.test(stem);
}

/**
 * Try spaced-dash parsing: "artist - title", "artist – title".
 */
function trySpacedDash(stem: string): { artist: string; title: string } | null {
  const match = ARTIST_TITLE_SEPARATOR_RE.exec(stem);
  if (!match || match.index === 0) return null;

  const artist = stem.slice(0, match.index).trim();
  const title = stem.slice(match.index + match[0].length).trim();
  if (!artist || !title) return null;
  return { artist, title };
}

/**
 * Try compact (no-space) dash parsing: "artist-title".
 * Only called when spaced-dash parsing failed AND the original
 * filename had a leading track number (reduces false positives
 * for standalone dashed titles).
 */
function tryCompactDash(stem: string): { artist: string; title: string } | null {
  const dashIndex = stem.indexOf("-");
  if (dashIndex <= 0 || dashIndex >= stem.length - 1) return null;
  const artist = stem.slice(0, dashIndex).trim();
  const title = stem.slice(dashIndex + 1).trim();
  if (!artist || !title) return null;
  return { artist, title };
}

/**
 * Parse a filename stem (basename without extension) into artist/title.
 *
 * Strategy:
 *   1. Strip leading track number.
 *   2. Try spaced-dash separator ("artist - title").
 *   3. If that fails AND the original had a leading track number,
 *      try compact-dash ("artist-title") – the number signals a
 *      structured filename where the first dash is likely the
 *      artist-title boundary.
 *
 * Returns null when the stem cannot be parsed.
 */
function splitArtistTitle(stem: string): { artist: string; title: string; kind: "spaced" | "compact" } | null {
  const hadNumber = hadLeadingTrackNumber(stem);
  const clean = cleanStem(stem);

  // 1. Try spaced dash
  const spaced = trySpacedDash(clean);
  if (spaced) return { ...spaced, kind: "spaced" };

  // 2. Try compact dash only for structured filenames (had leading number)
  if (hadNumber) {
    const compact = tryCompactDash(clean);
    if (compact) return { ...compact, kind: "compact" };
  }

  return null;
}

function normalizeArtistForSplit(artist: string): string {
  return artist.replace(/\s+_\s+/g, " / ");
}

export class FilenameTagInferenceService {
  inferFromFilenames(
    trackPaths: string[],
    options: FilenameTagInferenceOptions = {},
  ): FilenameTagInference[] {
    const includeTitle = options.title ?? true;
    const includeArtist = options.artist ?? true;
    const includeArtists = options.artists ?? true;
    const results: FilenameTagInference[] = [];

    for (const trackPath of trackPaths) {
      const stem = path.basename(trackPath, path.extname(trackPath));
      const parsed = splitArtistTitle(stem);
      if (!parsed) continue;

      const fields: WriteFields = {};
      if (includeTitle) fields.title = options.prettify ? prettifyTag(parsed.title) : parsed.title;
      if (includeArtist) fields.artist = options.prettify ? prettifyTag(parsed.artist) : parsed.artist;
      if (includeArtists) {
        fields.artists = splitArtistNames([normalizeArtistForSplit(parsed.artist)]);
        if (options.prettify && fields.artists) {
          fields.artists = fields.artists.map((a) => prettifyTag(a));
        }
      }

      if (Object.keys(fields).length === 0) continue;
      results.push({
        trackPath,
        fields,
        reason: parsed.kind === "compact"
          ? `Parsed "${path.basename(trackPath)}" as compact artist-title filename`
          : `Parsed "${path.basename(trackPath)}" as artist-title filename`,
      });
    }

    return results;
  }
}
