// @vitest-environment node
import { afterEach, describe, expect, it } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { writeTagsWithResult } from "../../electron/handlers/writer";
import { readTrackMetadata } from "../../electron/handlers/tracks";

const fixture = path.resolve("test/fixtures/tauri/media-corpus/minimal.wav");
const roots: string[] = [];

function copyFixture(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-wav-writer-"));
  roots.push(root);
  const target = path.join(root, "track.wav");
  fs.copyFileSync(fixture, target);
  return target;
}

function waveData(filePath: string): Buffer {
  const bytes = fs.readFileSync(filePath);
  if (bytes.subarray(0, 4).toString("ascii") !== "RIFF") throw new Error("Invalid RIFF");
  let offset = 12;
  while (offset + 8 <= bytes.length) {
    const id = bytes.subarray(offset, offset + 4).toString("ascii");
    const size = bytes.readUInt32LE(offset + 4);
    const dataStart = offset + 8;
    const end = dataStart + size;
    if (end > bytes.length) throw new Error("Truncated RIFF chunk");
    if (id === "data") return bytes.subarray(dataStart, end);
    offset = end + (size % 2);
  }
  throw new Error("Missing data chunk");
}

afterEach(() => {
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe("Electron WAV writer characterization for Tauri parity", () => {
  it("adds ID3 metadata by full rewrite while preserving PCM data", async () => {
    const file = copyFixture();
    const pcm = waveData(file);
    const result = await writeTagsWithResult(file, {
      title: "Replacement WAV",
      artist: "Replacement Artist",
      trackNumber: 7,
      trackTotal: 9,
      musicbrainzAlbumId: "replacement-mb-album",
    });
    expect(result).toEqual({
      outcome: "full_rewrite",
      reason: "format_requires_full_rewrite",
    });
    expect(waveData(file)).toEqual(pcm);
    expect(await readTrackMetadata(file)).toMatchObject({
      title: "Replacement WAV",
      artist: "Replacement Artist",
      trackNumber: 7,
      trackTotal: 9,
      musicbrainzAlbumId: "replacement-mb-album",
    });
  });

  it("skips an identical second patch without changing bytes", async () => {
    const file = copyFixture();
    const patch = { title: "Stable WAV", artist: "Stable Artist" };
    await writeTagsWithResult(file, patch);
    const before = fs.readFileSync(file);
    const result = await writeTagsWithResult(file, patch);
    expect(result).toEqual({ outcome: "skipped", reason: "unchanged" });
    expect(fs.readFileSync(file)).toEqual(before);
  });

  it("clears explicit null and preserves PCM data", async () => {
    const file = copyFixture();
    const pcm = waveData(file);
    await writeTagsWithResult(file, { title: "Remove" });
    await writeTagsWithResult(file, { title: null });
    expect((await readTrackMetadata(file)).title).toBeNull();
    expect(waveData(file)).toEqual(pcm);
  });
});
