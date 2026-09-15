import fs from "node:fs";
import { createHash } from "node:crypto";
import type { E2eManifest } from "./fixtures";

const manifest = JSON.parse(
  process.env.SOUNDROBE_E2E_MANIFEST ?? "null",
) as E2eManifest | null;

if (!manifest) {
  throw new Error("SOUNDROBE_E2E_MANIFEST is required");
}

/**
 * Live Search — requires live MusicBrainz/Discogs network access.
 * Skipped in the default CI suite; run explicitly:
 *   npm run build:e2e && npx wdio run wdio.conf.ts --spec e2e-tauri/live-search.spec.ts
 */
describe("Live manual search workflow", () => {
  it("searches MusicBrainz for Radiohead — OK Computer, previews match, cancels without writing", async function () {
    this.timeout(180_000);
    const beforeHash = createHash("sha256").update(fs.readFileSync(manifest.workflowTrack)).digest("hex");
    // Open library first
    await browser.execute(() => window.api.openFolderDialog());
    const openLibrary = await $("button=Open Library");
    await openLibrary.click();
    await browser.waitUntil(
      async () =>
        browser.execute(() => document.body.innerText.includes("Workflow One")),
      { timeout: 15_000 },
    );

    // Select the album in the sidebar; selecting a track leaves All Files active.
    await browser.execute(() => {
      const button = Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
        .find((candidate) => candidate.textContent?.includes("Workflow Album"));
      if (!button) throw new Error("Fixture album button not found");
      button.click();
    });
    await $("button=Search").waitForEnabled();
    await $("button=Search").click();
    await browser.waitUntil(
      () =>
        browser.execute(
          () => document.querySelector("[aria-label='Search releases']") !== null,
        ),
      { timeout: 5_000 },
    );

    // Native value setter dispatches the input event React observes.
    await browser.execute(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      for (const [placeholder, value] of [["Artist name", "Radiohead"], ["Album title", "OK Computer"]]) {
        const input = document.querySelector<HTMLInputElement>(`input[placeholder="${placeholder}"]`)!;
        setter.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }
    });
    await browser.execute(() => {
      const dialog = document.querySelector("[aria-label='Search releases']")!;
      const search = Array.from(dialog.querySelectorAll("button")).find((button) => button.textContent?.trim() === "Search")!;
      search.click();
    });
    await browser.waitUntil(
      () => browser.execute(() => !!document.querySelector("[aria-label='Filter track count']")),
      { timeout: 120_000, timeoutMsg: "Search catalog did not load" },
    );
    expect(await browser.execute(() => document.body.innerText.includes("Local album: 2 tracks"))).toBe(true);
    expect(await browser.execute(() => {
      const filter = document.querySelector<HTMLSelectElement>("[aria-label='Filter track count']")!;
      return !filter.disabled && Array.from(filter.options).some((option) => option.value === "12");
    })).toBe(true);

    // The dedicated native count command returns a complete total without enrichment.
    const count = await browser.execute(async () => {
      const page = await window.api.searchReleases({ provider: "musicbrainz", artist: "Radiohead", album: "OK Computer", pageSize: 1 });
      const release = page.results[0];
      if (!release) throw new Error("No release available for native count lookup");
      return { summary: release.trackCount, loaded: await window.api.releaseTrackCount(release.provider, release.id, release.kind) };
    });
    expect(count.loaded).toBeGreaterThan(0);
    if (count.summary !== undefined) expect(count.loaded).toBe(count.summary);

    // Exact local count excludes the 12-track editions; clearing restores them.
    await browser.execute(() => {
      document.querySelector<HTMLInputElement>("[aria-label='Search releases'] input[type='checkbox']")!.click();
    });
    expect(await browser.execute(() => {
      const dialog = document.querySelector("[aria-label='Search releases']")!;
      return !Array.from(dialog.querySelectorAll("button")).some((button) =>
        button.textContent?.includes("OK Computer") && button.textContent.includes("12 tracks"));
    })).toBe(true);
    await browser.execute(() => {
      Array.from(document.querySelectorAll("button")).find((button) => button.textContent === "Clear filters")!.click();
      const select = document.querySelector<HTMLSelectElement>("[aria-label='Filter track count']")!;
      select.value = "12";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await browser.execute(() => {
      const dialog = document.querySelector("[aria-label='Search releases']")!;
      Array.from(dialog.querySelectorAll("button")).find((button) => button.textContent?.includes("OK Computer"))!.click();
    });

    // Wait for track listing
    await browser.waitUntil(
      async () => {
        const text = await browser.execute(() => document.body.innerText);
        return text.includes("Airbag") || text.includes("Select this release");
      },
      { timeout: 10_000, timeoutMsg: "Detail view did not load" },
    );

    // Click Select this release
    await $("button=Select this release").click();

    // Confirm dialog should open
    await browser.waitUntil(
      () =>
        browser.execute(
          () =>
            document.querySelector("[aria-label='Confirm track mapping']") !== null,
        ),
      { timeout: 5_000, timeoutMsg: "Confirm dialog did not open" },
    );

    // Click Cancel — no metadata should be written
    await $("button=Cancel").click();

    await browser.waitUntil(
      () =>
        browser.execute(
          () =>
            document.querySelector("[aria-label='Confirm track mapping']") === null,
        ),
      { timeout: 5_000, timeoutMsg: "Confirm dialog did not close" },
    );

    // Verify original tags are preserved
    const tracks = await browser.execute(
      async (albumPath) => (await window.api.readAlbum(albumPath)).tracks,
      manifest.workflowAlbum,
    );
    expect(tracks[0].title).toBe("Workflow One");
    expect(createHash("sha256").update(fs.readFileSync(manifest.workflowTrack)).digest("hex")).toBe(beforeHash);
  });
});
