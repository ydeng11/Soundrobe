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
 *   just fe-smoke-search
 */
describe("Live manual search workflow", () => {
  it("searches MusicBrainz for Radiohead — OK Computer, previews match, cancels without writing", async () => {
    // Open library first
    await browser.execute(() => window.api.openFolderDialog());
    const openLibrary = await $("button=Open Library");
    await openLibrary.click();
    await browser.waitUntil(
      async () =>
        browser.execute(() => document.body.innerText.includes("Workflow One")),
      { timeout: 15_000 },
    );

    // Click Search button
    await $("button=Search").click();
    await browser.waitUntil(
      () =>
        browser.execute(
          () => document.querySelector("[aria-label='Search releases']") !== null,
        ),
      { timeout: 5_000 },
    );

    // Fill search form
    await browser.execute(() => {
      const inputs = document.querySelectorAll<HTMLInputElement>("input");
      for (const input of inputs) {
        if (input.placeholder?.includes("Artist")) input.value = "Radiohead";
        if (input.placeholder?.includes("Album")) input.value = "OK Computer";
      }
      // Trigger React onChange
      inputs.forEach((input) =>
        input.dispatchEvent(new Event("change", { bubbles: true })),
      );
    });

    // Click Search
    await $("button=Search").click();

    // Wait for results
    await browser.waitUntil(
      async () => {
        const text = await browser.execute(() => document.body.innerText);
        return text.includes("OK Computer");
      },
      { timeout: 15_000, timeoutMsg: "Search results did not appear" },
    );

    // Click first result to open detail
    await $("button=OK Computer").click();

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
  });
});
