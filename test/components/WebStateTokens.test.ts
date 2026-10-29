import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const tokens = readFileSync(new URL("../../src/web/tokens.css", import.meta.url), "utf8");
const web = readFileSync(new URL("../../src/web/web.css", import.meta.url), "utf8");

describe("browser workspace state-token contract", () => {
  it("keeps state roles tied to the measured reference palette", () => {
    expect(tokens).toMatch(/--sr-surface-strong:\s*oklch\(0\.924169\s+0\.00108\s+197\.138\)/);
    expect(tokens).toMatch(/--sr-neutral-content:\s*oklch\(0\.978315\s+0\.006688\s+295\.453\)/);
    expect(tokens).toMatch(/--sr-hover:\s*var\(--sr-surface-strong\)/);
    expect(tokens).toMatch(/--sr-selected:\s*var\(--sr-neutral\)/);
    const hoverValue = tokens.match(/--sr-hover:\s*([^;]+);/)?.[1];
    const selectedValue = tokens.match(/--sr-selected:\s*([^;]+);/)?.[1];
    expect(selectedValue).not.toBe(hoverValue);
    expect(tokens).toMatch(/--sr-selected-text:\s*var\(--sr-neutral-content\)/);
    expect(tokens).toMatch(/--sr-focus-ring:\s*var\(--sr-primary\)/);
    expect(tokens).toMatch(/--sr-disabled-opacity:\s*0\.5/);

    for (const token of ["--sr-hover", "--sr-selected", "--sr-selected-text", "--sr-focus-ring", "--sr-disabled-opacity"]) {
      expect(web).toContain(`var(${token})`);
    }
  });

  it("does not let hover disappear into the page surface", () => {
    expect(web).not.toMatch(/:hover[^{}]*\{[^}]*var\(--sr-surface-soft\)/s);
  });

  it("keeps selected treatment distinct from hover and gives every control a ring", () => {
    expect(web).toMatch(/\.sr-table tbody tr\.is-selected td\s*\{[^}]*var\(--sr-selected\)[^}]*var\(--sr-selected-text\)/s);
    expect(web).toMatch(/\.sr-nav__item\.is-active\s*\{[^}]*var\(--sr-selected\)[^}]*var\(--sr-selected-text\)/s);
    expect(web).toMatch(/outline:\s*2px solid var\(--sr-focus-ring\)/);
    expect(web).toMatch(/\.sr-textarea:focus-visible/);
    expect(web).toMatch(/\.sr-table thead th button:focus-visible/);
    expect(web).toMatch(/\.sr-cell--name span:focus-visible/);
    expect(web).toMatch(/\.sr-crumbs__item:focus-visible/);
    expect(web).not.toContain("filter: brightness(1.05)");
  });

  /**
   * The primary action's label is a dark colour on the reference's primary fill.
   * White on that fill is 2.72:1 and the darker step gave only 3.72:1; with a dark
   * label the hover must go LIGHTER, because darkening the fill drops the label to
   * 3.94:1 and breaks the floor.
   */
  it("keeps the primary button label legible at rest and while hovered", () => {
    expect(tokens).toMatch(/--sr-primary-hover:\s*oklch\(0\.73\s+0\.159168\s+293\.541\)/);
    const base = web.match(/\.sr-btn--primary\s*\{([^}]*)\}/s)?.[1] ?? "";
    expect(base).toContain("background: var(--sr-primary)");
    expect(base).toContain("color: var(--sr-text)");
    const hover = web.match(/\.sr-btn--primary:hover:not\(:disabled\)\s*\{([^}]*)\}/s)?.[1] ?? "";
    expect(hover).toContain("var(--sr-primary-hover)");
    expect(hover).toContain("var(--sr-text)");
    expect(web).not.toContain("--sr-primary-strong");
    // The white-on-accent role is gone entirely: it only ever existed for this label,
    // where it measured 2.72:1. Leaving the token invites the pairing back.
    expect(tokens).not.toMatch(/--sr-text-on-accent/);
    expect(web).not.toContain("var(--sr-text-on-accent)");
  });

  /**
   * 0.65 is a DELIBERATE DIVERGENCE: the reference does use 40% (var(--bc)/.4
   * appears six times), but 40% carries text in 12 rules at 2.33:1. The floor wins,
   * and the divergence is recorded in parity-matrix section 19.
   */
  it("keeps every text role at or above the accessibility floor", () => {
    expect(tokens).toMatch(/--sr-text-muted:\s*oklch\([^)]*\/\s*0\.65\)/);
    // The reference's 0.62 step is kept, but nothing may use it for body-sized text.
    expect(tokens).toMatch(/--sr-text-soft:\s*oklch\([^)]*\/\s*0\.62\)/);
    expect(web).not.toContain("var(--sr-text-soft)");
  });

  /**
   * The reference tints a status colour with its own alpha (bg-warning/20) rather
   * than a hand-picked hex, and it DOES pair status colours with text. Its base
   * status colours are unreadable as text on those tints (1.46:1 to 2.67:1), so
   * every status TEXT role uses the darkened step.
   */
  it("uses reference-measured status colours, tints and text roles", () => {
    expect(tokens).toMatch(/--sr-secondary:\s*oklch\(0\.894268\s+0\.054882\s+293\.283\)/);
    expect(tokens).toMatch(/--sr-success:\s*oklch\(0\.648\s+0\.15\s+160\)/);
    expect(tokens).toMatch(/--sr-warning:\s*oklch\(0\.8471\s+0\.199\s+83\.87\)/);
    expect(tokens).toMatch(/--sr-danger:\s*oklch\(0\.7176\s+0\.221\s+22\.18\)/);
    expect(tokens).toMatch(/--sr-success-soft:\s*oklch\(0\.648\s+0\.15\s+160\s*\/\s*0\.1\)/);
    expect(tokens).toMatch(/--sr-warning-soft:\s*oklch\(0\.8471\s+0\.199\s+83\.87\s*\/\s*0\.2\)/);
    expect(tokens).toMatch(/--sr-danger-soft:\s*oklch\(0\.7176\s+0\.221\s+22\.18\s*\/\s*0\.1\)/);
    for (const status of ["success", "warning", "danger"]) {
      expect(tokens).toMatch(new RegExp("--sr-" + status + "-text:\\s*oklch\\([^)]+\\)"));
      // badge, banner and any other status text must use the text role
      expect(web).not.toMatch(new RegExp("color:\\s*var\\(--sr-" + status + "\\);"));
    }
    expect(web).toMatch(/\.sr-badge--warning\s*\{[^}]*var\(--sr-warning-soft\)[^}]*var\(--sr-warning-text\)/s);
    expect(web).toMatch(/\.sr-banner--notice\s*\{[^}]*var\(--sr-success-soft\)[^}]*var\(--sr-success-text\)/s);
    expect(web).toMatch(/\.sr-banner--error\s*\{[^}]*var\(--sr-danger-soft\)[^}]*var\(--sr-danger-text\)/s);
  });

  it("keeps a review badge readable inside a selected folder row", () => {
    expect(web).toMatch(
      /\.sr-tree__node\.is-active \.sr-badge--warning\s*\{[^}]*background:\s*var\(--sr-warning\)[^}]*color:\s*var\(--sr-text\)/s,
    );
  });

  it("keeps compact editor actions visible while the mobile dock scrolls", () => {
    expect(web).toMatch(
      /@media \(max-width: 900px\) \{[\s\S]*?\.sr-editor--compact \.sr-editor__actions\s*\{[^}]*position:\s*sticky[^}]*bottom:\s*0[^}]*background:\s*var\(--sr-surface\)/s,
    );
    expect(web).toContain("scroll-padding-bottom: var(--sr-control-h)");
  });

  it("wraps Monitoring coverage without changing the editor's technical grid", () => {
    expect(web).toMatch(
      /@media \(max-width: 900px\) \{[\s\S]*?\.sr-main--monitoring \.sr-technical--monitoring\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/s,
    );
    expect(web).toMatch(
      /\.sr-main--monitoring \.sr-technical--monitoring \.sr-technical__row > span:last-child\s*\{[^}]*min-width:\s*0[^}]*overflow-wrap:\s*anywhere/s,
    );
  });

  it("keeps release actions within a narrow Monitoring row", () => {
    expect(web).toMatch(
      /\.sr-table--releases\s*\{[^}]*table-layout:\s*fixed/s,
    );
    expect(web).toMatch(
      /\.sr-release-path\s*\{[^}]*overflow-wrap:\s*anywhere/s,
    );
    expect(web).toMatch(
      /@media \(max-width: 900px\) \{[\s\S]*?\.sr-release-actions \.sr-btn\s*\{[^}]*height:\s*auto[^}]*white-space:\s*normal/s,
    );
  });

  it("keeps page actions and section navigation reachable while long pages scroll", () => {
    expect(web).toMatch(
      /\.sr-main--sources \.sr-page-header\s*\{[^}]*position:\s*sticky/s,
    );
    expect(web).toMatch(/\.sr-tabs--settings\s*\{[^}]*position:\s*sticky/s);
    expect(web).toMatch(
      /@media \(max-width: 900px\) \{[\s\S]*?\.sr-summary\s*\{[^}]*grid-template-columns:\s*repeat\(2,/s,
    );
  });

  it("uses a denser row height only for the album workspace table", () => {
    expect(tokens).toMatch(/--sr-row-h:\s*73px/);
    expect(tokens).toMatch(/--sr-album-row-h:\s*53px/);
    expect(web).toMatch(
      /\.sr-pane--workspace \.sr-table tbody tr,\s*\.sr-pane--workspace \.sr-table tbody td\s*\{[^}]*height:\s*var\(--sr-album-row-h\)/s,
    );
  });

  it("aligns compact editor actions to the right on desktop", () => {
    expect(web).toMatch(
      /\.sr-editor--compact \.sr-editor__actions\s*\{[^}]*justify-content:\s*flex-end/s,
    );
  });
});
