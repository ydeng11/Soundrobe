/**
 * Tests for lyrics.ts — encoding fixer + LyricsClient.
 */

import { describe, it, expect, vi, beforeAll, beforeEach, afterEach } from "vitest";
import {
  normalizeLyricsEncoding,
  readLocalLyrics,
  LyricsClient,
} from "../../electron/handlers/lyrics";
import { existsSync, unlinkSync, writeFileSync, mkdirSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

// Helper: generate known shift-jis bytes for longer Japanese text
function shiftJisBytes(text: string): Buffer {
  // Node 21+ has TextEncoder for shift-jis
  try {
    const enc = new TextEncoder("shift-jis");
    return Buffer.from(enc.encode(text));
  } catch {
    // Fallback: map common characters manually for tests
    const map: Record<string, number[]> = {
      "こ": [0x82, 0xb1],
      "ん": [0x82, 0xf1],
      "に": [0x82, 0xc9],
      "ち": [0x82, 0xbf],
      "は": [0x82, 0xcd],
      "世": [0x90, 0xa2],
      "界": [0x8a, 0x45],
      "、": [0x81, 0x41],
      "音": [0x89, 0xf6],
      "楽": [0x97, 0x6d],
      "を": [0x82, 0xed],
      "聴": [0x92, 0xb7],
      "き": [0x82, 0xab],
      "ま": [0x82, 0xdc],
      "し": [0x82, 0xb5],
      "ょ": [0x82, 0xe1],
      "う": [0x82, 0xa4],
      "日": [0x93, 0xfa],
      "い": [0x82, 0xa2],
      "天": [0x93, 0x73],
      "気": [0x8b, 0x43],
      "で": [0x82, 0xc5],
      "す": [0x82, 0xb7],
      "ね": [0x82, 0xcb],
    };
    const parts: number[] = [];
    for (const ch of text) {
      const bytes = map[ch];
      if (bytes) parts.push(...bytes);
      else parts.push(...Buffer.from(ch, "utf8")); // fallback for ASCII
    }
    return Buffer.from(parts);
  }
}

// Helper: generate known GBK bytes
function gbkBytes(text: string): Buffer {
  try {
    const enc = new TextEncoder("gbk");
    return Buffer.from(enc.encode(text));
  } catch {
    // Fallback map for common chars
    const map: Record<string, number[]> = {
      "你": [0xc4, 0xe3],
      "好": [0xba, 0xc3],
      "世": [0xca, 0xc0],
      "界": [0xbd, 0xe7],
      "音": [0xd2, 0xf4],
      "楽": [0xc0, 0xd6],
    };
    const parts: number[] = [];
    for (const ch of text) {
      const bytes = map[ch];
      if (bytes) parts.push(...bytes);
      else parts.push(...Buffer.from(ch, "utf8"));
    }
    return Buffer.from(parts);
  }
}

// ── normalizeLyricsEncoding ────────────────────────────────────────

describe("normalizeLyricsEncoding", () => {
  it("passes through clean UTF-8", () => {
    const input = "Hello world\nLine two";
    const result = normalizeLyricsEncoding(Buffer.from(input, "utf8"));
    expect(result).toBe(input);
  });

  it("handles empty buffer", () => {
    expect(normalizeLyricsEncoding(Buffer.alloc(0))).toBe("");
  });

  it("detects UTF-16LE BOM", () => {
    const input = "Hello";
    const utf16le = Buffer.concat([
      Buffer.from([0xff, 0xfe]),
      Buffer.from(input, "utf16le"),
    ]);
    const result = normalizeLyricsEncoding(utf16le);
    expect(result).toBe(input);
  });

  it("detects UTF-16BE BOM", () => {
    const input = "Hello";
    const beBuf = Buffer.alloc(input.length * 2);
    for (let i = 0; i < input.length; i++) {
      beBuf.writeUInt16BE(input.charCodeAt(i), i * 2);
    }
    const utf16be = Buffer.concat([Buffer.from([0xfe, 0xff]), beBuf]);
    const result = normalizeLyricsEncoding(utf16be);
    expect(result).toBe(input);
  });

  it("detects shift-jis encoded text", () => {
    // Longer distinctive Japanese text for reliable detection
    const text = "こんにちは、世界！音楽を聴きましょう。";
    const bytes = shiftJisBytes(text);
    const result = normalizeLyricsEncoding(bytes);
    expect(result).toBe(text);
  });

  it("detects GBK encoded text", () => {
    // Longer distinctive Chinese text for reliable detection
    const text = "你好世界！音乐非常好听。";
    const bytes = gbkBytes(text);
    const result = normalizeLyricsEncoding(bytes);
    expect(result).toBe(text);
  });

  it("falls back to UTF-8 when detection is uncertain", () => {
    const input = "Just some ASCII text";
    const result = normalizeLyricsEncoding(Buffer.from(input, "ascii"));
    expect(result).toBe(input);
  });

  it("handles mixed ASCII and CJK in shift-jis", () => {
    const text = "[00:01.00] こんにちは  Hello";
    const bytes = shiftJisBytes(text);
    const result = normalizeLyricsEncoding(bytes);
    expect(result).toBe(text);
  });
});

// ── readLocalLyrics ────────────────────────────────────────────────

describe("readLocalLyrics", () => {
  const tmpDir = join(tmpdir(), "auto-tagger-lyrics-test-" + Date.now());
  const audioPath = join(tmpDir, "song.mp3");

  beforeAll(() => {
    mkdirSync(tmpDir, { recursive: true });
  });

  // Clean up all .lrc and .txt files between tests
  afterEach(() => {
    for (const f of ["song.lrc", "song.txt"]) {
      const p = join(tmpDir, f);
      if (existsSync(p)) unlinkSync(p);
    }
  });

  it("returns null when no .lrc or .txt exists", () => {
    expect(readLocalLyrics(audioPath)).toBeNull();
  });

  it("reads a UTF-8 .lrc file", () => {
    const content = "[00:01.00]Test lyric line";
    writeFileSync(join(tmpDir, "song.lrc"), content, "utf8");
    const result = readLocalLyrics(audioPath);
    expect(result).toBe(content);
  });

  it("reads and converts a UTF-16LE .lrc file (with BOM)", () => {
    const content = "Plain text lyrics";
    const utf16le = Buffer.concat([
      Buffer.from([0xff, 0xfe]),
      Buffer.from(content, "utf16le"),
    ]);
    writeFileSync(join(tmpDir, "song.lrc"), utf16le);
    const result = readLocalLyrics(audioPath);
    expect(result).toBe(content);
  });

  it("reads and converts a shift-jis .lrc file", () => {
    // Longer Japanese text ensures jschardet identifies shift-jis
    const text = "[00:00.00]こんにちは、世界！";
    const bytes = shiftJisBytes(text);
    writeFileSync(join(tmpDir, "song.lrc"), bytes);
    const result = readLocalLyrics(audioPath);
    expect(result).toBe(text);
  });

  it("prefers .lrc over .txt", () => {
    const lrcContent = "LRC content";
    const txtContent = "TXT content";
    writeFileSync(join(tmpDir, "song.lrc"), lrcContent, "utf8");
    writeFileSync(join(tmpDir, "song.txt"), txtContent, "utf8");
    const result = readLocalLyrics(audioPath);
    expect(result).toBe(lrcContent);
  });

  it("reads .txt when no .lrc exists", () => {
    const content = "Text file lyrics only";
    writeFileSync(join(tmpDir, "song.txt"), content, "utf8");
    const result = readLocalLyrics(audioPath);
    expect(result).toBe(content);
  });
});

// ── LyricsClient ───────────────────────────────────────────────────

describe("LyricsClient", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("returns null when trackName is empty", async () => {
    const client = new LyricsClient();
    const result = await client.fetchLyrics("", "Artist");
    expect(result).toBeNull();
  });

  it("returns null when artistName is empty", async () => {
    const client = new LyricsClient();
    const result = await client.fetchLyrics("Track", "");
    expect(result).toBeNull();
  });

  it("returns parsed syncedLyrics on success", async () => {
    const mockJson = {
      plainLyrics: "Plain text fallback",
      syncedLyrics: "[00:01.00]Synced LRC line",
    };
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve(mockJson),
    });

    const client = new LyricsClient({ baseUrl: "http://test.api" });
    const result = await client.fetchLyrics("Some Song", "Some Artist");
    expect(result).toBe("[00:01.00]Synced LRC line");

    // Verify URL
    const callUrl = (global.fetch as ReturnType<typeof vi.fn>).mock.calls[0][0];
    expect(callUrl).toContain("track_name=Some+Song");
    expect(callUrl).toContain("artist_name=Some+Artist");
    expect(callUrl).toContain("http://test.api/get");
  });

  it("falls back to plainLyrics when syncedLyrics is absent", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ plainLyrics: "Plain text only" }),
    });

    const client = new LyricsClient({ baseUrl: "http://test.api" });
    const result = await client.fetchLyrics("Track", "Artist");
    expect(result).toBe("Plain text only");
  });

  it("sends optional duration param", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ plainLyrics: "abc" }),
    });

    const client = new LyricsClient({ baseUrl: "http://test.api" });
    await client.fetchLyrics("Track", "Artist", "Album", 245);
    const callUrl = (global.fetch as ReturnType<typeof vi.fn>).mock.calls[0][0];
    expect(callUrl).toContain("duration=245");
    expect(callUrl).toContain("album_name=Album");
  });

  it("returns null on non-ok response", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 404,
    });

    const client = new LyricsClient({ baseUrl: "http://test.api" });
    const result = await client.fetchLyrics("Track", "Artist");
    expect(result).toBeNull();
  });

  it("returns null on network error", async () => {
    global.fetch = vi.fn().mockRejectedValue(new Error("Network failure"));

    const client = new LyricsClient({ baseUrl: "http://test.api" });
    const result = await client.fetchLyrics("Track", "Artist");
    expect(result).toBeNull();
  });

  it("returns null on empty lyrics response", async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ plainLyrics: "", syncedLyrics: "" }),
    });

    const client = new LyricsClient({ baseUrl: "http://test.api" });
    const result = await client.fetchLyrics("Track", "Artist");
    expect(result).toBeNull();
  });
});
