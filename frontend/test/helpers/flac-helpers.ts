// ── Synthetic FLAC fixture helpers ──────────────────────────────────
// Build valid minimal FLAC headers with VORBIS_COMMENT blocks for tests.
// No audio data is included — just enough for music-metadata to parse.

const STREAMINFO_LEN = 34;

/** Create a minimal FLAC header with a valid STREAMINFO block. */
export function flacHeader(
  siIsLast: boolean,
  extraBlocks: Buffer[] = []
): Buffer {
  const parts: Buffer[] = [];
  parts.push(Buffer.from("fLaC", "ascii"));

  const si = Buffer.alloc(STREAMINFO_LEN);
  si.writeUInt16BE(4096, 0);
  si.writeUInt16BE(4096, 2);
  si[12] = 0x00;
  si[13] = 0xac;
  si[14] = 0x44;
  si[15] = 0x02;
  si[16] = 0x1f;

  const siHeader = Buffer.alloc(4);
  siHeader[0] = siIsLast ? 0x80 : 0x00;
  siHeader[1] = (si.length >> 16) & 0xff;
  siHeader[2] = (si.length >> 8) & 0xff;
  siHeader[3] = si.length & 0xff;
  parts.push(siHeader, si);

  for (const block of extraBlocks) {
    parts.push(block);
  }

  return Buffer.concat(parts);
}

/**
 * Create a FLAC header with a realistic sample count for the given duration.
 * music-metadata requires a valid minimum stream info to accept the file.
 */
export function flacHeaderWithDuration(
  siIsLast: boolean,
  durationSeconds: number,
  extraBlocks: Buffer[] = []
): Buffer {
  const parts: Buffer[] = [];
  parts.push(Buffer.from("fLaC", "ascii"));

  const sampleRate = 44_100;
  const totalSamples = BigInt(Math.round(sampleRate * durationSeconds));
  const packed =
    (BigInt(sampleRate) << 44n) |
    (1n << 41n) |
    (15n << 36n) |
    totalSamples;
  const si = Buffer.alloc(STREAMINFO_LEN);
  si.writeUInt16BE(4096, 0);
  si.writeUInt16BE(4096, 2);
  si.writeUIntBE(0, 4, 3);
  si.writeUIntBE(0, 7, 3);
  for (let i = 0; i < 8; i++) {
    si[10 + i] = Number((packed >> BigInt((7 - i) * 8)) & 0xffn);
  }

  const siHeader = Buffer.alloc(4);
  siHeader[0] = siIsLast ? 0x80 : 0x00;
  siHeader[1] = (si.length >> 16) & 0xff;
  siHeader[2] = (si.length >> 8) & 0xff;
  siHeader[3] = si.length & 0xff;
  parts.push(siHeader, si);

  for (const block of extraBlocks) {
    parts.push(block);
  }

  return Buffer.concat(parts);
}

/**
 * Build a VORBIS_COMMENT metadata block header + body.
 */
export function vorbisCommentBlock(
  comments: string[],
  options?: { headerOnly?: boolean; corruptLen?: number; isLast?: boolean }
): Buffer {
  const isLast = options?.isLast ?? true;
  const headerOnly = options?.headerOnly ?? false;

  const vendor = Buffer.from("auto-tagger-test", "utf8");
  const vLen = Buffer.alloc(4);
  vLen.writeUInt32LE(vendor.length);

  const cBufs: Buffer[] = [];
  for (const c of comments) {
    const cb = Buffer.from(c, "utf8");
    const cl = Buffer.alloc(4);
    cl.writeUInt32LE(cb.length);
    cBufs.push(cl, cb);
  }

  const n = Buffer.alloc(4);
  n.writeUInt32LE(comments.length);

  const body = Buffer.concat([vLen, vendor, n, ...cBufs]);

  const realBodyLen = body.length;
  const headerLen = headerOnly ? 0 : realBodyLen;
  const length = options?.corruptLen ?? headerLen;

  const header = Buffer.alloc(4);
  header[0] = (isLast ? 0x80 : 0x00) | 0x04;
  header[1] = (length >> 16) & 0xff;
  header[2] = (length >> 8) & 0xff;
  header[3] = length & 0xff;

  return Buffer.concat([header, headerOnly ? Buffer.alloc(0) : body]);
}

/** Build a PADDING block (type 1). */
export function paddingBlock(size: number): Buffer {
  const body = Buffer.alloc(size);
  const header = Buffer.alloc(4);
  header[0] = 0x01;
  header[1] = (size >> 16) & 0xff;
  header[2] = (size >> 8) & 0xff;
  header[3] = size & 0xff;
  return Buffer.concat([header, body]);
}
