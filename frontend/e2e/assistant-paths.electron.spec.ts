/**
 * E2E test: TrackData loaded through the full Electron pipeline
 * carries full absolute file paths.
 *
 * The assistant tools (tracks.search, tracks.inspect, etc.) build
 * their summary text from TrackData.path — without the full path
 * the AGENT cannot reference files. This test verifies that the
 * real file-system read pipeline produces correct absolute paths.
 *
 * Unlike the unit tests, this runs inside the real Electron runtime
 * using the built app (dist-electron/main.js) and real FLAC files.
 */

import { _electron as electron, expect, test } from "@playwright/test";
import electronPath from "electron";
import fs from "fs";
import os from "os";
import path from "path";

// ── Synthetic FLAC helpers ──────────────────────────────────────

function makeFlac(filePath: string, comments: string[]): void {
  const parts: Buffer[] = [Buffer.from("fLaC", "ascii")];

  const si = Buffer.alloc(34);
  si.writeUInt16BE(4096, 0);
  si.writeUInt16BE(4096, 2);
  const sampleRate = 44_100;
  const totalSamples = BigInt(Math.round(sampleRate * 200));
  const packed =
    (BigInt(sampleRate) << 44n) | (1n << 41n) | (15n << 36n) | totalSamples;
  for (let i = 0; i < 8; i++) {
    si[10 + i] = Number((packed >> BigInt((7 - i) * 8)) & 0xffn);
  }
  const siH = Buffer.alloc(4);
  siH[0] = 0x00;
  siH[1] = (si.length >> 16) & 0xff;
  siH[2] = (si.length >> 8) & 0xff;
  siH[3] = si.length & 0xff;
  parts.push(siH, si);

  const vendor = Buffer.from("e2e", "utf8");
  const vLen = Buffer.alloc(4);
  vLen.writeUInt32LE(vendor.length);
  const cBufs: Buffer[] = [];
  for (const c of comments) {
    const cb = Buffer.from(c, "utf8");
    const cl = Buffer.alloc(4);
    cl.writeUInt32LE(cb.length);
    cBufs.push(cl, cb);
  }
  const n = Buffer.alloc(4);
  n.writeUInt32LE(comments.length);
  const vorbis = Buffer.concat([vLen, vendor, n, ...cBufs]);
  const vH = Buffer.alloc(4);
  vH[0] = 0x80 | 0x04;
  vH[1] = (vorbis.length >> 16) & 0xff;
  vH[2] = (vorbis.length >> 8) & 0xff;
  vH[3] = vorbis.length & 0xff;
  parts.push(vH, vorbis);

  fs.writeFileSync(filePath, Buffer.concat(parts));
}

async function launchApp(home: string) {
  return electron.launch({
    executablePath: electronPath as unknown as string,
    args: [path.join(process.cwd(), "dist-electron/main.js")],
    env: {
      ...process.env,
      AUTO_TAGGER_E2E_RENDERER_PATH: path.join(process.cwd(), "dist/index.html"),
      ELECTRON_DISABLE_SECURITY_WARNINGS: "1",
      HOME: home,
    },
  });
}

// ── Tests ─────────────────────────────────────────────────────────

test.describe("assistant tool file path pipeline", () => {
  test("scanLibrary returns TrackData with full absolute paths", async () => {
    const root = fs.mkdtempSync(
      path.join(os.tmpdir(), "auto-tagger-e2e-assistant-paths-"),
    );
    const library = path.join(root, "library", "Incoming");
    const home = path.join(root, "home");
    fs.mkdirSync(library, { recursive: true });
    fs.mkdirSync(home, { recursive: true });
    fs.mkdirSync(path.join(home, ".auto-tagger"), { recursive: true });

    // Create 3 FLAC files at known absolute paths
    const files: string[] = [];
    for (let i = 0; i < 3; i++) {
      const fp = path.join(library, `0${i + 1}. Song ${["One", "Two", "Three"][i]}.flac`);
      files.push(fp);
      makeFlac(fp, [
        `TITLE=Song ${["One", "Two", "Three"][i]}`,
        `ARTIST=Artist ${["A", "B", "C"][i]}`,
        "ALBUM=Test Album",
        `TRACKNUMBER=${i + 1}`,
      ]);
    }

    const app = await launchApp(home);

    try {
      const page = await app.firstWindow();
      await page.waitForLoadState("domcontentloaded");

      // Open the library
      const openBtn = page.getByRole("button", { name: "Open Library" }).first();
      await expect(openBtn).toBeVisible({ timeout: 15000 });
      await openBtn.click();

      // Wait for the file grid to render track data
      await expect(
        page.getByText("Song One").first(),
      ).toBeVisible({ timeout: 20000 });

      await expect(
        page.getByText("Song Two").first(),
      ).toBeVisible({ timeout: 5000 });

      await expect(
        page.getByText("Song Three").first(),
      ).toBeVisible({ timeout: 5000 });

      // Use page.evaluate to access the React fiber and verify
      // the underlying TrackData has correct absolute paths.
      //
      // Access strategy: React components that render the file grid
      // store TrackData in their props. The file-grid-body is
      // rendered by FileGrid which receives tracks: TrackData[].
      const pathData = await page.evaluate(() => {
        // Try to find React fiber data for file grid rows.
        // Each FileGridRow renders with track={track} where
        // track.path is the absolute path. We look for the
        // keyed elements where the key IS the track path.
        const gridBody = document.querySelector('[data-testid="file-grid-body"]');
        if (!gridBody) return { error: "grid body not found" };

        // Get all rendered file grid rows and inspect their children
        const rows = gridBody.querySelectorAll(".flex.items-center");
        const results: string[] = [];
        for (const row of rows) {
          const text = row.textContent ?? "";
          results.push(text.slice(0, 80));
        }

        // Also try to extract the React internal fiber to get TrackData
        const firstChild = gridBody.firstElementChild;
        const fiberKey = Object.keys(firstChild ?? {}).find(
          (k) => k.startsWith("__reactFiber$") || k.startsWith("__reactInternalInstance$"),
        );

        return {
          rowCount: rows.length,
          rowTexts: results,
          hasReactFiber: !!fiberKey,
          fiberKeys: Object.keys(firstChild ?? {}).filter(
            (k) => k.startsWith("__react"),
          ),
        };
      });

      console.log("Rendered file grid:", JSON.stringify(pathData, null, 2));

      // Verify the file grid renders rows for all 3 tracks
      expect(pathData.rowCount).toBeGreaterThanOrEqual(3);

      // Verify track names appear in the rendered output
      const allText = pathData.rowTexts?.join(" ") ?? "";
      expect(allText).toContain("Song One");
      expect(allText).toContain("Song Two");
      expect(allText).toContain("Song Three");

      // Verify filenames appear
      expect(allText).toContain("Song One.flac") || expect(allText).toContain("01. Song One");
      expect(allText).toContain("Song Two.flac") || expect(allText).toContain("02. Song Two");
      expect(allText).toContain("Song Three.flac") || expect(allText).toContain("03. Song Three");

      const trackPaths = await page.evaluate(async (albumPath) => {
        const api = (window as any).api;
        if (!api) throw new Error("api not available");
        const detail = await api.readAlbum(albumPath);
        return detail.tracks.map((track: { path: string }) => track.path);
      }, library);

      expect(trackPaths.sort()).toEqual(files.sort());
      for (const trackPath of trackPaths) {
        expect(path.isAbsolute(trackPath)).toBe(true);
      }
    } finally {
      await app.close();
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
});
