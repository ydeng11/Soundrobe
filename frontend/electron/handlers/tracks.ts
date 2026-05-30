import { ipcMain } from "electron";
import { parseFile } from "music-metadata";
import fs from "fs";
import path from "path";
import type { CoverInfo } from "../preload";
import { writeTags, batchWriteTags, writeExtraTags, batchWriteExtraTags } from "./writer";
import type { ExtraTagUpdate, WriteFields } from "./writer";

export interface TrackData {
  path: string;
  title: string | null;
  artist: string | null;
  artists: string[];
  album: string | null;
  albumArtist: string | null;
  albumArtists: string[];
  trackNumber: number | null;
  trackTotal: number | null;
  discNumber: number | null;
  discTotal: number | null;
  year: string | null;
  genre: string | null;
  composer: string | null;
  comment: string | null;
  lyrics: string | null;
  compilation: boolean | null;
  musicbrainzTrackId: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  hasCover: boolean;
  sizeBytes: number;
  bitrate: number | null;
  sampleRate: number | null;
  codec: string;
  duration: number;
}

export interface ExtraTag {
  key: string;
  value: string;
  source: string;
}

const SUPPORTED_EXTENSIONS = new Set([
  ".mp3",
  ".flac",
  ".m4a",
  ".mp4",
  ".wav",
  ".ogg",
  ".opus",
  ".aiff",
]);

/**
 * Tags that are already shown in the MetadataEditor sidebar.
 * Extra Tags should NOT show these — they'd be duplicating the editor.
 * This set covers both ID3 frame IDs and Vorbis comment key names.
 */
const METADATA_EDITOR_KEYS = new Set([
  // Title
  "TIT2",
  "TITLE",
  // Artist
  "TPE1",
  "ARTIST",
  // Album
  "TALB",
  "ALBUM",
  // Album Artist
  "TPE2",
  "ALBUMARTIST",
  "ALBUM ARTIST",
  "ALBUMARTISTS",
  // Year
  "TDRC",
  "TYER",
  "DATE",
  "YEAR",
  // Track / Disc (numeric fields shown as range in editor)
  "TRCK",
  "TRACKNUMBER",
  "TRACKTOTAL",
  "TOTALTRACKS",
  "TPOS",
  "DISCNUMBER",
  "DISCTOTAL",
  "TOTALDISCS",
  // Genre
  "TCON",
  "GENRE",
  // Composer
  "TCOM",
  "COMPOSER",
  // Comment
  "COMM",
  "COMMENT",
  // Detailed metadata shown in the sidebar
  "USLT",
  "LYRICS",
  "SYLT",
  "COMPILATION",
  "TCMP",
  "MUSICBRAINZ TRACK ID",
  "MUSICBRAINZ ALBUM ID",
  "MUSICBRAINZ ARTIST ID",
  "MUSICBRAINZ_TRACKID",
  "MUSICBRAINZ_ALBUMID",
  "MUSICBRAINZ_ARTISTID",
  "METADATA_BLOCK_PICTURE",
  "APIC",
]);

export function isAudioFile(filePath: string): boolean {
  const ext = path.extname(filePath).toLowerCase();
  return SUPPORTED_EXTENSIONS.has(ext);
}

const COVER_NAMES = ["cover", "Cover", "COVER", "front", "Front", "FRONT", "folder", "Folder", "FOLDER", "albumart", "AlbumArt"];
const COVER_EXTS = [".jpg", ".jpeg", ".png"];

function detectExternalCover(albumPath: string): string | null {
  for (const name of COVER_NAMES) {
    for (const ext of COVER_EXTS) {
      const candidate = path.join(albumPath, `${name}${ext}`);
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }
  return null;
}

export async function readTrackMetadata(filePath: string): Promise<TrackData> {
  let metadata;
  try {
    metadata = await parseFile(filePath);
  } catch (error) {
    const fallback = readFlacMetadataFallback(filePath);
    if (fallback) return fallback;
    throw error;
  }
  const { common, format } = metadata;
  const stat = fs.statSync(filePath);
  if (
    path.extname(filePath).toLowerCase() === ".flac" &&
    (!Number.isFinite(format.duration) || (format.duration ?? 0) <= 0)
  ) {
    const fallback = readFlacMetadataFallback(filePath);
    if (fallback && fallback.duration > 0) return fallback;
  }

  // Format year as string
  let year: string | null = null;
  if (common.year) {
    year = String(common.year);
  } else if (common.date) {
    year = common.date.slice(0, 4);
  }

  // Determine if track has embedded cover art
  const hasCover = common.picture !== undefined && common.picture.length > 0;

  return {
    path: filePath,
    title: common.title ?? null,
    artist: common.artist ?? null,
    artists: common.artists ?? [],
    album: common.album ?? null,
    albumArtist: common.albumartist ?? null,
    albumArtists: common.albumartist ? [common.albumartist] : [],
    trackNumber: common.track?.no ?? null,
    trackTotal: common.track?.of ?? null,
    discNumber: common.disk?.no ?? null,
    discTotal: common.disk?.of ?? null,
    year,
    genre: common.genre?.[0] ?? null,
    composer: common.composer?.[0] ?? null,
    comment: (common.comment as string | undefined) ?? null,
    lyrics: (common.lyrics?.[0] as string | undefined) ?? null,
    compilation: common.compilation ?? null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover,
    sizeBytes: stat.size,
    bitrate: format.bitrate ?? null,
    sampleRate: format.sampleRate ?? null,
    codec: format.codec ?? format.container ?? "unknown",
    duration: format.duration ?? 0,
  };
}

export async function readExtraTags(filePath: string): Promise<ExtraTag[]> {
  const metadata = await parseFile(filePath, { duration: false });
  const rows: ExtraTag[] = [];
  const seen = new Set<string>();

  for (const [source, tags] of Object.entries(metadata.native)) {
    for (const tag of tags) {
      const key = normalizeNativeKey(tag.id, tag.value);
      if (!key || METADATA_EDITOR_KEYS.has(key.toUpperCase())) continue;

      const value = stringifyTagValue(tag.value);
      if (!value) continue;

      const identity = `${source}\0${key}\0${value}`;
      if (seen.has(identity)) continue;
      seen.add(identity);
      rows.push({ key, value, source });
    }
  }

  return rows.sort((a, b) => a.key.localeCompare(b.key));
}

function normalizeNativeKey(id: string, value: unknown): string {
  if (id.startsWith("TXXX:")) {
    const description = id.slice("TXXX:".length).trim();
    if (description) return description;
  }
  if (id === "TXXX" && isRecord(value)) {
    const description = value.description;
    if (typeof description === "string" && description.trim()) {
      return description.trim();
    }
  }
  return id;
}

function stringifyTagValue(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) {
    const parts = value.map((item) => stringifyTagValue(item)).filter(Boolean);
    return parts.length > 0 ? parts.join(", ") : null;
  }
  if (isRecord(value)) {
    if (typeof value.value === "string") return value.value;
    if (typeof value.text === "string") return value.text;
    if (typeof value.value === "number" || typeof value.value === "boolean") {
      return String(value.value);
    }
    if (typeof value.text === "number" || typeof value.text === "boolean") {
      return String(value.text);
    }
    if (Buffer.isBuffer(value.data) || Buffer.isBuffer(value.imageBuffer)) return null;
  }
  return null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function readFlacMetadataFallback(filePath: string): TrackData | null {
  if (path.extname(filePath).toLowerCase() !== ".flac") return null;

  const data = fs.readFileSync(filePath);
  if (data.length < 42 || data.subarray(0, 4).toString("ascii") !== "fLaC") {
    return null;
  }

  const streamInfo = readFlacStreamInfo(data);
  if (!streamInfo) return null;

  const comments = readFlacVorbisComments(data);
  const stat = fs.statSync(filePath);
  const track = splitNumberPair(firstComment(comments, "TRACKNUMBER"));
  const disc = splitNumberPair(firstComment(comments, "DISCNUMBER"));
  const date = firstComment(comments, "DATE") ?? firstComment(comments, "YEAR");
  const artistValues = comments.ARTIST ?? [];
  const albumArtist =
    firstComment(comments, "ALBUMARTIST") ??
    firstComment(comments, "ALBUM ARTIST");

  return {
    path: filePath,
    title: firstComment(comments, "TITLE") ?? path.basename(filePath),
    artist: firstComment(comments, "ARTIST"),
    artists: artistValues,
    album: firstComment(comments, "ALBUM"),
    albumArtist,
    albumArtists: albumArtist ? [albumArtist] : [],
    trackNumber: track.no,
    trackTotal: track.of,
    discNumber: disc.no,
    discTotal: disc.of,
    year: date ? date.slice(0, 4) : null,
    genre: firstComment(comments, "GENRE"),
    composer: firstComment(comments, "COMPOSER"),
    comment: firstComment(comments, "COMMENT"),
    lyrics: firstComment(comments, "LYRICS"),
    compilation: null,
    musicbrainzTrackId: firstComment(comments, "MUSICBRAINZ_TRACKID"),
    musicbrainzAlbumId: firstComment(comments, "MUSICBRAINZ_ALBUMID"),
    musicbrainzArtistId: firstComment(comments, "MUSICBRAINZ_ARTISTID"),
    hasCover: hasFlacPictureBlock(data),
    sizeBytes: stat.size,
    bitrate:
      streamInfo.duration > 0
        ? Math.round((stat.size * 8) / streamInfo.duration)
        : null,
    sampleRate: streamInfo.sampleRate,
    codec: "FLAC",
    duration: streamInfo.duration,
  };
}

function readFlacStreamInfo(
  data: Buffer
): { sampleRate: number | null; duration: number } | null {
  const type = data[4] & 0x7f;
  const length = readFlacBlockLength(data, 4);
  if (type !== 0 || length !== 34 || data.length < 42) return null;

  const offset = 8;
  const sampleRate =
    (data[offset + 10] << 12) |
    (data[offset + 11] << 4) |
    (data[offset + 12] >> 4);
  const totalSamples = Number(
    (BigInt(data[offset + 13] & 0x0f) << 32n) |
      BigInt(data.readUInt32BE(offset + 14))
  );

  return {
    sampleRate: sampleRate > 0 ? sampleRate : null,
    duration: sampleRate > 0 && totalSamples > 0 ? totalSamples / sampleRate : 0,
  };
}

function readFlacVorbisComments(data: Buffer): Record<string, string[]> {
  const block = findFlacBlock(data, 4);
  if (!block) return {};

  const result: Record<string, string[]> = {};
  let offset = block.dataOffset;
  const blockEnd = block.dataOffset + block.length;
  if (offset + 8 > blockEnd) return result;

  const vendorLength = data.readUInt32LE(offset);
  offset += 4 + vendorLength;
  if (offset + 4 > blockEnd) return result;

  const commentCount = data.readUInt32LE(offset);
  offset += 4;

  for (let i = 0; i < commentCount; i++) {
    if (offset + 4 > blockEnd) break;
    const commentLength = data.readUInt32LE(offset);
    offset += 4;
    if (offset + commentLength > blockEnd) break;

    const comment = data.toString("utf8", offset, offset + commentLength);
    offset += commentLength;
    const eqIndex = comment.indexOf("=");
    if (eqIndex <= 0) continue;

    const key = comment.slice(0, eqIndex).toUpperCase();
    const value = comment.slice(eqIndex + 1);
    result[key] ??= [];
    result[key].push(value);
  }

  return result;
}

function hasFlacPictureBlock(data: Buffer): boolean {
  return findFlacBlock(data, 6) !== null;
}

function findFlacBlock(
  data: Buffer,
  desiredType: number
): { dataOffset: number; length: number } | null {
  let offset = 4;
  while (offset + 4 <= data.length) {
    const header = data[offset];
    const type = header & 0x7f;
    const length = readFlacBlockLength(data, offset);
    const dataOffset = offset + 4;
    const nextOffset = dataOffset + length;

    if (type > 6 || nextOffset > data.length) break;
    if (type === desiredType) return { dataOffset, length };
    if (header & 0x80) break;

    offset = nextOffset;
  }
  return null;
}

function readFlacBlockLength(data: Buffer, offset: number): number {
  return (data[offset + 1] << 16) | (data[offset + 2] << 8) | data[offset + 3];
}

function firstComment(
  comments: Record<string, string[]>,
  key: string
): string | null {
  return comments[key]?.[0] ?? null;
}

function splitNumberPair(value: string | null): { no: number | null; of: number | null } {
  if (!value) return { no: null, of: null };
  const [no, of] = value.split("/");
  return { no: parsePositiveInt(no), of: parsePositiveInt(of) };
}

function parsePositiveInt(value: string | undefined): number | null {
  if (!value) return null;
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

/**
 * Concurrent map with a concurrency limit.
 * Preserves output order relative to input order.
 */
async function mapLimit<T, U>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<U>,
): Promise<U[]> {
  const results: U[] = new Array(items.length);
  let nextIndex = 0;

  const worker = async (): Promise<void> => {
    while (nextIndex < items.length) {
      const i = nextIndex++;
      results[i] = await fn(items[i]);
    }
  };

  const workers = Array.from({ length: Math.min(limit, items.length) }, () => worker());
  await Promise.all(workers);
  return results;
}

/** Read all track metadata for an album directory. */
export async function readAlbum(
  albumPath: string
): Promise<{
  path: string;
  name: string;
  artistHint: string;
  albumHint: string;
  tracks: TrackData[];
  coverInfo: CoverInfo;
  status: string;
}> {
  const entries = fs.readdirSync(albumPath);
  const audioFiles: string[] = [];

  for (const entry of entries) {
    if (entry.startsWith(".")) continue;
    const fullPath = path.join(albumPath, entry);
    if (fs.statSync(fullPath).isFile() && isAudioFile(fullPath)) {
      audioFiles.push(fullPath);
    }
  }
  audioFiles.sort();

  let errorCount = 0;

  const tracks = await mapLimit(audioFiles, 6, async (audioFile) => {
    try {
      return await readTrackMetadata(audioFile);
    } catch {
      errorCount++;
      const fileStat = fs.statSync(audioFile);
      return minimalTrack(audioFile, fileStat.size);
    }
  });

  const dirName = path.basename(albumPath);
  const parentDir = path.basename(path.dirname(albumPath));

  const externalCover = detectExternalCover(albumPath);
  const hasEmbeddedCover = tracks.some((t) => t.hasCover);

  const coverInfo: CoverInfo = {
    path: externalCover,
    source: externalCover ? "external" : hasEmbeddedCover ? "embedded" : "missing",
    dataUrl: null,
  };

  let status: string;
  if (errorCount === 0) status = "ok";
  else if (errorCount < tracks.length) status = "warning";
  else status = "error";

  return {
    path: albumPath,
    name: dirName,
    artistHint: parentDir,
    albumHint: dirName,
    tracks,
    coverInfo,
    status,
  };
}

function minimalTrack(filePath: string, sizeBytes: number): TrackData {
  return {
    path: filePath,
    title: path.basename(filePath),
    artist: null,
    artists: [],
    album: null,
    albumArtist: null,
    albumArtists: [],
    trackNumber: null,
    trackTotal: null,
    discNumber: null,
    discTotal: null,
    year: null,
    genre: null,
    composer: null,
    comment: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover: false,
    sizeBytes,
    bitrate: null,
    sampleRate: null,
    codec: "unknown",
    duration: 0,
  };
}

export function registerTrackHandlers(): void {
  ipcMain.handle("album:read", async (_event, albumPath: string) => {
    return readAlbum(albumPath);
  });

  ipcMain.handle(
    "track:write",
    async (
      _event,
      trackPath: string,
      fields: Record<string, unknown>
    ): Promise<TrackData> => {
      // Cast fields to WriteFields (unknown keys are safely ignored by writer)
      const writeFields = fields as unknown as WriteFields;
      await writeTags(trackPath, writeFields);
      // Re-read and return updated metadata — gracefully fall back to a
      // minimal track if the parser can't read the file (e.g. FLAC comment
      // block was restructured by the writer).
      try {
        return await readTrackMetadata(trackPath);
      } catch {
        const stat = fs.statSync(trackPath);
        return minimalTrack(trackPath, stat.size);
      }
    }
  );

  ipcMain.handle(
    "tracks:batch-write",
    async (
      _event,
      updates: Array<{ path: string; fields: Record<string, unknown> }>
    ): Promise<TrackData[]> => {
      const writeUpdates = updates.map((u) => ({
        path: u.path,
        fields: u.fields as unknown as WriteFields,
      }));
      await batchWriteTags(writeUpdates);
      // Re-read all updated tracks
      return Promise.all(updates.map((u) => readTrackMetadata(u.path)));
    }
  );

  ipcMain.handle(
    "track:extra-tags:read",
    async (_event, trackPath: string): Promise<ExtraTag[]> => {
      return readExtraTags(trackPath);
    }
  );

  ipcMain.handle(
    "track:extra-tags:write",
    async (
      _event,
      trackPath: string,
      tags: ExtraTagUpdate[]
    ): Promise<TrackData> => {
      await writeExtraTags(trackPath, tags);
      return readTrackMetadata(trackPath);
    }
  );

  ipcMain.handle(
    "tracks:batch-write-extra-tags",
    async (
      _event,
      updates: Array<{ path: string; tags: ExtraTagUpdate[] }>
    ): Promise<TrackData[]> => {
      await batchWriteExtraTags(updates);
      return Promise.all(updates.map((u) => readTrackMetadata(u.path)));
    }
  );
}
