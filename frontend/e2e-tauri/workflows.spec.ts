import path from "node:path";
import type { AuditTrackResult } from "../src/shared/desktop-api";
import type { E2eManifest } from "./fixtures";

const manifest = JSON.parse(
  process.env.AUTO_TAGGER_E2E_MANIFEST ?? "null",
) as E2eManifest | null;

if (!manifest) {
  throw new Error("AUTO_TAGGER_E2E_MANIFEST is required");
}

async function clickButton(label: string, exact = true): Promise<void> {
  const clicked = await browser.execute(
    ({ buttonLabel, exactMatch }) => {
      const button = Array.from(document.querySelectorAll("button")).find((candidate) => {
        const text = candidate.textContent?.trim() ?? "";
        return exactMatch ? text === buttonLabel : text.includes(buttonLabel);
      });
      button?.click();
      return Boolean(button);
    },
    { buttonLabel: label, exactMatch: exact },
  );
  if (!clicked) throw new Error(`Button not found: ${label}`);
}

async function clickTrack(trackPath: string): Promise<void> {
  const clicked = await browser.execute((targetPath) => {
    const row = Array.from(
      document.querySelectorAll<HTMLElement>("[data-testid^='file-row-']"),
    ).find(
      (candidate) => candidate.dataset.testid === `file-row-${targetPath}`,
    );
    row?.click();
    return Boolean(row);
  }, trackPath);
  if (!clicked) throw new Error(`Track row not found: ${trackPath}`);
}

async function clickDialogButton(dialogLabel: string, buttonLabel: string): Promise<void> {
  const clicked = await browser.execute(
    ({ dialogName, buttonName }) => {
      const dialog = document.querySelector(
        `[role='dialog'][aria-label='${dialogName}']`,
      );
      const button = Array.from(dialog?.querySelectorAll("button") ?? []).find(
        (candidate) => candidate.textContent?.trim() === buttonName,
      );
      button?.click();
      return Boolean(button);
    },
    { dialogName: dialogLabel, buttonName: buttonLabel },
  );
  if (!clicked) throw new Error(`Dialog button not found: ${dialogLabel} / ${buttonLabel}`);
}

describe("Tauri desktop workflows", () => {
  it("reveals the native main window after renderer boot", async () => {
    const visible = await browser.tauri.execute((tauri) =>
      tauri.core.invoke<boolean>("plugin:window|is_visible", { label: "main" }),
    );

    expect(visible).toBe(true);
  });

  it("preserves absolute paths through the native library pipeline", async () => {
    const selectedPath = await browser.execute(() => window.api.openFolderDialog());
    expect(selectedPath).toBe(manifest.library);

    const openLibrary = await $("button=Open Library");
    await openLibrary.click();
    await browser.waitUntil(
      async () =>
        browser.execute(() => document.body.innerText.includes("Workflow One")),
      {
        timeout: 15_000,
        timeoutMsg: "the renderer did not display the selected temporary library",
      },
    );

    const trackPaths = await browser.execute(async (albumPath) => {
      const detail = await window.api.readAlbum(albumPath);
      return detail.tracks.map((track) => track.path);
    }, manifest.workflowAlbum);

    expect(trackPaths).toContain(manifest.workflowTrack);
    expect(trackPaths.every((trackPath) => path.isAbsolute(trackPath))).toBe(true);
  });

  it("writes standard and extra metadata through the native safe writer", async () => {
    const updated = await browser.execute(
      async ({ albumPath, trackPath }) => {
        await window.api.writeTrack(trackPath, { title: "Workflow One Updated" });
        await window.api.writeExtraTags(trackPath, [
          { key: "MOOD", value: "Focused" },
          { key: "CATALOGNUMBER", value: "TAURI-E2E-001" },
        ]);
        const album = await window.api.readAlbum(albumPath);
        return {
          track: album.tracks.find((candidate) => candidate.path === trackPath),
          extraTags: await window.api.readExtraTags(trackPath),
        };
      },
      { albumPath: manifest.workflowAlbum, trackPath: manifest.workflowTrack },
    );

    expect(updated.track?.title).toBe("Workflow One Updated");
    expect(updated.extraTags).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: "CATALOGNUMBER", value: "TAURI-E2E-001" }),
        expect.objectContaining({ key: "MOOD", value: "Focused" }),
      ]),
    );
  });

  it("previews and applies deterministic assistant organization", async () => {
    const result = await browser.execute(
      async ({ albumPath, libraryPath }) => {
        const tracks = (await window.api.readAlbum(albumPath)).tracks;
        const event = await window.api.assistantSend({
          message: "group files into album folders",
          apiKey: "",
          libraryPath,
          activeAlbumPath: albumPath,
          selectedTrackPaths: tracks.map((track) => track.path),
          tracks,
          albums: [],
          autonomous: false,
        });
        const batches = await window.api.assistantGetBatches();
        const batchId =
          event.data && typeof event.data === "object" && "actionBatchId" in event.data
            ? String(event.data.actionBatchId)
            : null;
        const batch = batches.find((candidate) => candidate.id === batchId);
        if (!batch) throw new Error("Assistant did not create a pending batch");
        const applied = await window.api.assistantApplyActions(batch.id);
        const destinations = batch.actions.flatMap((action) =>
          action.destinationPath ? [action.destinationPath] : [],
        );
        return {
          eventType: event.type,
          applied,
          destinations,
          sourcesExist: await Promise.all(
            tracks.map((track) => window.api.checkFileExists(track.path)),
          ),
          destinationsExist: await Promise.all(
            destinations.map((destination) => window.api.checkFileExists(destination)),
          ),
        };
      },
      { albumPath: manifest.incomingAlbum, libraryPath: manifest.library },
    );

    expect(result.eventType).toBe("action_batch_created");
    expect(result.applied.success).toBe(true);
    expect(result.destinations).toHaveLength(manifest.incomingTracks.length);
    expect(result.sourcesExist).toEqual([false, false]);
    expect(result.destinationsExist).toEqual([true, true]);
  });

  it("audits and applies deterministic metadata fixes", async () => {
    const result = await browser.execute(async (albumPath) => {
      const findings = await window.api.runAlbumAudit(albumPath);
      const applied = await window.api.applyAuditFixes([{ albumPath, results: findings }]);
      const track = (await window.api.readAlbum(albumPath)).tracks[0];
      return { findings, applied, track };
    }, manifest.auditAlbum);

    expect(
      result.findings.some(
        (finding: AuditTrackResult) => finding.autoFixEligible === true,
      ),
    ).toBe(true);
    expect(result.applied.fixed).toBeGreaterThan(0);
    expect(result.track.title).toBe("Song");
    expect(result.track.album).toBe("Audit Album");
    expect(result.track.year).toBe("2020");
    expect(result.track.trackNumber).toBe(1);
  });

  it("auto-tags an album through the offline native task pipeline", async () => {
    const result = await browser.execute(async (albumPath) => {
      const taskId = await window.api.autoTagAlbum(albumPath);
      for (let attempt = 0; attempt < 100; attempt += 1) {
        const progress = await window.api.getTaskProgress(taskId);
        if (progress && progress.status !== "running") {
          return {
            progress,
            track: (await window.api.readAlbum(albumPath)).tracks[0],
          };
        }
        await new Promise((resolve) => window.setTimeout(resolve, 50));
      }
      throw new Error("Offline auto-tag task did not reach a terminal state");
    }, manifest.autoTagAlbum);

    expect(result.progress.status).toBe("completed");
    expect(result.track.title).toBe("Offline Song");
    expect(result.track.album).toBe("Offline Album");
    expect(result.track.albumArtist).toBe("Offline Artist");
  });

  it("converts a title into artist and title tags through the renderer", async () => {
    await clickButton("Convert Album", false);
    await browser.waitUntil(() =>
      browser.execute(() => document.body.innerText.includes("E2E Artist - E2E Song")),
    );
    await clickTrack(manifest.convertTrack);
    await clickButton("Convert");
    const dialog = await $("[role='dialog'][aria-label='Convert']");
    await dialog.waitForDisplayed();
    await clickDialogButton("Convert", "Tag -> Tags");
    await browser.waitUntil(() =>
      browser.execute(() =>
        document.body.innerText.includes("Artist=E2E Artist, Title=E2E Song"),
      ),
    );
    await clickDialogButton("Convert", "Convert");

    await browser.waitUntil(
      async () => {
        const current = await browser.execute(async (albumPath) =>
          (await window.api.readAlbum(albumPath)).tracks[0],
        manifest.convertAlbum);
        return current.title === "E2E Song" && current.artist === "E2E Artist"
          ? current
          : false;
      },
      { timeout: 10_000, timeoutMsg: "converted metadata was not written" },
    );
    const track = await browser.execute(async (albumPath) =>
      (await window.api.readAlbum(albumPath)).tracks[0],
    manifest.convertAlbum);
    expect(track.title).toBe("E2E Song");
    expect(track.artist).toBe("E2E Artist");
  });

  it("numbers tracks through the renderer and native batch writer", async () => {
    await clickButton("Number Album", false);
    await browser.waitUntil(() =>
      browser.execute(() => document.body.innerText.includes("01-first.flac")),
    );
    await clickTrack(manifest.numberTracks[0]);
    await clickButton("Number");
    await clickButton("By filename (A-Z)");

    await browser.waitUntil(
      async () => {
        const current = await browser.execute(async (albumPath) =>
          (await window.api.readAlbum(albumPath)).tracks,
        manifest.numberAlbum);
        return current.every(
          (track) => track.trackTotal === 3 && track.trackNumber !== 9,
        )
          ? current
          : false;
      },
      { timeout: 10_000, timeoutMsg: "numbered metadata was not written" },
    );
    const tracks = await browser.execute(async (albumPath) =>
      (await window.api.readAlbum(albumPath)).tracks,
    manifest.numberAlbum);
    const byFilename = new Map(
      tracks.map((track) => [path.basename(track.path), track.trackNumber]),
    );
    expect(byFilename.get("01-first.flac")).toBe(1);
    expect(byFilename.get("02-second.flac")).toBe(2);
    expect(byFilename.get("03-third.flac")).toBe(3);
  });
});
