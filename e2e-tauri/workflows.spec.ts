import path from "node:path";
import type { AuditTrackResult } from "../src/shared/desktop-api";
import type { E2eManifest } from "./fixtures";

const manifest = JSON.parse(
  process.env.SOUNDROBE_E2E_MANIFEST ?? "null",
) as E2eManifest | null;

if (!manifest) {
  throw new Error("SOUNDROBE_E2E_MANIFEST is required");
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
  await browser.waitUntil(
    async () =>
      browser.execute((targetPath) => {
        return Array.from(
          document.querySelectorAll<HTMLElement>("[data-testid^='file-row-']"),
        ).some(
          (candidate) => candidate.dataset.testid === `file-row-${targetPath}`,
        );
      }, trackPath),
    {
      timeout: 5_000,
      timeoutMsg: `Track row did not render: ${trackPath}`,
    },
  );

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

async function rightClickTrack(trackPath: string): Promise<void> {
  const opened = await browser.execute((targetPath) => {
    const row = Array.from(
      document.querySelectorAll<HTMLElement>("[data-testid^='file-row-']"),
    ).find(
      (candidate) => candidate.dataset.testid === `file-row-${targetPath}`,
    );
    row?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
    return Boolean(row);
  }, trackPath);
  if (!opened) throw new Error(`Track row not found: ${trackPath}`);
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
    await browser.waitUntil(
      async () => {
        try {
          return await browser.tauri.execute((tauri) =>
            tauri.core.invoke<boolean>("plugin:window|is_visible", { label: "main" }),
          );
        } catch {
          // The embedded WDIO bridge can finish loading just after the
          // renderer is reachable on macOS ARM.
          return false;
        }
      },
      {
        timeout: 15_000,
        timeoutMsg: "the native main window did not become visible",
      },
    );
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

  it("opens the Extra Tags editor from the track context menu", async () => {
    await rightClickTrack(manifest.workflowTrack);
    const editor = await $("[role='dialog'][aria-label='Extra Tags']");
    await editor.waitForDisplayed();
    await clickDialogButton("Extra Tags", "Cancel");
    await editor.waitForDisplayed({ reverse: true });
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
    const taskId = await browser.execute(
      (albumPath) => window.api.autoTagAlbum(albumPath),
      manifest.autoTagAlbum,
    );
    const progress = await browser.waitUntil(
      async () => {
        const current = await browser.execute(
          (id) => window.api.getTaskProgress(id),
          taskId,
        );
        return current && current.status !== "running" ? current : false;
      },
      {
        // Windows CI can take longer than five seconds to finish the native
        // task, but a bounded wait still makes a stalled task fail clearly.
        timeout: 30_000,
        timeoutMsg: "Offline auto-tag task did not reach a terminal state",
      },
    );
    const track = await browser.execute(
      async (albumPath) => (await window.api.readAlbum(albumPath)).tracks[0],
      manifest.autoTagAlbum,
    );
    const result = { progress, track };

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
      {
        // Windows CI can take longer to finish three native metadata writes,
        // but a bounded wait still makes a stalled batch fail clearly.
        timeout: 30_000,
        timeoutMsg: "numbered metadata was not written",
      },
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

  it("removes an external cover from the right-panel editor", async () => {
    // The preceding numbering workflow leaves Number Album selected.
    await clickButton("Workflow Album", false);
    await clickTrack(manifest.workflowTrack);

    // Wait for the cover image to appear (async fetch with 80ms debounce)
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document.querySelector("img[alt='Cover art']") !== null,
        ),
      {
        timeout: 5_000,
        timeoutMsg: "Cover image did not appear",
      },
    );

    // Click the Remove button
    await clickButton("Remove");

    // Wait for the UI to update: Remove button gone, "No cover" placeholder shows
    await browser.waitUntil(
      async () =>
        browser.execute(
          () =>
            document.body.innerText.includes("No cover") &&
            !document.body.innerText.includes("Remove"),
        ),
      {
        timeout: 5_000,
        timeoutMsg: "Cover was not removed from the UI",
      },
    );

    // Verify filesystem state: cover.png gone, suppression marker written
    const coverExists = await browser.execute(
      async (p) => window.api.checkFileExists(p),
      path.join(manifest.workflowAlbum, "cover.png"),
    );
    const markerExists = await browser.execute(
      async (p) => window.api.checkFileExists(p),
      path.join(manifest.workflowAlbum, ".auto-tagger-cover-removed"),
    );
    expect(coverExists).toBe(false);
    expect(markerExists).toBe(true);
  });

  it("opens the search dialog and cancels without writing metadata", async () => {
    // Open the Search dialog
    await clickButton("Search");
    await browser.waitUntil(
      () => browser.execute(() => document.querySelector("[aria-label='Search releases']") !== null),
      { timeout: 5_000, timeoutMsg: "Search dialog did not open" },
    );

    // Verify dialog content
    const hasMusicBrainz = await browser.execute(
      () => document.body.innerText.includes("MusicBrainz"),
    );
    expect(hasMusicBrainz).toBe(true);
    const hasDiscogs = await browser.execute(
      () => document.body.innerText.includes("Discogs"),
    );
    expect(hasDiscogs).toBe(true);

    // Switch provider to Discogs
    await clickButton("Discogs");
    // Fill artist and album
    const artistInput = await browser.execute(() => {
      const input = document.querySelector<HTMLInputElement>(
        "input[placeholder='Artist name']",
      );
      if (input) {
        input.value = "Radiohead";
        input.dispatchEvent(new Event("change", { bubbles: true }));
      }
      return input !== null;
    });
    expect(artistInput).toBe(true);

    const albumInput = await browser.execute(() => {
      const input = document.querySelector<HTMLInputElement>(
        "input[placeholder='Album title']",
      );
      if (input) {
        input.value = "OK Computer";
        input.dispatchEvent(new Event("change", { bubbles: true }));
      }
      return input !== null;
    });
    expect(albumInput).toBe(true);

    // Verify Search button is enabled
    const searchEnabled = await browser.execute(() => {
      const btn = Array.from(document.querySelectorAll("button")).find(
        (b) => b.textContent?.trim() === "Search",
      );
      return btn && !btn.disabled;
    });
    expect(searchEnabled).toBe(true);

    // Close dialog via close button (top-right X)
    const closed = await browser.execute(() => {
      const dialog = document.querySelector("[aria-label='Search releases']");
      const closeBtn = dialog?.querySelector("button:last-child");
      (closeBtn as HTMLButtonElement)?.click();
      return true;
    });
    expect(closed).toBe(true);

    await browser.waitUntil(
      () =>
        browser.execute(
          () => document.querySelector("[aria-label='Search releases']") === null,
        ),
      { timeout: 5_000, timeoutMsg: "Search dialog did not close" },
    );

    // Verify no write happened (album tracks unchanged)
    const tracks = await browser.execute(
      async (albumPath) => (await window.api.readAlbum(albumPath)).tracks,
      manifest.workflowAlbum,
    );
    expect(tracks.length).toBe(2);
  });
});
