import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const tokens = readFileSync(new URL("../../src/web/tokens.css", import.meta.url), "utf8");
const web = readFileSync(new URL("../../src/web/web.css", import.meta.url), "utf8");

/**
 * The type scale is the workspace's vertical rhythm. Hard-coded sizes drift, and an
 * unpaired font-size falls back to the font's `normal` leading, which is tighter
 * than every pairing the reference theme uses — so both are contracts, not style.
 */
describe("browser workspace type scale", () => {
  it("declares no literal font sizes or line heights", () => {
    expect(web).not.toMatch(/font-size:\s*\d/);
    expect(web).not.toMatch(/line-height:\s*[\d.]/);
  });

  it("pairs every font size with a line height token", () => {
    const lines = web.split("\n");
    let depth = 0;
    let selector = "";
    let sizes = 0;
    let leadings = 0;
    const unpaired = [];
    lines.forEach((line) => {
      const t = line.trim();
      if (t.endsWith("{")) {
        if (depth === 0) {
          selector = t.replace("{", "").trim();
          sizes = 0;
          leadings = 0;
        }
        depth++;
      } else if (t === "}") {
        if (depth === 1 && sizes > 0 && leadings === 0) unpaired.push(selector);
        depth--;
      } else if (depth > 0 && /^font-size:/.test(t)) sizes++;
      else if (depth > 0 && /^line-height:/.test(t)) leadings++;
    });
    expect(unpaired).toEqual([]);
  });

  it("keeps the measured steps, and 10px is one of them", () => {
    // The reference uses 0.625rem (10px) for its count badge, so the smallest step
    // is measured rather than a one-off literal on the folder-tree twisty.
    expect(tokens).toMatch(/--sr-fs-3xs:\s*10px;/);
    for (const step of ["--sr-fs-base: 16px", "--sr-fs-body: 14px", "--sr-fs-sm: 13.12px", "--sr-fs-xs: 12.48px", "--sr-fs-2xs: 11.52px"]) {
      expect(tokens).toContain(step);
    }
    // Each leading is a pairing measured in the reference theme.
    expect(tokens).toMatch(/--sr-lh-display:\s*1\.12;/);
    expect(tokens).toMatch(/--sr-lh-tight:\s*1\.333;/);
    expect(tokens).toMatch(/--sr-lh-base:\s*1\.43;/);
    expect(tokens).toMatch(/--sr-lh-relaxed:\s*1\.5;/);
    expect(tokens).toMatch(/--sr-lh-glyph:\s*1;/);
  });
});
