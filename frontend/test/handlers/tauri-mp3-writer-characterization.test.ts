// @vitest-environment node
import { afterEach, describe, expect, it } from "vitest";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  writeTagsWithResult,
  type WriteFields,
} from "../../electron/handlers/writer";
import { readTrackMetadata } from "../../electron/handlers/tracks";

const fixture = path.resolve(
  "test/fixtures/tauri/media-corpus/minimal.mp3",
);
const roots: string[] = [];

function copyFixture(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-mp3-writer-"));
  roots.push(root);
  const target = path.join(root, "track.mp3");
  fs.copyFileSync(fixture, target);
  return target;
}

function mpegPayloadHash(filePath: string): string {
  const bytes = fs.readFileSync(filePath);
  let offset = 0;
  if (bytes.subarray(0, 3).toString("ascii") === "ID3") {
    if (bytes.length < 10) throw new Error("Truncated ID3v2 header");
    const size =
      (bytes[6] << 21) |
      (bytes[7] << 14) |
      (bytes[8] << 7) |
      bytes[9];
    offset = 10 + size;
  }
  return createHash("sha256").update(bytes.subarray(offset)).digest("hex");
}

async function writeAndRead(filePath: string, fields: WriteFields) {
  const before = fs.readFileSync(filePath);
  const payloadBefore = mpegPayloadHash(filePath);
  const outcome = await writeTagsWithResult(filePath, fields);
  const after = fs.readFileSync(filePath);
  const metadata = await readTrackMetadata(filePath);
  return {
    outcome,
    metadata,
    fullBytesChanged: !after.equals(before),
    payloadUnchanged: mpegPayloadHash(filePath) === payloadBefore,
  };
}

afterEach(() => {
  for (const root of roots.splice(0)) {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

describe("Electron MP3 writer characterization for Tauri parity", () => {
  it("reports the legacy full-rewrite outcome for an identical field patch", async () => {
    const result = await writeAndRead(copyFixture(), { title: "Corpus MP3" });

    expect(result.outcome).toEqual({
      outcome: "full_rewrite",
      reason: "format_requires_full_rewrite",
    });
    expect(result.metadata.title).toBe("Corpus MP3");
    expect(result.payloadUnchanged).toBe(true);
    // Legacy node-id3 rewrites metadata bytes even when the requested value is
    // identical. The Tauri writer intentionally improves this to a true no-op.
    expect(result.fullBytesChanged).toBe(true);
  });

  it("distinguishes an omitted field from an explicit null clear", async () => {
    const omitted = await writeAndRead(copyFixture(), {});
    const cleared = await writeAndRead(copyFixture(), { title: null });

    expect(omitted.metadata.title).toBe("Corpus MP3");
    expect(cleared.metadata.title).toBeNull();
    expect(omitted.payloadUnchanged).toBe(true);
    expect(cleared.payloadUnchanged).toBe(true);
  });

  it("updates one field without changing MPEG audio payload bytes", async () => {
    const result = await writeAndRead(copyFixture(), { title: "Changed title" });

    expect(result.metadata.title).toBe("Changed title");
    expect(result.fullBytesChanged).toBe(true);
    expect(result.payloadUnchanged).toBe(true);
  });

  it("round-trips rich standard and provider fields while preserving audio", async () => {
    const result = await writeAndRead(copyFixture(), {
      title: "Replacement",
      artists: ["Primary", "Guest"],
      trackNumber: 7,
      trackTotal: 9,
      discNumber: 2,
      discTotal: 3,
      description: "Replacement description",
      lyrics: "Replacement lyrics",
      musicbrainzTrackId: "replacement-mb-track",
      discogsReleaseId: "replacement-discogs-release",
    });

    expect(result.metadata).toMatchObject({
      title: "Replacement",
      artists: ["Primary", "Guest"],
      trackNumber: 7,
      trackTotal: 9,
      discNumber: 2,
      discTotal: 3,
      description: "Replacement description",
      musicbrainzTrackId: "replacement-mb-track",
      discogsReleaseId: "replacement-discogs-release",
    });
    expect((result.metadata.lyrics as unknown as { text: string }).text).toBe(
      "Replacement lyrics",
    );
    expect(result.payloadUnchanged).toBe(true);
  });
});
