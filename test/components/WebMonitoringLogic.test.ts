/**
 * These tests pin decisions, not formatting.
 *
 * The point of the module is that a person can read a schedule and a run's age
 * without doing arithmetic, and that the page never states something it cannot
 * stand behind — so the assertions below are about meaning (does the wording
 * match what cron will actually do?) and about refusal (does it decline to guess?).
 */

import { describe, expect, it } from "vitest";
import type { BackgroundRun, MonitoredRelease } from "../../src/shared/web-adapter";
import {
  describeCron,
  describeRunItemReason,
  describeRunItems,
  itemsNeedingAction,
  facetCounts,
  filterLog,
  filterReleases,
  formatRunDuration,
  levelCounts,
  levelOf,
  parseLog,
  relativeTime,
} from "../../src/web/monitoring";

const release = (path: string, status: MonitoredRelease["status"]): MonitoredRelease => ({
  path,
  status,
});

describe("relativeTime", () => {
  it("counts down to a future run, because a countdown is the whole point of a schedule", () => {
    const now = new Date("2026-09-19T10:00:00.000Z");
    expect(relativeTime("2026-09-19T12:14:00.000Z", now)).toBe("in 2h 14m");
    expect(relativeTime("2026-09-19T10:00:30.000Z", now)).toBe("just now");
  });

  it("says how long ago a past run was, in the largest unit that fits", () => {
    const now = new Date("2026-09-19T10:00:00.000Z");
    expect(relativeTime("2026-09-19T09:48:00.000Z", now)).toBe("12m ago");
    expect(relativeTime("2026-09-17T07:00:00.000Z", now)).toBe("2d 3h ago");
  });

  it("returns the raw value rather than inventing a time it cannot parse", () => {
    // A wrong countdown would look authoritative and be silently wrong.
    expect(relativeTime("not-a-date")).toBe("not-a-date");
    expect(relativeTime(null)).toBe("—");
  });
});

describe("formatRunDuration", () => {
  it("measures a finished run from its own start and end", () => {
    expect(formatRunDuration("2026-09-19T10:00:00.000Z", "2026-09-19T10:01:20.000Z")).toBe("1m 20s");
  });

  it("measures a run that has not finished against now, so a stuck run is visible", () => {
    const now = new Date("2026-09-19T10:05:00.000Z");
    expect(formatRunDuration("2026-09-19T10:00:00.000Z", null, now)).toBe("5m");
  });

  it("does not print a negative duration when clocks disagree", () => {
    expect(formatRunDuration("2026-09-19T10:00:00.000Z", "2026-09-19T09:59:00.000Z")).toBe("—");
  });
});

describe("facetCounts", () => {
  it("counts each status and leads with the ones needing attention", () => {
    const facets = facetCounts([
      release("/m/a", "healthy"),
      release("/m/b", "needs_review"),
      release("/m/c", "needs_review"),
      release("/m/d", "failed"),
    ]);
    expect(facets.map((f) => [f.label, f.count])).toEqual([
      ["Needs review", 2],
      ["Failed", 1],
      ["Tagged", 1],
    ]);
  });

  it("omits statuses nothing is in, so the filter row is not mostly zeroes", () => {
    expect(facetCounts([release("/m/a", "healthy")]).map((f) => f.status)).toEqual(["healthy"]);
  });
});

describe("filterReleases", () => {
  const all = [
    release("/music/Radiohead/Kid A", "needs_review"),
    release("/music/Portishead/Dummy", "healthy"),
    release("/music/Radiohead/Amnesiac", "failed"),
  ];

  it("narrows to one status, which is how a count becomes a way in", () => {
    expect(filterReleases(all, "needs_review", "").map((r) => r.path)).toEqual([
      "/music/Radiohead/Kid A",
    ]);
  });

  it("matches the path case-insensitively and combines with the status", () => {
    expect(filterReleases(all, "all", "radiohead").map((r) => r.path)).toEqual([
      "/music/Radiohead/Kid A",
      "/music/Radiohead/Amnesiac",
    ]);
    expect(filterReleases(all, "failed", "radiohead")).toHaveLength(1);
    expect(filterReleases(all, "failed", "portishead")).toHaveLength(0);
  });

  it("treats whitespace as no query rather than matching nothing", () => {
    expect(filterReleases(all, "all", "   ")).toHaveLength(3);
  });
});

describe("levelOf", () => {
  it("reads both shapes our own log lines use", () => {
    expect(levelOf("2026-09-19T10:00:00Z ERROR failed to write")).toBe("error");
    expect(levelOf("2026-09-19T10:00:00Z level=WARN skipped")).toBe("warn");
    expect(levelOf('2026-09-19T10:00:00Z level="INFO" started')).toBe("info");
  });

  it("does not assume an unrecognised line is info", () => {
    // Guessing "info" would let a real error be filtered out of sight.
    expect(levelOf("a bare continuation line")).toBe("unknown");
  });

  it("treats FATAL as an error, since that is strictly more severe", () => {
    expect(levelOf("FATAL could not bind")).toBe("error");
  });
});

describe("filterLog and levelCounts", () => {
  const lines = parseLog([
    "INFO started",
    "INFO scanning",
    "ERROR write failed",
    "WARN skipped",
    "orphan line",
  ]);

  it("counts every level so a filter can say what it would show before you click", () => {
    expect(levelCounts(lines)).toMatchObject({ info: 2, error: 1, warn: 1, unknown: 1 });
  });

  it("filters by level and by text together", () => {
    expect(filterLog(lines, "error", "").map((l) => l.text)).toEqual(["ERROR write failed"]);
    expect(filterLog(lines, "all", "scan").map((l) => l.text)).toEqual(["INFO scanning"]);
    expect(filterLog(lines, "info", "scan").map((l) => l.text)).toEqual(["INFO scanning"]);
  });

  it("includes unrecognised lines only when no level filter is applied", () => {
    expect(filterLog(lines, "unknown", "").map((l) => l.text)).toEqual(["orphan line"]);
    expect(filterLog(lines, "all", "").map((l) => l.text)).toHaveLength(5);
  });
});

describe("describeCron", () => {
  it("describes every preset the reference ships, in words a person can check", () => {
    // These are the presets the settings page offers; each one must read
    // unambiguously, because the raw expression is what people get wrong.
    expect(describeCron("*/1 * * * *")).toBe("Every minute");
    expect(describeCron("*/3 * * * *")).toBe("Every 3 minutes");
    expect(describeCron("*/10 * * * *")).toBe("Every 10 minutes");
    expect(describeCron("0 * * * *")).toBe("Hourly at minute 0");
    expect(describeCron("0 */2 * * *")).toBe("Every 2 hours at minute 0");
    expect(describeCron("0 0 * * *")).toBe("Every day at 00:00");
  });

  it("describes the common narrower schedules correctly", () => {
    expect(describeCron("30 3 * * *")).toBe("Every day at 03:30");
    expect(describeCron("0 9 * * 1")).toBe("Every Monday at 09:00");
    expect(describeCron("0 0 15 * *")).toBe("Day 15 of the month at 00:00");
  });

  it("refuses to describe an expression whose meaning it would get wrong", () => {
    // `* */2 * * *` fires every minute of every second hour. Wording it as
    // "every 2 hours" would understate it sixtyfold, so it must return null and
    // let the raw fields be shown instead.
    expect(describeCron("* */2 * * *")).toBeNull();
    expect(describeCron("0 0 * * 1,3,5")).toBeNull();
    expect(describeCron("0 0 1 1 *")).toBeNull();
  });

  it("rejects anything that is not five fields, rather than guessing a field's role", () => {
    expect(describeCron("0 0 * *")).toBeNull();
    expect(describeCron("0 0 * * * *")).toBeNull();
    expect(describeCron("")).toBeNull();
  });
});

const runWith = (items: BackgroundRun["items"], itemsTruncated = false): BackgroundRun => ({
  id: "bg-run-1-0",
  trigger: "manual",
  startedAt: "2026-09-19T10:00:00.000Z",
  finishedAt: "2026-09-19T10:01:00.000Z",
  status: "failed",
  tagged: 1,
  failed: 1,
  message: "Tagged 1, 1 failed",
  taskId: null,
  items,
  itemsTruncated,
});

describe("run ledger", () => {
  it("keeps only the releases a retry should cover, because retrying a healthy one re-tags the library", () => {
    const run = runWith([
      { path: "/music/A", status: "healthy", authority: "discogs" },
      { path: "/music/B", status: "failed", reason: "permission denied" },
      { path: "/music/C", status: "needs_review", reason: "no_authoritative_match" },
      { path: "/music/D", status: "tagging" },
    ]);

    expect(itemsNeedingAction(run).map((item) => item.path)).toEqual(["/music/B", "/music/C"]);
  });

  it("says what happened to a release from the reason, the source, or both", () => {
    // "Which source tagged this" and "why did it not" are different questions,
    // and a line that answers the wrong one is worse than a blank.
    expect(describeRunItemReason({ path: "/music/A", status: "healthy", authority: "discogs" }))
      .toBe("via discogs");
    expect(describeRunItemReason({ path: "/music/B", status: "failed", reason: "permission denied" }))
      .toBe("permission denied");
    expect(
      describeRunItemReason({
        path: "/music/C",
        status: "needs_review",
        reason: "no_authoritative_match",
        authority: "musicbrainz",
      }),
    ).toBe("no_authoritative_match · via musicbrainz");
    expect(describeRunItemReason({ path: "/music/D", status: "healthy" })).toBe("—");
  });

  it("admits when the ledger dropped releases instead of reading as a complete list", () => {
    // A capped ledger that reads as complete is how a failure gets missed.
    expect(describeRunItems(runWith([{ path: "/music/A", status: "healthy" }]))).toBe(
      "1 release recorded",
    );
    expect(
      describeRunItems(runWith([{ path: "/music/A", status: "healthy" }], true)),
    ).toContain("dropped");
  });
});
