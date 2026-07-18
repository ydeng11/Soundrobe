describe("Tauri desktop shell", () => {
  it("boots the renderer and exposes the native application contract", async () => {
    await expect(browser).toHaveTitle("Soundrobe");
    const openLibrary = await $("button=Open Library");
    await openLibrary.waitForDisplayed();

    const info = await browser.tauri.execute(async ({ core }) =>
      core.invoke<{ identifier: string; runtime: string; version: string }>("app_info"),
    );

    expect(info.identifier).toBe("com.ihelio.soundrobe");
    expect(info.runtime).toBe("tauri");
    expect(info.version).toBe("0.1.0");
  });
});
