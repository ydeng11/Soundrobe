// @vitest-environment node
import { afterEach, describe, expect, it } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { writeTagsWithResult } from "../../electron/handlers/writer";
import { readTrackMetadata } from "../../electron/handlers/tracks";

const fixtureRoot = path.resolve("test/fixtures/tauri/media-corpus");
const temporaryRoots: string[] = [];

function copyFixture(name: string): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-mp4-writer-"));
  temporaryRoots.push(root);
  const target = path.join(root, name);
  fs.copyFileSync(path.join(fixtureRoot, name), target);
  return target;
}

function atomPayload(filePath: string, wanted: string): Buffer {
  const bytes = fs.readFileSync(filePath);
  let offset = 0;
  while (offset + 8 <= bytes.length) {
    const size = bytes.readUInt32BE(offset);
    const kind = bytes.subarray(offset + 4, offset + 8).toString("ascii");
    if (size < 8 || offset + size > bytes.length) {
      throw new Error(`Invalid top-level MP4 atom at ${offset}`);
    }
    if (kind === wanted) return bytes.subarray(offset + 8, offset + size);
    offset += size;
  }
  throw new Error(`Missing ${wanted} atom`);
}

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

describe("Electron MP4 writer characterization for Tauri parity", () => {
  for (const name of ["minimal.m4a", "minimal.mp4"]) {
    it(`characterizes placeholder metadata replacement for ${name}`, async () => {
      const file = copyFixture(name);
      const before = fs.readFileSync(file);
      const audioBefore = atomPayload(file, "mdat");
      const result = await writeTagsWithResult(file, {
        title: "Replacement MP4",
        artist: "Replacement Artist",
      });

      expect(result).toEqual({
        outcome: "full_rewrite",
        reason: "format_requires_full_rewrite",
      });
      expect(fs.readFileSync(file).equals(before)).toBe(false);
      expect(atomPayload(file, "mdat")).toEqual(audioBefore);
      await expect(readTrackMetadata(file)).rejects.toThrow("End-Of-Stream");
    });
  }
});
