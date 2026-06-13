/**
 * Tests for APEv2 tag writer (Monkey's Audio .ape support).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import path from "path";
import fs from "fs";
import os from "os";
import { readFile, writeFile } from "fs/promises";

// Test the writer functions through the public API
import { writeTags, writeExtraTags, writeTagsWithOutcome } from "../../electron/handlers/writer";

// ── Binary-level helpers (replicated from writer.ts for isolated testing) ──

function buildApeTagItems(
  entries: Array<{ key: string; value: string }>,
): { items: Buffer; count: number } {
  const chunks: Buffer[] = [];
  let count = 0;
  for (const { key, value } of entries) {
    if (!key || value == null || value === "") continue;
    const keyBuf = Buffer.from(key.toUpperCase() + "\0", "utf8");
    const valBuf = Buffer.from(value, "utf8");
    const header = Buffer.alloc(8);
    header.writeUInt32LE(valBuf.length, 0);
    header.writeUInt32LE(0x20000000, 4);
    chunks.push(header, keyBuf, valBuf);
    count++;
  }
  return { items: Buffer.concat(chunks), count };
}

function buildApeFooter(tagSize: number, itemCount: number): Buffer {
  const footer = Buffer.alloc(32);
  footer.write("APETAGEX", 0, 8, "ascii");
  footer.writeUInt32LE(2000, 8);
  footer.writeUInt32LE(tagSize, 12);
  footer.writeUInt32LE(itemCount, 16);
  footer.writeUInt32LE(0x80000000, 20);
  return footer;
}

/**
 * Compute the end of Monkey's Audio data from MAC descriptor.
 */
function computeAudioEnd(data: Buffer): number | null {
  if (data.length < 52) return null;
  if (data.toString("ascii", 0, 4) !== "MAC ") return null;
  const descriptorBytes = data.readUInt32LE(8);
  const headerBytes = data.readUInt32LE(12);
  if (data.length < descriptorBytes + headerBytes) return null;
  const seekTableBytes = data.readUInt32LE(16);
  const headerDataBytes = data.readUInt32LE(20);
  const apeFrameDataBytes = data.readUInt32LE(24);
  const terminatingDataBytes = data.readUInt32LE(32);
  const forwardBytes = seekTableBytes + headerDataBytes + apeFrameDataBytes + terminatingDataBytes;
  return descriptorBytes + headerBytes + forwardBytes;
}

/**
 * Find the earliest byte at which APEv2/ID3v1 tag data begins.
 */
function getApeTagStart(data: Buffer): number | null {
  if (data.length < 32) return null;
  const preamble = Buffer.from("APETAGEX", "ascii");
  const offsets: number[] = [];
  let searchFrom = 0;
  while (true) {
    const idx = data.indexOf(preamble, searchFrom);
    if (idx === -1) break;
    offsets.push(idx);
    searchFrom = idx + 1;
  }
  if (offsets.length === 0) return null;
  let earliest = data.length;
  for (const offset of offsets) {
    const flags = data.readUInt32LE(offset + 20);
    const isHeader = !!(flags & 0x20000000);
    const tagSize = data.readUInt32LE(offset + 12);
    if (isHeader) {
      if (offset < earliest) earliest = offset;
    } else {
      const itemsStart = offset + 32 - tagSize;
      if (itemsStart >= 0 && itemsStart < earliest) {
        const hdrOff = itemsStart - 32;
        if (
          hdrOff >= 0 &&
          data.toString("ascii", hdrOff, hdrOff + 8) === "APETAGEX" &&
          (data.readUInt32LE(hdrOff + 20) & 0x20000000)
        ) {
          if (hdrOff < earliest) earliest = hdrOff;
        } else {
          earliest = itemsStart;
        }
      }
    }
  }
  const audioEnd = computeAudioEnd(data);
  if (audioEnd !== null && earliest > audioEnd) {
    return audioEnd;
  }
  return earliest < data.length ? earliest : null;
}

/**
 * Search backwards from the end for "APETAGEX" footer (used by parseApeTagsFromBuffer).
 */
function findApeFooterOffset(data: Buffer): number {
  const limit = Math.min(data.length, 2048);
  for (let offset = data.length - 8; offset >= data.length - limit; offset--) {
    if (data.toString("ascii", offset, offset + 8) === "APETAGEX") {
      const flags = data.readUInt32LE(offset + 20);
      if (!(flags & 0x20000000)) return offset;
    }
  }
  return -1;
}

function stripApeTag(data: Buffer): Buffer {
  const tagStart = getApeTagStart(data);
  if (tagStart === null) return data;
  return data.subarray(0, tagStart);
}

/**
 * Parse APEv2 tags from a buffer for test verification.
 * Returns a map of key → values (supports duplicate keys).
 */
function parseApeTagsFromBuffer(data: Buffer): Map<string, string[]> {
  const result = new Map<string, string[]>();
  if (data.length < 32) return result;

  const footerOffset = findApeFooterOffset(data);
  if (footerOffset < 0) return result;

  const tagSize = data.readUInt32LE(footerOffset + 12);
  const itemCount = data.readUInt32LE(footerOffset + 16);
  if (tagSize < 32 || tagSize > data.length) return result;

  const itemsStart = footerOffset + 32 - tagSize;
  if (itemsStart < 0 || itemsStart >= footerOffset) return result;

  let offset = itemsStart;
  for (let i = 0; i < itemCount; i++) {
    if (offset + 8 > footerOffset) break;
    const valSize = data.readUInt32LE(offset);
    /* const flags = */ data.readUInt32LE(offset + 4);
    offset += 8;

    const nullIdx = data.indexOf(0, offset);
    if (nullIdx < 0 || nullIdx >= footerOffset) break;
    const key = data.toString("utf8", offset, nullIdx);
    offset = nullIdx + 1;

    const value = data.toString("utf8", offset, Math.min(offset + valSize, footerOffset));
    offset += valSize;

    if (!result.has(key)) result.set(key, []);
    result.get(key)!.push(value);
  }

  return result;
}

// ── Unit tests for binary helpers ───────────────────────────────────

describe("APEv2 binary helpers", () => {
  it("buildApeTagItems produces correct item count", () => {
    const { items, count } = buildApeTagItems([
      { key: "TITLE", value: "Test" },
      { key: "ARTIST", value: "Artist" },
    ]);
    expect(count).toBe(2);
    expect(items.length).toBeGreaterThan(16);
  });

  it("buildApeTagItems skips empty values", () => {
    const { items, count } = buildApeTagItems([
      { key: "TITLE", value: "Test" },
      { key: "ARTIST", value: "" },
      { key: "ALBUM", value: "" },
    ]);
    expect(count).toBe(1);
  });

  it("buildApeTagItems handles UTF-8 values", () => {
    const { items, count } = buildApeTagItems([
      { key: "TITLE", value: "刺猬乐队 - 幻象波谱星" },
    ]);
    expect(count).toBe(1);
    const nullIdx = items.indexOf(0, 8);
    expect(nullIdx).toBeGreaterThan(8);
    const key = items.toString("utf8", 8, nullIdx);
    expect(key).toBe("TITLE");
    const value = items.toString("utf8", nullIdx + 1);
    expect(value).toBe("刺猬乐队 - 幻象波谱星");
  });

  it("buildApeFooter has correct structure", () => {
    const footer = buildApeFooter(100, 5);
    expect(footer.length).toBe(32);
    expect(footer.toString("ascii", 0, 8)).toBe("APETAGEX");
    expect(footer.readUInt32LE(8)).toBe(2000);
    expect(footer.readUInt32LE(12)).toBe(100);
    expect(footer.readUInt32LE(16)).toBe(5);
    expect(footer.readUInt32LE(20)).toBe(0x80000000);
    for (let i = 24; i < 32; i++) {
      expect(footer[i]).toBe(0);
    }
  });

  it("stripApeTag removes APEv2 tag from end of buffer", () => {
    const audio = Buffer.alloc(512, 0xab);
    const { items, count } = buildApeTagItems([{ key: "TITLE", value: "Song" }]);
    const tagSize = items.length + 32;
    const footer = buildApeFooter(tagSize, count);
    const tagged = Buffer.concat([audio, items, footer]);

    const stripped = stripApeTag(tagged);
    expect(stripped.length).toBe(512);
    expect(stripped.equals(audio)).toBe(true);
  });

  it("stripApeTag returns original buffer if no APETAGEX footer", () => {
    const buf = Buffer.alloc(64, 0xab);
    expect(stripApeTag(buf).equals(buf)).toBe(true);
  });

  it("stripApeTag handles buffer smaller than 32 bytes", () => {
    const buf = Buffer.alloc(16);
    expect(stripApeTag(buf).equals(buf)).toBe(true);
  });

  it("parseApeTagsFromBuffer correctly reads back written tags", () => {
    const entries = [
      { key: "TITLE", value: "Song Title" },
      { key: "ARTIST", value: "Artist Name" },
      { key: "ALBUM", value: "Album Name" },
    ];
    const { items, count } = buildApeTagItems(entries);
    const tagSize = items.length + 32;
    const footer = buildApeFooter(tagSize, count);
    const buf = Buffer.concat([Buffer.alloc(256), items, footer]);

    const tags = parseApeTagsFromBuffer(buf);
    expect(tags.get("TITLE")).toEqual(["Song Title"]);
    expect(tags.get("ARTIST")).toEqual(["Artist Name"]);
    expect(tags.get("ALBUM")).toEqual(["Album Name"]);
  });

  it("parseApeTagsFromBuffer handles duplicate keys", () => {
    const entries = [
      { key: "ARTIST", value: "Artist A" },
      { key: "ARTIST", value: "Artist B" },
    ];
    const { items, count } = buildApeTagItems(entries);
    const tagSize = items.length + 32;
    const footer = buildApeFooter(tagSize, count);
    const buf = Buffer.concat([Buffer.alloc(256), items, footer]);

    const tags = parseApeTagsFromBuffer(buf);
    expect(tags.get("ARTIST")).toEqual(["Artist A", "Artist B"]);
  });
});

// ── Integration tests with writeTags / writeExtraTags ──────────────

describe("APE tag write round-trip (binary verification)", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "ape-test-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writeTags appends APEv2 tag structure to a file", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "test.ape");
    await writeFile(filePath, audioBuffer);

    await writeTags(filePath, {
      title: "Test Song",
      artist: "Test Artist",
      album: "Test Album",
      year: "2024",
      track: "1/10",
      genre: "Rock",
    });

    const data = await readFile(filePath);
    // File should be larger than original (tags appended)
    expect(data.length).toBeGreaterThan(4096);

    // Footer should be present
    const footerOffset = data.length - 32;
    expect(data.toString("ascii", footerOffset, footerOffset + 8)).toBe("APETAGEX");

    // Parse tags back
    const tags = parseApeTagsFromBuffer(data);
    expect(tags.get("TITLE")).toEqual(["Test Song"]);
    expect(tags.get("ARTIST")).toEqual(["Test Artist"]);
    expect(tags.get("ALBUM")).toEqual(["Test Album"]);
    expect(tags.get("DATE")).toEqual(["2024"]);
    expect(tags.get("TRACK")).toEqual(["1/10"]);
    expect(tags.get("GENRE")).toEqual(["Rock"]);
  });

  it("writeTags merges with existing tags (does not drop untouched fields)", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "merge.ape");
    await writeFile(filePath, audioBuffer);

    // Write initial tags
    await writeTags(filePath, {
      title: "Old Title",
      artist: "Old Artist",
      album: "Old Album",
    });

    // Overwrite only title (artist and album should be preserved)
    await writeTags(filePath, {
      title: "New Title",
    });

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);
    expect(tags.get("TITLE")).toEqual(["New Title"]);
    // Artist and album preserved because not touched
    expect(tags.get("ARTIST")).toEqual(["Old Artist"]);
    expect(tags.get("ALBUM")).toEqual(["Old Album"]);
  });

  it("writeTags deletes a field when set to null", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "delete.ape");
    await writeFile(filePath, audioBuffer);

    await writeTags(filePath, {
      title: "Track",
      artist: "Someone",
      album: "Some Album",
    });

    // Delete artist
    await writeTags(filePath, {
      artist: null,
    });

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);
    expect(tags.get("TITLE")).toEqual(["Track"]);
    expect(tags.has("ARTIST")).toBe(false);
    expect(tags.get("ALBUM")).toEqual(["Some Album"]);
  });

  it("writeTags handles multi-value artists", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "multi-artist.ape");
    await writeFile(filePath, audioBuffer);

    await writeTags(filePath, {
      title: "Duet",
      artist: "Primary Artist",
      artists: ["Second Artist", "Third Artist"],
    });

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);
    // ARTIST should have all three (primary + multi-value)
    expect(tags.get("ARTIST")?.length).toBe(3);
  });

  it("writeTags returns full_rewrite outcome", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "outcome.ape");
    await writeFile(filePath, audioBuffer);

    const outcome = await writeTagsWithOutcome(filePath, {
      title: "Test",
    });
    expect(outcome).toBe("full_rewrite");
  });

  it("writeExtraTags preserves standard tags and adds custom ones", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "extra.ape");
    await writeFile(filePath, audioBuffer);

    // Write standard tags
    await writeTags(filePath, {
      title: "Track One",
      artist: "Some Artist",
    });

    // Write extra tags
    await writeExtraTags(filePath, [
      { key: "CUSTOM_TAG", value: "custom-value" },
      { key: "ANOTHER_KEY", value: "hello-world" },
    ]);

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);
    expect(tags.get("TITLE")).toEqual(["Track One"]);
    expect(tags.get("ARTIST")).toEqual(["Some Artist"]);
    expect(tags.get("CUSTOM_TAG")).toEqual(["custom-value"]);
    expect(tags.get("ANOTHER_KEY")).toEqual(["hello-world"]);
  });

  it("writeExtraTags round-trips with readExtraTags format", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "roundtrip.ape");
    await writeFile(filePath, audioBuffer);

    await writeTags(filePath, { title: "Round Trip", artist: "Tester" });
    await writeExtraTags(filePath, [
      { key: "MY_TAG", value: "my-value" },
    ]);

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);
    expect(tags.get("TITLE")).toEqual(["Round Trip"]);
    expect(tags.get("ARTIST")).toEqual(["Tester"]);
    expect(tags.get("MY_TAG")).toEqual(["my-value"]);
  });

  // ── Converter simulation tests ────────────────────────────────────

  it("converter-style partial write preserves other fields (simulates 'filename-to-tags')", async () => {
    // Simulates: file has artist+album, converter extracts only track+title from filename
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "converter-partial.ape");
    await writeFile(filePath, audioBuffer);

    // 1. Pre-existing tags (as if file was already tagged)
    await writeTags(filePath, {
      title: "Original Title",
      artist: "Original Artist",
      album: "Original Album",
      year: "2020",
      genre: "Rock",
    });

    // 2. Converter extracts { track, title } from filename pattern "%{track}% - %{title}%"
    //    matching filename "01 - New Title.ape"
    await writeTags(filePath, {
      track: "1",
      title: "New Title",
    });

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);

    // Title updated
    expect(tags.get("TITLE")).toEqual(["New Title"]);
    // Track added
    expect(tags.get("TRACK")).toEqual(["1"]);
    // Other fields preserved (not touched by converter)
    expect(tags.get("ARTIST")).toEqual(["Original Artist"]);
    expect(tags.get("ALBUM")).toEqual(["Original Album"]);
    expect(tags.get("DATE")).toEqual(["2020"]);
    expect(tags.get("GENRE")).toEqual(["Rock"]);
  });

  it("converter then undo restores original field values (simulates Cmd+Z)", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "converter-undo.ape");
    await writeFile(filePath, audioBuffer);

    // 1. Initial state
    await writeTags(filePath, {
      title: "Original Title",
      artist: "Original Artist",
      album: "Original Album",
    });

    // 2. Converter writes { title: "New Title" } (from filename "01 - New Title.ape")
    await writeTags(filePath, {
      title: "New Title",
    });

    // 3. Undo writes back only { title: "Original Title" }
    //    (undo snapshot only contains fields that were changed)
    await writeTags(filePath, {
      title: "Original Title",
    });

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);

    // Title restored
    expect(tags.get("TITLE")).toEqual(["Original Title"]);
    // Artist and album preserved through both operations
    expect(tags.get("ARTIST")).toEqual(["Original Artist"]);
    expect(tags.get("ALBUM")).toEqual(["Original Album"]);
  });

  it("converter pattern 'Track - Title' extracts and writes correctly", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "track-title.ape");
    await writeFile(filePath, audioBuffer);

    // Pre-existing tags
    await writeTags(filePath, {
      artist: "Some Artist",
      album: "Some Album",
    });

    // Simulate filename-to-tags with pattern "%{track}% - %{title}%"
    // Filename: "01 - Song Title.ape" → parsed: { track: "01", title: "Song Title" }
    await writeTags(filePath, {
      track: "1",
      title: "Song Title",
    });

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);

    expect(tags.get("TITLE")).toEqual(["Song Title"]);
    expect(tags.get("TRACK")).toEqual(["1"]);
    expect(tags.get("ARTIST")).toEqual(["Some Artist"]);
    expect(tags.get("ALBUM")).toEqual(["Some Album"]);
  });

  it("converter 'tags-to-filename' does not alter tags at all", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "tags-to-filename.ape");
    await writeFile(filePath, audioBuffer);

    await writeTags(filePath, {
      title: "Keep Me",
      artist: "Keep Artist",
      album: "Keep Album",
    });

    // 'tags-to-filename' writes NO tags - it only renames files.
    // Simulate by writing with no track field change (empty-ish write)
    // This should preserve all existing tags
    await writeTags(filePath, {});

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);

    expect(tags.get("TITLE")).toEqual(["Keep Me"]);
    expect(tags.get("ARTIST")).toEqual(["Keep Artist"]);
    expect(tags.get("ALBUM")).toEqual(["Keep Album"]);
  });

  it("writeTags with empty fields does not corrupt the file", async () => {
    const audioBuffer = Buffer.alloc(4096, 0x55);
    const filePath = path.join(tmpDir, "empty-fields.ape");
    await writeFile(filePath, audioBuffer);

    // Write some tags
    await writeTags(filePath, { title: "Has Title", artist: "Has Artist" });

    // Write with no meaningful fields (like tags-to-filename converter path)
    await writeTags(filePath, {});

    const data = await readFile(filePath);
    const tags = parseApeTagsFromBuffer(data);
    expect(tags.get("TITLE")).toEqual(["Has Title"]);
    expect(tags.get("ARTIST")).toEqual(["Has Artist"]);
  });

  // ── Header+footer + ID3v1 regression tests ─────────────────────────

  it("stripApeTag handles ID3v1 after APEv2 tag", async () => {
    // File: [audio][old APEv2 header][old items][old footer][ID3v1]
    const audio = Buffer.alloc(256, 0xaa);
    const items = buildApeTagItems([{ key: "TITLE", value: "Old" }]).items;
    const sz = items.length + 32;
    const header = makeApeBlock(sz, 1, 0xA0000000);
    const footer = makeApeBlock(sz, 1, 0x80000000);
    const id3 = Buffer.alloc(128, 0);
    id3.write("TAG", 0, 3, "ascii");
    const tagged = Buffer.concat([audio, header, items, footer, id3]);

    const body = stripApeTag(tagged);
    expect(body.length).toBe(256);
    expect(body.equals(audio)).toBe(true);
    expect(body.indexOf("APETAGEX")).toBe(-1);
    expect(body.indexOf("TAG")).toBe(-1); // ID3v1 should also be stripped
  });

  it("writeTags replaces tags when file has both APEv2 and ID3v1", async () => {
    // Build valid APE file with [MAC][old APEv2 header+items+footer][ID3v1]
    const audioSize = 4096;
    const d = Buffer.alloc(52);
    d.write("MAC ", 0, 4, "ascii");
    d.writeUInt32LE(2000000, 4);
    d.writeUInt32LE(52, 8);
    d.writeUInt32LE(24, 12);
    d.writeUInt32LE(0, 16);
    d.writeUInt32LE(0, 20);
    d.writeUInt32LE(audioSize, 24);
    d.writeUInt32LE(0, 28);
    d.writeUInt32LE(0, 32);
    const h = Buffer.alloc(24);
    h.writeUInt16LE(0, 0);
    h.writeUInt16LE(0, 2);
    h.writeUInt32LE(4608, 4);
    h.writeUInt32LE(0, 8);
    h.writeUInt32LE(1, 12);
    h.writeUInt16LE(16, 16);
    h.writeUInt16LE(2, 18);
    h.writeUInt32LE(44100, 20);
    const audio = Buffer.alloc(audioSize, 0x55);
    const oldItems = buildApeTagItems([
      { key: "TITLE", value: "Old Title" },
      { key: "ARTIST", value: "Old Artist" },
    ]).items;
    const oldCnt = 2;
    const oldSz = oldItems.length + 32;
    // Header+footer old tag
    const oldHeader = makeApeBlock(oldSz, oldCnt, 0xA0000000);
    const oldFooter = makeApeBlock(oldSz, oldCnt, 0x80000000);
    // ID3v1
    const id3 = Buffer.alloc(128, 0);
    id3.write("TAG", 0, 3, "ascii");
    id3.write("v1 Title", 63, 8, "utf8");

    const filePath = path.join(tmpDir, "id3-apetag.ape");
    await writeFile(filePath, Buffer.concat([d, h, audio, oldHeader, oldItems, oldFooter, id3]));

    // Write new tags
    await writeTags(filePath, { title: "New Title" });

    // Verify with music-metadata
    const { parseFile } = await import("music-metadata");
    const meta = await parseFile(filePath);
    expect(meta.common.title).toBe("New Title");
    // Artist preserved (merge)
    expect(meta.common.artist).toBe("Old Artist");

    // Verify no stale APEv2 remains
    const raw = await readFile(filePath);
    let apexCount = 0;
    for (let i = 0; i <= raw.length - 32; i++) {
      if (raw.toString("ascii", i, i + 8) === "APETAGEX") apexCount++;
    }
    expect(apexCount).toBe(1); // only the new footer
    // Verify ID3v1 was also stripped
    expect(raw.toString("ascii", raw.length - 128, raw.length - 125)).not.toBe("TAG");
  });

  // ── Header+footer regression tests ─────────────────────────────────

  /** Helper to build an APEv2 header or footer block (32 bytes). */
  function makeApeBlock(tagSize: number, count: number, flags: number): Buffer {
    const b = Buffer.alloc(32);
    b.write("APETAGEX", 0, 8, "ascii");
    b.writeUInt32LE(2000, 8);
    b.writeUInt32LE(tagSize, 12);
    b.writeUInt32LE(count, 16);
    b.writeUInt32LE(flags, 20);
    return b;
  }

  it("stripApeTag removes APEv2 header+items+footer (regression: stale header bug)", async () => {
    // Build a buffer with header+items+footer layout
    const audio = Buffer.alloc(256, 0xaa);
    const items = Buffer.concat([
      buildApeTagItems([{ key: "TITLE", value: "Old" }]).items,
    ]);
    const count = 1;
    const tagSize = items.length + 32;
    const header = makeApeBlock(tagSize, count, 0xE0000000);  // isHeader|containsHeader|containsFooter
    const footer = makeApeBlock(tagSize, count, 0x80000000);  // containsHeader
    const tagged = Buffer.concat([audio, header, items, footer]);

    const body = stripApeTag(tagged);
    expect(body.length).toBe(256);
    expect(body.equals(audio)).toBe(true);
    expect(body.indexOf("APETAGEX")).toBe(-1);
  });

  it("writeTags replaces existing header+footer APEv2 tag without leaving stale header", async () => {
    // Build a minimally valid APE-like file with a header+footer tag
    const audioSize = 4096;
    const d = Buffer.alloc(52);
    d.write("MAC ", 0, 4, "ascii");
    d.writeUInt32LE(2000000, 4);
    d.writeUInt32LE(52, 8);
    d.writeUInt32LE(24, 12);
    d.writeUInt32LE(0, 16);
    d.writeUInt32LE(0, 20);
    d.writeUInt32LE(audioSize, 24);
    d.writeUInt32LE(0, 28);
    d.writeUInt32LE(0, 32);
    const h = Buffer.alloc(24);
    h.writeUInt16LE(0, 0);
    h.writeUInt16LE(0, 2);
    h.writeUInt32LE(4608, 4);
    h.writeUInt32LE(0, 8);
    h.writeUInt32LE(1, 12);
    h.writeUInt16LE(16, 16);
    h.writeUInt16LE(2, 18);
    h.writeUInt32LE(44100, 20);
    const audio = Buffer.alloc(audioSize, 0x55);

    // Initial tag: header+items+footer (like a real APE file tagged by another tool)
    const oldItems = buildApeTagItems([
      { key: "TITLE", value: "Old Title" },
      { key: "ARTIST", value: "Old Artist" },
    ]).items;
    const oldCount = 2;
    const oldTagSize = oldItems.length + 32;
    const oldHeader = makeApeBlock(oldTagSize, oldCount, 0xE0000000);
    const oldFooter = makeApeBlock(oldTagSize, oldCount, 0x80000000);

    const filePath = path.join(tmpDir, "stale-header.ape");
    await writeFile(filePath, Buffer.concat([d, h, audio, oldHeader, oldItems, oldFooter]));

    // Write new tags (like the sidebar editor would)
    await writeTags(filePath, { title: "New Title" });

    // Verify with music-metadata that the tags are correctly read back
    const { parseFile } = await import("music-metadata");
    const meta = await parseFile(filePath);
    expect(meta.common.title).toBe("New Title");
    // Old artist should be preserved because fields.artist was undefined (merge semantics)
    expect(meta.common.artist).toBe("Old Artist");

    // Also verify no stale header remains in the file
    const data = await readFile(filePath);
    // Count APETAGEX occurrences — should only be the footer (at end)
    let count = 0;
    for (let i = 0; i < data.length - 7; i++) {
      if (data.toString("ascii", i, i + 8) === "APETAGEX") count++;
    }
    expect(count).toBe(1);
  });
});
