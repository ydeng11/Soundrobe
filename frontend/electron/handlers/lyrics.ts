/**
 * Lyrics client and encoding fixer.
 *
 * Provides:
 *  - `normalizeLyricsEncoding(buffer)` — detect and convert lyrics bytes to a
 *    clean UTF-8 string, handling BOM and common non-UTF‑8 encodings.
 *  - `LyricsClient` — downloads LRC/plain-text lyrics from a lyrics API.
 *
 * The encoding fixer is applied to ALL lyrics sources (local files + downloads).
 */

import { existsSync, readFileSync } from "node:fs";
import * as jschardet from "jschardet";

// ── Encoding detection & fixer ──────────────────────────────────────

/**
 * Detect encoding from bytes and return a clean UTF‑8 string.
 *
 * Detection order:
 *  1. BOM (UTF‑16LE / UTF‑16BE)
 *  2. jschardet universal detection
 *  3. Fallback to UTF‑8 as-is
 */
export function normalizeLyricsEncoding(data: Buffer): string {
  if (data.length === 0) return "";

  // --- BOM detection ---
  if (data.length >= 2 && data[0] === 0xff && data[1] === 0xfe) {
    // UTF‑16LE with BOM
    return data.subarray(2).toString("utf16le");
  }
  if (data.length >= 2 && data[0] === 0xfe && data[1] === 0xff) {
    // UTF‑16BE with BOM → swap bytes, then decode as UTF‑16LE
    return Buffer.from(data.subarray(2)).swap16().toString("utf16le");
  }

  // --- jschardet detection for non-BOM bytes ---
  const detected = jschardet.detect(data);
  const rawEncoding = detected?.encoding ?? "";
  const encoding = rawEncoding.toLowerCase().replace(/[_-]/g, "");
  const confidence = detected?.confidence ?? 0;

  // If detected as valid UTF‑8 (or ASCII), return directly as UTF‑8
  if (
    encoding === "utf8" ||
    encoding === "utf-8" ||
    encoding === "ascii" ||
    encoding === ""
  ) {
    return data.toString("utf8");
  }

  // Map common jschardet labels to TextDecoder labels
  const encodingMap: Record<string, string> = {
    shiftjis: "shift-jis",
    shift_jis: "shift-jis",
    sjis: "shift-jis",
    gbk: "gbk",
    gb2312: "gbk",
    gb18030: "gb18030",
    big5: "big5",
    eucjp: "euc-jp",
    euckr: "euc-kr",
    "euc-kr": "euc-kr",
    iso2022jp: "iso-2022-jp",
    iso88591: "iso-8859-1",
    "iso-8859-1": "iso-8859-1",
    latin1: "iso-8859-1",
    windows1252: "windows-1252",
    "windows-1252": "windows-1252",
    koi8r: "koi8-r",
    koi8_r: "koi8-r",
  };

  const label = encodingMap[encoding];
  if (label) {
    try {
      const decoder = new TextDecoder(label, { fatal: false });
      const result = decoder.decode(data);

      // If confidence is low, or the decoded result still has replacement
      // chars (indicating wrong encoding), try CJK fallbacks.
      const hasReplacement = result.includes("\ufffd");
      const isWesternEncoding =
        encoding === "iso88591" ||
        encoding === "latin1" ||
        encoding === "windows1252";

      if (
        hasReplacement ||
        (isWesternEncoding && confidence < 0.95)
      ) {
        // Try common CJK encodings in order
        const fallbacks = ["shift-jis", "gbk", "big5", "euc-kr", "euc-jp"];
        for (const fb of fallbacks) {
          try {
            const fbDecoder = new TextDecoder(fb, { fatal: false });
            const fbResult = fbDecoder.decode(data);
            // If no replacement chars, it's likely correct
            if (!fbResult.includes("\ufffd")) {
              return fbResult;
            }
          } catch {
            // skip unsupported labels
          }
        }
      }

      return result;
    } catch {
      // TextDecoder doesn't support this label — fall through
    }
  }

  // Fallback: return as UTF‑8 (best effort)
  return data.toString("utf8");
}

/**
 * Read a local `.lrc` or `.txt` lyric file next to the given audio file,
 * applying encoding detection/fix. Returns the lyric text, or null if no
 * local file exists.
 */
export function readLocalLyrics(filePath: string): string | null {
  for (const ext of [".lrc", ".txt"]) {
    const lyricsPath = filePath.replace(/\.[^.]+$/, ext);
    if (!existsSync(lyricsPath)) continue;
    const data = readFileSync(lyricsPath);
    return normalizeLyricsEncoding(data);
  }
  return null;
}

// ── Lyrics API client ───────────────────────────────────────────────

const DEFAULT_LYRICS_API_BASE = "https://lrclib.net/api";
const USER_AGENT = "auto-tagger/0.1.0 (https://github.com/auto-tagger)";

export class LyricsClient {
  private baseUrl: string;
  private userAgent: string;

  constructor(options?: { baseUrl?: string; userAgent?: string }) {
    this.baseUrl = (options?.baseUrl ?? DEFAULT_LYRICS_API_BASE).replace(
      /\/+$/,
      "",
    );
    this.userAgent = options?.userAgent ?? USER_AGENT;
  }

  /**
   * Fetch lyrics for a track from the configured API.
   *
   * GET `<baseUrl>/get?track_name=<track>&artist_name=<artist>`
   * with optional `album_name` and `duration`.
   *
   * Returns cleaned UTF‑8 lyric text, or null on any failure.
   */
  async fetchLyrics(
    trackName: string,
    artistName: string,
    albumName?: string,
    duration?: number,
  ): Promise<string | null> {
    if (!trackName || !artistName) return null;

    const params = new URLSearchParams();
    params.set("track_name", trackName);
    params.set("artist_name", artistName);
    if (albumName) params.set("album_name", albumName);
    if (duration != null && duration > 0) {
      params.set("duration", String(Math.round(duration)));
    }

    const url = `${this.baseUrl}/get?${params.toString()}`;

    try {
      const response = await fetch(url, {
        headers: { "User-Agent": this.userAgent },
        signal: AbortSignal.timeout(10_000),
      });

      if (!response.ok) {
        console.debug(
          `[lyrics] Not found: "${trackName}" by "${artistName}" (HTTP ${response.status})`,
        );
        return null;
      }

      // The API returns JSON with "plainLyrics" and/or "syncedLyrics" fields
      const body = (await response.json()) as {
        plainLyrics?: string;
        syncedLyrics?: string;
        instrumental?: boolean;
      };

      // Prefer synced LRC, fall back to plain text
      const raw =
        (body.syncedLyrics ?? body.plainLyrics ?? "") as string;

      if (!raw) {
        const reason = body.instrumental ? "instrumental track" : "no lyrics in response";
        console.debug(
          `[lyrics] ${reason}: "${trackName}" by "${artistName}"`,
        );
        return null;
      }

      // Normalize encoding (the API returns UTF‑8 JSON, but we decode defensively)
      return normalizeLyricsEncoding(Buffer.from(raw, "utf8"));
    } catch (err) {
      console.warn(
        `[lyrics] Fetch failed for "${trackName}" by "${artistName}":`,
        (err as Error).message,
      );
      return null;
    }
  }
}
