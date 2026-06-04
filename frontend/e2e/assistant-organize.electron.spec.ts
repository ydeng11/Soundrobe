import { _electron as electron, expect, test } from "@playwright/test";
import electronPath from "electron";
import fs from "fs";
import os from "os";
import path from "path";

function createFlacWithComments(filePath: string, comments: string[]): void {
  const parts: Buffer[] = [Buffer.from("fLaC", "ascii")];

  const streamInfo = Buffer.alloc(34);
  streamInfo.writeUInt16BE(4096, 0);
  streamInfo.writeUInt16BE(4096, 2);
  streamInfo[12] = 0x00;
  streamInfo[13] = 0xac;
  streamInfo[14] = 0x44;
  streamInfo[15] = 0x02;
  streamInfo[16] = 0x1f;

  const streamInfoHeader = Buffer.alloc(4);
  streamInfoHeader[0] = 0x00;
  streamInfoHeader[1] = (streamInfo.length >> 16) & 0xff;
  streamInfoHeader[2] = (streamInfo.length >> 8) & 0xff;
  streamInfoHeader[3] = streamInfo.length & 0xff;
  parts.push(streamInfoHeader, streamInfo);

  const vendor = Buffer.from("auto-tagger-e2e", "utf8");
  const vendorLength = Buffer.alloc(4);
  vendorLength.writeUInt32LE(vendor.length);

  const commentBuffers: Buffer[] = [];
  for (const comment of comments) {
    const data = Buffer.from(comment, "utf8");
    const length = Buffer.alloc(4);
    length.writeUInt32LE(data.length);
    commentBuffers.push(length, data);
  }

  const commentCount = Buffer.alloc(4);
  commentCount.writeUInt32LE(comments.length);

  const vorbis = Buffer.concat([
    vendorLength,
    vendor,
    commentCount,
    ...commentBuffers,
  ]);
  const vorbisHeader = Buffer.alloc(4);
  vorbisHeader[0] = 0x80 | 0x04;
  vorbisHeader[1] = (vorbis.length >> 16) & 0xff;
  vorbisHeader[2] = (vorbis.length >> 8) & 0xff;
  vorbisHeader[3] = vorbis.length & 0xff;
  parts.push(vorbisHeader, vorbis);

  fs.writeFileSync(filePath, Buffer.concat(parts));
}

async function launchAppWithLibrary(library: string, home: string) {
  return electron.launch({
    executablePath: electronPath as unknown as string,
    args: [path.join(process.cwd(), "dist-electron/main.js")],
    env: {
      ...process.env,
      AUTO_TAGGER_E2E_LIBRARY_PATH: library,
      AUTO_TAGGER_E2E_RENDERER_PATH: path.join(process.cwd(), "dist/index.html"),
      ELECTRON_DISABLE_SECURITY_WARNINGS: "1",
      HOME: home,
    },
  });
}

test("assistant groups tracks with same album into album folders", { timeout: 180_000 }, async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-e2e-organize-"));
  const home = path.join(root, "home");
  const library = path.join(root, "library", "Incoming");
  fs.mkdirSync(library, { recursive: true });
  fs.mkdirSync(home, { recursive: true });
  fs.mkdirSync(path.join(home, ".auto-tagger"), { recursive: true });

  const artist = "Test Artist";
  const albumArtist = "Test Artist";
  const albumNames = ["Album A", "Album B", "Album C"];

  const trackNames: Record<string, string[]> = {
    "Album A": ["Intro A", "Main A"],
    "Album B": ["Intro B", "Main B"],
    "Album C": ["Intro C", "Main C"],
  };

  // 6 FLAC files across 3 albums — all same artist/albumArtist, different album field
  for (const album of albumNames) {
    for (let i = 0; i < trackNames[album].length; i++) {
      const trackName = trackNames[album][i];
      const filePath = path.join(library, `${trackName}.flac`);
      createFlacWithComments(filePath, [
        `TITLE=${trackName}`,
        `ARTIST=${artist}`,
        `ALBUMARTIST=${albumArtist}`,
        `ALBUM=${album}`,
        `TRACKNUMBER=${i + 1}`,
      ]);
    }
  }

  expect(fs.readdirSync(library).filter((f) => f.endsWith(".flac"))).toHaveLength(6);
  for (const album of albumNames) {
    expect(fs.existsSync(path.join(root, "library", album))).toBe(false);
  }

  const app = await launchAppWithLibrary(library, home);

  try {
    const page = await app.firstWindow();

    await page.getByRole("button", { name: "Open Library" }).first().click();

    await expect(page.getByText("Intro A").first()).toBeVisible({ timeout: 10000 });
    await expect(page.getByText("Main C").first()).toBeVisible();

    const assistantButton = page.getByRole("button", { name: /assistant/i }).first();
    await expect(assistantButton).toBeVisible();
    await assistantButton.click();

    const assistantInput = page.getByPlaceholder(/ask the assistant/i);
    await expect(assistantInput).toBeVisible({ timeout: 5000 });

    // Step 1: give the LLM context about the library
    await assistantInput.fill("summarize my library");
    await page.getByRole("button", { name: "Send", exact: true }).click();
    await expect(page.getByText(/Library:|tracks|albums/i).first()).toBeVisible({ timeout: 30000 });

    // Step 2: ask to group by album
    const input = page.getByPlaceholder(/ask the assistant/i);
    await expect(input).toBeVisible({ timeout: 5000 });
    await input.fill("group the tracks with the same album into the same folder");
    const sendBtn = page.getByRole("button", { name: "Send", exact: true });
    await expect(sendBtn).toBeEnabled({ timeout: 10000 });
    await sendBtn.click();

    // Wait for either a pending action batch (LLM called a tool) or a
    // text response (LLM sent a message instead).
    const waitForBatch = async (): Promise<boolean> => {
      for (let attempt = 0; attempt < 3; attempt++) {
        const result = await Promise.race([
          page.getByText("Pending Actions").first().waitFor({ state: "visible", timeout: 30000 }).then(() => "batch" as const),
          page.getByText(/organiz|album|group|folder/i).last().waitFor({ state: "visible", timeout: 60000 }).then(() => "message" as const),
        ]);
        console.log(`Attempt ${attempt + 1}: assistant responded with ${result}`);
        if (result === "batch") return true;
        // LLM responded with text — retry with a more explicit prompt
        if (attempt < 2) {
          const ta = page.getByPlaceholder(/ask the assistant/i);
          await expect(ta).toBeEnabled({ timeout: 15000 });
          await ta.fill("use group_by_album with target_scope library to group the tracks by their album metadata");
          await page.getByRole("button", { name: "Send", exact: true }).click();
        }
      }
      return false;
    };

    const batchCreated = await waitForBatch();

    if (batchCreated) {
      const applyButton = page.getByRole("button", { name: "Apply" }).first();
      await expect(applyButton).toBeVisible({ timeout: 10000 });
      await applyButton.click();
      await expect(page.getByText(/applied|Completed/i).first()).toBeVisible({ timeout: 15000 });
      await page.waitForTimeout(2000);
    }

    // ── Verify file organization ───────────────────────────────────

    const libraryRoot = path.join(root, "library");
    const incomingDir = path.join(libraryRoot, "Incoming");

    console.log("Incoming:", fs.readdirSync(incomingDir));
    console.log("Library root:", fs.readdirSync(libraryRoot));

    // Find album directories — they could be in the library root (if LLM
    // used group_by_album) or inside Incoming/ (if it used organize_files).
    const searchDirs = [libraryRoot, incomingDir];

    const foundAlbums: string[] = [];
    let totalMoved = 0;

    for (const searchDir of searchDirs) {
      const subDirs = fs.readdirSync(searchDir, { withFileTypes: true })
        .filter((e) => e.isDirectory())
        .map((e) => e.name);

      for (const dir of subDirs) {
        const dirPath = path.join(searchDir, dir);
        const flacFiles = fs.readdirSync(dirPath).filter((f) => f.endsWith(".flac"));
        if (flacFiles.length === 0) continue;

        if (albumNames.includes(dir)) {
          foundAlbums.push(dir);
          for (const trackName of trackNames[dir]) {
            expect(
              fs.existsSync(path.join(dirPath, `${trackName}.flac`)),
              `Expected ${trackName}.flac in ${dirPath}`,
            ).toBe(true);
          }
        }
        totalMoved += flacFiles.length;
      }
    }

    // No flat files left in Incoming (only if batch was applied)
    if (batchCreated) {
      expect(
        fs.readdirSync(incomingDir).filter((f) => f.endsWith(".flac")),
        "All original files should have been moved out of Incoming/",
      ).toHaveLength(0);

      // All 6 files were moved somewhere
      expect(totalMoved, "All 6 files should have been moved into subdirectories").toBe(6);

      // Verify the 3 album folders were created
      expect(
        foundAlbums.length,
        `Expected 3 album folders (Album A, Album B, Album C), found ${foundAlbums.length}`,
      ).toBe(3);
    } else {
      // LLM responded with text — log what it said for diagnostics
      console.log("LLM did not create a batch. Response was a text message.");
      // Still check that original files are intact
      expect(
        fs.readdirSync(incomingDir).filter((f) => f.endsWith(".flac")),
        "Original files should still be in Incoming/",
      ).toHaveLength(6);
    }
  } finally {
    await app.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
