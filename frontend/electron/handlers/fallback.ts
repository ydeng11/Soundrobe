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

const AUDIO_EXTENSIONS = new Set([".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus", ".ape"]);

function isAudioFilePath(inputPath: string): boolean {
  try {
    return statSync(inputPath).isFile();
  } catch {
    const name = basename(inputPath);
    if (/[《》「」【】]/.test(name)) return false;
    return AUDIO_EXTENSIONS.has(extname(name).toLowerCase());
  }
}

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

const DATE_PREFIX_RE = /^(\d{4})[-.](?:0[1-9]|1[0-2])(?!\d)(?:[-.](?:0[1-9]|[12]\d|3[01]))?\s*/; // "2003-04", "2007-09-28", or "2005.08"
const YEAR_PREFIX_RE = /^(\d{4})\s*[.-]\s*/; // "2017-" or "2018."
const YEAR_FROM_PREFIX_RE = /^(\d{4})[-.]/; // capture year from "2003-"
const STANDALONE_YEAR_RE = /(?:^|[^\d])((?:19|20)\d{2})(?!\d)/;
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

  // 4. Standalone year in names like "黄绮珊《时光》2018 .wav".
  const m4 = STANDALONE_YEAR_RE.exec(name);
  if (m4) return m4[1];

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
export function cleanAlbumFolderName(
  name: string,
  artistHint?: string | null,
): string {
  let cleaned = cleanFolderName(name);

  const artist = artistHint ? cleanFolderName(artistHint).trim() : "";
  if (artist) {
    const afterArtist = stripRepeatedArtistFromAlbumFolder(cleaned, artist);
    if (afterArtist) return afterArtist;
  }

  // Also strip leading "Year - " prefix that's not just a date prefix
  const yearDash = /^(\d{4})\s*[-—]\s*/.exec(cleaned);
  if (yearDash) {
    cleaned = cleaned.slice(yearDash[0].length);
  }
  return cleaned;
}

function normalizeArtistPrefix(name: string): string {
  return name.toLowerCase().replace(/[\s._:：\-—–]+/g, "");
}

function cleanAlbumRemainder(name: string): string {
  let cleaned = name.trim().replace(/^[\s._:：\-—–]+/, "");
  cleaned = cleanFolderName(cleaned);
  cleaned = cleaned.replace(/^[\s._:：\-—–]+/, "").trim();
  return cleaned;
}

function stripRepeatedArtistFromAlbumFolder(
  albumFolder: string,
  artist: string,
): string | null {
  for (const prefix of repeatedArtistPrefixes(albumFolder, artist)) {
    const afterPrefix = albumFolder.slice(prefix.length);
    if (
      looksLikeArtistAlbumSeparator(afterPrefix, true) ||
      (prefix !== artist && /^\s+/.test(afterPrefix))
    ) {
      const cleaned = cleanAlbumRemainder(afterPrefix);
      if (cleaned) return stripEnglishArtistFromRemainder(cleaned, artist) ?? cleaned;
    }
  }

  const artistIndex = albumFolder.indexOf(artist);
  if (artistIndex <= 0) return null;

  const beforeArtist = albumFolder.slice(0, artistIndex);
  const afterArtist = albumFolder.slice(artistIndex + artist.length);
  if (
    looksLikeCategoryArtistSeparator(beforeArtist) &&
    looksLikeArtistAlbumSeparator(afterArtist, false)
  ) {
    const cleaned = cleanAlbumRemainder(afterArtist);
    if (cleaned) return stripEnglishArtistFromRemainder(cleaned, artist) ?? cleaned;
  }

  return null;
}

function repeatedArtistPrefixes(
  albumFolder: string,
  artist: string,
): string[] {
  const prefixes = new Set<string>();
  if (albumFolder.startsWith(artist)) prefixes.add(artist);

  const normalizedArtist = normalizeArtistPrefix(artist);
  if (!normalizedArtist) return [...prefixes];

  const firstSpace = albumFolder.search(/\s/);
  if (firstSpace > 0) {
    const firstToken = albumFolder.slice(0, firstSpace);
    if (normalizeArtistPrefix(firstToken).startsWith(normalizedArtist)) {
      prefixes.add(firstToken);
    }
  }

  return [...prefixes].sort((a, b) => b.length - a.length);
}

function looksLikeCategoryArtistSeparator(text: string): boolean {
  return /(?:\s{2,}|[._:：\-—–])\s*$/.test(text);
}

function looksLikeArtistAlbumSeparator(
  text: string,
  allowYearAfterSingleSpace: boolean,
): boolean {
  if (/^\s{2,}/.test(text)) return true;
  if (/^\s*[-—–]\s+/.test(text)) return true;
  if (/^[._:：\-—–]+/.test(text)) return true;
  return allowYearAfterSingleSpace && /^\s+(?:19|20)\d{2}\b/.test(text);
}

/**
 * When the artist is pure CJK (no Latin) and the album remainder starts
 * with an English name that is the artist's English name, strip it.
 * e.g. "George Lam Ultimate Sound Vol. II" → "Ultimate Sound Vol. II"
 * when artist is "林子祥" (George Lam's Chinese name).
 */
function stripEnglishArtistFromRemainder(
  cleaned: string,
  artist: string,
): string | null {
  if (artist.match(/[\u4e00-\u9fff]/) && !artist.match(/[a-zA-Z]/)) {
    const latinLead = cleaned.match(
      /^([A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+)?)\s+(.+)/,
    );
    if (latinLead) return latinLead[2];
  }
  return null;
}

// ── Path parsing ────────────────────────────────────────────────────

/**
 * Parse artist, album, and year hints from an Artist/Album path.
 */
export function parseAlbumPath(filePath: string): LookupRequest {
  const isFile = isAudioFilePath(filePath);
  const albumPath = isFile ? dirname(filePath) : filePath;
  const albumName = basename(albumPath);
  const parentName = basename(dirname(albumPath));

  let artistHint: string | null = null;
  let albumHint: string | null = null;
  let yearHint: string | null = extractYearFromName(albumName);

  // Detect CD subfolder pattern
  if (CD_SUBFOLDER_RE.test(albumName)) {
    const grandparent = basename(dirname(dirname(albumPath)));
    artistHint = grandparent ? cleanFolderName(grandparent) : null;
    const parent = basename(dirname(albumPath));
    albumHint = parent ? cleanAlbumFolderName(parent, artistHint) : null;
    yearHint = parent ? extractYearFromName(parent) : yearHint;
  } else {
    artistHint = parentName ? cleanFolderName(parentName) : null;
    albumHint = cleanAlbumFolderName(albumName, artistHint) || null;
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
  const scanned = await scanAlbumFilesWithTags(filePath);

  const tagArtist = scanned.artist;
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
    albumHint = folderRequest.albumHint || scanned.album;
  }

  return {
    path: filePath,
    artistHint,
    albumHint,
    yearHint: folderRequest.yearHint || scanned.year,
    musicbrainzAlbumId: scanned.musicbrainzAlbumId,
    musicbrainzArtistId: scanned.musicbrainzArtistId,
    discogsReleaseId: scanned.discogsReleaseId,
    discogsArtistId: scanned.discogsArtistId,
    tracks: scanned.tracks,
  };
}

interface AlbumFileScan {
  artist: string | null;
  album: string | null;
  year: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  discogsReleaseId: string | null;
  discogsArtistId: string | null;
  tracks: TrackCandidate[];
}

async function scanAlbumFilesWithTags(filePath: string): Promise<AlbumFileScan> {
  const isFile = isAudioFilePath(filePath);
  const dirPath = isFile ? dirname(filePath) : filePath;
  const result = {
    artist: null as string | null,
    album: null as string | null,
    year: null as string | null,
    musicbrainzAlbumId: null as string | null,
    musicbrainzArtistId: null as string | null,
    discogsReleaseId: null as string | null,
    discogsArtistId: null as string | null,
    tracks: [] as TrackCandidate[],
  } satisfies AlbumFileScan;

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
      if (!AUDIO_EXTENSIONS.has(ext)) continue;
      index++;
      const filenameHint = trackHintFromFilename(entry);

      try {
        const meta = await readTrackMetadata(fullPath);
        result.artist ??= meta.artist || meta.albumArtist;
        result.album ??= meta.album;
        result.year ??= meta.year;
        result.musicbrainzAlbumId ??= meta.musicbrainzAlbumId;
        result.musicbrainzArtistId ??= meta.musicbrainzArtistId;
        result.discogsReleaseId ??= meta.discogsReleaseId;
        result.discogsArtistId ??= meta.discogsArtistId;
        result.tracks.push(
          makeTrackCandidate({
            title: meta.title || filenameHint.title,
            artist: meta.artist || filenameHint.artist,
            artists: meta.artists.length > 0 ? meta.artists : filenameHint.artists,
            trackNumber: filenameHint.trackNumber ?? meta.trackNumber ?? index,
            trackTotal: null, // filled after all tracks are collected
            discNumber: meta.discNumber,
            musicbrainzTrackId: meta.musicbrainzTrackId,
            length: meta.duration,
            genre: meta.genre,
          }),
        );
      } catch {
        // Read failed — use filename as title hint
        result.tracks.push(
          makeTrackCandidate({
            title: filenameHint.title,
            artist: filenameHint.artist,
            artists: filenameHint.artists,
            trackNumber: filenameHint.trackNumber ?? index,
          }),
        );
      }
    }
  } catch {
    // directory can't be read
  }

  const total = result.tracks.length;
  for (const track of result.tracks) {
    track.trackTotal = total;
  }

  return result;
}

// ── Track hints ─────────────────────────────────────────────────────

/**
 * Build track candidates from audio file metadata.
 */
export async function trackHintsFromPath(filePath: string): Promise<TrackCandidate[]> {
  return (await scanAlbumFilesWithTags(filePath)).tracks;
}

function trackHintFromFilename(
  filename: string,
): { title: string; artist: string | null; artists: string[]; trackNumber: number | null } {
  const ext = extname(filename);
  const stem = filename.slice(0, filename.length - ext.length).trim();
  const match = /^(\d{1,3})\s*[.\-_\s]+\s*(.+)$/.exec(stem);
  const trackNumber = match ? Number(match[1]) : null;
  const withoutTrackNumber = match ? match[2].trim() || stem : stem;
  const artistTitle = splitFilenameArtistTitle(withoutTrackNumber);
  if (artistTitle) {
    return {
      title: cleanFilenameTitle(artistTitle.title),
      artist: artistTitle.artist,
      artists: [artistTitle.artist],
      trackNumber,
    };
  }
  return {
    title: cleanFilenameTitle(withoutTrackNumber),
    artist: null,
    artists: [],
    trackNumber,
  };
}

function splitFilenameArtistTitle(stem: string): { artist: string; title: string } | null {
  const separator = /\s[-–—]\s/.exec(stem);
  if (!separator || separator.index === 0) return null;
  const artist = stem.slice(0, separator.index).trim();
  const title = stem.slice(separator.index + separator[0].length).trim();
  if (!artist || !title) return null;
  return { artist, title };
}

function cleanFilenameTitle(title: string): string {
  const original = title.trim();
  let cleaned = original.replace(/\s*\([^)]*(?:bit|hz|khz|wav|flac|mp3|ape)[^)]*\)\s*$/i, "").trim();
  if (cleaned !== original) {
    cleaned = cleaned.replace(/\s*\([^)]*\)\s*$/u, "").trim();
  }
  return cleaned || title.trim();
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
    musicbrainzAlbumId: request.musicbrainzAlbumId,
    musicbrainzArtistId: request.musicbrainzArtistId,
    discogsReleaseId: request.discogsReleaseId,
    discogsArtistId: request.discogsArtistId,
    tracks: request.tracks,
    source: "folder",
  });
}
