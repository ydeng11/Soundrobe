import { describe, it, expect, beforeEach, afterEach } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";
import { readDirectory } from "../../electron/handlers/directory";

describe("readDirectory — unparseable file", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "dir-test-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("uses stat.size when music-metadata cannot parse a file", async () => {
    // Create a valid-looking .flac file with garbage payload (will fail parse)
    const filePath = path.join(tmpDir, "corrupt.flac");
    const size = 1234567;
    const buf = Buffer.alloc(size);
    // Write a minimal FLAC header so it passes extension check, but
    // music-metadata will still fail on the corrupt metadata blocks.
    buf.write("fLaC", 0, "ascii");
    fs.writeFileSync(filePath, buf);

    const result = await readDirectory(tmpDir);
    expect(result.tracks.length).toBe(1);
    const track = result.tracks[0];
    // sizeBytes must match actual file size, not 0
    expect(track.sizeBytes).toBe(size);
    expect(track.codec).toBe("unknown");
    expect(track.title).toBe("corrupt.flac");
  });

  it("reports correct size for multiple unparseable files", async () => {
    const sizes = [999999, 888888, 777777];
    for (let i = 0; i < sizes.length; i++) {
      const fp = path.join(tmpDir, `corrupt${i}.flac`);
      const buf = Buffer.alloc(sizes[i]);
      buf.write("fLaC", 0, "ascii");
      fs.writeFileSync(fp, buf);
    }

    const result = await readDirectory(tmpDir);
    expect(result.tracks.length).toBe(sizes.length);
    for (let i = 0; i < sizes.length; i++) {
      expect(result.tracks[i].sizeBytes).toBe(sizes[i]);
    }
  });

  it("returns accurate size for parseable .flac files", async () => {
    // Create a minimal valid FLAC using the same pattern as writer tests
    const fp = path.join(tmpDir, "valid.flac");
    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));

    // STREAMINFO block (type 0, length 34)
    const si = Buffer.alloc(34);
    si.writeUInt16BE(4096, 0);
    si.writeUInt16BE(4096, 2);
    const siHeader = Buffer.alloc(4);
    siHeader[0] = 0x80; // isLast, type 0
    siHeader[1] = (si.length >> 16) & 0xff;
    siHeader[2] = (si.length >> 8) & 0xff;
    siHeader[3] = si.length & 0xff;
    parts.push(siHeader, si);

    // Append a minimal audio frame so the file is valid-ish
    const frame = Buffer.alloc(100);
    parts.push(frame);

    fs.writeFileSync(fp, Buffer.concat(parts));
    const expectedSize = fs.statSync(fp).size;

    const result = await readDirectory(tmpDir);
    expect(result.tracks.length).toBe(1);
    expect(result.tracks[0].sizeBytes).toBe(expectedSize);
  });
});
