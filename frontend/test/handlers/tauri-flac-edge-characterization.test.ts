// @vitest-environment node
import { afterEach, describe, expect, it } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { parseFile } from "music-metadata";
import { writeTagsWithResult } from "../../electron/handlers/writer";

const fixtureRoot = path.resolve("test/fixtures/tauri/writer-corpus");
const roots: string[] = [];

function copy(name: string): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-flac-edge-"));
  roots.push(root);
  const target = path.join(root, name);
  fs.copyFileSync(path.join(fixtureRoot, name), target);
  return target;
}

function layout(bytes: Buffer): { audioOffset: number; types: number[] } {
  expect(bytes.subarray(0, 4).toString("ascii")).toBe("fLaC");
  const types: number[] = [];
  let offset = 4;
  for (;;) {
    const type = bytes[offset] & 0x7f;
    const last = !!(bytes[offset] & 0x80);
    const length = bytes.readUIntBE(offset + 1, 3);
    types.push(type);
    offset += 4 + length;
    if (last) return { audioOffset: offset, types };
  }
}

function trailingApeStart(bytes: Buffer): number {
  if (bytes.subarray(-32, -24).toString("ascii") !== "APETAGEX") return bytes.length;
  return bytes.length - bytes.readUInt32LE(bytes.length - 20);
}

afterEach(() => {
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe("Electron FLAC repair-edge characterization for Tauri parity", () => {
  it("removes trailing APEv2 and preserves FLAC audio bytes", async () => {
    const file = copy("flac-trailing-ape.flac");
    const before = fs.readFileSync(file);
    const { audioOffset } = layout(before);
    const audio = before.subarray(audioOffset, trailingApeStart(before));
    const result = await writeTagsWithResult(file, { album: "Correct Album" });
    const after = fs.readFileSync(file);
    expect(result).toEqual({ outcome: "full_rewrite", reason: "ape_tag_removed" });
    expect(after.includes(Buffer.from("APETAGEX"))).toBe(false);
    expect(after.subarray(layout(after).audioOffset)).toEqual(audio);
    expect((await parseFile(file, { duration: false })).common.album).toBe("Correct Album");
  });

  it("neutralizes appended ghost Vorbis comments without changing length", async () => {
    const file = copy("flac-ghost-vc.flac");
    const before = fs.readFileSync(file);
    const result = await writeTagsWithResult(file, { title: "RealTitle" });
    const after = fs.readFileSync(file);
    expect(result).toEqual({ outcome: "full_rewrite", reason: "ghost_vorbis_removed" });
    expect(after.length).toBeGreaterThan(before.length);
    const beforeLayout = layout(before);
    const afterLayout = layout(after);
    const beforePayload = before.subarray(beforeLayout.audioOffset);
    const expectedPayload = Buffer.from(beforePayload);
    const beforeGhost = beforePayload.lastIndexOf(Buffer.from("auto-tagger"));
    expectedPayload.writeUInt32LE(0, beforeGhost - 4);
    expect(after.subarray(afterLayout.audioOffset)).toEqual(expectedPayload);
  });

  it("collapses duplicate metadata Vorbis comments to one block", async () => {
    const file = copy("flac-duplicate-vc.flac");
    expect(layout(fs.readFileSync(file)).types.filter((type) => type === 4)).toHaveLength(2);
    const result = await writeTagsWithResult(file, { title: "Canonical Title" });
    const after = fs.readFileSync(file);
    expect(result).toEqual({ outcome: "metadata_rewrite", reason: "duplicate_vorbis_removed" });
    expect(layout(after).types.filter((type) => type === 4)).toHaveLength(1);
    expect((await parseFile(file, { duration: false })).common.title).toBe("Canonical Title");
  });

  it("uses full rewrite when growth exceeds the eight-byte padding", async () => {
    const file = copy("flac-insufficient-padding.flac");
    const before = fs.readFileSync(file);
    const audio = before.subarray(layout(before).audioOffset);
    const result = await writeTagsWithResult(file, {
      title: "Expanded",
      lyrics: "lyrics".repeat(500),
    });
    const after = fs.readFileSync(file);
    expect(result).toEqual({ outcome: "full_rewrite", reason: "insufficient_metadata_space" });
    expect(after.subarray(layout(after).audioOffset)).toEqual(audio);
    expect(after.length).toBeGreaterThan(before.length);
  });
});
