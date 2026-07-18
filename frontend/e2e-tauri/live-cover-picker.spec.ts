import { execFile } from "node:child_process";

describe("Native cover picker", () => {
  const test = process.platform === "darwin" ? it : it.skip;

  test("returns null when the real image picker is cancelled", async () => {
    const cancelPicker = new Promise<void>((resolve, reject) => {
      setTimeout(() => {
        execFile(
          "osascript",
          [
            "-e",
            [
              'tell application "System Events"',
              'tell application process "soundrobe"',
              "set frontmost to true",
              "key code 53",
              "end tell",
              "end tell",
            ].join("\n"),
          ],
          (error) => (error ? reject(error) : resolve()),
        );
      }, 2_000);
    });

    const [result] = await Promise.all([
      browser.execute(async () => window.api.setCover("/private/tmp")),
      cancelPicker,
    ]);

    expect(result).toBeNull();
  });
});
