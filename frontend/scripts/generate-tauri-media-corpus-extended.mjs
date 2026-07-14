#!/usr/bin/env node
/**
 * Explicitly regenerate static reader fixtures that need a real encoder.
 *
 * Tests never invoke ffmpeg. Run this manually, then refresh normalized
 * expected.json with:
 *   UPDATE_MEDIA_CORPUS=1 npx vitest run test/handlers/media-corpus.test.ts
 */
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const outputRoot = resolve("test/fixtures/tauri/media-corpus");
mkdirSync(outputRoot, { recursive: true });

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

for (const [name, args] of outputs) {
  const result = spawnSync("ffmpeg", [...common, ...args, `${outputRoot}/${name}`], {
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  console.log(`generated ${name}`);
}
