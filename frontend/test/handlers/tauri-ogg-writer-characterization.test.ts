// @vitest-environment node
import { afterEach, describe, expect, it } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { writeTagsWithResult } from "../../electron/handlers/writer";
import { readTrackMetadata } from "../../electron/handlers/tracks";

const root = path.resolve("test/fixtures/tauri/writer-corpus");
const temporaryRoots: string[] = [];

function copyFixture(name: string): string {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-ogg-writer-"));
  temporaryRoots.push(temporary);
  const target = path.join(temporary, name);
  fs.copyFileSync(path.join(root, name), target);
  return target;
}

function oggPackets(filePath: string): Buffer[] {
  const bytes = fs.readFileSync(filePath);
  const packets: Buffer[] = [];
  let packet = Buffer.alloc(0);
  let offset = 0;
  while (offset + 27 <= bytes.length) {
    if (bytes.subarray(offset, offset + 4).toString("ascii") !== "OggS") {
      throw new Error(`Invalid OGG page at ${offset}`);
    }
    const segmentCount = bytes[offset + 26];
    const tableStart = offset + 27;
    const dataStart = tableStart + segmentCount;
    let dataOffset = dataStart;
    for (const length of bytes.subarray(tableStart, dataStart)) {
      packet = Buffer.concat([packet, bytes.subarray(dataOffset, dataOffset + length)]);
      dataOffset += length;
      if (length < 255) {
        packets.push(packet);
        packet = Buffer.alloc(0);
      }
    }
    offset = dataOffset;
  }
  if (offset !== bytes.length || packet.length !== 0) {
    throw new Error("Truncated OGG stream");
  }
  return packets;
}

function audioPackets(filePath: string): Buffer[] {
  return oggPackets(filePath).filter(
    (packet) =>
      !packet.subarray(0, 7).equals(Buffer.from("\x01vorbis")) &&
      !packet.subarray(0, 7).equals(Buffer.from("\x03vorbis")) &&
      !packet.subarray(0, 7).equals(Buffer.from("\x05vorbis")) &&
      !packet.subarray(0, 8).equals(Buffer.from("OpusHead")) &&
      !packet.subarray(0, 8).equals(Buffer.from("OpusTags")),
  );
}

afterEach(() => {
  for (const temporary of temporaryRoots.splice(0)) {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
});

describe("Electron OGG/Opus writer characterization for Tauri parity", () => {
  it("rewrites OGG comments and preserves encoded audio packets", async () => {
    const file = copyFixture("vorbis.ogg");
    const beforeAudio = audioPackets(file);
    const result = await writeTagsWithResult(file, {
      title: "Replacement OGG",
      artist: "Replacement Artist",
      trackNumber: 7,
      trackTotal: 9,
      discogsReleaseId: "replacement-discogs-release",
    });

    expect(result).toEqual({
      outcome: "full_rewrite",
      reason: "container_requires_full_rewrite",
    });
    expect(audioPackets(file)).toEqual(beforeAudio);
    expect(await readTrackMetadata(file)).toMatchObject({
      title: "Replacement OGG",
      artist: "Replacement Artist",
      trackNumber: 7,
      trackTotal: 9,
      discogsReleaseId: "replacement-discogs-release",
    });
  });

  it("legacy identical OGG patch still rewrites the container", async () => {
    const file = copyFixture("vorbis.ogg");
    const before = fs.readFileSync(file);
    const result = await writeTagsWithResult(file, { title: "Corpus Encoded" });
    expect(result).toEqual({
      outcome: "full_rewrite",
      reason: "container_requires_full_rewrite",
    });
    expect(fs.readFileSync(file).equals(before)).toBe(false);
    expect(audioPackets(file)).toEqual(audioPackets(path.join(root, "vorbis.ogg")));
  });

  it("characterizes silent true-Opus update failure", async () => {
    const file = copyFixture("opus.opus");
    const before = fs.readFileSync(file);
    const result = await writeTagsWithResult(file, { title: "Replacement Opus" });

    expect(result).toEqual({
      outcome: "full_rewrite",
      reason: "container_requires_full_rewrite",
    });
    expect(fs.readFileSync(file)).toEqual(before);
    expect((await readTrackMetadata(file)).title).toBe("Corpus Encoded");
  });
});
