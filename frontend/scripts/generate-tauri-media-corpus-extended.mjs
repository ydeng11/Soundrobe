#!/usr/bin/env node
/**
 * Explicitly regenerate static reader fixtures that need a real encoder.
 *
 * Tests never invoke ffmpeg. Run this manually, then refresh normalized
 * expected.json with:
 *   UPDATE_MEDIA_CORPUS=1 npx vitest run test/handlers/media-corpus.test.ts
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const outputRoot = resolve("test/fixtures/tauri/media-corpus");
const writerRoot = resolve("test/fixtures/tauri/writer-corpus");
mkdirSync(outputRoot, { recursive: true });
mkdirSync(writerRoot, { recursive: true });

const common = [
  "-hide_banner",
  "-loglevel",
  "error",
  "-y",
  "-f",
  "lavfi",
  "-i",
  "anullsrc=r=44100:cl=stereo",
  "-t",
  "0.05",
];
const metadata = [
  "-metadata",
  "title=Corpus Encoded",
  "-metadata",
  "artist=Corpus Artist",
  "-metadata",
  "album=Corpus Album",
  "-metadata",
  "date=2024",
  "-metadata",
  "track=4/12",
  "-metadata",
  "disc=1/2",
  "-metadata",
  "genre=Electronic",
];
const outputs = [
  ["minimal.m4a", ["-c:a", "aac", "-b:a", "64k", ...metadata]],
  ["minimal.mp4", ["-c:a", "aac", "-b:a", "64k", ...metadata]],
  ["minimal.opus", ["-c:a", "libopus", "-b:a", "64k", ...metadata]],
  ["minimal.aiff", ["-c:a", "pcm_s16be", ...metadata]],
];

function oggCrc32(bytes) {
  let crc = 0;
  for (const byte of bytes) {
    crc ^= byte << 24;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc & 0x80000000) !== 0 ? (crc << 1) ^ 0x04c11db7 : crc << 1;
    }
  }
  return crc >>> 0;
}

function canonicalizeOggSerial(file) {
  const bytes = readFileSync(file);
  let offset = 0;
  while (offset + 27 <= bytes.length) {
    if (bytes.subarray(offset, offset + 4).toString("ascii") !== "OggS") {
      throw new Error(`Invalid OGG page at ${offset}: ${file}`);
    }
    const segments = bytes[offset + 26];
    const tableEnd = offset + 27 + segments;
    const bodySize = bytes
      .subarray(offset + 27, tableEnd)
      .reduce((sum, value) => sum + value, 0);
    const pageEnd = tableEnd + bodySize;
    if (pageEnd > bytes.length) throw new Error(`Truncated OGG page: ${file}`);
    bytes.writeUInt32LE(0x4155544f, offset + 14); // "AUTO" in LE bytes
    bytes.writeUInt32LE(0, offset + 22);
    bytes.writeUInt32LE(oggCrc32(bytes.subarray(offset, pageEnd)), offset + 22);
    offset = pageEnd;
  }
  if (offset !== bytes.length) throw new Error(`Trailing OGG bytes: ${file}`);
  writeFileSync(file, bytes);
}

function run(output, args) {
  const result = spawnSync("ffmpeg", [...common, ...args, output], {
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  if (output.endsWith(".opus") || output.endsWith(".ogg")) {
    canonicalizeOggSerial(output);
  }
  console.log(`generated ${output}`);
}

for (const [name, args] of outputs) {
  run(`${outputRoot}/${name}`, args);
}
run(`${writerRoot}/padded.flac`, ["-c:a", "flac", ...metadata]);
run(`${writerRoot}/vorbis.ogg`, [
  "-c:a",
  "vorbis",
  "-strict",
  "experimental",
  "-q:a",
  "2",
  ...metadata,
]);
run(`${writerRoot}/opus.opus`, ["-c:a", "libopus", "-b:a", "64k", ...metadata]);
