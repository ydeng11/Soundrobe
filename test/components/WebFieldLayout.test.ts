import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const web = readFileSync(new URL("../../src/web/web.css", import.meta.url), "utf8");

/**
 * The field layout is a contract, not styling.
 *
 * Three defects came from one rule set, each measured on the running service:
 *
 * - A hinted `.sr-field` has three children but the grid had two columns, so the
 *   hint auto-placed into the *label* column: "Required, unless an album is given"
 *   rendered 104px wide and wrapped to three lines in Search, and the Chinese-script
 *   note to seven lines in Settings, while the control column sat empty beside it.
 * - `white-space: nowrap` plus a 104px column ellipsised the labels themselves:
 *   nine of twelve setting names were truncated, one of them hiding 295px of text.
 * - The gap between a control and its own hint was the same as the gap between
 *   fields, so a hint read as belonging to the field below it.
 *
 * These assertions exist because each is invisible in a unit test and only shows up
 * as a cramped screen.
 */
describe("browser workspace field layout", () => {
  const rule = (selector: string): string => {
    const start = web.indexOf(selector);
    expect(start, `missing rule for ${selector}`).toBeGreaterThan(-1);
    return web.slice(start, web.indexOf("}", start));
  };

  it("puts a hint under its own control, not in the label column", () => {
    // Without this the third grid child lands in the label column and a sentence
    // wraps to three or seven lines in a gutter.
    const hint = rule(".sr-field:not(.sr-field--stacked) > .sr-field__static");
    expect(hint).toMatch(/grid-column:\s*2/);
  });

  it("never truncates a field label", () => {
    // A field name cut off mid-word is a missing label, not a compact layout.
    const label = rule(".sr-field__label {");
    expect(label).not.toMatch(/text-overflow:\s*ellipsis/);
    expect(label).not.toMatch(/white-space:\s*nowrap/);
  });

  it("renders a hint as an aside rather than at full body weight", () => {
    // At the body colour it competed with the value it described.
    const hint = rule(".sr-field > .sr-field__static");
    expect(hint).toMatch(/color:\s*var\(--sr-text-muted\)/);
    expect(hint).toMatch(/font-size:\s*var\(--sr-fs-xs\)/);
  });

  it("separates a control from its hint more tightly than two fields", () => {
    const field = rule(".sr-field {");
    const gap = field.match(/gap:\s*([\d.]+)px\s+([\d.]+)px/);
    expect(gap, "the field gap must be an explicit row/column pair").toBeTruthy();
    const rowGap = Number(gap?.[1]);
    const columnGap = Number(gap?.[2]);
    // The section's own gap (10px) is what separates fields, so the control-to-hint
    // step has to be smaller than it or the hint groups with the wrong field.
    expect(rowGap).toBeLessThan(10);
    expect(rowGap).toBeLessThan(columnGap);
  });

  it("keeps the label gutter wide enough for a real field name", () => {
    const field = rule(".sr-field {");
    const columns = field.match(/grid-template-columns:\s*(\d+)px/);
    expect(columns).toBeTruthy();
    // 104px could not hold "Lyrics service URL" (119px) without an ellipsis.
    expect(Number(columns?.[1])).toBeGreaterThanOrEqual(120);
  });
});
