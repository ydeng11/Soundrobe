import type { E2eManifest } from "./fixtures";
const fixture = JSON.parse(process.env.SOUNDROBE_E2E_MANIFEST ?? "null") as E2eManifest | null;
const phase = process.env.SOUNDROBE_E2E_AUTHORITY_PHASE;
if (!fixture || !["unmarked", "owner-edit", "foreign", "owner-restart"].includes(phase ?? "")) {
  throw new Error("Run the authority scenario with npm run test:e2e:authority");
}

async function textIncludes(part: string) {
  await browser.waitUntil(() => browser.execute((value) => document.body.innerText.includes(value), part), {
    timeout: 12_000, timeoutMsg: `Did not see ${part}`,
  });
}
async function applyTitle(title: string) {
  await $("input[placeholder='Track title']").setValue(title);
  const apply = await $("button=Apply Changes");
  await apply.waitForEnabled();
  await apply.click();
}
async function reviewWithTitle(title: string, album: string) {
  await $("button=Results").click();
  await textIncludes(album);
  const clicked = await browser.execute((albumPath) => {
    const button = [...document.querySelectorAll("button")].find((element) =>
      element.textContent?.trim() === "Review album" && element.parentElement?.innerText.includes(albumPath));
    button?.click();
    return Boolean(button);
  }, album);
  expect(clicked).toBe(true);
  await textIncludes(title);
}
// The embedded WebDriver cannot retrieve WKWebView's native confirmation
// dialog. Accept the expected prompt and exercise the real UI/native action.
async function confirmAction(button: string, expectedPrompt: string) {
  await browser.execute(() => {
    const state = window as Window & { authorityConfirm?: string };
    window.confirm = (message?: string) => { state.authorityConfirm = message; return true; };
  });
  await $(`button=${button}`).click();
  await browser.waitUntil(() => browser.execute((expected) =>
    (window as Window & { authorityConfirm?: string }).authorityConfirm?.includes(expected) === true,
    expectedPrompt,
  ));
}

describe(`Cross-profile authority desktop: ${phase}`, () => {
  it("uses the native library, metadata inspector and persisted Results", async () => {
    await $("button=Open Library").click();
    await textIncludes(phase === "unmarked" || phase === "owner-edit" ? "Workflow One" : "Authority Owner Title");
    const selected = await browser.execute((target) => {
      const row = [...document.querySelectorAll<HTMLElement>("[data-testid^='file-row-']")]
        .find((element) => element.dataset.testid === `file-row-${target}`);
      row?.click();
      return Boolean(row);
    }, fixture.workflowTrack);
    expect(selected).toBe(true);
    await $("input[placeholder='Track title']").waitForDisplayed();
    if (phase === "unmarked") {
      await applyTitle("Unmarked edit must fail");
      await textIncludes("library has not been nominated for writing");
    } else if (phase === "owner-edit") {
      await applyTitle("Authority Owner Title");
      await textIncludes("Authority Owner Title");
    } else if (phase === "foreign") {
      await applyTitle("Foreign profile must fail");
      await textIncludes("library belongs to a different writer profile");
      await reviewWithTitle("An album reversal was interrupted", fixture.convertAlbum);
      await confirmAction("Resume album reversal", "Restore this album");
      await textIncludes("library belongs to a different writer profile");
    } else if (phase === "owner-restart") {
      await reviewWithTitle("Resolve the interrupted write", fixture.workflowAlbum);
      await confirmAction("Keep current file", "Keep the current contents");
      await textIncludes("Its interrupted-write review is closed");
      await $("button=Done").click();
      await reviewWithTitle("An album reversal was interrupted", fixture.convertAlbum);
      await confirmAction("Resume album reversal", "Restore this album");
      await textIncludes("Applied · Reverted");
    }
  });
});
