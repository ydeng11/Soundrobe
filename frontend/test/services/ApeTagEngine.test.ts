import { describe, expect, it } from "vitest";
import {
  buildApeFooter,
  buildApeTagItems,
  locateApeTag,
  parseApeTag,
  parseApeTagItems,
  stripApeTag,
} from "../../electron/services/ApeTagEngine";

function makeApeBlock(tagSize: number, itemCount: number, flags: number): Buffer {
  const block = Buffer.alloc(32);
  block.write("APETAGEX", 0, 8, "ascii");
  block.writeUInt32LE(2000, 8);
  block.writeUInt32LE(tagSize, 12);
  block.writeUInt32LE(itemCount, 16);
  block.writeUInt32LE(flags, 20);
  return block;
}

function id3v1Tail(): Buffer {
  const tail = Buffer.alloc(128, 0);
  tail.write("TAG", 0, 3, "ascii");
  return tail;
}

function minimalMacAudio(audioSize: number): Buffer {
  const descriptor = Buffer.alloc(52);
  descriptor.write("MAC ", 0, 4, "ascii");
  descriptor.writeUInt32LE(2000000, 4);
  descriptor.writeUInt32LE(52, 8);
  descriptor.writeUInt32LE(24, 12);
  descriptor.writeUInt32LE(0, 16);
  descriptor.writeUInt32LE(0, 20);
  descriptor.writeUInt32LE(audioSize, 24);
  descriptor.writeUInt32LE(0, 28);
  descriptor.writeUInt32LE(0, 32);

  const header = Buffer.alloc(24);
  header.writeUInt32LE(4608, 4);
  header.writeUInt32LE(0, 8);
  header.writeUInt32LE(1, 12);
  header.writeUInt32LE(44100, 20);

  return Buffer.concat([descriptor, header, Buffer.alloc(audioSize, 0x55)]);
}

describe("ApeTagEngine locateApeTag", () => {
  it("locates a footer-only tag and strips from the tag item start", () => {
    const audio = Buffer.alloc(256, 0xaa);
    const { items, count } = buildApeTagItems([{ key: "TITLE", value: "Song" }]);
    const footer = buildApeFooter(items.length + 32, count);
    const data = Buffer.concat([audio, items, footer]);

    const located = locateApeTag(data);

    expect(located?.position).toBe(audio.length);
    expect(located?.footerOffset).toBe(audio.length + items.length);
    expect(located?.tagSize).toBe(items.length + 32);
    expect(located?.itemCount).toBe(1);
    expect(located?.hasHeader).toBe(false);
    expect(located?.hasFooter).toBe(true);
    expect(stripApeTag(data).equals(audio)).toBe(true);
  });

  it("locates a header+footer tag and strips from the header", () => {
    const audio = Buffer.alloc(256, 0xaa);
    const { items, count } = buildApeTagItems([{ key: "TITLE", value: "Old" }]);
    const tagSize = items.length + 32;
    const header = makeApeBlock(tagSize, count, 0xe0000000);
    const footer = makeApeBlock(tagSize, count, 0x80000000);
    const data = Buffer.concat([audio, header, items, footer]);

    const located = locateApeTag(data);

    expect(located?.position).toBe(audio.length);
    expect(located?.hasHeader).toBe(true);
    expect(located?.hasFooter).toBe(true);
    expect(stripApeTag(data).equals(audio)).toBe(true);
  });

  it("uses footer-relative item start when ID3v1 trails the APEv2 footer", () => {
    const audio = Buffer.alloc(256, 0xaa);
    const { items, count } = buildApeTagItems([{ key: "TITLE", value: "Old" }]);
    const tagSize = items.length + 32;
    const footer = buildApeFooter(tagSize, count);
    const data = Buffer.concat([audio, items, footer, id3v1Tail()]);

    const tag = parseApeTag(data);

    expect(tag.items).toEqual([{ key: "TITLE", value: "Old", type: "text", readonly: false }]);
    expect(stripApeTag(data).equals(audio)).toBe(true);
  });

  it("strips from MAC audio end when a gap precedes the first APETAGEX marker", () => {
    const audio = minimalMacAudio(4096);
    const gap = Buffer.alloc(15, 0);
    const { items, count } = buildApeTagItems([{ key: "TITLE", value: "Gap" }]);
    const tagSize = items.length + 32;
    const header = makeApeBlock(tagSize, count, 0xe0000000);
    const footer = makeApeBlock(tagSize, count, 0x80000000);
    const data = Buffer.concat([audio, gap, header, items, footer]);

    const located = locateApeTag(data);

    expect(located?.position).toBe(audio.length);
    expect(stripApeTag(data).equals(audio)).toBe(true);
  });

  it("ignores malformed signatures instead of throwing", () => {
    const data = Buffer.alloc(64, 0);
    data.write("APETAGEX", 10, 8, "ascii");
    data.writeUInt32LE(16, 22);
    data.writeUInt32LE(999999, 26);

    expect(() => locateApeTag(data)).not.toThrow();
    expect(locateApeTag(data)).toBeNull();
    expect(parseApeTagItems(data)).toEqual([]);
  });

  it("reports impossible tag sizes without exposing parsed items", () => {
    const data = Buffer.concat([Buffer.alloc(64, 0xaa), makeApeBlock(16, 1, 0x80000000)]);

    const tag = parseApeTag(data);

    expect(locateApeTag(data)).toBeNull();
    expect(tag.items).toEqual([]);
    expect(tag.issues).toContain("invalid_tag_size");
  });

  it("reports impossible item counts without exposing parsed items", () => {
    const data = Buffer.concat([Buffer.alloc(64, 0xaa), makeApeBlock(32, 100_001, 0x80000000)]);

    const tag = parseApeTag(data);

    expect(locateApeTag(data)).toBeNull();
    expect(tag.items).toEqual([]);
    expect(tag.issues).toContain("invalid_item_count");
  });

  it("reports out-of-bounds item regions without throwing", () => {
    const data = Buffer.alloc(128, 0);
    makeApeBlock(64, 0, 0x80000000).copy(data, 10);

    const tag = parseApeTag(data);

    expect(locateApeTag(data)).toBeNull();
    expect(tag.items).toEqual([]);
    expect(tag.issues).toContain("invalid_item_region");
  });

  it("reports header-only tags as missing a footer", () => {
    const data = Buffer.concat([Buffer.alloc(64, 0xaa), makeApeBlock(32, 0, 0x20000000)]);

    const tag = parseApeTag(data);

    expect(locateApeTag(data)).toBeNull();
    expect(tag.items).toEqual([]);
    expect(tag.issues).toContain("missing_footer");
  });
});

describe("ApeTagEngine parse and serialize", () => {
  it("round-trips UTF-8 text items and preserves duplicate keys", () => {
    const { items, count } = buildApeTagItems([
      { key: "TITLE", value: "我们最后的话" },
      { key: "ARTIST", value: "刺猬" },
      { key: "ARTIST", value: "Hedgehog" },
    ]);
    const data = Buffer.concat([Buffer.alloc(128), items, buildApeFooter(items.length + 32, count)]);

    expect(parseApeTagItems(data)).toEqual([
      { key: "TITLE", value: "我们最后的话" },
      { key: "ARTIST", value: "刺猬" },
      { key: "ARTIST", value: "Hedgehog" },
    ]);
  });

  it("reports validation issues and returns no items for invalid UTF-8 text", () => {
    const key = Buffer.from("TITLE\0", "utf8");
    const value = Buffer.from([0xc3, 0x28]);
    const itemHeader = Buffer.alloc(8);
    itemHeader.writeUInt32LE(value.length, 0);
    itemHeader.writeUInt32LE(0x20000000, 4);
    const items = Buffer.concat([itemHeader, key, value]);
    const data = Buffer.concat([Buffer.alloc(64), items, buildApeFooter(items.length + 32, 1)]);

    const tag = parseApeTag(data);

    expect(tag.items).toEqual([]);
    expect(tag.issues).toContain("invalid_utf8");
  });

  it("does not expose binary APE items as text items", () => {
    const key = Buffer.from("COVER ART (FRONT)\0", "utf8");
    const value = Buffer.from([1, 2, 3, 4]);
    const itemHeader = Buffer.alloc(8);
    itemHeader.writeUInt32LE(value.length, 0);
    itemHeader.writeUInt32LE(0x40000000, 4);
    const items = Buffer.concat([itemHeader, key, value]);
    const data = Buffer.concat([Buffer.alloc(64), items, buildApeFooter(items.length + 32, 1)]);

    const tag = parseApeTag(data);

    expect(tag.items).toEqual([]);
    expect(tag.issues).toContain("unsupported_item_type");
  });

  it("reports missing item key terminators without throwing", () => {
    const keyWithoutTerminator = Buffer.from("TITLE", "utf8");
    const value = Buffer.from("Song", "utf8");
    const itemHeader = Buffer.alloc(8);
    itemHeader.writeUInt32LE(value.length, 0);
    itemHeader.writeUInt32LE(0x20000000, 4);
    const items = Buffer.concat([itemHeader, keyWithoutTerminator, value]);
    const data = Buffer.concat([Buffer.alloc(64), items, buildApeFooter(items.length + 32, 1)]);

    const tag = parseApeTag(data);

    expect(tag.items).toEqual([]);
    expect(tag.issues).toContain("missing_key_terminator");
  });
});
