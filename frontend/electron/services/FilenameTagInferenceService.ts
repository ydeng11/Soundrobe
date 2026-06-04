import path from "node:path";
import { splitArtistNames } from "../handlers/candidates";
import type { WriteFields } from "../handlers/writer";

export interface FilenameTagInference {
  trackPath: string;
  fields: WriteFields;
  reason: string;
}

export interface FilenameTagInferenceOptions {
  title?: boolean;
  artist?: boolean;
  artists?: boolean;
}

const LEADING_TRACK_NUMBER_RE = /^\s*(?:disc\s*)?\d{1,3}(?:[._ -]+|\s+)/i;
const ARTIST_TITLE_SEPARATOR_RE = /\s[-–—]\s/;

function cleanStem(stem: string): string {
  return stem
    .replace(LEADING_TRACK_NUMBER_RE, "")
    .replace(/\s+/g, " ")
    .trim();
}

function splitArtistTitle(stem: string): { artist: string; title: string } | null {
  const clean = cleanStem(stem);
  const match = ARTIST_TITLE_SEPARATOR_RE.exec(clean);
  if (!match || match.index === 0) return null;

  const artist = clean.slice(0, match.index).trim();
  const title = clean.slice(match.index + match[0].length).trim();
  if (!artist || !title) return null;
  return { artist, title };
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
      if (includeTitle) fields.title = parsed.title;
      if (includeArtist) fields.artist = parsed.artist;
      if (includeArtists) {
        fields.artists = splitArtistNames([normalizeArtistForSplit(parsed.artist)]);
      }

      if (Object.keys(fields).length === 0) continue;
      results.push({
        trackPath,
        fields,
        reason: `Parsed "${path.basename(trackPath)}" as artist-title filename`,
      });
    }

    return results;
  }
}
