import { ipcMain } from "electron";
import { parseFile } from "music-metadata";
import fs from "fs";
import { readFile } from "fs/promises";
import path from "path";
import type { CoverInfo } from "../preload";
import { writeTags, parseApeTagItems } from "./writer";
import type { ExtraTagUpdate, WriteFields } from "./writer";
import { getDefaultWriteQueue } from "../services/TagWriteQueue";
import { mapConcurrent, LOCAL_READ_CONCURRENCY } from "../services/concurrency";
import logger from "./debug";

// Set of file extensions that support extra tag writing.
const EXTRA_TAG_EXTENSIONS = new Set([".mp3", ".flac", ".ogg", ".opus", ".wav", ".ape"]);

function isExtraTagSupported(filePath: string): boolean {
  return EXTRA_TAG_EXTENSIONS.has(path.extname(filePath).toLowerCase());
}

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
  description: string | null;
  lyrics: string | null;
  compilation: boolean | null;
  musicbrainzTrackId: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  discogsArtistId: string | null;
  discogsReleaseId: string | null;
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
  ".ape",
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
  // Album Artist (singular — right-panel editable)
  "TPE2",
  "ALBUMARTIST",
  "ALBUM ARTIST",
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
  // Embedded artwork — not shown as extra tags
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

/**
 * Wrapper around parseFile with a timeout to prevent hanging on
 * corrupt or problematic files (e.g. large files on slow external drives).
 */
function parseFileWithTimeout(filePath: string, timeoutMs = 30000): ReturnType<typeof parseFile> {
  return Promise.race([
    parseFile(filePath),
    new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error(`parseFile timed out after ${timeoutMs}ms: ${filePath}`)), timeoutMs),
    ),
  ]);
}

export async function readTrackMetadata(filePath: string): Promise<TrackData> {
  let metadata;
  try {
    metadata = await parseFileWithTimeout(filePath);
  } catch (error) {
    const fallback = readFlacMetadataFallback(filePath);
    if (fallback) return fallback;
    throw error;
  }
  const { common, format } = metadata;
  const stat = fs.statSync(filePath);
  if (
    path.extname(filePath).toLowerCase() === ".flac" &&
    (!Number.isFinite(format.duration) || format.duration! <= 0)
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

  const hasCover = (common.picture?.length ?? 0) > 0;

  const result: TrackData = {
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
    comment: Array.isArray(common.comment)
        ? (common.comment[0]?.text ?? null)
        : (common.comment as string | undefined) ?? null,
    description: extractNativeTag(metadata, "DESCRIPTION") ?? null,
    lyrics: (common.lyrics?.[0] as string | undefined) ?? null,
    compilation: common.compilation ?? null,
    musicbrainzTrackId: extractNativeTag(metadata, "MUSICBRAINZ_TRACKID") ?? null,
    musicbrainzAlbumId: extractNativeTag(metadata, "MUSICBRAINZ_ALBUMID") ?? null,
    musicbrainzArtistId: extractNativeTag(metadata, "MUSICBRAINZ_ARTISTID") ?? null,
    discogsArtistId: extractNativeTag(metadata, "DISCOGS_ARTIST_ID") ?? null,
    discogsReleaseId: extractNativeTag(metadata, "DISCOGS_RELEASE_ID") ?? null,
    hasCover,
    sizeBytes: stat.size,
    bitrate: format.bitrate ?? null,
    sampleRate: format.sampleRate ?? null,
    codec: format.codec ?? format.container ?? "unknown",
    duration: format.duration ?? 0,
  };

  // Safety net: for APE files, if common fields are blank but raw APEv2
  // items exist (e.g. music-metadata couldn't map them), use raw items
  if (path.extname(filePath).toLowerCase() === ".ape" && !result.title) {
    try {
      const raw = await readFile(filePath);
      const items = parseApeTagItems(raw);
      if (items.length > 0) {
        const tags = new Map<string, string[]>();
        for (const { key, value } of items) {
          const u = key.toUpperCase();
          if (!tags.has(u)) tags.set(u, []);
          tags.get(u)!.push(value);
        }
        const get = (k: string): string | null => {
          const v = tags.get(k);
          return v && v.length > 0 ? v[0] : null;
        };
        const parseComposite = (val: string | null): { no: number | null; of: number | null } => {
          if (!val) return { no: null, of: null };
          const parts = val.split("/");
          const no = parts[0] ? parseInt(parts[0], 10) || null : null;
          const of = parts[1] ? parseInt(parts[1], 10) || null : null;
          return { no, of };
        };
        const apeTrack = parseComposite(get("TRACK"));
        const apeDisc = parseComposite(get("DISC"));
        return {
          path: filePath,
          title: result.title ?? get("TITLE"),
          artist: result.artist ?? get("ARTIST"),
          artists: result.artists.length > 0 ? result.artists : (tags.get("ARTIST") ?? []),
          album: result.album ?? get("ALBUM"),
          albumArtist: result.albumArtist ?? get("ALBUM ARTIST"),
          albumArtists: result.albumArtist ? [result.albumArtist] : get("ALBUM ARTIST") ? [get("ALBUM ARTIST")!] : [],
          trackNumber: result.trackNumber ?? apeTrack.no,
          trackTotal: result.trackTotal ?? apeTrack.of,
          discNumber: result.discNumber ?? apeDisc.no,
          discTotal: result.discTotal ?? apeDisc.of,
          year: result.year ?? get("DATE"),
          genre: result.genre ?? get("GENRE"),
          composer: result.composer ?? get("COMPOSER"),
          comment: result.comment ?? get("COMMENT"),
          description: result.description ?? get("DESCRIPTION"),
          lyrics: result.lyrics ?? get("LYRICS"),
          compilation: result.compilation,
          musicbrainzTrackId: null,
          musicbrainzAlbumId: null,
          musicbrainzArtistId: null,
          discogsArtistId: null,
          discogsReleaseId: null,
          hasCover: false,
          sizeBytes: stat.size,
          bitrate: format.bitrate ?? null,
          sampleRate: format.sampleRate ?? null,
          codec: format.codec ?? format.container ?? "unknown",
          duration: format.duration ?? 0,
        };
      }
    } catch {
      // ignore fallback failures
    }
  }

  return result;
}

export async function readExtraTags(filePath: string): Promise<ExtraTag[]> {
  let metadata;
  try {
    metadata = await parseFile(filePath, { duration: false });
  } catch (error) {
    // Gracefully handle corrupt/unsupported files (e.g. malformed picture tags in FLAC)
    logger.debug("extra-tags", `Failed to parse file for extra tags: ${filePath}`, {
      error: error instanceof Error ? error.message : String(error),
    });
    return [];
  }
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

  // APEv2 fallback: if native tags are empty, try raw parsing
  if (rows.length === 0 && path.extname(filePath).toLowerCase() === ".ape") {
    try {
      const raw = await readFile(filePath);
      const items = parseApeTagItems(raw);
      for (const { key, value } of items) {
        if (!key || METADATA_EDITOR_KEYS.has(key.toUpperCase())) continue;
        const identity = `APEv2\0${key}\0${value}`;
        if (seen.has(identity)) continue;
        seen.add(identity);
        rows.push({ key, value, source: "APEv2" });
      }
    } catch {
      // ignore
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

/**
 * Extract a tag value from music-metadata's native format by key name.
 * Searches all native formats (vorbis, id3v2.4, etc.) for the given key.
 * For ID3v2 TXXX frames, matches against the description field.
 * Returns the first matching value, or null if not found.
 */
/**
 * Normalize a tag key or description for comparison by removing
 * underscores, hyphens, and spaces and uppercasing.
 * This allows Vorbis-style "MUSICBRAINZ_TRACKID" to match
 * TXXX-style "MusicBrainz Track Id" and vice versa.
 */
function normalizeTagKey(s: string): string {
  return s.replace(/[\s_-]/g, "").toUpperCase();
}

/**
 * Extract a tag value from music-metadata's native format by key name.
 * Searches all native formats (vorbis, id3v2.4, etc.) for the given key.
 * For ID3v2 TXXX frames, matches against the description field.
 * Uses normalized comparison so "MUSICBRAINZ_TRACKID" matches
 * "MusicBrainz Track Id" (underscores vs spaces).
 * Returns the first matching value, or null if not found.
 */
function extractNativeTag(
  metadata: Awaited<ReturnType<typeof parseFile>>,
  key: string,
): string | null {
  const normalizedKey = normalizeTagKey(key);
  for (const [, tags] of Object.entries(metadata.native)) {
    for (const tag of tags) {
      if (typeof tag.id !== "string") continue;
      const tagId = tag.id.toUpperCase();

      // Direct key match (FLAC Vorbis, APEv2) — normalized
      if (normalizeTagKey(tag.id) === normalizedKey) {
        if (typeof tag.value === "string") return tag.value;
        const rec = tag.value;
        if (isRecord(rec) && typeof rec.value === "string") return rec.value;
        if (isRecord(rec) && typeof rec.text === "string") return rec.text;
      }

      // TXXX frame with description embedded in tag.id (e.g. "TXXX:MusicBrainz Track Id")
      if (tagId.startsWith("TXXX:")) {
        const description = tag.id.slice("TXXX:".length);
        if (normalizeTagKey(description) === normalizedKey) {
          if (typeof tag.value === "string") return tag.value;
          if (isRecord(tag.value) && typeof tag.value.value === "string") return tag.value.value;
          if (isRecord(tag.value) && typeof tag.value.text === "string") return tag.value.text;
        }
      }

      // TXXX frame with description in value object
      if (tagId === "TXXX" && isRecord(tag.value)) {
        const description = (tag.value.description as string) ?? "";
        if (normalizeTagKey(description) === normalizedKey) {
          if (typeof tag.value.value === "string") return tag.value.value;
          if (typeof tag.value.text === "string") return tag.value.text;
        }
      }
    }
  }
  return null;
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
    description: firstComment(comments, "DESCRIPTION"),
    lyrics: firstComment(comments, "LYRICS"),
    compilation: null,
    musicbrainzTrackId: firstComment(comments, "MUSICBRAINZ_TRACKID"),
    musicbrainzAlbumId: firstComment(comments, "MUSICBRAINZ_ALBUMID"),
    musicbrainzArtistId: firstComment(comments, "MUSICBRAINZ_ARTISTID"),
    discogsArtistId: firstComment(comments, "DISCOGS_ARTIST_ID"),
    discogsReleaseId: firstComment(comments, "DISCOGS_RELEASE_ID"),
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

  const tracks = await mapConcurrent(audioFiles, LOCAL_READ_CONCURRENCY, async (audioFile) => {
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
    description: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    discogsArtistId: null,
    discogsReleaseId: null,
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
      const writeFields = fields as unknown as WriteFields;
      const result = await getDefaultWriteQueue().submitOne(trackPath, writeFields);
      if (!result.success) {
        throw new Error(result.error ?? "Write failed");
      }
      // Re-read and return updated metadata — gracefully fall back to a
      // minimal track if the parser can't read the file.
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
        filePath: u.path,
        fields: u.fields as unknown as WriteFields,
      }));
      const writeResults = await getDefaultWriteQueue().submit(writeUpdates);

      // Surface write failures clearly before readback
      const failures = writeResults.filter((r) => !r.success);
      if (failures.length > 0) {
        const details = failures
          .map((f) => `${f.filePath}: ${f.error ?? "unknown error"}`)
          .join("; ");
        throw new Error(`Write failed for ${failures.length} file(s): ${details}`);
      }

      // Re-read all updated tracks using bounded concurrency
      return mapConcurrent(
        updates,
        LOCAL_READ_CONCURRENCY,
        async (u) => {
          try {
            return await readTrackMetadata(u.path);
          } catch {
            const stat = fs.statSync(u.path);
            return minimalTrack(u.path, stat.size);
          }
        },
      );
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
      const result = await getDefaultWriteQueue().submitOne(trackPath, undefined, tags);
      if (!result.success) {
        throw new Error(result.error ?? "Write failed");
      }
      return readTrackMetadata(trackPath);
    }
  );

  ipcMain.handle(
    "tracks:batch-write-extra-tags",
    async (
      _event,
      updates: Array<{ path: string; tags: ExtraTagUpdate[] }>
    ): Promise<TrackData[]> => {
      // Filter to only formats that support extra tags; warn for skipped files
      const filtered = updates.filter((u) => {
        const ok = isExtraTagSupported(u.path);
        if (!ok) {
          logger.debug(
            "write",
            `Skipping extra-tag write for unsupported format: ${u.path}`,
          );
        }
        return ok;
      });

      if (filtered.length === 0) {
        return mapConcurrent(
          updates,
          LOCAL_READ_CONCURRENCY,
          async (u) => {
            try {
              return await readTrackMetadata(u.path);
            } catch {
              const stat = fs.statSync(u.path);
              return minimalTrack(u.path, stat.size);
            }
          },
        );
      }

      const writeUpdates = filtered.map((u) => ({
        filePath: u.path,
        extraTags: u.tags,
      }));
      const results = await getDefaultWriteQueue().submit(writeUpdates);

      // Surface any queue failures before readback
      const failures = results.filter((r) => !r.success);
      if (failures.length > 0) {
        const errMsg = failures
          .map((f) => `${f.filePath}: ${f.error ?? "unknown error"}`)
          .join("; ");
        throw new Error(`Batch extra-tag write failed for ${failures.length} file(s): ${errMsg}`);
      }

      // Re-read all updated tracks using bounded concurrency
      return mapConcurrent(
        updates,
        LOCAL_READ_CONCURRENCY,
        async (u) => {
          try {
            return await readTrackMetadata(u.path);
          } catch {
            const stat = fs.statSync(u.path);
            return minimalTrack(u.path, stat.size);
          }
        },
      );
    }
  );

  /** Rename a single audio file on disk and return updated metadata. */
  ipcMain.handle(
    "track:rename",
    async (_event, oldPath: string, newPath: string): Promise<TrackData> => {
      // Ensure the target directory exists
      const newDir = path.dirname(newPath);
      if (!fs.existsSync(newDir)) {
        fs.mkdirSync(newDir, { recursive: true });
      }
      await fs.promises.rename(oldPath, newPath);
      return readTrackMetadata(newPath);
    }
  );

  /** Check if a file exists on disk. */
  ipcMain.handle(
    "file:exists",
    async (_event, filePath: string): Promise<boolean> => {
      return fs.existsSync(filePath);
    }
  );
}
