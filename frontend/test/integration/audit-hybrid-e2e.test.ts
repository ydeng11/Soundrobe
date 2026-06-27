// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseFile } from "music-metadata";
import { applyAuditFixes } from "../../electron/handlers/audit";
import { readTrackMetadata } from "../../electron/handlers/tracks";
import { buildDeterministicAuditFindings } from "../../electron/services/AuditRuleEngine";
import { flacHeaderWithDuration, vorbisCommentBlock } from "../helpers/flac-helpers";

vi.mock("electron", () => ({
  ipcMain: {
    handle: vi.fn(),
  },
  BrowserWindow: {
    getAllWindows: vi.fn(() => []),
  },
}));

function writeFlac(filePath: string, comments: string[]): void {
  const block = vorbisCommentBlock(comments, { isLast: true });
  writeFileSync(filePath, Buffer.concat([
    flacHeaderWithDuration(false, 200, [block]),
    Buffer.from([0xff, 0xf8, 0x69, 0x18]),
    Buffer.alloc(100),
  ]));
}

describe("hybrid audit FLAC write path", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = `/private/tmp/auto-tagger-audit-hybrid-${process.pid}-${Date.now()}`;
    mkdirSync(tmpDir, { recursive: true });
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("auto-fixes deterministic core-tag findings on copied FLAC media", async () => {
    const albumDir = join(tmpDir, "Artist", "2020 - Album");
    mkdirSync(albumDir, { recursive: true });
    const first = join(albumDir, "01. A & B - Song.flac");
    const second = join(albumDir, "02. Second Song.flac");

    writeFlac(first, [
      "TITLE=Wrong",
      "ARTIST=Wrong Artist",
      "ALBUM=Wrong Album",
      "ALBUMARTIST=Wrong Artist",
      "DATE=2019",
      "TRACKNUMBER=9",
    ]);
    writeFlac(second, [
      "TITLE=Second Song",
      "ARTIST=Artist",
      "ARTISTS=Artist",
      "ALBUM=Album",
      "ALBUMARTIST=Artist",
      "DATE=2020",
      "TRACKNUMBER=2/2",
    ]);

    const metas = [
      await readTrackMetadata(first),
      await readTrackMetadata(second),
    ];
    const findings = buildDeterministicAuditFindings(
      "Artist",
      "2020 - Album",
      metas,
      ["01. A & B - Song.flac", "02. Second Song.flac"],
    );

    const fixed = await applyAuditFixes([first, second], findings);
    const updated = await parseFile(first);

    expect(fixed).toBeGreaterThan(0);
    expect(updated.common.title).toBe("Song");
    expect(updated.common.artist).toBe("A & B");
    expect(updated.common.artists).toEqual(["A", "B"]);
    expect(updated.common.album).toBe("Album");
    expect(updated.common.albumartist).toBe("Artist");
    expect(updated.common.year).toBe(2020);
    expect(updated.common.track.no).toBe(1);
    expect(updated.common.track.of).toBe(2);
  });

  it("surfaces ambiguous artist splitting without writing a fix", async () => {
    const albumDir = join(tmpDir, "Artist", "Album");
    mkdirSync(albumDir, { recursive: true });
    const filePath = join(albumDir, "Thunderstruck.flac");

    writeFlac(filePath, [
      "TITLE=Thunderstruck",
      "ARTIST=AC/DC",
      "ALBUM=Album",
      "ALBUMARTIST=Artist",
    ]);

    const meta = await readTrackMetadata(filePath);
    const findings = buildDeterministicAuditFindings(
      "Artist",
      "Album",
      [{ ...meta, artists: [] }],
      ["Thunderstruck.flac"],
    );

    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({
        field: "artists",
        status: "warning",
        autoFixEligible: false,
      }),
    ]));

    const fixed = await applyAuditFixes([filePath], findings);
    const updated = await parseFile(filePath);

    expect(fixed).toBe(0);
    expect(updated.common.artist).toBe("AC/DC");
    expect(updated.common.artists).not.toEqual(["AC", "DC"]);
  });
});
