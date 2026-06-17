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
  const userDataDir = path.join(home, "electron-profile");
  const xdgConfigHome = path.join(home, ".config");
  const xdgCacheHome = path.join(home, ".cache");
  fs.mkdirSync(userDataDir, { recursive: true });
  fs.mkdirSync(xdgConfigHome, { recursive: true });
  fs.mkdirSync(xdgCacheHome, { recursive: true });

  return electron.launch({
    executablePath: electronPath as unknown as string,
    args: [
      "--password-store=basic",
      "--use-mock-keychain",
      `--user-data-dir=${userDataDir}`,
      path.join(process.cwd(), "dist-electron/main.js"),
    ],
    env: {
      ...process.env,
      AUTO_TAGGER_E2E_LIBRARY_PATH: library,
      AUTO_TAGGER_E2E_RENDERER_PATH: path.join(process.cwd(), "dist/index.html"),
      ELECTRON_DISABLE_SECURITY_WARNINGS: "1",
      HOME: home,
      XDG_CONFIG_HOME: xdgConfigHome,
      XDG_CACHE_HOME: xdgCacheHome,
    },
  });
}

test("Number Tracks — by filename A-Z assigns sequential track numbers", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-number-"));
  const home = path.join(root, "home");
  const library = path.join(root, "library");
  const album = path.join(library, "Artist", "Test Album");
  fs.mkdirSync(album, { recursive: true });
  fs.mkdirSync(home, { recursive: true });

  // Create 3 tracks with intentionally wrong track numbers
  createFlacWithComments(path.join(album, "03-third.flac"), [
    "TITLE=Third Track",
    "TRACKNUMBER=3",
    "TRACKTOTAL=10",
  ]);
  createFlacWithComments(path.join(album, "01-first.flac"), [
    "TITLE=First Track",
    "TRACKNUMBER=1",
    "TRACKTOTAL=10",
  ]);
  createFlacWithComments(path.join(album, "02-second.flac"), [
    "TITLE=Second Track",
    "TRACKNUMBER=2",
    "TRACKTOTAL=10",
  ]);

  const app = await launchAppWithLibrary(library, home);

  try {
    const page = await app.firstWindow();
    await page.getByRole("button", { name: "Open Library" }).first().click();

    // Wait for the tracks to load — we should see all 3 files
    await expect(page.getByText("03-third.flac").first()).toBeVisible();
    await expect(page.getByText("01-first.flac").first()).toBeVisible();
    await expect(page.getByText("02-second.flac").first()).toBeVisible();

    // Click the first row to select it and focus the grid
    await page.getByText("01-first.flac").first().click();

    // Click the Number button to open the dropdown
    await page.getByRole("button", { name: "Number" }).click();

    // The dropdown should be visible with ordering rules
    await expect(page.getByText("Number tracks by…")).toBeVisible();

    // Select "By filename (A-Z)"
    await page.getByText("By filename (A-Z)").click();

    // Wait for saving to finish
    await page.waitForTimeout(1000);

    // Click the file-grid header "Track" to sort by track number ascending
    // So we can see the new numbers
    await page.getByRole("button", { name: "Track" }).click();

    // Now verify that track numbers are sequential by clicking rows and
    // checking the inspector panel's track field
    // 01-first.flac should now be track 1/3
    await page.getByText("01-first.flac").first().click();
    await page.waitForTimeout(300);

    // Click the file grid body to ensure focus is there, then check track field
    // The track field in the inspector shows format like "1/3"
    // We need to find the input that shows track info
    const trackField = page.getByPlaceholder("1/10");
    await expect(trackField).toHaveValue("1/3");

    // Click 02-second.flac — should be track 2/3
    await page.getByText("02-second.flac").first().click();
    await page.waitForTimeout(300);
    await expect(trackField).toHaveValue("2/3");

    // Click 03-third.flac — should be track 3/3
    await page.getByText("03-third.flac").first().click();
    await page.waitForTimeout(300);
    await expect(trackField).toHaveValue("3/3");
  } finally {
    await app.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("Number Tracks — by title Z-A reverses title order", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-number2-"));
  const home = path.join(root, "home");
  const library = path.join(root, "library");
  const album = path.join(library, "Artist", "Alpha Album");
  fs.mkdirSync(album, { recursive: true });
  fs.mkdirSync(home, { recursive: true });

  // Create tracks with unordered titles
  createFlacWithComments(path.join(album, "z.flac"), [
    "TITLE=Zebra",
    "TRACKNUMBER=1",
  ]);
  createFlacWithComments(path.join(album, "a.flac"), [
    "TITLE=Alpha",
    "TRACKNUMBER=2",
  ]);
  createFlacWithComments(path.join(album, "m.flac"), [
    "TITLE=Moon",
    "TRACKNUMBER=3",
  ]);

  const app = await launchAppWithLibrary(library, home);

  try {
    const page = await app.firstWindow();
    await page.getByRole("button", { name: "Open Library" }).first().click();

    await expect(page.getByText("z.flac").first()).toBeVisible();

    // Select the album by clicking in the grid
    await page.getByText("z.flac").first().click();

    // Open Number dropdown and select title Z-A
    await page.getByRole("button", { name: "Number" }).click();
    await page.getByText("By title (Z-A)").click();

    await page.waitForTimeout(1000);

    // Click the track header to sort by track number ascending
    await page.getByRole("button", { name: "Track" }).click();

    // Track 1 should be Zebra (Z first in Z-A order)
    await page.getByText("z.flac").first().click();
    await page.waitForTimeout(300);
    const trackField = page.getByPlaceholder("1/10").first();
    await expect(trackField).toHaveValue("1/3");

    // Track 3 should be Alpha (A last in Z-A order)
    await page.getByText("a.flac").first().click();
    await page.waitForTimeout(300);
    await expect(trackField).toHaveValue("3/3");
  } finally {
    await app.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
