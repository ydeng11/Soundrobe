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

test("Convert splits an existing title tag into artist and title tags", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-convert-"));
  const home = path.join(root, "home");
  const library = path.join(root, "library");
  const album = path.join(library, "Artist", "Album");
  fs.mkdirSync(album, { recursive: true });
  fs.mkdirSync(home, { recursive: true });

  createFlacWithComments(path.join(album, "01-song.flac"), [
    "TITLE=E2E Artist - E2E Song",
    "ARTIST=Old Artist",
    "ALBUM=E2E Album",
    "TRACKNUMBER=1",
  ]);

  const app = await launchAppWithLibrary(library, home);

  try {
    const page = await app.firstWindow();
    await page.getByRole("button", { name: "Open Library" }).first().click();

    await expect(page.getByText("E2E Artist - E2E Song").first()).toBeVisible();
    await page.getByText("E2E Artist - E2E Song").first().click();

    await page.getByRole("button", { name: "Convert" }).click();
    const dialog = page.getByRole("dialog", { name: "Convert" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Regex" })).toHaveCount(0);
    await expect(dialog.getByRole("button", { name: "Custom" })).toHaveCount(0);

    await dialog.getByRole("button", { name: "Tag -> Tags" }).click();
    await expect(dialog.getByText("Artist=E2E Artist, Title=E2E Song")).toBeVisible();
    await dialog.getByRole("button", { name: "Convert" }).click();

    await expect(page.getByRole("dialog", { name: "Convert" })).toHaveCount(0);
    await expect(page.getByDisplayValue("E2E Song")).toBeVisible();
    await expect(page.getByDisplayValue("E2E Artist")).toBeVisible();
  } finally {
    await app.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
