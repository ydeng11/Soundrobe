import path from "path";
import { randomUUID } from "crypto";
import { readFile, writeFile, open, rename, unlink } from "fs/promises";
import * as NodeID3 from "node-id3";
import {
  buildApeFooter,
  buildApeTagItems,
  parseApeTagItems,
  stripApeTag,
} from "../services/ApeTagEngine";
export { findApeFooterOffset, parseApeTagItems } from "../services/ApeTagEngine";

type NodeID3Module = typeof import("node-id3");
const nodeId3 = ((NodeID3 as unknown as { default?: NodeID3Module }).default ??
  NodeID3) as NodeID3Module;

/**
 * Read ID3v2 tags from a file using node-id3's async path.
 * Wraps the callback-based API in a promise to avoid blocking the main process.
 */
function readNodeId3Tags(filePath: string): Promise<NodeID3.Tags> {
  return new Promise((resolve, reject) => {
    nodeId3.read(filePath, {}, (err: Error | null, tags: NodeID3.Tags | null) => {
      if (err) reject(err);
      else resolve(tags ?? {});
    });
  });
}

/**
 * Write ID3v2 tags to a file using node-id3's async path.
 */
function writeNodeId3Tags(tags: NodeID3.Tags, filePath: string): Promise<void> {
  return new Promise((resolve, reject) => {
    nodeId3.write(tags, filePath, (err: Error | null) => {
      if (err) reject(err);
      else resolve();
    });
  });
}

/**
 * Mapping of field names → tag specs for each format.
 *
 * Fields param follows a normalized schema:
 *   title, artist, artists, album, albumArtist, albumArtists, year,
 *   track, disc, trackNumber, trackTotal, discNumber, discTotal,
 *   genre, composer, comment, description, lyrics, compilation,
 *   musicbrainzTrackId, musicbrainzAlbumId, musicbrainzArtistId,
 *   coverData, coverMime
 */

export interface WriteFields {
  title?: string | null;
  artist?: string | null;
  artists?: string[] | string | null;
  album?: string | null;
  albumArtist?: string | null;
  albumArtists?: string[] | string | null;
  year?: string | null;
  track?: string | null; // "1" or "1/10"
  trackNumber?: number | null;
  trackTotal?: number | null;
  disc?: string | null; // "1" or "1/2"
  discNumber?: number | null;
  discTotal?: number | null;
  genre?: string | null;
  composer?: string | null;
  comment?: string | null;
  description?: string | null;
  lyrics?: string | null;
  compilation?: boolean | null;
  musicbrainzTrackId?: string | null;
  musicbrainzAlbumId?: string | null;
  musicbrainzArtistId?: string | null;
  discogsArtistId?: string | null;
  discogsReleaseId?: string | null;
  coverData?: Buffer | null;
  coverMime?: string | null;
}

export interface ExtraTagUpdate {
  key: string;
  value: string;
}

/**
 * Internal write outcome — tracks what kind of disk I/O was performed.
 * Renderer-facing APIs are unchanged; this is used by the queue/debug path.
 */
export type WriteOutcome = "skipped" | "in_place" | "metadata_rewrite" | "full_rewrite";

/** Describes one FLAC metadata block in the file layout. */
interface FlacBlockInfo {
  type: number;
  headerOffset: number;
  dataOffset: number;
  length: number;
  isLast: boolean;
}

const STANDARD_VORBIS_TAGS = new Set([
  "TITLE",
  "ARTIST",
  "ARTISTS",
  "ALBUM",
  "ALBUMARTIST",
  "ALBUM ARTIST",
  "DATE",
  "YEAR",
  "GENRE",
  "COMPOSER",
  "LYRICS",
  "UNSYNCEDLYRICS",
  "UNSYNCHRONISEDLYRICS",
  "TRACK",
  "TRACKNUMBER",
  "TRACKTOTAL",
  "TOTALTRACKS",
  "DISC",
  "DISCNUMBER",
  "DISCTOTAL",
  "TOTALDISCS",
  "METADATA_BLOCK_PICTURE",
]);

const EXTRA_TAG_RESERVED_EXCEPTIONS = new Set(["ARTISTS"]);

/**
 * Convert our normalized fields into format-specific tag objects.
 */

function fieldsToID3v2(fields: WriteFields): NodeID3.Tags {
  const tags: NodeID3.Tags = {};
  if (fields.title !== undefined) tags.title = fields.title ?? undefined;
  if (fields.artist !== undefined) tags.artist = fields.artist ?? undefined;
  if (fields.album !== undefined) tags.album = fields.album ?? undefined;
  if (fields.albumArtist !== undefined) tags.performerInfo = fields.albumArtist ?? undefined;
  if (fields.year !== undefined) tags.year = fields.year ?? undefined;
  if (fields.trackNumber !== undefined || fields.track !== undefined) {
    const rawTrack = fields.trackNumber ?? (fields.track ? parseInt(fields.track) : undefined);
    tags.trackNumber = formatPosition(rawTrack, fields.trackTotal) ?? undefined;
  }
  if (fields.discNumber !== undefined || fields.disc !== undefined) {
    const rawDisc = fields.discNumber ?? (fields.disc ? parseInt(fields.disc) : undefined);
    tags.partOfSet = formatPosition(rawDisc, fields.discTotal) ?? undefined;
  }
  if (fields.genre !== undefined) tags.genre = fields.genre ?? undefined;
  if (fields.composer !== undefined) {
    tags.composer = fields.composer ?? undefined;
  }
  if (fields.comment !== undefined) {
    tags.comment = fields.comment
      ? { language: "eng", text: fields.comment }
      : undefined;
  }
  if (fields.lyrics !== undefined) {
    tags.unsynchronisedLyrics = fields.lyrics
      ? { language: "eng", text: fields.lyrics }
      : undefined;
  }
  if (fields.coverData) {
    tags.image = {
      mime: fields.coverMime ?? "image/jpeg",
      type: { id: 3, name: "front cover" },
      description: "Cover",
      imageBuffer: fields.coverData,
    };
  }
  return tags;
}

/**
 * Write tags to an MP3 file using node-id3.
 */
async function writeMp3(filePath: string, fields: WriteFields): Promise<void> {
  const tags = fieldsToID3v2(fields);
  const existingTags = await readNodeId3Tags(filePath);
  const custom = mergeMp3UserDefinedText(existingTags.userDefinedText, fields);
  const mergedTags = { ...existingTags, ...tags };
  if (custom !== undefined) {
    mergedTags.userDefinedText = custom;
  } else {
    delete mergedTags.userDefinedText;
  }
  await writeNodeId3Tags(mergedTags, filePath);
}

function mergeMp3UserDefinedText(
  current: NodeID3.Tags["userDefinedText"],
  fields: WriteFields,
): NodeID3.Tags["userDefinedText"] {
  const rows = toArray(current).filter((row) => row.description);
  const upserts: Array<{ description: string; value: string | string[] | null | undefined }> = [
    { description: "ARTISTS", value: normalizeListValue(fields.artists) },
    { description: "ALBUMARTISTS", value: normalizeListValue(fields.albumArtists) },
    { description: "MusicBrainz Track Id", value: fields.musicbrainzTrackId },
    { description: "MusicBrainz Album Id", value: fields.musicbrainzAlbumId },
    { description: "MusicBrainz Artist Id", value: fields.musicbrainzArtistId },
    { description: "Discogs Artist Id", value: fields.discogsArtistId },
    { description: "Discogs Release Id", value: fields.discogsReleaseId },
    { description: "COMPILATION", value: compilationToTag(fields.compilation) },
    { description: "DESCRIPTION", value: fields.description },
  ];

  for (const { description, value } of upserts) {
    if (value === undefined) continue;
    const text = Array.isArray(value) ? value.join("; ") : value;
    const index = rows.findIndex((row) => row.description === description);
    if (!text) {
      if (index >= 0) rows.splice(index, 1);
      continue;
    }
    const next = { description, value: text };
    if (index >= 0) rows[index] = next;
    else rows.push(next);
  }

  return rows.length > 0 ? rows : undefined;
}

async function writeMp3ExtraTags(filePath: string, extraTags: ExtraTagUpdate[]): Promise<void> {
  const existingTags = await readNodeId3Tags(filePath);
  const preserved = toArray(existingTags.userDefinedText).filter(
    (tag) =>
      tag.description &&
      isReservedExtraTagKey(tag.description) &&
      !EXTRA_TAG_RESERVED_EXCEPTIONS.has(tag.description.trim().toUpperCase()),
  );
  const custom = normalizeExtraTags(extraTags, EXTRA_TAG_RESERVED_EXCEPTIONS).map((tag) => ({
    description: tag.key,
    value: tag.value,
  }));
  const nextTags: NodeID3.Tags = {
    ...existingTags,
    userDefinedText: [...preserved, ...custom],
  };
  await writeNodeId3Tags(nextTags, filePath);
}

/**
 * Write extra tags to a WAV file's embedded ID3v2 chunk.
 * WAV uses an `id3 ` RIFF chunk with standard ID3v2 frames,
 * so the same node-id3 logic as MP3 extra tags applies.
 */
async function writeWavExtraTags(filePath: string, extraTags: ExtraTagUpdate[]): Promise<WriteOutcome> {
  const data = await readFile(filePath);
  if (
    data.length < 12 ||
    data.toString("ascii", 0, 4) !== "RIFF" ||
    data.toString("ascii", 8, 12) !== "WAVE"
  ) {
    throw new Error("Invalid WAV file");
  }

  let existingTags: NodeID3.Tags = {};
  let existingId3Offset = -1;
  let existingId3Size = 0;
  let hasListInfoChunk = false;
  const chunks: Buffer[] = [];

  for (let offset = 12; offset + 8 <= data.length; ) {
    const id = data.toString("ascii", offset, offset + 4);
    const size = data.readUInt32LE(offset + 4);
    const end = offset + 8 + size + (size % 2);
    if (end > data.length || end < offset + 8) break;
    if (id === "id3 " || id === "ID3 ") {
      existingTags = {
        ...existingTags,
        ...nodeId3.read(data.subarray(offset + 8, offset + 8 + size)),
      };
      existingId3Offset = offset;
      existingId3Size = size;
    } else if (id === "LIST" && size >= 4 && data.toString("ascii", offset + 8, offset + 12) === "INFO") {
      // Strip legacy LIST INFO chunk (corrupt/mangled metadata)
      hasListInfoChunk = true;
    } else {
      chunks.push(data.subarray(offset, end));
    }
    offset = end;
  }

  // Apply extra-tag update (same logic as MP3)
  const preserved = toArray(existingTags.userDefinedText).filter(
    (tag) =>
      tag.description &&
      isReservedExtraTagKey(tag.description) &&
      !EXTRA_TAG_RESERVED_EXCEPTIONS.has(tag.description.trim().toUpperCase()),
  );
  const custom = normalizeExtraTags(extraTags, EXTRA_TAG_RESERVED_EXCEPTIONS).map((tag) => ({
    description: tag.key,
    value: tag.value,
  }));
  const mergedTags: NodeID3.Tags = {
    ...existingTags,
    userDefinedText: [...preserved, ...custom],
  };

  const id3Payload = nodeId3.create(mergedTags);

  // Try in-place: existing chunk has enough room (skip if LIST INFO needs stripping)
  if (!hasListInfoChunk && existingId3Offset >= 0 && id3Payload.length <= existingId3Size) {
    const dataOffset = existingId3Offset + 8;
    const existingPayload = data.subarray(dataOffset, dataOffset + existingId3Size);
    if (isWavId3ChunkUnchanged(existingPayload, id3Payload)) {
      return "skipped";
    }

    const handle = await open(filePath, "r+");
    try {
      await handle.write(id3Payload, 0, id3Payload.length, dataOffset);
      if (id3Payload.length < existingId3Size) {
        const zeroBuf = Buffer.alloc(existingId3Size - id3Payload.length);
        await handle.write(zeroBuf, 0, zeroBuf.length, dataOffset + id3Payload.length);
      }
    } finally {
      await handle.close();
    }
    return "in_place";
  }

  // Full rewrite: append new ID3 chunk
  const id3Chunk = wavChunk("id3 ", id3Payload);
  chunks.push(id3Chunk);

  const body = Buffer.concat([Buffer.from("WAVE", "ascii"), ...chunks]);
  const header = Buffer.alloc(8);
  header.write("RIFF", 0, 4, "ascii");
  header.writeUInt32LE(body.length, 4);
  await writeFile(filePath, Buffer.concat([header, body]));
  return "full_rewrite";
}

/**
 * Write Vorbis comments to a FLAC / OGG / OPUS file.
 * Vorbis comments are stored as FLAC metadata blocks or OGG page comments
 * as a series of KEY=VALUE strings (UTF-8), prefixed by a 32-bit count.
 */
async function writeVorbis(
  filePath: string,
  fields: WriteFields
): Promise<WriteOutcome> {
  const data = await readFile(filePath);
  const existing = readVorbisComments(data);
  const updated = { ...existing };

  setVorbisField(updated, "TITLE", fields.title);
  setVorbisField(updated, "ARTIST", fields.artist);
  setVorbisList(updated, "ARTISTS", fields.artists);
  setVorbisField(updated, "ALBUM", fields.album);
  setVorbisField(updated, "ALBUMARTIST", fields.albumArtist);
  setVorbisList(updated, "ALBUMARTISTS", fields.albumArtists);
  setVorbisField(updated, "DATE", fields.year);
  setVorbisField(updated, "GENRE", fields.genre);
  setVorbisField(updated, "COMPOSER", fields.composer);
  setVorbisField(updated, "COMMENT", fields.comment);
  setVorbisField(updated, "DESCRIPTION", fields.description);
  setVorbisField(updated, "TRACKNUMBER", fields.trackNumber ?? fields.track);
  setVorbisField(updated, "TRACKTOTAL", fields.trackTotal);
  setVorbisField(updated, "DISCNUMBER", fields.discNumber ?? fields.disc);
  setVorbisField(updated, "DISCTOTAL", fields.discTotal);
  setVorbisField(updated, "LYRICS", fields.lyrics);
  setVorbisField(updated, "MUSICBRAINZ_TRACKID", fields.musicbrainzTrackId);
  setVorbisField(updated, "MUSICBRAINZ_ALBUMID", fields.musicbrainzAlbumId);
  setVorbisField(updated, "MUSICBRAINZ_ARTISTID", fields.musicbrainzArtistId);
  setVorbisField(updated, "DISCOGS_ARTIST_ID", fields.discogsArtistId);
  setVorbisField(updated, "DISCOGS_RELEASE_ID", fields.discogsReleaseId);
  setVorbisField(updated, "COMPILATION", compilationToTag(fields.compilation));
  // Never store cover art in Vorbis comments — it bloats the block and causes
  // metadata chain breaks with large images. Strip any existing entry too.
  delete updated.METADATA_BLOCK_PICTURE;

  const outcome = await writeVorbisComments(filePath, data, updated);

  // For FLAC files, write cover as a native METADATA_BLOCK_PICTURE (type 6)
  // instead of inside Vorbis comments, to avoid bloating the Vorbis block.
  if (fields.coverData && filePath.toLowerCase().endsWith(".flac")) {
    const pictureData = buildFlacPictureBlock(fields.coverData, fields.coverMime ?? "image/jpeg");
    const fileBuf = await readFile(filePath);
    await writeFlacNonVorbisBlock(filePath, fileBuf, 6, pictureData);
  }

  return outcome;
}

interface VorbisDict {
  [key: string]: string[];
}

function setVorbisField(
  comments: VorbisDict,
  key: string,
  value: string | number | null | undefined
): void {
  if (value === undefined) return;
  const text = value == null ? "" : String(value);
  if (text === "") {
    delete comments[key];
  } else {
    comments[key] = [text];
  }
}

function setVorbisList(
  comments: VorbisDict,
  key: string,
  value: string[] | string | null | undefined,
): void {
  if (value === undefined) return;
  const values = normalizeListValue(value);
  if (values.length === 0) {
    delete comments[key];
  } else {
    comments[key] = values;
  }
}

/**
 * Read Vorbis comments from a buffer. Returns dict of key → values.
 */
function readVorbisComments(buf: Buffer): VorbisDict {
  const result: VorbisDict = {};
  // FLAC: comments are in metadata block type 4 (VORBIS_COMMENT)
  // Find fLaC marker (some files have ID3v2 tag prepended)
  const flacOffset = buf.indexOf("fLaC");
  let offset = flacOffset >= 0 ? flacOffset + 4 : 4; // skip "fLaC"
  while (offset < buf.length) {
    const isLast = buf[offset] >> 7; // bit 7 = isLastBlock
    const type = buf[offset] & 0x7f;
    const length =
      (buf[offset + 1] << 16) |
      (buf[offset + 2] << 8) |
      buf[offset + 3];
    offset += 4;

    if (type === 4) {
      return parseVorbisCommentBlock(buf, offset, length);
    }

    if (isLast) break;
    offset += length;
  }
  return result;
}

/**
 * Parse a Vorbis comment block (FLAC metadata block type 4 or OGG comment).
 * Format:
 *   4 bytes: vendor string length
 *   N bytes: vendor string (UTF-8)
 *   4 bytes: number of comment fields
 *   For each: 4 bytes length + N bytes comment (UTF-8 "KEY=VALUE")
 */
function parseVorbisCommentBlock(
  buf: Buffer,
  offset: number,
  length: number
): VorbisDict {
  const result: VorbisDict = {};
  const blockEnd = offset + length;

  // Guard against truncated block
  if (offset + 8 > blockEnd) return result;

  // Vendor string length + string
  const vendorLen = buf.readUInt32LE(offset);
  offset += 4 + vendorLen;

  if (offset + 4 > blockEnd) return result;

  // Number of comments
  const numComments = buf.readUInt32LE(offset);
  offset += 4;

  for (let i = 0; i < numComments; i++) {
    if (offset + 4 > blockEnd) break;
    const commentLen = buf.readUInt32LE(offset);
    offset += 4;
    if (offset + commentLen > blockEnd) break;
    const comment = buf.toString("utf8", offset, offset + commentLen);
    offset += commentLen;

    const eqIdx = comment.indexOf("=");
    if (eqIdx > 0) {
      const key = comment.substring(0, eqIdx).toUpperCase();
      const value = comment.substring(eqIdx + 1);
      if (!result[key]) result[key] = [];
      result[key].push(value);
    }
  }

  return result;
}

/**
 * Strip APEv2 tag from the end of a FLAC file buffer.
 *
 * APEv2 tags are non-standard in FLAC files. They are sometimes injected by
 * Chinese streaming services (e.g. QQ Music) and cause `music-metadata` to
 * override Vorbis comment values with stale APE data.
 *
 * Returns the buffer with the APE tag removed, or the original buffer if
 * no APE tag is found.
 */
function hasApeTag(buf: Buffer): boolean {
  if (buf.length < 64) return false;
  const footerStart = buf.length - 32;
  return footerStart >= 0 && buf.toString("ascii", footerStart, footerStart + 8) === "APETAGEX";
}

function stripApeTagFromBuffer(buf: Buffer): Buffer {
  // APE tag footer is 32 bytes at the very end of the file.
  if (buf.length < 64) return buf;

  // The footer must be exactly at the end — APE tags are always last.
  const footerStart = buf.length - 32;
  if (footerStart < 0) return buf;

  // Check for 'APETAGEX' magic at the expected footer position
  if (buf.toString("ascii", footerStart, footerStart + 8) !== "APETAGEX") return buf;

  const tagSize = buf.readUInt32LE(footerStart + 12); // includes footer
  if (tagSize < 32 || tagSize > buf.length) return buf;

  const dataStart = buf.length - tagSize;
  if (dataStart < 0) return buf;

  return buf.subarray(0, dataStart);
}

/**
 * Write Vorbis comments into a FLAC/OGG file buffer.
 */
async function writeVorbisComments(
  filePath: string,
  origBuf: Buffer,
  comments: VorbisDict
): Promise<WriteOutcome> {
  // Never store cover art in Vorbis comments — it bloats the block and causes
  // metadata chain breaks with large images. Strip any existing entry too.
  delete comments.METADATA_BLOCK_PICTURE;

  // Build the comment block body
  const vendorString = Buffer.from("auto-tagger", "utf8");
  const vendorLen = Buffer.alloc(4);
  vendorLen.writeUInt32LE(vendorString.length);

  // Build comment entries
  const commentEntries: Buffer[] = [];
  let commentCount = 0;
  for (const [key, values] of Object.entries(comments)) {
    for (const value of values) {
      const entry = Buffer.from(`${key}=${value}`, "utf8");
      const entryLen = Buffer.alloc(4);
      entryLen.writeUInt32LE(entry.length);
      commentEntries.push(entryLen, entry);
      commentCount++;
    }
  }

  const numComments = Buffer.alloc(4);
  numComments.writeUInt32LE(commentCount);

  const commentBlock = Buffer.concat([
    vendorLen,
    vendorString,
    numComments,
    ...commentEntries,
  ]);

  if (filePath.toLowerCase().endsWith(".flac")) {
    // Strip non-standard APEv2 tags (injected by QQ Music etc.) that would
    // override Vorbis comment values in music-metadata reads.
    const hasApe = hasApeTag(origBuf);
    let cleaned = hasApe ? stripApeTagFromBuffer(origBuf) : origBuf;

    // Neutralize ghost Vorbis Comment blocks embedded in the audio data.
    // music-metadata reads all VCs and merges them, so stale values in a
    // ghost VC override the correct ones in the metadata chain.
    const ghost = neutralizeGhostVorbisComments(cleaned);
    cleaned = ghost.buf;

    if (hasApe || ghost.found) {
      // Full rewrite required: can't remove bytes via in-place write
      const layout = parseFlacLayout(cleaned);
      await writeFlacWithPaddingFallback(filePath, cleaned, layout, commentBlock);
      return "full_rewrite";
    }
    return await writeFlacMetadataBlock(filePath, cleaned, 4, commentBlock);
  } else if (
    filePath.toLowerCase().endsWith(".ogg") ||
    filePath.toLowerCase().endsWith(".opus")
  ) {
    await writeOggVorbisComments(filePath, commentBlock);
    return "full_rewrite";
  } else {
    // Unsupported Vorbis container — fall through
    await writeFile(filePath, origBuf);
    return "full_rewrite";
  }
}

/** CRC32 table for OGG page checksums. */
const OGG_CRC_TABLE = new Int32Array(256);
for (let i = 0; i < 256; i++) {
  let c = i;
  for (let j = 0; j < 8; j++) {
    c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  OGG_CRC_TABLE[i] = c;
}

function oggCrc32(buf: Buffer): number {
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    crc = OGG_CRC_TABLE[(crc ^ buf[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/**
 * Write Vorbis comments to an OGG/OPUS file by finding and replacing the
 * Vorbis comment page (packet type 3).
 */
async function writeOggVorbisComments(filePath: string, commentBlock: Buffer): Promise<void> {
  const fileBuf = await readFile(filePath);
  let offset = 0;

  while (offset + 27 <= fileBuf.length) {
    const magic = fileBuf.toString("ascii", offset, offset + 4);
    if (magic !== "OggS") {
      offset++;
      continue;
    }

    const numSegments = fileBuf[offset + 26];

    if (offset + 27 + numSegments > fileBuf.length) break;

    // Read segment table and calculate total segment data size
    const segTable: number[] = [];
    let segDataSize = 0;
    for (let s = 0; s < numSegments; s++) {
      const sz = fileBuf[offset + 27 + s];
      segTable.push(sz);
      segDataSize += sz;
    }

    const pageDataStart = offset + 27 + numSegments;
    const pageDataEnd = pageDataStart + segDataSize;
    if (pageDataEnd > fileBuf.length) break;

    // Check if this page contains a Vorbis comment header (packet type 3)
    if (segDataSize >= 7) {
      const firstByte = fileBuf[pageDataStart];
      const magicVorbis = fileBuf.toString("ascii", pageDataStart + 1, pageDataStart + 7);
      if (firstByte === 3 && magicVorbis === "vorbis") {
        // Found the Vorbis comment page
        const pageStart = offset;

        // Find where the old Vorbis comment packet ends.
        // In OGG, a packet ends when a segment has a value < 255.
        // The old comment packet's end position within pageDataStart is
        // the sum of segment sizes until the first segment < 255 (or all
        // segments if they are all exactly 255, meaning it continues on
        // the next page).
        let oldPacketEnd = pageDataStart;
        for (let s = 0; s < numSegments; s++) {
          const sz = segTable[s];
          oldPacketEnd += sz;
          if (sz < 255) break; // packet boundary
        }

        // If oldPacketEnd > pageDataEnd, the packet spans multiple pages
        // — skip to avoid corrupting the file
        if (oldPacketEnd > pageDataEnd) {
          offset = pageDataEnd;
          continue;
        }

        // Data after the old comment packet within the same page
        // (typically the Vorbis setup header) must be preserved.
        const tailData = fileBuf.subarray(
          Math.min(oldPacketEnd, pageDataEnd),
          pageDataEnd,
        );

        // Build new comment packet
        const newPacket = Buffer.concat([
          Buffer.from([3]), // packet type
          Buffer.from("vorbis", "ascii"),
          commentBlock,
          Buffer.from([1]), // framing flag
        ]);

        // Combined payload: new comment packet + preserved tail (setup header etc.)
        const combinedPayload = Buffer.concat([newPacket, tailData]);

        // Build segment table for the combined payload
        const newSegTable: number[] = [];
        let remaining = combinedPayload.length;
        while (remaining > 0) {
          newSegTable.push(Math.min(remaining, 255));
          remaining -= 255;
        }
        const newSegTableBuf = Buffer.from(newSegTable);

        const afterPage = fileBuf.subarray(pageDataEnd);

        // Build new page header
        const newPageHeader = Buffer.alloc(27);
        fileBuf.copy(newPageHeader, 0, pageStart, pageStart + 22); // copy up to CRC
        newPageHeader.writeUInt32LE(0, 22); // zero CRC for calculation
        newPageHeader[26] = newSegTable.length; // num segments

        const newPage = Buffer.concat([
          newPageHeader,
          newSegTableBuf,
          combinedPayload,
        ]);

        // Calculate CRC
        const crcVal = oggCrc32(newPage);
        newPage.writeUInt32LE(crcVal, 22);

        const result = Buffer.concat([
          fileBuf.subarray(0, pageStart),
          newPage,
          afterPage,
        ]);

        await writeFile(filePath, result);
        return;
      }
    }

    offset = pageDataEnd;
  }

  // If no Vorbis comment page found, write unchanged
  await writeFile(filePath, fileBuf);
}

/** Parse FLAC metadata block layout from a buffer. */
function findFlacMarker(buf: Buffer): number {
  return buf.indexOf("fLaC");
}

function parseFlacLayout(buf: Buffer): { blocks: FlacBlockInfo[]; audioOffset: number; flacOffset: number } {
  const blocks: FlacBlockInfo[] = [];
  
  // Find fLaC marker (some files have ID3v2 tag prepended)
  const flacOffset = findFlacMarker(buf);
  let offset = flacOffset >= 0 ? flacOffset + 4 : 4; // skip "fLaC"
  
  while (offset + 4 <= buf.length) {
    const isLast = !!(buf[offset] >> 7);
    const type = buf[offset] & 0x7f;
    const length =
      (buf[offset + 1] << 16) |
      (buf[offset + 2] << 8) |
      buf[offset + 3];
    const headerOffset = offset;
    const dataOffset = offset + 4;

    if (type > 6 || length > 20_000_000 || dataOffset + length > buf.length) break;

    blocks.push({ type, headerOffset, dataOffset, length, isLast });
    if (isLast) break;
    offset = dataOffset + length;
  }

  const last = blocks[blocks.length - 1];
  const audioOffset = last ? last.dataOffset + last.length : offset;
  return { blocks, audioOffset, flacOffset };
}

function buildFlacBlockHeader(type: number, dataLength: number, isLast: boolean): Buffer {
  const h = Buffer.alloc(4);
  h[0] = (isLast ? 0x80 : 0x00) | (type & 0x7f);
  h[1] = (dataLength >> 16) & 0xff;
  h[2] = (dataLength >> 8) & 0xff;
  h[3] = dataLength & 0xff;
  return h;
}

/**
 * Neutralize ghost Vorbis Comment blocks embedded in the FLAC audio data.
 *
 * Some files contain a second Vorbis Comment block after the audio data
 * starts — typically from a previous tagging session that wrote a VC
 * (including METADATA_BLOCK_PICTURE) at a wrong offset. music-metadata
 * reads both VCs and merges them, causing stale values to override the
 * correct ones in the FLAC metadata chain.
 *
 * This function finds such ghost VCs by searching for the vendor string
 * "auto-tagger" in the audio data area, then zeroes the vendor-length
 * field so the parser fails to parse the block and skips it.
 *
 * Returns the (possibly modified) buffer.
 */
function neutralizeGhostVorbisComments(buf: Buffer): { buf: Buffer; found: boolean } {
  const layout = parseFlacLayout(buf);
  const audioStart = layout.audioOffset;
  if (audioStart <= 0 || audioStart >= buf.length) return { buf, found: false };

  const vendorBuf = Buffer.from("auto-tagger", "utf8");
  let result = buf;
  let searchFrom = audioStart;
  let found = false;

  while (searchFrom < buf.length) {
    const vendorPos = result.indexOf(vendorBuf, searchFrom);
    if (vendorPos < 0) break;

    // Ghost VC header: 4-byte vendor-length + vendor string
    // Verify the length field matches before zeroing
    if (vendorPos < 4) { searchFrom = vendorPos + 1; continue; }
    const claimedLen = result.readUInt32LE(vendorPos - 4);
    if (claimedLen !== vendorBuf.length) { searchFrom = vendorPos + 1; continue; }

    // Zero the vendor-length field so the parser fails to read the block
    if (result === buf) result = Buffer.from(buf); // copy-on-write
    result.writeUInt32LE(0, vendorPos - 4);
    found = true;
    searchFrom = vendorPos + 1;
  }
  return { buf: result, found };
}

/** Compare Vorbis comment payloads for semantic equality (ignoring vendor string order). */
function isVorbisUnchanged(
  buf: Buffer,
  block: FlacBlockInfo,
  newPayload: Buffer,
): boolean {
  const oldComments = parseVorbisCommentBlock(buf, block.dataOffset, block.length);
  const newComments = parseVorbisCommentBlock(newPayload, 0, newPayload.length);

  const oldEntries: Array<{ k: string; v: string }> = [];
  for (const [k, vals] of Object.entries(oldComments)) {
    for (const v of vals) oldEntries.push({ k, v });
  }
  const newEntries: Array<{ k: string; v: string }> = [];
  for (const [k, vals] of Object.entries(newComments)) {
    for (const v of vals) newEntries.push({ k, v });
  }

  if (oldEntries.length !== newEntries.length) return false;
  oldEntries.sort((a, b) => a.k.localeCompare(b.k) || a.v.localeCompare(b.v));
  newEntries.sort((a, b) => a.k.localeCompare(b.k) || a.v.localeCompare(b.v));
  for (let i = 0; i < oldEntries.length; i++) {
    if (oldEntries[i].k !== newEntries[i].k || oldEntries[i].v !== newEntries[i].v) return false;
  }
  return true;
}

/**
 * Replace or append a FLAC metadata block.
 */
async function writeFlacMetadataBlock(
  filePath: string,
  buf: Buffer,
  blockType: number,
  blockData: Buffer
): Promise<WriteOutcome> {
  // Safety guard: abort if file doesn't contain "fLaC" marker.
  // Some FLAC files have ID3v2 tags prepended (non-standard but common).
  const flacOffset = buf.indexOf("fLaC");
  if (buf.length < 4 || flacOffset < 0) {
    throw new Error(
      `Cannot write FLAC metadata: file does not contain fLaC marker (${filePath})`,
    );
  }

  // For non-Vorbis blocks, use the simple full-rewrite path
  if (blockType !== 4) {
    await writeFlacNonVorbisBlock(filePath, buf, blockType, blockData);
    return "full_rewrite";
  }

  const layout = parseFlacLayout(buf);
  const existing = layout.blocks.find((b) => b.type === 4);

  // No existing Vorbis block — do a full rewrite
  if (!existing) {
    await writeFlacWithPaddingFallback(filePath, buf, layout, blockData);
    return "full_rewrite";
  }

  // Ghost VCs from prior writes may have created multiple Vorbis Comment
  // blocks in the chain. In-place writes can't remove blocks, so force
  // a full rewrite that strips them via stripTrailingVorbisBlocks.
  const vcCount = layout.blocks.filter((b) => b.type === 4).length;
  if (vcCount > 1) {
    await writeFlacWithPaddingFallback(filePath, buf, layout, blockData);
    return "full_rewrite";
  }

  // Skip entirely if comments are unchanged
  if (isVorbisUnchanged(buf, existing, blockData)) {
    return "skipped";
  }

  // Fast path 1: new payload fits inside existing block space.
  // When the payload shrinks, we MUST update the header length and convert
  // leftover bytes to a valid PADDING block — leaving stale zeros inside a
  // Vorbis comment block causes flac -t BAD_METADATA errors.
  if (blockData.length <= existing.length) {
    const leftover = existing.length - blockData.length;

    // If leftover is 1–3 bytes, we cannot fit a PADDING header — fall through
    // to the full-rewrite path which rebuilds the block chain cleanly.
    if (leftover === 0 || leftover >= 4) {
      const handle = await open(filePath, "r+");
      try {
        await handle.write(blockData, 0, blockData.length, existing.dataOffset);
        if (leftover >= 4) {
          // Convert leftover space to a PADDING block inheriting isLast
          const padHdr = buildFlacBlockHeader(1, leftover - 4, existing.isLast);
          await handle.write(
            padHdr, 0, 4, existing.dataOffset + blockData.length,
          );
          // Update Vorbis header: correct length, isLast = false
          const vorbisHdr = buildFlacBlockHeader(
            4, blockData.length, false,
          );
          await handle.write(vorbisHdr, 0, 4, existing.headerOffset);
        }
      } finally {
        await handle.close();
      }
      return "in_place";
    }
  }

  // Fast path 2: next block is PADDING with enough combined room
  const existingIdx = layout.blocks.indexOf(existing);
  const nextBlock = layout.blocks[existingIdx + 1];
  const neededExtra = blockData.length - existing.length;

  if (nextBlock && nextBlock.type === 1 && neededExtra > 0 && nextBlock.length >= neededExtra) {
    // nextBlock.length is PADDING *data* bytes (header excluded).
    // After consuming neededExtra bytes from that data, the remaining
    // PADDING data bytes are exactly nextBlock.length - neededExtra.
    // Do not subtract 4 again when declaring the new PADDING block length,
    // or a 4-byte metadata/audio gap is created.
    const remainingPaddingData = nextBlock.length - neededExtra;
    // The PADDING header shifts forward by neededExtra bytes.
    const newPadHdrOffset = nextBlock.headerOffset + neededExtra;
    // PADDING block still exists (possibly 0-byte body), so Vorbis is not last.
    const vorbisHdr = buildFlacBlockHeader(4, blockData.length, false);
    const handle = await open(filePath, "r+");
    try {
      // Update Vorbis block header
      await handle.write(vorbisHdr, 0, 4, existing.headerOffset);
      // Write new payload (extends into old padding area)
      await handle.write(blockData, 0, blockData.length, existing.dataOffset);
      // Rewrite PADDING header at its shifted position.
      const padHdr = buildFlacBlockHeader(1, remainingPaddingData, nextBlock.isLast);
      await handle.write(padHdr, 0, 4, newPadHdrOffset);
    } finally {
      await handle.close();
    }
    return "in_place";
  }

  // Fallback: full rewrite with 64 KiB PADDING
  await writeFlacWithPaddingFallback(filePath, buf, layout, blockData);
  return "full_rewrite";
}

/**
 * Ensure only one FLAC metadata block has isLast=true (the very last one).
 */
function fixLastFlacBlock(buf: Buffer): void {
  const flacOffset = findFlacMarker(buf);
  if (flacOffset < 0) return;

  let offset = flacOffset + 4;
  let lastMetadataBlockOffset = -1;

  while (offset + 4 <= buf.length) {
    const type = buf[offset] & 0x7f;
    const length =
      (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3];

    const isLast = buf[offset] >> 7;
    const blockStart = offset;
    const nextOffset = offset + 4 + length;
    if (type > 6 || nextOffset > buf.length || length > 20_000_000) break;

    buf[blockStart] = type & 0x7f;
    lastMetadataBlockOffset = blockStart;
    offset = nextOffset;

    if (isLast || offset >= buf.length) break;
  }

  if (lastMetadataBlockOffset >= 0) {
    buf[lastMetadataBlockOffset] |= 0x80;
  }
}

/** Full rewrite of a non-Vorbis FLAC block (keeps existing behavior). */
async function writeFlacNonVorbisBlock(
  filePath: string,
  buf: Buffer,
  blockType: number,
  blockData: Buffer,
): Promise<void> {
  const flacOffset = findFlacMarker(buf);
  if (flacOffset < 0) {
    throw new Error(
      `Cannot write FLAC metadata: file does not contain fLaC marker (${filePath})`,
    );
  }

  const header = buildFlacBlockHeader(blockType, blockData.length, false);
  let offset = flacOffset + 4;
  let found = false;

  while (offset + 4 <= buf.length) {
    const isLastBlock = buf[offset] >> 7;
    const type = buf[offset] & 0x7f;
    const length =
      (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3];
    const blockStart = offset;
    offset += 4;

    if (type === blockType) {
      const before = buf.subarray(0, blockStart);
      const after = buf.subarray(blockStart + 4 + length);
      const newHeader = Buffer.from(header);
      if (isLastBlock) newHeader[0] |= 0x80;
      const result = Buffer.concat([before, newHeader, blockData, after]);
      fixLastFlacBlock(result);
      await writeFile(filePath, result);
      found = true;
      break;
    }

    if (isLastBlock) break;
    if (length === 0) break;
    offset += length;
  }

  if (!found) {
    const streamInfoHeaderOffset = flacOffset + 4;
    const streamInfoLen =
      (buf[streamInfoHeaderOffset + 1] << 16) |
      (buf[streamInfoHeaderOffset + 2] << 8) |
      buf[streamInfoHeaderOffset + 3];
    const insOffset = streamInfoHeaderOffset + 4 + streamInfoLen;
    const before = buf.subarray(0, insOffset);
    const after = buf.subarray(insOffset);
    const newHeader = Buffer.from(header);
    newHeader[0] = blockType & 0x7f;
    const result = Buffer.concat([before, newHeader, blockData, after]);
    const resultFlacOffset = findFlacMarker(result);
    if (resultFlacOffset >= 0) result[resultFlacOffset + 4] &= 0x7f;
    fixLastFlacBlock(result);
    await writeFile(filePath, result);
  }
}

/**
 * Strip any Vorbis Comment blocks (type=4) from a FLAC metadata segment.
 * The middle segment sits between the primary VC and audio data; any
 * type=4 block here is a ghost from a prior write.
 */
function stripTrailingVorbisBlocks(segment: Buffer): Buffer {
  for (let offset = 0; offset + 4 <= segment.length; ) {
    const byte0 = segment[offset];
    const isLast = !!(byte0 >> 7);
    const type = byte0 & 0x7f;
    const length =
      (segment[offset + 1] << 16) |
      (segment[offset + 2] << 8) |
      segment[offset + 3];

    if (type > 6 || length > 20_000_000 || offset + 4 + length > segment.length) break;
    if (type === 4) return segment.subarray(0, offset);
    if (isLast) break;
    offset += 4 + length;
  }
  return segment;
}

function normalizeFlacRewriteMiddle(segment: Buffer): Buffer {
  const blocks: Buffer[] = [];

  for (let offset = 0; offset + 4 <= segment.length; ) {
    const byte0 = segment[offset];
    const isLast = !!(byte0 >> 7);
    const type = byte0 & 0x7f;
    const length =
      (segment[offset + 1] << 16) |
      (segment[offset + 2] << 8) |
      segment[offset + 3];
    const nextOffset = offset + 4 + length;

    if (type > 6 || length > 20_000_000 || nextOffset > segment.length) break;
    if (type !== 1 && type !== 4) {
      const block = Buffer.from(segment.subarray(offset, nextOffset));
      block[0] &= 0x7f;
      blocks.push(block);
    }
    offset = nextOffset;
    if (isLast) break;
  }

  return Buffer.concat(blocks);
}

/** Fallback: replace Vorbis block, preserve non-Vorbis metadata, and add 64 KiB PADDING before audio. */
async function writeFlacWithPaddingFallback(
  filePath: string,
  buf: Buffer,
  layout: { blocks: FlacBlockInfo[]; audioOffset: number; flacOffset: number },
  newBlockData: Buffer,
): Promise<void> {
  const PADDING_SIZE = 65536;

  // Build new Vorbis block (not last — padding will follow)
  const newVorbis = Buffer.concat([
    buildFlacBlockHeader(4, newBlockData.length, false),
    newBlockData,
  ]);

  // Build PADDING block (isLast=true so fixLastFlacBlock doesn't scan past it
  // into audio data — which would interpret audio bytes as metadata blocks)
  const padding = Buffer.concat([
    buildFlacBlockHeader(1, PADDING_SIZE, true),
    Buffer.alloc(PADDING_SIZE),
  ]);

  const vorbisBlock = layout.blocks.find((b) => b.type === 4);
  const audio = buf.subarray(layout.audioOffset);

  // Build: [prefix] + newVorbis + [middle] + padding + audio
  let prefix: Buffer;
  let middle: Buffer;
  if (vorbisBlock) {
    // Replace existing Vorbis, keep other metadata blocks in order
    prefix = buf.subarray(0, vorbisBlock.headerOffset);
    middle = buf.subarray(
      vorbisBlock.headerOffset + 4 + vorbisBlock.length,
      layout.audioOffset,
    );
  } else {
    // No existing Vorbis — insert after STREAMINFO
    const streamInfoEnd = layout.flacOffset + 4 + 4 + (layout.blocks[0]?.length ?? 0);
    prefix = buf.subarray(0, streamInfoEnd);
    middle = buf.subarray(streamInfoEnd, layout.audioOffset);
  }

  // Drop old padding and ghost Vorbis blocks from the preserved middle.
  // The fresh padding below becomes the only final metadata block.
  middle = normalizeFlacRewriteMiddle(stripTrailingVorbisBlocks(middle));

  const result = Buffer.concat([prefix, newVorbis, middle, padding, audio]);
  const resultFlacOffset = findFlacMarker(result);
  if (resultFlacOffset >= 0) result[resultFlacOffset + 4] &= 0x7f; // clear isLast on STREAMINFO
  fixLastFlacBlock(result);
  await writeFile(filePath, result);
}

/**
 * Write tags to an M4A/MP4 file by manipulating the moov.udta.meta.ilst atom.
 * (Simplified: replaces the entire metadata if present, or appends a minimal one.)
 */
async function writeMp4(filePath: string, fields: WriteFields): Promise<void> {
  // For MVP: Read → parse existing tags → create a new moov.udta with metadata
  // and write back. This is a placeholder that re-writes the file with
  // a best-effort metadata atom.
  //
  // MP4 metadata atoms are complex. For now we use a simpler approach:
  // write to a companion JSON sidecar (like Picard) — no.
  // Instead, we write minimal iTunes atoms.
  await writeMinimalMp4Tags(filePath, fields);
}

/**
 * Write iTunes-compatible tags to M4A using minimal atom manipulation.
 * Handles: ©nam, ©ART, ©alb, ©day, ©gen, ©wrt, ©lyr, aART
 */
async function writeMinimalMp4Tags(filePath: string, fields: WriteFields): Promise<void> {
  const buf = await readFile(filePath);

  // Build metadata atoms
  const atoms: Buffer[] = [];

  // Helper: create a data atom
  const makeData = (value: string, typeCode = 1): Buffer => {
    const utf8 = Buffer.from(value, "utf8");
    const dataHeader = Buffer.alloc(8);
    dataHeader.writeUInt32BE(utf8.length + 8 + 8); // size of data atom
    dataHeader.write("data", 4, 4, "ascii");
    const dataType = Buffer.alloc(4);
    dataType.writeUInt32BE(typeCode); // 1 = UTF-8 text
    const locale = Buffer.alloc(4);
    locale.writeUInt32BE(0);
    return Buffer.concat([dataHeader, dataType, locale, utf8]);
  };

  const makeAtom = (fourcc: string, value: string): Buffer => {
    const data = makeData(value);
    const atom = Buffer.alloc(8);
    atom.writeUInt32BE(data.length + 8);
    atom.write(fourcc, 4, 4, "ascii");
    return Buffer.concat([atom, data]);
  };

  if (fields.title !== undefined) atoms.push(makeAtom("\xa9nam", fields.title ?? ""));
  if (fields.artist !== undefined) atoms.push(makeAtom("\xa9ART", fields.artist ?? ""));
  if (fields.album !== undefined) atoms.push(makeAtom("\xa9alb", fields.album ?? ""));
  if (fields.albumArtist !== undefined) atoms.push(makeAtom("aART", fields.albumArtist ?? ""));
  if (fields.year !== undefined) atoms.push(makeAtom("\xa9day", fields.year ?? ""));
  if (fields.genre !== undefined) atoms.push(makeAtom("\xa9gen", fields.genre ?? ""));
  if (fields.composer !== undefined) atoms.push(makeAtom("\xa9wrt", fields.composer ?? ""));
  if (fields.comment !== undefined) atoms.push(makeAtom("\xa9cmt", fields.comment ?? ""));
  if (fields.lyrics !== undefined) atoms.push(makeAtom("\xa9lyr", fields.lyrics ?? ""));
  if (fields.coverData !== undefined && fields.coverData) atoms.push(makeCoverAtom(fields.coverData));

  // Find existing moov.udta.meta.ilst or moov.udta and replace
  const ilstAtom = Buffer.concat(atoms);
  const ilstHeader = Buffer.alloc(8);
  ilstHeader.writeUInt32BE(ilstAtom.length + 8);
  ilstHeader.write("ilst", 4, 4, "ascii");
  const fullIlst = Buffer.concat([ilstHeader, ilstAtom]);

  const metaHeader = Buffer.alloc(8);
  metaHeader.writeUInt32BE(fullIlst.length + 12 + 4); // size
  metaHeader.write("meta", 4, 4, "ascii");
  const metaVersion = Buffer.from([0x00, 0x00, 0x00, 0x00]); // version + flags
  const metaAtom = Buffer.concat([metaHeader, metaVersion, fullIlst]);

  // Simple approach: append moov.udta.meta.ilst if no existing metadata
  // For MVP, write back as-is (best effort)
  try {
    const result = replaceOrAppendAtom(buf, "moov", [Buffer.from("\xa9nam"), Buffer.from("ilst")], metaAtom);
    await writeFile(filePath, result);
  } catch {
    // If atom replacement fails, write unmodified
    await writeFile(filePath, buf);
  }
}

/**
 * Find and replace a nested set of atoms, or append if not found.
 */
function replaceOrAppendAtom(
  buf: Buffer,
  _parentName: string,
  _path: Buffer[],
  newAtom: Buffer
): Buffer {
  // For MVP: simple append approach
  // Check for existing 'meta' atom in moov.udta
  for (let i = 0; i < buf.length - 8; i++) {
    if (
      buf[i] === 0x6d && // 'm'
      buf[i + 1] === 0x65 && // 'e'
      buf[i + 2] === 0x74 && // 't'
      buf[i + 3] === 0x61 // 'a'
    ) {
      // Found 'meta' - replace everything from here
      const before = buf.subarray(0, i - 4); // before the size field
      const atomSize = buf.readUInt32BE(i - 4);
      const after = buf.subarray(i - 4 + atomSize);
      return Buffer.concat([before, newAtom, after]);
    }
  }
  // Not found — append to end (breaks moov structure, but MVP)
  return Buffer.concat([buf, newAtom]);
}

/**
 * Write a WAV file's embedded ID3v2 chunk.
 * Picard/Foobar-style WAV tags use an `id3 ` RIFF chunk, which supports Unicode.
 */
async function writeWav(
  filePath: string,
  fields: WriteFields,
): Promise<WriteOutcome> {
  const data = await readFile(filePath);
  if (data.length < 12 || data.toString("ascii", 0, 4) !== "RIFF" || data.toString("ascii", 8, 12) !== "WAVE") {
    throw new Error("Invalid WAV file");
  }

  let existingTags: NodeID3.Tags = {};
  let existingId3Offset = -1;
  let existingId3Size = 0;
  let hasListInfoChunk = false;
  const chunks: Buffer[] = [];

  for (let offset = 12; offset + 8 <= data.length;) {
    const id = data.toString("ascii", offset, offset + 4);
    const size = data.readUInt32LE(offset + 4);
    const end = offset + 8 + size + (size % 2);
    if (end > data.length || end < offset + 8) break;
    if (id === "id3 " || id === "ID3 ") {
      existingTags = {
        ...existingTags,
        ...nodeId3.read(data.subarray(offset + 8, offset + 8 + size)),
      };
      existingId3Offset = offset;
      existingId3Size = size;
    } else if (id === "LIST" && size >= 4 && data.toString("ascii", offset + 8, offset + 12) === "INFO") {
      // Strip legacy LIST INFO chunk (corrupt/mangled metadata)
      hasListInfoChunk = true;
    } else {
      chunks.push(data.subarray(offset, end));
    }
    offset = end;
  }

  const id3Payload = buildWavId3Payload(fields, existingTags);

  // Try in-place: existing chunk has enough room (including potential pad byte)
  // Skip in-place when LIST INFO needs stripping (forces full rewrite)
  if (!hasListInfoChunk && existingId3Offset >= 0 && id3Payload.length <= existingId3Size) {
    const dataOffset = existingId3Offset + 8;
    const existingPayload = data.subarray(dataOffset, dataOffset + existingId3Size);
    if (isWavId3ChunkUnchanged(existingPayload, id3Payload)) {
      return "skipped";
    }

    const handle = await open(filePath, "r+");
    try {
      await handle.write(id3Payload, 0, id3Payload.length, dataOffset);
      if (id3Payload.length < existingId3Size) {
        const zeroBuf = Buffer.alloc(existingId3Size - id3Payload.length);
        await handle.write(zeroBuf, 0, zeroBuf.length, dataOffset + id3Payload.length);
      }
    } finally {
      await handle.close();
    }
    return "in_place";
  }

  // Full rewrite
  const id3Chunk = wavChunk("id3 ", id3Payload);
  chunks.push(id3Chunk);

  const body = Buffer.concat([Buffer.from("WAVE", "ascii"), ...chunks]);
  const header = Buffer.alloc(8);
  header.write("RIFF", 0, 4, "ascii");
  header.writeUInt32LE(body.length, 4);
  await writeFile(filePath, Buffer.concat([header, body]));
  return "full_rewrite";
}

function buildWavId3Payload(fields: WriteFields, existingTags: NodeID3.Tags = {}): Buffer {
  const tags = fieldsToID3v2(fields);
  const custom = mergeMp3UserDefinedText(existingTags.userDefinedText, fields);
  const mergedTags = { ...existingTags, ...tags };
  if (custom !== undefined) {
    mergedTags.userDefinedText = custom;
  } else {
    delete mergedTags.userDefinedText;
  }
  return nodeId3.create(mergedTags);
}

function wavChunk(id: string, payload: Buffer): Buffer {
  const header = Buffer.alloc(8);
  header.write(id, 0, 4, "ascii");
  header.writeUInt32LE(payload.length, 4);
  const pad = payload.length % 2 === 1 ? Buffer.from([0]) : Buffer.alloc(0);
  return Buffer.concat([header, payload, pad]);
}

function isWavId3ChunkUnchanged(existingChunk: Buffer, nextPayload: Buffer): boolean {
  if (nextPayload.length > existingChunk.length) return false;
  if (!existingChunk.subarray(0, nextPayload.length).equals(nextPayload)) return false;

  for (let i = nextPayload.length; i < existingChunk.length; i++) {
    if (existingChunk[i] !== 0x00) return false;
  }

  return true;
}

function normalizeExtraTags(
  extraTags: ExtraTagUpdate[],
  allowedReservedKeys: Set<string> = new Set(),
): ExtraTagUpdate[] {
  const result: ExtraTagUpdate[] = [];
  const seen = new Set<string>();

  for (const tag of extraTags) {
    const key = tag.key.trim();
    const value = tag.value.trim();
    const normalizedKey = key.toUpperCase();
    if (!key || !value) continue;
    if (isReservedExtraTagKey(normalizedKey) && !allowedReservedKeys.has(normalizedKey)) {
      continue;
    }

    const identity = `${normalizedKey}\0${value}`;
    if (seen.has(identity)) continue;
    seen.add(identity);
    result.push({ key, value });
  }

  return result;
}

function isReservedExtraTagKey(key: string): boolean {
  return STANDARD_VORBIS_TAGS.has(key.trim().toUpperCase());
}

async function writeVorbisExtraTags(filePath: string, extraTags: ExtraTagUpdate[]): Promise<WriteOutcome> {
  const data = await readFile(filePath);
  const comments = readVorbisComments(data);

  for (const key of Object.keys(comments)) {
    const normalizedKey = key.toUpperCase();
    if (!STANDARD_VORBIS_TAGS.has(normalizedKey) || EXTRA_TAG_RESERVED_EXCEPTIONS.has(normalizedKey)) {
      delete comments[key];
    }
  }

  for (const tag of normalizeExtraTags(extraTags, EXTRA_TAG_RESERVED_EXCEPTIONS)) {
    const key = tag.key.toUpperCase();
    comments[key] ??= [];
    comments[key].push(tag.value);
  }

  return await writeVorbisComments(filePath, data, comments);
}

// ── APEv2 tag writer (Monkey's Audio .ape) ────────────────────────────

async function replaceFileAtomically(filePath: string, data: Buffer): Promise<void> {
  const tempPath = path.join(
    path.dirname(filePath),
    `.${path.basename(filePath)}.${process.pid}.${Date.now()}.${randomUUID()}.tmp`,
  );
  let handle: Awaited<ReturnType<typeof open>> | null = null;

  try {
    handle = await open(tempPath, "wx");
    await handle.writeFile(data);
    await handle.sync();
    await handle.close();
    handle = null;
    await rename(tempPath, filePath);
  } catch (error) {
    if (handle) await handle.close().catch(() => undefined);
    await unlink(tempPath).catch(() => undefined);
    throw error;
  }
}

/**
 * Write APEv2 tags to a Monkey's Audio (.ape) file.
 *
 * Follows the same merge pattern as the Vorbis writer:
 * 1. Read existing APEv2 items from the file
 * 2. Start with those as defaults
 * 3. Overlay fields from `WriteFields` (undefined = keep, null = delete, string = set)
 * 4. Write the merged result
 */
async function writeApe(filePath: string, fields: WriteFields): Promise<void> {
  const data = await readFile(filePath);

  // 1. Read existing items into a key→values map (supports duplicate keys)
  const existing = parseApeTagItems(data);
  const merged = new Map<string, string[]>();
  for (const { key, value } of existing) {
    const upper = key.toUpperCase();
    if (!merged.has(upper)) merged.set(upper, []);
    merged.get(upper)!.push(value);
  }

  // 2. Helper: set or delete a single-value field
  const setField = (key: string, value: string | null | undefined) => {
    if (value === undefined) return; // keep existing
    if (value == null || value === "") {
      merged.delete(key); // delete
    } else {
      merged.set(key, [value]); // replace
    }
  };

  // 3. Helper: replace a multi-value field
  const setList = (key: string, values: string[] | string | null | undefined) => {
    if (values === undefined) return; // keep existing
    const list = normalizeListValue(values);
    if (list.length === 0) {
      merged.delete(key);
    } else {
      merged.set(key, list);
    }
  };

  // 4. Apply fields on top of existing
  setField("TITLE", fields.title);
  setField("ALBUM", fields.album);
  setField("ALBUM ARTIST", fields.albumArtist);
  setField("DATE", fields.year);
  setField("GENRE", fields.genre);
  setField("COMPOSER", fields.composer);
  setField("COMMENT", fields.comment);
  setField("DESCRIPTION", fields.description);
  setField("LYRICS", fields.lyrics);
  // Artist: merge primary + multi-value if either is provided
  if (fields.artist !== undefined || fields.artists !== undefined) {
    const mergedArtists = new Set<string>();
    if (fields.artist) mergedArtists.add(fields.artist);
    for (const v of normalizeListValue(fields.artists)) mergedArtists.add(v);
    if (mergedArtists.size > 0) {
      merged.set("ARTIST", [...mergedArtists]);
    } else {
      merged.delete("ARTIST");
    }
  }

  // Album Artist: merge primary + multi-value
  if (fields.albumArtist !== undefined || fields.albumArtists !== undefined) {
    const mergedAlbumArtists = new Set<string>();
    if (fields.albumArtist) mergedAlbumArtists.add(fields.albumArtist);
    for (const v of normalizeListValue(fields.albumArtists)) mergedAlbumArtists.add(v);
    if (mergedAlbumArtists.size > 0) {
      merged.set("ALBUM ARTIST", [...mergedAlbumArtists]);
    } else {
      merged.delete("ALBUM ARTIST");
    }
  }

  setField("COMPILATION", compilationToTag(fields.compilation));
  setField("MUSICBRAINZ_TRACKID", fields.musicbrainzTrackId);
  setField("MUSICBRAINZ_ALBUMID", fields.musicbrainzAlbumId);
  setField("MUSICBRAINZ_ARTISTID", fields.musicbrainzArtistId);
  setField("DISCOGS_ARTIST_ID", fields.discogsArtistId);
  setField("DISCOGS_RELEASE_ID", fields.discogsReleaseId);

  // Track / Disc (composite fields)
  const trackVal = fields.trackNumber != null
    ? (fields.trackTotal != null ? `${fields.trackNumber}/${fields.trackTotal}` : String(fields.trackNumber))
    : fields.track;
  setField("TRACK", trackVal);

  const discVal = fields.discNumber != null
    ? (fields.discTotal != null ? `${fields.discNumber}/${fields.discTotal}` : String(fields.discNumber))
    : fields.disc;
  setField("DISC", discVal);

  // 5. Flatten map back to entries
  const entries: Array<{ key: string; value: string }> = [];
  for (const [key, values] of merged) {
    for (const value of values) {
      entries.push({ key, value });
    }
  }

  if (entries.length === 0) {
    await replaceFileAtomically(filePath, stripApeTag(data));
    return;
  }

  const { items, count } = buildApeTagItems(entries);
  const tagSize = items.length + 32;
  const body = stripApeTag(data);
  await replaceFileAtomically(filePath, Buffer.concat([body, items, buildApeFooter(tagSize, count)]));
}

/**
 * Write extra tags to a Monkey's Audio (.ape) file using APEv2.
 * Preserves standard Vorbis-compatible tag keys, replaces non-standard ones
 * with the given extra tags (matching the Vorbis extra-tag behavior).
 */
async function writeApeExtraTags(filePath: string, extraTags: ExtraTagUpdate[]): Promise<void> {
  const data = await readFile(filePath);
  const existing = parseApeTagItems(data);

  // Preserve standard (reserved) keys; drop non-standard ones
  const kept = existing.filter(
    (t) => STANDARD_VORBIS_TAGS.has(t.key.toUpperCase()) && !EXTRA_TAG_RESERVED_EXCEPTIONS.has(t.key.toUpperCase()),
  );

  // Apply extra tag upserts
  const extraEntries: Array<{ key: string; value: string }> = [];
  for (const tag of normalizeExtraTags(extraTags, EXTRA_TAG_RESERVED_EXCEPTIONS)) {
    extraEntries.push({ key: tag.key.toUpperCase(), value: tag.value });
  }

  const finalEntries = kept.concat(extraEntries);

  if (finalEntries.length === 0) {
    await replaceFileAtomically(filePath, stripApeTag(data));
    return;
  }

  const { items, count } = buildApeTagItems(finalEntries);
  const tagSize = items.length + 32;
  const body = stripApeTag(data);
  await replaceFileAtomically(filePath, Buffer.concat([body, items, buildApeFooter(tagSize, count)]));
}

/**
 * Main entry point: detect format and write tags.
 */
export async function writeTags(
  filePath: string,
  fields: WriteFields
): Promise<void> {
  const ext = path.extname(filePath).toLowerCase();

  switch (ext) {
    case ".mp3":
      await writeMp3(filePath, fields);
      break;
    case ".flac":
      await writeVorbis(filePath, fields);
      break;
    case ".ogg":
    case ".opus":
      await writeVorbis(filePath, fields);
      break;
    case ".m4a":
    case ".mp4":
      await writeMp4(filePath, fields);
      break;
    case ".wav":
      await writeWav(filePath, fields);
      break;
    case ".aiff":
      throw new Error("AIFF metadata writing is not supported");
    case ".ape":
      await writeApe(filePath, fields);
      break;
    default:
      throw new Error(`Unsupported format for writing: ${ext}`);
  }
}

export async function writeExtraTags(
  filePath: string,
  extraTags: ExtraTagUpdate[]
): Promise<void> {
  const ext = path.extname(filePath).toLowerCase();

  switch (ext) {
    case ".mp3":
      await writeMp3ExtraTags(filePath, extraTags);
      break;
    case ".flac":
    case ".ogg":
    case ".opus":
      await writeVorbisExtraTags(filePath, extraTags);
      break;
    case ".wav":
      await writeWavExtraTags(filePath, extraTags);
      break;
    case ".ape":
      await writeApeExtraTags(filePath, extraTags);
      break;
    default:
      throw new Error(`Extra tag editing is not supported for ${ext || "this file type"}`);
  }
}

/**
 * Like writeTags but returns the internal WriteOutcome for queue/debug use.
 */
export async function writeTagsWithOutcome(
  filePath: string,
  fields: WriteFields
): Promise<WriteOutcome> {
  const ext = path.extname(filePath).toLowerCase();
  // Formats whose writers return an outcome: flac, ogg, opus, wav
  if (ext === ".flac" || ext === ".ogg" || ext === ".opus") {
    return await writeVorbis(filePath, fields);
  }
  if (ext === ".wav") {
    return await writeWav(filePath, fields);
  }
  // All others: delegate to writeTags (which throws for unsupported)
  await writeTags(filePath, fields);
  return "full_rewrite";
}

/**
 * Like writeExtraTags but returns the internal WriteOutcome for queue/debug use.
 */
export async function writeExtraTagsWithOutcome(
  filePath: string,
  extraTags: ExtraTagUpdate[]
): Promise<WriteOutcome> {
  const ext = path.extname(filePath).toLowerCase();
  // Formats whose writers return an outcome: flac, ogg, opus, wav
  if (ext === ".flac" || ext === ".ogg" || ext === ".opus") {
    return await writeVorbisExtraTags(filePath, extraTags);
  }
  if (ext === ".wav") {
    return await writeWavExtraTags(filePath, extraTags);
  }
  // All others: delegate to writeExtraTags (which throws for unsupported)
  await writeExtraTags(filePath, extraTags);
  return "full_rewrite";
}

/** Yields control back to the event loop so the renderer can repaint. */
function yieldToEventLoop(): Promise<void> {
  return new Promise<void>((resolve) => setImmediate(resolve));
}

/**
 * Concurrency lock: prevents duplicate / overlapping batch write operations.
 * Only covers batch writes (batchWriteTags / batchWriteExtraTags);
 * single-track writes are fast and unguarded.
 */
let batchWriteInProgress = false;

/**
 * Check whether a batch write operation is currently in progress.
 * Used by the main process to prevent app close during writes.
 */
export function isBatchWriteInProgress(): boolean {
  return batchWriteInProgress;
}

/**
 * Error thrown when a batch write is attempted while another is in progress.
 */
export class BatchWriteConflictError extends Error {
  constructor() {
    super("A batch tag write is already in progress. Please wait for it to complete.");
    this.name = "BatchWriteConflictError";
  }
}

/**
 * Run a batch operation under the write lock.
 * Throws BatchWriteConflictError if another batch is already running.
 */
async function runWithBatchLock(fn: () => Promise<void>): Promise<void> {
  if (batchWriteInProgress) {
    throw new BatchWriteConflictError();
  }
  batchWriteInProgress = true;
  try {
    await fn();
  } finally {
    batchWriteInProgress = false;
  }
}

export async function batchWriteExtraTags(
  updates: Array<{ path: string; tags: ExtraTagUpdate[] }>
): Promise<void> {
  await runWithBatchLock(async () => {
    for (const update of updates) {
      await writeExtraTags(update.path, update.tags);
      await yieldToEventLoop();
    }
  });
}

/**
 * Batch write tags to multiple files.
 */
export async function batchWriteTags(
  updates: Array<{ path: string; fields: WriteFields }>
): Promise<void> {
  await runWithBatchLock(async () => {
    for (const update of updates) {
      await writeTags(update.path, update.fields);
      await yieldToEventLoop();
    }
  });
}

function normalizeListValue(value: string[] | string | null | undefined): string[] {
  if (value == null) return [];
  const values = Array.isArray(value) ? value : value.split(/[;,]/);
  return values.map((item) => item.trim()).filter(Boolean);
}

/** Map a nullable boolean compilation field to an ID3/Vorbis tag value. */
function compilationToTag(value: boolean | null | undefined): string | null | undefined {
  if (value == null) return undefined;
  return value ? "1" : null;
}

/** Normalize T | T[] | null | undefined to T[]. */
function toArray<T>(value: T | T[] | null | undefined): T[] {
  if (Array.isArray(value)) return value;
  if (value != null) return [value];
  return [];
}

function formatPosition(current: number | null | undefined, total: number | null | undefined): string | null {
  if (current == null) return null;
  return total == null ? String(current) : `${current}/${total}`;
}

function buildFlacPictureBlock(data: Buffer, mimeType: string): Buffer {
  const mime = Buffer.from(mimeType, "utf8");
  const desc = Buffer.from("Cover", "utf8");
  const header = Buffer.alloc(32);
  let offset = 0;
  header.writeUInt32BE(3, offset); offset += 4;
  header.writeUInt32BE(mime.length, offset); offset += 4;
  const descLen = Buffer.alloc(4);
  descLen.writeUInt32BE(desc.length);
  const imageMeta = Buffer.alloc(20);
  imageMeta.writeUInt32BE(data.length, 16);
  return Buffer.concat([
    header.subarray(0, offset),
    mime,
    descLen,
    desc,
    imageMeta,
    data,
  ]);
}

function makeCoverAtom(data: Buffer): Buffer {
  const dataHeader = Buffer.alloc(8);
  dataHeader.writeUInt32BE(data.length + 16);
  dataHeader.write("data", 4, 4, "ascii");
  const dataType = Buffer.alloc(4);
  dataType.writeUInt32BE(13);
  const locale = Buffer.alloc(4);
  locale.writeUInt32BE(0);
  const dataAtom = Buffer.concat([dataHeader, dataType, locale, data]);
  const covrHeader = Buffer.alloc(8);
  covrHeader.writeUInt32BE(dataAtom.length + 8);
  covrHeader.write("covr", 4, 4, "ascii");
  return Buffer.concat([covrHeader, dataAtom]);
}
