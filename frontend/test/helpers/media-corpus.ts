/**
 * Shared tiny-media fixture generator for Electron↔Rust parity.
 *
 * The byte constructors are extracted from `writer.test.ts` and
 * `tracks.test.ts`; they deliberately make only the minimum container/audio
 * structures needed by the existing Electron reader/writer tests. Rust must
 * consume committed output from `generateMediaCorpus`, never independently
 * synthesize an "equivalent" corpus.
 */

import fs from "node:fs";
import path from "node:path";
import * as NodeID3 from "node-id3";

import { writeTags } from "../../electron/handlers/writer";
import { flacHeader, vorbisCommentBlock } from "./flac-helpers";

export const GENERATED_MEDIA_CORPUS_FILES = [
  "minimal.mp3",
  "minimal.flac",
  "minimal.wav",
  "minimal.ogg",
  "ape-id3v1-fallback.ape",
  "malformed-truncated.flac",
  "malformed-vorbis-length.flac",
] as const;

/** Real-encoder fixtures regenerated only by scripts/generate-tauri-media-corpus-extended.mjs. */
export const ENCODED_MEDIA_CORPUS_FILES = [
  "minimal.m4a",
  "minimal.mp4",
  "minimal.opus",
  "minimal.aiff",
] as const;

export const MEDIA_CORPUS_FILES = [
  ...GENERATED_MEDIA_CORPUS_FILES,
  ...ENCODED_MEDIA_CORPUS_FILES,
] as const;

/**
 * Create a minimal valid MP3 with ID3v2 tags and an MPEG sync frame so
 * music-metadata recognizes it as audio.
 */
export function createMinimalMp3(
  filePath: string,
  initialTags?: Record<string, string>,
): void {
  if (initialTags) {
    NodeID3.write(
      {
        title: initialTags.title,
        artist: initialTags.artist,
        album: initialTags.album,
        year: initialTags.year,
        genre: initialTags.genre,
        trackNumber: initialTags.trackNumber
          ? parseInt(initialTags.trackNumber, 10)
          : undefined,
      },
      filePath,
    );
  } else {
    NodeID3.write({}, filePath);
  }

  // MPEG1 Layer3, 128kbps / 44100Hz / stereo.
  const fd = fs.openSync(filePath, "a");
  const frame = Buffer.alloc(417);
  frame[0] = 0xff;
  frame[1] = 0xfb;
  frame[2] = (9 << 4) | (0 << 2);
  frame[3] = 0x02;
  fs.writeSync(fd, frame, 0, frame.length);
  fs.closeSync(fd);
}

/** Create a minimal FLAC with STREAMINFO and optional Vorbis comments. */
export function createMinimalFlac(
  filePath: string,
  title?: string,
  artist?: string,
  album?: string,
  extraComments: string[] = [],
): void {
  const parts: Buffer[] = [Buffer.from("fLaC", "ascii")];

  // STREAMINFO (metadata block type 0, size 34).
  const streamInfo = Buffer.alloc(34);
  streamInfo.writeUInt16BE(4096, 0);
  streamInfo.writeUInt16BE(4096, 2);
  // 44100Hz, stereo, 16-bit.
  streamInfo[12] = 0x00;
  streamInfo[13] = 0xac;
  streamInfo[14] = 0x44;
  streamInfo[15] = 0x02;
  streamInfo[16] = 0x1f;

  const streamInfoHeader = Buffer.alloc(4);
  const hasVorbis = Boolean(title || artist || album || extraComments.length);
  streamInfoHeader[0] = hasVorbis ? 0x00 : 0x80;
  streamInfoHeader[1] = (streamInfo.length >> 16) & 0xff;
  streamInfoHeader[2] = (streamInfo.length >> 8) & 0xff;
  streamInfoHeader[3] = streamInfo.length & 0xff;
  parts.push(streamInfoHeader, streamInfo);

  if (hasVorbis) {
    const comments: string[] = [];
    if (title) comments.push(`TITLE=${title}`);
    if (artist) comments.push(`ARTIST=${artist}`);
    if (album) comments.push(`ALBUM=${album}`);
    comments.push(...extraComments);

    const vendor = Buffer.from("libFLAC 1.3.2", "utf8");
    const vendorLength = Buffer.alloc(4);
    vendorLength.writeUInt32LE(vendor.length);
    const commentBuffers: Buffer[] = [];
    for (const comment of comments) {
      const raw = Buffer.from(comment, "utf8");
      const length = Buffer.alloc(4);
      length.writeUInt32LE(raw.length);
      commentBuffers.push(length, raw);
    }
    const commentCount = Buffer.alloc(4);
    commentCount.writeUInt32LE(comments.length);
    const vorbis = Buffer.concat([
      vendorLength,
      vendor,
      commentCount,
      ...commentBuffers,
    ]);
    const vorbisHeader = Buffer.alloc(4);
    vorbisHeader[0] = 0x80 | 0x04; // last | VORBIS_COMMENT
    vorbisHeader[1] = (vorbis.length >> 16) & 0xff;
    vorbisHeader[2] = (vorbis.length >> 8) & 0xff;
    vorbisHeader[3] = vorbis.length & 0xff;
    parts.push(vorbisHeader, vorbis);
  }

  fs.writeFileSync(filePath, Buffer.concat(parts));
}

export function findFlacOffset(buffer: Buffer): number {
  return buffer.indexOf(Buffer.from("fLaC", "ascii"));
}

export function readPrependedId3End(buffer: Buffer): number | null {
  if (buffer.subarray(0, 3).toString("ascii") !== "ID3") return null;
  return (
    10 +
    ((buffer[6] & 0x7f) << 21) +
    ((buffer[7] & 0x7f) << 14) +
    ((buffer[8] & 0x7f) << 7) +
    (buffer[9] & 0x7f)
  );
}

export function riffChunk(id: string, payload: Buffer): Buffer {
  const header = Buffer.alloc(8);
  header.write(id, 0, 4, "ascii");
  header.writeUInt32LE(payload.length, 4);
  const pad = payload.length % 2 === 1 ? Buffer.from([0]) : Buffer.alloc(0);
  return Buffer.concat([header, payload, pad]);
}

/** Return a minimal PCM WAV byte stream. */
export function minimalWavAudio(): Buffer {
  const format = Buffer.alloc(16);
  format.writeUInt16LE(1, 0); // PCM
  format.writeUInt16LE(1, 2); // mono
  format.writeUInt32LE(44100, 4);
  format.writeUInt32LE(88200, 8);
  format.writeUInt16LE(2, 12);
  format.writeUInt16LE(16, 14);

  const data = Buffer.alloc(882);
  const body = Buffer.concat([
    Buffer.from("WAVE", "ascii"),
    riffChunk("fmt ", format),
    riffChunk("data", data),
  ]);
  const header = Buffer.alloc(8);
  header.write("RIFF", 0, 4, "ascii");
  header.writeUInt32LE(body.length, 4);
  return Buffer.concat([header, body]);
}

export function createMinimalWav(filePath: string): void {
  fs.writeFileSync(filePath, minimalWavAudio());
}

/** Minimal Monkey's Audio container bytes used by the raw APEv2 fallback. */
export function minimalApeAudio(): Buffer {
  const descriptor = Buffer.alloc(52);
  descriptor.write("MAC ", 0, 4, "ascii");
  descriptor.writeUInt32LE(2_000_000, 4);
  descriptor.writeUInt32LE(52, 8);
  descriptor.writeUInt32LE(24, 12);
  descriptor.writeUInt32LE(0, 16);
  descriptor.writeUInt32LE(0, 20);
  descriptor.writeUInt32LE(4096, 24);
  descriptor.writeUInt32LE(0, 28);
  descriptor.writeUInt32LE(0, 32);

  const header = Buffer.alloc(24);
  header.writeUInt32LE(4608, 4);
  header.writeUInt32LE(0, 8);
  header.writeUInt32LE(1, 12);
  header.writeUInt16LE(16, 16);
  header.writeUInt16LE(2, 18);
  header.writeUInt32LE(44100, 20);

  return Buffer.concat([descriptor, header, Buffer.alloc(4096, 0x55)]);
}

/** A trailing ID3v1 block that makes music-metadata use the raw APE fallback. */
export function id3v1Tail(): Buffer {
  const id3 = Buffer.alloc(128, 0);
  id3.write("TAG", 0, 3, "ascii");
  return id3;
}

export function oggCrc32(buffer: Buffer): number {
  let crc = 0xffffffff;
  const table = new Int32Array(256);
  for (let i = 0; i < 256; i++) {
    let value = i;
    for (let j = 0; j < 8; j++) {
      value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
    }
    table[i] = value;
  }
  for (let i = 0; i < buffer.length; i++) {
    crc = table[(crc ^ buffer[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** Build one small OGG page with a single packet. */
export function buildOggPage(
  headerType: number,
  granulePosition: bigint,
  serial: number,
  sequence: number,
  packetData: Buffer,
): Buffer {
  const segmentTable = Buffer.from([packetData.length]);
  const header = Buffer.alloc(27);
  let offset = 0;
  header.write("OggS", offset, 4, "ascii");
  offset += 4;
  header[offset++] = 0;
  header[offset++] = headerType;
  header.writeBigUInt64LE(granulePosition, offset);
  offset += 8;
  header.writeUInt32LE(serial, offset);
  offset += 4;
  header.writeUInt32LE(sequence, offset);
  offset += 4;
  offset += 4; // CRC placeholder
  header[offset++] = segmentTable.length;

  const page = Buffer.concat([header, segmentTable, packetData]);
  page.writeUInt32LE(0, 22);
  page.writeUInt32LE(oggCrc32(page), 22);
  return page;
}

export function commentEntry(key: string, value: string): Buffer {
  const raw = Buffer.from(`${key}=${value}`, "utf8");
  const length = Buffer.alloc(4);
  length.writeUInt32LE(raw.length);
  return Buffer.concat([length, raw]);
}

/** Create a minimal OGG Vorbis file with identification + comment pages. */
export function createMinimalOgg(
  filePath: string,
  title?: string,
  artist?: string,
  album?: string,
): void {
  const serial = 0x12345678;
  const identification = Buffer.alloc(29);
  identification[0] = 1;
  identification.write("vorbis", 1, 6, "ascii");
  identification.writeUInt32LE(0, 7);
  identification[11] = 2;
  identification.writeUInt32LE(44100, 12);
  identification.writeUInt32LE(0, 16);
  identification.writeUInt32LE(160000, 20);
  identification.writeUInt32LE(0, 24);
  identification[28] = 0b00010000;
  const identificationPage = buildOggPage(2, 0n, serial, 0, identification);

  const vendor = Buffer.from("auto-tagger-test", "utf8");
  const comments: Buffer[] = [];
  if (title) comments.push(commentEntry("TITLE", title));
  if (artist) comments.push(commentEntry("ARTIST", artist));
  if (album) comments.push(commentEntry("ALBUM", album));

  let offset = 0;
  const commentHeader = Buffer.alloc(7 + 4 + vendor.length + 4);
  commentHeader[offset++] = 3;
  commentHeader.write("vorbis", offset, 6, "ascii");
  offset += 6;
  commentHeader.writeUInt32LE(vendor.length, offset);
  offset += 4;
  vendor.copy(commentHeader, offset);
  offset += vendor.length;
  commentHeader.writeUInt32LE(comments.length, offset);
  const commentPage = buildOggPage(
    0,
    0n,
    serial,
    1,
    Buffer.concat([commentHeader, ...comments, Buffer.from([1])]),
  );
  fs.writeFileSync(filePath, Buffer.concat([identificationPage, commentPage]));
}

/**
 * Generate the exact committed media corpus. The APE artifact is intentionally
 * written through the existing Electron writer, then given an ID3v1 trailer, to
 * force the same raw-APEv2 fallback path covered by tracks.test.ts.
 */
export async function generateMediaCorpus(root: string): Promise<void> {
  fs.mkdirSync(root, { recursive: true });
  const mp3Path = path.join(root, "minimal.mp3");
  createMinimalMp3(mp3Path);
  // Rewrite through the production Electron writer so the committed reader
  // corpus covers standard/repeated/custom/provider/artwork ID3 behavior—not
  // just an empty MPEG frame.
  await writeTags(mp3Path, {
    title: "Corpus MP3",
    artist: "Corpus Artist",
    artists: ["Corpus Artist", "Featured Artist"],
    album: "Corpus Album",
    albumArtist: "Corpus Album Artist",
    albumArtists: ["Corpus Album Artist", "Guest Album Artist"],
    year: "2024",
    trackNumber: 3,
    trackTotal: 12,
    discNumber: 1,
    discTotal: 2,
    genre: "Rock",
    composer: "Corpus Composer",
    comment: "Corpus Comment",
    description: "Corpus Description",
    lyrics: "Corpus Lyrics",
    compilation: true,
    musicbrainzTrackId: "corpus-mb-track",
    musicbrainzAlbumId: "corpus-mb-album",
    musicbrainzArtistId: "corpus-mb-artist",
    discogsArtistId: "12345",
    discogsReleaseId: "67890",
    coverData: Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9WlNq8sAAAAASUVORK5CYII=",
      "base64",
    ),
    coverMime: "image/png",
  });
  createMinimalFlac(
    path.join(root, "minimal.flac"),
    "Corpus FLAC",
    "Corpus Artist",
    "Corpus Album",
    ["MUSICBRAINZ_ALBUMID=corpus-mb-album"],
  );
  createMinimalWav(path.join(root, "minimal.wav"));
  createMinimalOgg(
    path.join(root, "minimal.ogg"),
    "Corpus OGG",
    "Corpus Artist",
    "Corpus Album",
  );

  const apePath = path.join(root, "ape-id3v1-fallback.ape");
  fs.writeFileSync(apePath, minimalApeAudio());
  await writeTags(apePath, {
    title: "Corpus APE",
    artist: "Corpus APE Artist",
    album: "Corpus APE Album",
    trackNumber: 1,
    genre: "Alternative Rock",
  });
  fs.appendFileSync(apePath, id3v1Tail());

  fs.writeFileSync(path.join(root, "malformed-truncated.flac"), Buffer.from("fLaC"));
  const malformedVorbis = Buffer.concat([
    flacHeader(false, [vorbisCommentBlock(["TITLE=Broken"], { corruptLen: 100 })]),
    Buffer.alloc(100),
  ]);
  fs.writeFileSync(path.join(root, "malformed-vorbis-length.flac"), malformedVorbis);
}
