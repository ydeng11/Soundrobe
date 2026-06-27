import { _electron as electron, expect, test } from "@playwright/test";
import electronPath from "electron";
import fs from "fs";
import os from "os";
import path from "path";
import { parseFile } from "music-metadata";

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

test("Audit — writes deterministic FLAC fixes and surfaces manual review", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-audit-"));
  const home = path.join(root, "home");
  const library = path.join(root, "library");
  const album = path.join(library, "Artist", "2020 - Album");
  fs.mkdirSync(album, { recursive: true });
  fs.mkdirSync(home, { recursive: true });

  const fixFile = path.join(album, "01. Song.flac");
  const manualFile = path.join(album, "Thunderstruck.flac");
  createFlacWithComments(fixFile, [
    "TITLE=Wrong",
    "ARTIST=Artist",
    "ARTISTS=Artist",
    "ALBUM=Wrong Album",
    "ALBUMARTIST=Artist",
    "DATE=2019",
    "GENRE=Pop",
    "TRACKNUMBER=9",
  ]);
  createFlacWithComments(manualFile, [
    "TITLE=Thunderstruck",
    "ARTIST=AC/DC",
    "ALBUM=Album",
    "ALBUMARTIST=Artist",
    "DATE=2020",
    "GENRE=Rock",
  ]);

  const app = await launchAppWithLibrary(library, home);

  try {
    const page = await app.firstWindow();
    await page.getByRole("button", { name: "Open Library" }).first().click();

    await expect(page.getByText("01. Song.flac").first()).toBeVisible();
    await expect(page.getByText("Thunderstruck.flac").first()).toBeVisible();

    await page.getByRole("button", { name: "Audit" }).click();

    await expect.poll(async () => (await parseFile(fixFile)).common.title, { timeout: 10_000 }).toBe("Song");
    await page.getByRole("button", { name: "2020 - Album Artist 2" }).click();
    await expect(page.getByText("Audit Results")).toBeVisible({ timeout: 10_000 });
    await expect(page.getByText("Fixed").first()).toBeVisible();
    await expect(page.getByText("deterministic 98%").first()).toBeVisible();
    await expect(page.getByText(/Artists may need manual splitting/).first()).toBeVisible();

    const fixed = await parseFile(fixFile);
    expect(fixed.common.title).toBe("Song");
    expect(fixed.common.album).toBe("Album");
    expect(fixed.common.year).toBe(2020);
    expect(fixed.common.track.no).toBe(1);

    const manual = await parseFile(manualFile);
    expect(manual.common.artist).toBe("AC/DC");
    expect(manual.common.artists).not.toEqual(["AC", "DC"]);
  } finally {
    await app.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
