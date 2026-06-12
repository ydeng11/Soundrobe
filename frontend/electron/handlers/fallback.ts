/**
 * Folder-structure fallback lookup helpers.
 * Ported from Python auto_tagger.integrations.fallback.
 *
 * Parses a file path into artist/album/year hints using folder names,
 * existing audio tags, and filename patterns.
 */

import { readdirSync, statSync } from "node:fs";
import { join, dirname, basename, extname } from "node:path";
import { readTrackMetadata } from "./tracks";
import {
  type AlbumCandidate,
  type LookupRequest,
  type TrackCandidate,
  makeAlbumCandidate,
  makeTrackCandidate,
} from "./candidates";

// ── Regex patterns ──────────────────────────────────────────────────

const COMPILATION_FOLDER_SET = new Set([
  "compilations",
  "compilation",
  "various artists",
  "various",
  "va",
  "soundtracks",
  "soundtrack",
  "ost",
  "samplers",
  "sampler",
  "christmas",
]);

/**
 * Check if a folder name indicates a compilation/sampler rather than a single artist.
 */
export function isCompilationFolder(name: string | null | undefined): boolean {
  if (!name) return false;
  const normalized = name.trim().toLowerCase().replace(/[ _]+/g, " ");
  return COMPILATION_FOLDER_SET.has(normalized);
}

/** Album artist used when the folder indicates a compilation. */
const VARIOUS_ARTISTS = "Various Artists";

const DATE_PREFIX_RE = /^(\d{4})[-.](?:0[1-9]|1[0-2])\s*/; // "2003-04" or "2005.08"
const YEAR_PREFIX_RE = /^(\d{4})[.-]\s*/; // "2017-" or "2018."
const YEAR_FROM_PREFIX_RE = /^(\d{4})[-.]/; // capture year from "2003-"
const BOOKMARKS_RE = /[《》「」【】\[\]]/g;
const EXTRA_SUFFIX_RE = /\s*\([^)]*\)\s*$/;
const FORMAT_SUFFIX_RE = /\[?(flac|mp3|wav|aac|ogg|m4a|wma|ape|flac\s*分轨|wav\s*分轨)\]?\s*$/i;
const CD_SUBFOLDER_RE = /(?:[Cc][Dd]|[Dd][Ii][Ss][CcKk]|ディスク)\s*\d+\s*$/;
const EDITION_KEYWORDS_RE = /\s*(?:香港首版|台湾首版|引进版|日本版|欧版|美版|内地版|中国大陆版|大陆版|德国版|澳洲版|新加坡版|马来西亚版|韩版)\s*/gi;
const DISC_COUNT_RE = /\s*\d+\s*(?:CD|Disc|ディスク)\s*$/i;

// ── Year extraction ─────────────────────────────────────────────────

/**
 * Extract a 4-digit year from a folder name.
 * Tries leading date prefix, inside bookmarks, or parenthesized.
 */
export function extractYearFromName(name: string): string | null {
  // 1. Leading date prefix
  const m1 = YEAR_FROM_PREFIX_RE.exec(name);
  if (m1) return m1[1];

  // 2. Inside Chinese bookmarks: 《2011-重译》
  const m2 = /[《（（\[]\s*(\d{4})\s*[-.]/.exec(name);
  if (m2) return m2[1];

  // 3. Parenthesized year: (2011) or [2011]
  const m3 = /[\[(（]\s*(\d{4})\s*[\])）]/.exec(name);
  if (m3) return m3[1];

  return null;
}

// ── Folder name cleaning ────────────────────────────────────────────

/**
 * Clean a folder name for use as a lookup hint.
 * Strips date prefixes, bookmarks, edition annotations, format suffixes.
 */
export function cleanFolderName(name: string): string {
  // First try extracting content from inside Chinese bookmarks
  const bracketed = name.match(/《([^》]+)》/);
  if (bracketed) {
    let inner = bracketed[1];
    inner = inner.replace(DATE_PREFIX_RE, "");
    inner = inner.replace(YEAR_PREFIX_RE, "");
    inner = inner.replace(EDITION_KEYWORDS_RE, "");
    inner = inner.replace(DISC_COUNT_RE, "");
    inner = inner.trim();
    if (inner) return inner;
  }

  // Fallback: standard cleanup on full name
  let cleaned = name.replace(DATE_PREFIX_RE, "");
  cleaned = cleaned.replace(YEAR_PREFIX_RE, "");
  cleaned = cleaned.replace(BOOKMARKS_RE, "");
  cleaned = cleaned.replace(EDITION_KEYWORDS_RE, "");
  cleaned = cleaned.replace(FORMAT_SUFFIX_RE, "");
  cleaned = cleaned.replace(EXTRA_SUFFIX_RE, "");
  cleaned = cleaned.replace(DISC_COUNT_RE, "");
  cleaned = cleaned.trim();
  return cleaned || name;
}

/**
 * Clean an album folder name for use as the album hint.
 * Strips the leading "Year - " prefix.
 */
export function cleanAlbumFolderName(name: string): string {
  let cleaned = cleanFolderName(name);
  // Also strip leading "Year - " prefix that's not just a date prefix
  const yearDash = /^(\d{4})\s*[-—]\s*/.exec(cleaned);
  if (yearDash) {
    cleaned = cleaned.slice(yearDash[0].length);
  }
  return cleaned;
}

// ── Path parsing ────────────────────────────────────────────────────

/**
 * Parse artist, album, and year hints from an Artist/Album path.
 */
export function parseAlbumPath(filePath: string): LookupRequest {
  const isFile = !!extname(basename(filePath));
  const albumPath = isFile ? dirname(filePath) : filePath;
  const albumName = basename(albumPath);
  const parentName = basename(dirname(albumPath));

  let artistHint: string | null = null;
  let albumHint: string | null = cleanFolderName(albumName) || null;
  let yearHint: string | null = extractYearFromName(albumName);

  // Detect CD subfolder pattern
  if (CD_SUBFOLDER_RE.test(albumName)) {
    const grandparent = basename(dirname(dirname(albumPath)));
    artistHint = grandparent ? cleanFolderName(grandparent) : null;
    const parent = basename(dirname(albumPath));
    albumHint = parent ? cleanFolderName(parent) : albumHint;
    yearHint = parent ? extractYearFromName(parent) : yearHint;
  } else {
    artistHint = parentName ? cleanFolderName(parentName) : null;
  }

  return {
    path: filePath,
    artistHint,
    albumHint,
    yearHint,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    discogsReleaseId: null,
    discogsArtistId: null,
    tracks: [],
  };
}

/**
 * Build a LookupRequest using both folder names and existing file tags.
 * Tag values take priority over folder names.
 */
export async function parseAlbumWithTags(filePath: string): Promise<LookupRequest> {
  const folderRequest = parseAlbumPath(filePath);
  const tagHints = await readAlbumTagsFromFirstFile(filePath);

  const tagArtist = tagHints.artist;
  const folderArtist = folderRequest.artistHint;

  let artistHint: string | null;
  let albumHint: string | null;

  if (isCompilationFolder(folderArtist)) {
    // Albums under "Compilations" folders are various-artist compilations
    artistHint = VARIOUS_ARTISTS;
    albumHint = folderRequest.albumHint;
  } else if (tagArtist && folderArtist && tagArtist.toLowerCase() !== folderArtist.toLowerCase()) {
    // Tag artist doesn't match folder — folder structure is more trustworthy
    artistHint = folderArtist;
    albumHint = folderRequest.albumHint;
  } else {
    artistHint = tagArtist || folderArtist;
    albumHint = folderRequest.albumHint || tagHints.album;
  }

  const tracks = await trackHintsFromPath(filePath);

  return {
    path: filePath,
    artistHint,
    albumHint,
    yearHint: folderRequest.yearHint || tagHints.year,
    musicbrainzAlbumId: tagHints.musicbrainzAlbumId,
    musicbrainzArtistId: tagHints.musicbrainzArtistId,
    discogsReleaseId: tagHints.discogsReleaseId,
    discogsArtistId: tagHints.discogsArtistId,
    tracks,
  };
}

/**
 * Read album-level tags from the first audio file in the directory.
 */
async function readAlbumTagsFromFirstFile(
  path: string,
): Promise<{
  artist: string | null;
  album: string | null;
  year: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  discogsReleaseId: string | null;
  discogsArtistId: string | null;
}> {
  const isFile = !!extname(basename(path));
  const dirPath = isFile ? dirname(path) : path;

  try {
    const entries = readdirSync(dirPath).sort();
    for (const entry of entries) {
      const fullPath = join(dirPath, entry);
      if (statSync(fullPath).isFile()) {
        const ext = extname(entry).toLowerCase();
        if ([".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus"].includes(ext)) {
          try {
            const meta = await readTrackMetadata(fullPath);
            if (meta.album || meta.artist) {
              return {
                artist: meta.artist || meta.albumArtist,
                album: meta.album,
                year: meta.year,
                musicbrainzAlbumId: meta.musicbrainzAlbumId,
                musicbrainzArtistId: meta.musicbrainzArtistId,
                discogsReleaseId: meta.discogsReleaseId,
                discogsArtistId: meta.discogsArtistId,
              };
            }
          } catch {
            continue;
          }
        }
      }
    }
  } catch {
    // directory doesn't exist or can't be read
  }
  return { artist: null, album: null, year: null, musicbrainzAlbumId: null, musicbrainzArtistId: null, discogsReleaseId: null, discogsArtistId: null };
}

// ── Track hints ─────────────────────────────────────────────────────

/**
 * Build track candidates from audio file metadata.
 */
export async function trackHintsFromPath(filePath: string): Promise<TrackCandidate[]> {
  const isFile = !!extname(basename(filePath));
  const dirPath = isFile ? dirname(filePath) : filePath;
  const tracks: TrackCandidate[] = [];

  try {
    const entries = readdirSync(dirPath).sort();
    let index = 0;
    for (const entry of entries) {
      const fullPath = join(dirPath, entry);
      try {
        if (!statSync(fullPath).isFile()) continue;
      } catch {
        continue;
      }
      const ext = extname(entry).toLowerCase();
      if (![".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus"].includes(ext)) continue;
      index++;

      try {
        const meta = await readTrackMetadata(fullPath);
        tracks.push(
          makeTrackCandidate({
            title: meta.title || entry.replace(ext, ""),
            artist: meta.artist,
            artists: meta.artists,
            trackNumber: meta.trackNumber ?? index,
            trackTotal: null, // filled after all tracks are collected
            discNumber: meta.discNumber,
            musicbrainzTrackId: meta.musicbrainzTrackId,
            length: meta.duration,
            genre: meta.genre,
          }),
        );
      } catch {
        // Read failed — use filename as title hint
        tracks.push(
          makeTrackCandidate({
            title: entry.replace(ext, ""),
            trackNumber: index,
          }),
        );
      }
    }
  } catch {
    // directory can't be read
  }

  // Fill track totals
  const total = tracks.length;
  for (const track of tracks) {
    track.trackTotal = total;
  }

  return tracks;
}

// ── Folder fallback candidate ───────────────────────────────────────

/**
 * Build a low-confidence fallback candidate from folder and file hints.
 */
export function candidateFromFolder(request: LookupRequest): AlbumCandidate {
  const artist = request.artistHint;
  const album = request.albumHint;
  const year = request.yearHint;

  const isCompilation = isCompilationFolder(artist);
  const albumArtist = isCompilation ? VARIOUS_ARTISTS : artist;
  const albumArtists = isCompilation
    ? [VARIOUS_ARTISTS]
    : artist
      ? [artist]
      : [];

  return makeAlbumCandidate({
    artist: albumArtist,
    artists: albumArtists,
    album,
    albumArtist,
    albumArtists,
    year,
    tracks: request.tracks,
    source: "folder",
  });
}
