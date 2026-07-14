// @vitest-environment node
import { afterEach, describe, expect, it } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { writeTagsWithResult } from "../../electron/handlers/writer";
import { readTrackMetadata } from "../../electron/handlers/tracks";

const fixture = path.resolve("test/fixtures/tauri/writer-corpus/padded.flac");
const roots: string[] = [];

function copyFixture(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-flac-writer-"));
  roots.push(root);
  const target = path.join(root, "track.flac");
  fs.copyFileSync(fixture, target);
  return target;
}

function flacAudioOffset(bytes: Buffer): number {
  if (bytes.subarray(0, 4).toString("ascii") !== "fLaC") {
    throw new Error("Invalid FLAC marker");
  }
  let offset = 4;
  while (offset + 4 <= bytes.length) {
    const last = (bytes[offset] & 0x80) !== 0;
    const length =
      (bytes[offset + 1] << 16) |
      (bytes[offset + 2] << 8) |
      bytes[offset + 3];
    offset += 4 + length;
    if (offset > bytes.length) throw new Error("Truncated FLAC metadata");
    if (last) return offset;
  }
  throw new Error("Missing final FLAC metadata block");
}

async function mutate(filePath: string, fields: Parameters<typeof writeTagsWithResult>[1]) {
  const before = fs.readFileSync(filePath);
  const audioBefore = before.subarray(flacAudioOffset(before));
  const result = await writeTagsWithResult(filePath, fields);
  const after = fs.readFileSync(filePath);
  return {
    result,
    metadata: await readTrackMetadata(filePath),
    bytesEqual: before.equals(after),
    sizeEqual: before.length === after.length,
    audioEqual: audioBefore.equals(after.subarray(flacAudioOffset(after))),
  };
}

afterEach(() => {
  for (const root of roots.splice(0)) {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

describe("Electron FLAC writer characterization for Tauri parity", () => {
  it("skips an identical patch without changing bytes", async () => {
    const result = await mutate(copyFixture(), { title: "Corpus Encoded" });
    expect(result.result).toEqual({ outcome: "skipped", reason: "unchanged" });
    expect(result.bytesEqual).toBe(true);
    expect(result.audioEqual).toBe(true);
  });

  it("updates within the padded metadata region without moving audio", async () => {
    const result = await mutate(copyFixture(), {
      title: "Replacement FLAC title",
      artist: "Replacement Artist",
      trackNumber: 7,
      trackTotal: 9,
      discNumber: 2,
      discTotal: 3,
      musicbrainzAlbumId: "replacement-mb-album",
      discogsReleaseId: "replacement-discogs-release",
    });
    expect(result.result).toEqual({
      outcome: "metadata_rewrite",
      reason: "metadata_region_repacked",
    });
    expect(result.sizeEqual).toBe(true);
    expect(result.audioEqual).toBe(true);
    expect(result.metadata).toMatchObject({
      title: "Replacement FLAC title",
      artist: "Replacement Artist",
      trackNumber: 7,
      trackTotal: 9,
      discNumber: 2,
      discTotal: 3,
      musicbrainzAlbumId: "replacement-mb-album",
      discogsReleaseId: "replacement-discogs-release",
    });
  });

  it("distinguishes explicit null clear from omission", async () => {
    const omitted = await mutate(copyFixture(), {});
    const cleared = await mutate(copyFixture(), { title: null });
    expect(omitted.metadata.title).toBe("Corpus Encoded");
    expect(cleared.metadata.title).toBeNull();
    expect(omitted.audioEqual).toBe(true);
    expect(cleared.audioEqual).toBe(true);
  });
});
