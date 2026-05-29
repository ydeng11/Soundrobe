/**
 * Smoke test for lyrics download + encoding fixer.
 *
 * Uses the real album at /Volumes/downloads/蛋堡/2009-Winter Sweet[flac]
 * to verify the full pipeline:
 *   read metadata → download lyrics → fix encoding → write to tag → verify
 *
 * Run with: SMOKE_TEST=1 npx vitest run test/handlers/lyrics-smoke.test.ts
 * or directly: node --experimental-vm-modules test/handlers/lyrics-smoke.test.ts
 */

import { describe, it, expect } from "vitest";

const SMOKE_ALBUM = "/Volumes/downloads/蛋堡/2009-Winter Sweet[flac]";
const RUN_SMOKE = process.env.SMOKE_TEST === "1" && process.env.CI !== "true";

// Skip all tests unless SMOKE_TEST=1
const itSmoke = RUN_SMOKE ? it : it.skip;

describe("Lyrics smoke test — 蛋堡 Winter Sweet", () => {
  itSmoke("downloadAlbumLyrics fetches and writes lyrics for all tracks", async () => {
    const { downloadAlbumLyrics } = await import(
      "../../electron/handlers/auto-tag"
    );
    const { normalizeLyricsEncoding } = await import(
      "../../electron/handlers/lyrics"
    );
    const { readTrackMetadata } = await import(
      "../../electron/handlers/tracks"
    );

    // Step 1: run the download
    const count = await downloadAlbumLyrics(SMOKE_ALBUM);
    expect(count).toBeGreaterThan(0);
    console.log(`✓ downloadAlbumLyrics returned ${count} track(s)`);

    // Step 2: verify lyrics were written to tags
    const { readdirSync } = await import("node:fs");
    const { join } = await import("node:path");

    const files = readdirSync(SMOKE_ALBUM)
      .filter((f) => f.endsWith(".flac"))
      .sort();

    let verified = 0;
    for (const file of files) {
      const filePath = join(SMOKE_ALBUM, file);
      const meta = await readTrackMetadata(filePath);

      if (meta.lyrics) {
        verified++;

        // lyrics may be a string or an object with .text (USLT frame)
        const lyricsText =
          typeof meta.lyrics === "string"
            ? meta.lyrics
            : (meta.lyrics as { text?: string }).text ?? "";

        if (!lyricsText) {
          console.log(`  ~ ${file}: lyrics object without text`);
          continue;
        }

        // Check the encoding is clean — no replacement characters
        const normalized = normalizeLyricsEncoding(
          Buffer.from(lyricsText, "utf8"),
        );
        expect(normalized).not.toContain("�");
        expect(normalized).toBe(lyricsText); // already clean

        // For Chinese artist 蛋堡, lyrics should contain valid CJK text
        const hasChinese = /[\u4e00-\u9fff]/.test(lyricsText);
        if (hasChinese) {
          console.log(`  ✓ ${file}: ${lyricsText.slice(0, 60)}…`);
        } else {
          console.log(`  ~ ${file}: lyrics present (non-CJK): ${lyricsText.slice(0, 60)}…`);
        }
      } else {
        console.log(`  ~ ${file}: no lyrics returned`);
      }
    }

    // At least some tracks should have gotten lyrics
    expect(verified).toBeGreaterThan(0);
    console.log(`✓ ${verified}/${files.length} tracks have lyrics after download`);
  });

  itSmoke("encoding fixer handles Chinese text correctly", async () => {
    const { readTrackMetadata } = await import(
      "../../electron/handlers/tracks"
    );
    const { readdirSync } = await import("node:fs");
    const { join } = await import("node:path");

    const files = readdirSync(SMOKE_ALBUM)
      .filter((f) => f.endsWith(".flac"))
      .sort();

    for (const file of files) {
      const filePath = join(SMOKE_ALBUM, file);
      const meta = await readTrackMetadata(filePath);

      if (meta.lyrics) {
        const lyricsText =
          typeof meta.lyrics === "string"
            ? meta.lyrics
            : (meta.lyrics as { text?: string }).text ?? "";

        if (!lyricsText) continue;

        // Verify no mojibake: Chinese text decoded from a wrong encoding
        // would show replacement chars or garbled Latin-1
        const hasReplacement = lyricsText.includes("�");
        const hasLatin1Garbage = /[€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ]/.test(lyricsText);

        expect(hasReplacement).toBe(false);
        expect(hasLatin1Garbage).toBe(false);
      }
    }
  });
});
