import fs from "fs";
import path from "path";
import * as NodeID3 from "node-id3";

/**
 * Mapping of field names → tag specs for each format.
 *
 * Fields param follows a normalized schema:
 *   title, artist, album, year, track (string "1" or "1/10" format),
 *   genre, composer, comment, compilation, trackNumber (number),
 *   trackTotal (number), discNumber (number), discTotal (number)
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
  lyrics?: string | null;
  compilation?: boolean | null;
  musicbrainzTrackId?: string | null;
  musicbrainzAlbumId?: string | null;
  musicbrainzArtistId?: string | null;
  coverData?: Buffer | null;
  coverMime?: string | null;
}

export interface ExtraTagUpdate {
  key: string;
  value: string;
}

const STANDARD_VORBIS_TAGS = new Set([
  "TITLE",
  "ARTIST",
  "ARTISTS",
  "ALBUM",
  "ALBUMARTIST",
  "ALBUM ARTIST",
  "ALBUMARTISTS",
  "DATE",
  "YEAR",
  "GENRE",
  "COMPOSER",
  "COMMENT",
  "LYRICS",
  "UNSYNCEDLYRICS",
  "UNSYNCHRONISEDLYRICS",
  "TRACKNUMBER",
  "TRACKTOTAL",
  "TOTALTRACKS",
  "DISCNUMBER",
  "DISCTOTAL",
  "TOTALDISCS",
  "MUSICBRAINZ TRACK ID",
  "MUSICBRAINZ ALBUM ID",
  "MUSICBRAINZ ARTIST ID",
  "MUSICBRAINZ_TRACKID",
  "MUSICBRAINZ_ALBUMID",
  "MUSICBRAINZ_ARTISTID",
  "COMPILATION",
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
  if (fields.trackNumber !== undefined || fields.track !== undefined)
    tags.trackNumber = formatPosition(fields.trackNumber ?? (fields.track ? parseInt(fields.track) : undefined), fields.trackTotal) ?? undefined;
  if (fields.discNumber !== undefined || fields.disc !== undefined)
    tags.partOfSet = formatPosition(fields.discNumber ?? (fields.disc ? parseInt(fields.disc) : undefined), fields.discTotal) ?? undefined;
  if (fields.genre !== undefined) tags.genre = fields.genre ?? undefined;
  if (fields.composer !== undefined)
    tags.composer = fields.composer ?? undefined;
  if (fields.comment !== undefined)
    tags.comment = fields.comment
      ? { language: "eng", text: fields.comment }
      : undefined;
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
function writeMp3(filePath: string, fields: WriteFields): void {
  const tags = fieldsToID3v2(fields);
  const existingTags = NodeID3.read(filePath);
  const custom = mergeMp3UserDefinedText(existingTags.userDefinedText, fields);
  const mergedTags = { ...existingTags, ...tags };
  if (custom !== undefined) {
    mergedTags.userDefinedText = custom;
  } else {
    delete mergedTags.userDefinedText;
  }
  NodeID3.write(mergedTags, filePath);
}

function mergeMp3UserDefinedText(
  current: NodeID3.Tags["userDefinedText"],
  fields: WriteFields,
): NodeID3.Tags["userDefinedText"] {
  const rows = (Array.isArray(current) ? current : current ? [current] : []).filter(
    (row) => row.description,
  );
  const upserts: Array<{ description: string; value: string | string[] | null | undefined }> = [
    { description: "ARTISTS", value: normalizeListValue(fields.artists) },
    { description: "ALBUMARTISTS", value: normalizeListValue(fields.albumArtists) },
    { description: "MusicBrainz Track Id", value: fields.musicbrainzTrackId },
    { description: "MusicBrainz Album Id", value: fields.musicbrainzAlbumId },
    { description: "MusicBrainz Artist Id", value: fields.musicbrainzArtistId },
    { description: "COMPILATION", value: fields.compilation == null ? undefined : fields.compilation ? "1" : null },
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

function writeMp3ExtraTags(filePath: string, extraTags: ExtraTagUpdate[]): void {
  const existingTags = NodeID3.read(filePath);
  const preserved = (Array.isArray(existingTags.userDefinedText)
    ? existingTags.userDefinedText
    : existingTags.userDefinedText
      ? [existingTags.userDefinedText]
      : []
  ).filter(
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
  NodeID3.write(nextTags, filePath);
}

/**
 * Write Vorbis comments to a FLAC / OGG / OPUS file.
 * Vorbis comments are stored as FLAC metadata blocks or OGG page comments
 * as a series of KEY=VALUE strings (UTF-8), prefixed by a 32-bit count.
 */
function writeVorbis(
  filePath: string,
  fields: WriteFields,
  blockType: number = 4
): void {
  const data = fs.readFileSync(filePath);
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
  setVorbisField(updated, "TRACKNUMBER", fields.trackNumber ?? fields.track);
  setVorbisField(updated, "TRACKTOTAL", fields.trackTotal);
  setVorbisField(updated, "DISCNUMBER", fields.discNumber ?? fields.disc);
  setVorbisField(updated, "DISCTOTAL", fields.discTotal);
  setVorbisField(updated, "LYRICS", fields.lyrics);
  setVorbisField(updated, "MUSICBRAINZ_TRACKID", fields.musicbrainzTrackId);
  setVorbisField(updated, "MUSICBRAINZ_ALBUMID", fields.musicbrainzAlbumId);
  setVorbisField(updated, "MUSICBRAINZ_ARTISTID", fields.musicbrainzArtistId);
  setVorbisField(updated, "COMPILATION", fields.compilation == null ? undefined : fields.compilation ? "1" : null);
  if (fields.coverData) {
    updated.METADATA_BLOCK_PICTURE = [
      buildFlacPictureBlock(fields.coverData, fields.coverMime ?? "image/jpeg").toString("base64"),
    ];
  }

  writeVorbisComments(filePath, data, updated, blockType);
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
  let offset = 4; // skip "fLaC"
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
 * Write Vorbis comments into a FLAC/OGG file buffer.
 */
function writeVorbisComments(
  filePath: string,
  origBuf: Buffer,
  comments: VorbisDict,
  _blockType: number = 4
): void {
  // Build the comment block body
  const vendorString = Buffer.from("auto-tagger", "utf8");
  const vendorLen = Buffer.alloc(4);
  vendorLen.writeUInt32LE(vendorString.length);

  // Build comment entries
  const commentEntries: Buffer[] = [];
  for (const [key, values] of Object.entries(comments)) {
    for (const value of values) {
      const entry = Buffer.from(`${key}=${value}`, "utf8");
      const entryLen = Buffer.alloc(4);
      entryLen.writeUInt32LE(entry.length);
      commentEntries.push(entryLen, entry);
    }
  }

  const numComments = Buffer.alloc(4);
  numComments.writeUInt32LE(commentEntries.length / 2);

  const commentBlock = Buffer.concat([
    vendorLen,
    vendorString,
    numComments,
    ...commentEntries,
  ]);

  if (filePath.toLowerCase().endsWith(".flac")) {
    writeFlacMetadataBlock(filePath, origBuf, 4, commentBlock);
  } else if (
    filePath.toLowerCase().endsWith(".ogg") ||
    filePath.toLowerCase().endsWith(".opus")
  ) {
    writeOggVorbisComments(filePath, commentBlock);
  } else {
    // Unsupported Vorbis container — fall through
    fs.writeFileSync(filePath, origBuf);
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
function writeOggVorbisComments(filePath: string, commentBlock: Buffer): void {
  const fileBuf = fs.readFileSync(filePath);
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

        fs.writeFileSync(filePath, result);
        return;
      }
    }

    offset = pageDataEnd;
  }

  // If no Vorbis comment page found, write unchanged
  fs.writeFileSync(filePath, fileBuf);
}

/**
 * Replace or append a FLAC metadata block.
 */
function writeFlacMetadataBlock(
  filePath: string,
  _origBuf: Buffer,
  blockType: number,
  blockData: Buffer
): void {
  const buf = fs.readFileSync(filePath);

  // isLast flag is corrected in the replacement logic below
  const header = Buffer.alloc(4);
  header[0] = blockType & 0x7f;
  header[1] = (blockData.length >> 16) & 0xff;
  header[2] = (blockData.length >> 8) & 0xff;
  header[3] = blockData.length & 0xff;

  // Find existing VORBIS_COMMENT block and replace it
  let offset = 4; // skip "fLaC"
  let found = false;

  while (offset + 4 <= buf.length) {
    const isLastBlock = buf[offset] >> 7;
    const type = buf[offset] & 0x7f;
    const length =
      (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3];
    const blockStart = offset;
    offset += 4;

    if (type === blockType) {
      // Replace this block
      const before = buf.subarray(0, blockStart);
      const after = buf.subarray(blockStart + 4 + length);
      const newHeader = Buffer.from(header);
      if (isLastBlock) {
        newHeader[0] |= 0x80;
      } else {
        newHeader[0] = blockType & 0x7f;
      }
      const result = Buffer.concat([before, newHeader, blockData, after]);
      fixLastFlacBlock(result);
      fs.writeFileSync(filePath, result);
      found = true;
      break;
    }

    if (isLastBlock) break;
    if (length === 0) break;
    offset += length;
  }

  if (!found) {
    // Insert after STREAMINFO (block 0)
    let insOffset = 4;
    const streamInfoLen =
      (buf[5] << 16) | (buf[6] << 8) | buf[7];
    insOffset += 4 + streamInfoLen;

    const before = buf.subarray(0, insOffset);
    const after = buf.subarray(insOffset);
    const newHeader = Buffer.from(header);
    newHeader[0] = blockType & 0x7f;
    const result = Buffer.concat([before, newHeader, blockData, after]);
    result[4] &= 0x7f;
    fixLastFlacBlock(result);
    fs.writeFileSync(filePath, result);
  }
}

/**
 * Ensure only one FLAC metadata block has isLast=true (the very last one).
 */
function fixLastFlacBlock(buf: Buffer): void {
  let offset = 4;
  let lastMetadataBlockOffset = -1;

  while (offset + 4 <= buf.length) {
    const type = buf[offset] & 0x7f;
    const length =
      (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3];

    const isLast = buf[offset] >> 7;
    const blockStart = offset;
    const nextOffset = offset + 4 + length;
    if (type > 6 || nextOffset > buf.length || length > 5_000_000) break;

    buf[blockStart] = type & 0x7f;
    lastMetadataBlockOffset = blockStart;
    offset = nextOffset;

    if (isLast || offset >= buf.length) break;
  }

  if (lastMetadataBlockOffset >= 0) {
    buf[lastMetadataBlockOffset] |= 0x80;
  }
}

/**
 * Write tags to an M4A/MP4 file by manipulating the moov.udta.meta.ilst atom.
 * (Simplified: replaces the entire metadata if present, or appends a minimal one.)
 */
function writeMp4(filePath: string, fields: WriteFields): void {
  // For MVP: Read → parse existing tags → create a new moov.udta with metadata
  // and write back. This is a placeholder that re-writes the file with
  // a best-effort metadata atom.
  //
  // MP4 metadata atoms are complex. For now we use a simpler approach:
  // write to a companion JSON sidecar (like Picard) — no.
  // Instead, we write minimal iTunes atoms.
  writeMinimalMp4Tags(filePath, fields);
}

/**
 * Write iTunes-compatible tags to M4A using minimal atom manipulation.
 * Handles: ©nam, ©ART, ©alb, ©day, ©gen, ©wrt, ©lyr, aART
 */
function writeMinimalMp4Tags(filePath: string, fields: WriteFields): void {
  const buf = fs.readFileSync(filePath);

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
    fs.writeFileSync(filePath, result);
  } catch {
    // If atom replacement fails, write unmodified
    fs.writeFileSync(filePath, buf);
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
 * Write a WAV file's RIFF INFO chunk.
 * WAV tags go in a `LIST` chunk with `INFO` sub-chunks.
 */
function writeWav(_filePath: string, _fields: WriteFields): void {
  // WAV tag writing is non-standard; skip for MVP.
  // Most WAV files don't have tag editing support.
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

function writeVorbisExtraTags(filePath: string, extraTags: ExtraTagUpdate[]): void {
  const data = fs.readFileSync(filePath);
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

  writeVorbisComments(filePath, data, comments, 4);
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
      writeMp3(filePath, fields);
      break;
    case ".flac":
      writeVorbis(filePath, fields, 4);
      break;
    case ".ogg":
    case ".opus":
      writeVorbis(filePath, fields, 4);
      break;
    case ".m4a":
    case ".mp4":
      writeMp4(filePath, fields);
      break;
    case ".wav":
    case ".aiff":
      writeWav(filePath, fields);
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
      writeMp3ExtraTags(filePath, extraTags);
      break;
    case ".flac":
    case ".ogg":
    case ".opus":
      writeVorbisExtraTags(filePath, extraTags);
      break;
    default:
      throw new Error(`Extra tag editing is not supported for ${ext || "this file type"}`);
  }
}

export async function batchWriteExtraTags(
  updates: Array<{ path: string; tags: ExtraTagUpdate[] }>
): Promise<void> {
  for (const update of updates) {
    await writeExtraTags(update.path, update.tags);
  }
}

/**
 * Batch write tags to multiple files.
 */
export async function batchWriteTags(
  updates: Array<{ path: string; fields: WriteFields }>
): Promise<void> {
  for (const update of updates) {
    await writeTags(update.path, update.fields);
  }
}

function normalizeListValue(value: string[] | string | null | undefined): string[] {
  if (value == null) return [];
  const values = Array.isArray(value) ? value : value.split(/[;,]/);
  return values.map((item) => item.trim()).filter(Boolean);
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
