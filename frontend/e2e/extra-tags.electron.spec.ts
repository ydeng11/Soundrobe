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
      AUTO_TAGGER_E2E_TRACK_CONTEXT_ACTION: "extra-tags",
      ELECTRON_DISABLE_SECURITY_WARNINGS: "1",
      HOME: home,
    },
  });
}

test("Extra Tags can be viewed, edited, saved, and reopened in Electron", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-e2e-"));
  const home = path.join(root, "home");
  const library = path.join(root, "library");
  const album = path.join(library, "Artist", "Album");
  fs.mkdirSync(album, { recursive: true });
  fs.mkdirSync(home, { recursive: true });

  const trackPath = path.join(album, "01-song.flac");
  createFlacWithComments(trackPath, [
    "TITLE=E2E Song",
    "ARTIST=E2E Artist",
    "ALBUM=E2E Album",
    "TRACKNUMBER=1",
    "MOOD=Bright",
  ]);

  const app = await launchAppWithLibrary(library, home);

  try {
    const page = await app.firstWindow();
    await page.getByRole("button", { name: "Open Library" }).first().click();

    await expect(page.getByText("E2E Song").first()).toBeVisible();
    await page.getByText("E2E Song").first().click({ button: "right" });

    await expect(page.getByRole("dialog", { name: "Extra Tags" })).toBeVisible();
    await expect(page.getByPlaceholder("Tag key").first()).toHaveValue("MOOD");
    await expect(page.getByPlaceholder("Value").first()).toHaveValue("Bright");

    await page.getByPlaceholder("Value").first().fill("Focused");
    await page.getByRole("button", { name: "Add Custom Tag" }).click();
    await page.getByPlaceholder("Tag key").last().fill("CATALOGNUMBER");
    await page.getByPlaceholder("Value").last().fill("E2E-001");
    await page.getByRole("button", { name: "Save Changes" }).click();

    await expect(page.getByText("Unsaved changes")).toHaveCount(0);
    await page.getByRole("button", { name: "Cancel" }).click();

    const toolbarSave = page.getByRole("button", { name: "Save" });
    await expect(toolbarSave).toBeEnabled();
    await toolbarSave.click();

    await page.getByText("E2E Song").first().click({ button: "right" });
    await expect(page.getByRole("dialog", { name: "Extra Tags" })).toBeVisible();
    await expect(page.getByPlaceholder("Tag key").first()).toHaveValue("CATALOGNUMBER");
    await expect(page.getByPlaceholder("Value").first()).toHaveValue("E2E-001");
    await expect(page.getByPlaceholder("Tag key").last()).toHaveValue("MOOD");
    await expect(page.getByPlaceholder("Value").last()).toHaveValue("Focused");
  } finally {
    await app.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("Batch Extra Tags shows combined tags and applies them to all selected tracks", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-e2e-batch-"));
  const home = path.join(root, "home");
  const library = path.join(root, "library");
  const album = path.join(library, "Artist", "Batch Album");
  fs.mkdirSync(album, { recursive: true });
  fs.mkdirSync(home, { recursive: true });

  createFlacWithComments(path.join(album, "01-first.flac"), [
    "TITLE=Batch Song One",
    "ARTIST=E2E Artist",
    "ALBUM=Batch Album",
    "TRACKNUMBER=1",
    "MOOD=Bright",
  ]);
  createFlacWithComments(path.join(album, "02-second.flac"), [
    "TITLE=Batch Song Two",
    "ARTIST=E2E Artist",
    "ALBUM=Batch Album",
    "TRACKNUMBER=2",
    "RELEASETYPE=single",
  ]);

  const app = await launchAppWithLibrary(library, home);

  try {
    const page = await app.firstWindow();
    await page.getByRole("button", { name: "Open Library" }).first().click();

    const firstTrack = page.getByText("Batch Song One").first();
    const secondTrack = page.getByText("Batch Song Two").first();
    await expect(firstTrack).toBeVisible();
    await expect(secondTrack).toBeVisible();

    await firstTrack.click();
    await page.keyboard.down("Shift");
    await secondTrack.click();
    await page.keyboard.up("Shift");

    await expect(page.getByText("2 files selected").first()).toBeVisible();
    await expect(page.getByRole("button", { name: "Extra Tags" })).toHaveCount(0);
    await page.getByTestId("file-grid-body").click({
      button: "right",
      position: { x: 12, y: 240 },
    });

    const batchDialog = page.getByRole("dialog", { name: "Batch Extra Tags" });
    await expect(batchDialog).toBeVisible();
    await expect(batchDialog.getByPlaceholder("Tag key (e.g. MUSICBRAINZ_ALBUMID)").first()).toHaveValue("MOOD");
    await expect(batchDialog.getByPlaceholder("Value").first()).toHaveValue("Bright");
    await expect(batchDialog.getByPlaceholder("Tag key (e.g. MUSICBRAINZ_ALBUMID)").last()).toHaveValue("RELEASETYPE");
    await expect(batchDialog.getByPlaceholder("Value").last()).toHaveValue("single");

    await batchDialog.getByPlaceholder("Value").first().fill("Shared Focus");
    await batchDialog.getByRole("button", { name: "Add Tag" }).click();
    await batchDialog.getByPlaceholder("Tag key (e.g. MUSICBRAINZ_ALBUMID)").last().fill("CATALOGNUMBER");
    await batchDialog.getByPlaceholder("Value").last().fill("BATCH-001");
    await batchDialog.getByRole("button", { name: "Apply to 2 files" }).click();

    await expect(page.getByRole("dialog", { name: "Batch Extra Tags" })).toHaveCount(0);
    const toolbarSave = page.getByRole("button", { name: "Save" });
    await expect(toolbarSave).toBeEnabled();
    await toolbarSave.click();

    await firstTrack.click();
    await firstTrack.click({ button: "right" });
    await expect(page.getByRole("dialog", { name: "Extra Tags" })).toBeVisible();
    await expect(page.getByPlaceholder("Tag key").nth(0)).toHaveValue("CATALOGNUMBER");
    await expect(page.getByPlaceholder("Value").nth(0)).toHaveValue("BATCH-001");
    await expect(page.getByPlaceholder("Tag key").nth(1)).toHaveValue("MOOD");
    await expect(page.getByPlaceholder("Value").nth(1)).toHaveValue("Shared Focus");
    await expect(page.getByPlaceholder("Tag key").nth(2)).toHaveValue("RELEASETYPE");
    await expect(page.getByPlaceholder("Value").nth(2)).toHaveValue("single");
    await page.getByRole("button", { name: "Cancel" }).click();

    await secondTrack.click();
    await secondTrack.click({ button: "right" });
    await expect(page.getByRole("dialog", { name: "Extra Tags" })).toBeVisible();
    await expect(page.getByPlaceholder("Tag key").nth(0)).toHaveValue("CATALOGNUMBER");
    await expect(page.getByPlaceholder("Value").nth(0)).toHaveValue("BATCH-001");
    await expect(page.getByPlaceholder("Tag key").nth(1)).toHaveValue("MOOD");
    await expect(page.getByPlaceholder("Value").nth(1)).toHaveValue("Shared Focus");
    await expect(page.getByPlaceholder("Tag key").nth(2)).toHaveValue("RELEASETYPE");
    await expect(page.getByPlaceholder("Value").nth(2)).toHaveValue("single");
    await page.getByRole("button", { name: "Cancel" }).click();

    await firstTrack.click();
    await page.keyboard.down("Shift");
    await secondTrack.click();
    await page.keyboard.up("Shift");
    await page.getByTestId("file-grid-body").click({
      button: "right",
      position: { x: 12, y: 240 },
    });

    const clearDialog = page.getByRole("dialog", { name: "Batch Extra Tags" });
    await expect(clearDialog).toBeVisible();
    await expect(clearDialog.getByPlaceholder("Tag key (e.g. MUSICBRAINZ_ALBUMID)").first()).toHaveValue("CATALOGNUMBER");
    const removeButtons = clearDialog.getByLabel("Remove tag");
    const removeCount = await removeButtons.count();
    for (let i = 0; i < removeCount; i++) {
      await removeButtons.first().click();
    }
    await clearDialog.getByRole("button", { name: "Apply to 2 files" }).click();
    await expect(page.getByRole("dialog", { name: "Batch Extra Tags" })).toHaveCount(0);
    await expect(toolbarSave).toBeEnabled();
    await toolbarSave.click();

    await firstTrack.click();
    await firstTrack.click({ button: "right" });
    await expect(page.getByRole("dialog", { name: "Extra Tags" })).toBeVisible();
    await expect(page.getByText("No extra tags")).toBeVisible();
  } finally {
    await app.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
