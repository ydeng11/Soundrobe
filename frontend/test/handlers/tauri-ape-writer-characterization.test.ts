// @vitest-environment node
import { afterEach, describe, expect, it } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { writeTagsWithResult } from "../../electron/handlers/writer";
import { readTrackMetadata } from "../../electron/handlers/tracks";

const fixture = path.resolve(
  "test/fixtures/tauri/media-corpus/ape-id3v1-fallback.ape",
);
const roots: string[] = [];

function copyFixture(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-ape-writer-"));
  roots.push(root);
  const target = path.join(root, "track.ape");
  fs.copyFileSync(fixture, target);
  return target;
}

function audioCore(filePath: string): Buffer {
  const bytes = fs.readFileSync(filePath);
  let end = bytes.length;
  if (end >= 128 && bytes.subarray(end - 128, end - 125).toString("ascii") === "TAG") {
    end -= 128;
  }
  if (end >= 32 && bytes.subarray(end - 32, end - 24).toString("ascii") === "APETAGEX") {
    const size = bytes.readUInt32LE(end - 20);
    if (size < 32 || size > end) throw new Error("Invalid APEv2 footer size");
    end -= size;
  }
  return bytes.subarray(0, end);
}

afterEach(() => {
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe("Electron APE writer characterization for Tauri parity", () => {
  it("writes rich APEv2 fields while preserving the original body", async () => {
    const file = copyFixture();
    const body = audioCore(file);
    const result = await writeTagsWithResult(file, {
      title: "Replacement APE",
      artist: "Primary",
      artists: ["Primary", "Guest"],
      album: "Replacement Album",
      trackNumber: 7,
      trackTotal: 9,
      discNumber: 2,
      discTotal: 3,
      musicbrainzAlbumId: "replacement-mb-album",
      discogsReleaseId: "replacement-discogs-release",
    });
    expect(result).toEqual({
      outcome: "full_rewrite",
      reason: "format_requires_full_rewrite",
    });
    expect(audioCore(file)).toEqual(body);
    expect(fs.readFileSync(file).subarray(-128, -125).toString("ascii")).not.toBe("TAG");
    expect(await readTrackMetadata(file)).toMatchObject({
      title: "Replacement APE",
      artist: "Primary",
      artists: ["Primary", "Guest"],
      album: "Replacement Album",
      trackNumber: 7,
      trackTotal: 9,
      discNumber: 2,
      discTotal: 3,
      musicbrainzAlbumId: "replacement-mb-album",
      discogsReleaseId: "replacement-discogs-release",
    });
  });

  it("reports full rewrite for an identical patch but emits identical bytes", async () => {
    const file = copyFixture();
    const patch = { title: "Stable APE", artist: "Stable Artist" };
    await writeTagsWithResult(file, patch);
    const before = fs.readFileSync(file);
    const result = await writeTagsWithResult(file, patch);
    expect(result).toEqual({
      outcome: "full_rewrite",
      reason: "format_requires_full_rewrite",
    });
    expect(fs.readFileSync(file)).toEqual(before);
  });

  it("clears explicit null without changing the original body", async () => {
    const file = copyFixture();
    const body = audioCore(file);
    await writeTagsWithResult(file, { title: "Remove" });
    await writeTagsWithResult(file, { title: null });
    expect((await readTrackMetadata(file)).title).toBeNull();
    expect(audioCore(file)).toEqual(body);
  });
});
