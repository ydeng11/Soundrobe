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
  runFileCoverage,
  runEvidenceLines,
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


describe("durable auto-tag evidence", () => {
  it("separates written files from complete release coverage and explains provider outages", () => {
    const lines = runEvidenceLines({
      path: "/music/Partial", status: "needs_review",
      evidence: {
        written: 1,
        providerAttempts: [{provider: "musicbrainz", status: "unavailable", diagnostic: "HTTP 503", retryAfterSeconds: 90}],
        diagnostics: [{stage: "coverage", localFiles: 1, verifiedTracks: 1, writtenFiles: 1, releaseTracks: 12, completeAlbum: false}],
      },
    });
    expect(lines.join("\n")).toContain("musicbrainz: unavailable (HTTP 503)");
    expect(lines.join("\n")).toContain("1/1 files verified; 1 written; 12 release tracks; album incomplete");
    expect(runEvidenceLines({path: "/old", status: "needs_review"})).toEqual(["No diagnostic evidence was retained for this run."]);
  });
});


describe("file coverage", () => {
  it("counts partial file writes without pretending folders or editions are complete", () => {
    const run = runWith([{path:"/partial",status:"needs_review",evidence:{written:2,diagnostics:[{stage:"coverage",localFiles:3,writtenFiles:2,providerVerifiedCoreTracks:2,providerVerifiedAlbumTracks:0,providerVerifiedEditionTracks:0,fieldCoverage:{album:0,editionId:0}}]}}]);
    expect(runFileCoverage(run)).toEqual({filesConsidered:3,filesWritten:2,foldersWithWrites:1,providerVerifiedCoreTracks:2,albumVerifiedTracks:0,editionVerifiedTracks:0});
    const old = runWith([{path:"/old",status:"needs_review",evidence:{diagnostics:[{stage:"coverage",localFiles:3,writtenFiles:2,fieldCoverage:{album:2,editionId:0}}]}}]);
    expect(runFileCoverage(old)).toMatchObject({filesWritten:2,providerVerifiedCoreTracks:null,albumVerifiedTracks:null,editionVerifiedTracks:null});
    expect(runFileCoverage(runWith([]))).toBeNull();
    expect(runFileCoverage({...run,itemsTruncated:true})).toBeNull();
  });
});

it("shows conflicting candidate identities instead of making a Halo collision look like a match", () => {
  const lines = runEvidenceLines({path:"/music/Halo",status:"needs_review",evidence:{diagnostics:[{stage:"acoustid",validationComplete:true,trackMatches:[{fileName:"Halo Radio Edit.mp3",reason:"AcoustID returned tied recording identities",candidateCount:3,omittedCandidates:0,candidates:[
    {recordingId:"beyonce-id",authoritativeTitle:"Halo",artistCredits:["Beyoncé"],score:0.968,disambiguation:"radio edit",disposition:"supported"},
    {recordingId:"luke-id",authoritativeTitle:"Even Though I'm Leaving",artistCredits:["Luke Combs"],score:0.968,disposition:"supported"},
  ]}]}]}});
  expect(lines.join("\n")).toContain("Beyoncé");
  expect(lines.join("\n")).toContain("Luke Combs");
  expect(lines.join("\n")).toContain("beyonce-id");
  expect(lines.join("\n")).toContain("luke-id");
  expect(lines.join("\n")).toContain("radio edit");
  expect(lines.join("\n")).toContain("tied recording identities");
});

it("bounds detailed evidence and discloses omitted candidates and unfinished validation", () => {
  const lines=runEvidenceLines({path:"/music/Many",status:"needs_review",evidence:{diagnostics:[{stage:"acoustid",validationComplete:false,trackMatches:Array.from({length:10},(_,i)=>({fileName:`${i}.mp3`,candidateCount:24,omittedCandidates:4,candidates:Array.from({length:20},(_,j)=>({recordingId:`${i}-${j}`,authoritativeTitle:"Song",artistCredits:["Singer"],score:0.95,disposition:"supported"}))}))}]}});
  expect(lines.filter(line=>line.startsWith("Candidate:")).length).toBeLessThanOrEqual(48);
  expect(lines.join("\n")).toContain("omitted");
  expect(lines.join("\n")).toContain("Validation unfinished");
});

describe("track-core completion scope", () => {
  it("explains verified physical files without requiring edition tags or writes", () => {
    const lines = runEvidenceLines({
      path: "/music/Personal Collection", status: "healthy",
      evidence: { completionBasis: "verified_track_core", diagnostics: [{
        stage: "coverage", localFiles: 2, verifiedTracks: 2, writtenFiles: 0,
        completeTrackCore: true, completeAlbum: false,
      }] },
    });
    expect(lines).toContain("All 2 physical files had verified titles and artists; edition unresolved.");
    expect(lines.join(" ")).not.toContain("album incomplete");
    expect(runEvidenceLines({ path: "/music/Single", status: "healthy",
      evidence: { completionBasis: "verified_track_core", diagnostics: [{
        stage: "coverage", localFiles: 1, completeTrackCore: true,
      }] },
    })).toContain("The physical file had a verified title and artists; edition unresolved.");
  });

  it("does not infer strict completion from old field counts", () => {
    const lines = runEvidenceLines({
      path: "/music/Legacy", status: "healthy",
      evidence: { diagnostics: [{ stage: "coverage", localFiles: 2,
        verifiedTracks: 2, writtenFiles: 2, completeAlbum: false }] },
    });
    expect(lines.join(" ")).not.toContain("had verified titles and artists");
  });
});

it("labels core-only completion with its actual scope", () => {
  expect(describeRunItemReason({path: "/music/Collection", status: "healthy",
    reason: "verified_track_core", authority: "musicbrainz"}))
    .toBe("Track titles/artists verified; edition unresolved · via musicbrainz");
});

it("explains a format readback conflict without promoting provider field counts", () => {
  const lines = runEvidenceLines({path: "/music/APE", status: "needs_review",
    evidence: {diagnostics: [{stage: "acoustid", coreReadbackMatched: false},
      {stage: "coverage", localFiles: 1, verifiedTracks: 1, completeTrackCore: false}]}});
  expect(lines).toContain("Written title or ordered artist credits did not match the verified fields.");
  expect(lines.join(" ")).not.toContain("had verified titles and artists");
});
