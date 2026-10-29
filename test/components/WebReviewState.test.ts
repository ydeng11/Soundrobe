/**
 * Review state shown in the workspace.
 *
 * The point of these cases is that a flagged album has to be findable and
 * fixable from the workspace, so a mis-keyed path would silently hide the album
 * a person was told to look at. That is why the trailing-separator cases are
 * here rather than left to the service.
 */

import { describe, expect, it } from "vitest";
import {
  albumNameOf,
  isNeedsReview,
  needsReviewPaths,
  normalizeAlbumPath,
} from "../../src/web/review";
import type { MonitoredRelease } from "../../src/shared/web-adapter";

function release(path: string, status: MonitoredRelease["status"]): MonitoredRelease {
  return { path, status };
}

describe("normalizeAlbumPath", () => {
  it("treats a directory with and without a trailing slash as the same album", () => {
    expect(normalizeAlbumPath("/music/Artist/Album/")).toBe("/music/Artist/Album");
    expect(normalizeAlbumPath("/music/Artist/Album")).toBe("/music/Artist/Album");
  });

  it("keeps a lone root separator instead of collapsing it to nothing", () => {
    // Stripping this would produce an empty string that matches no folder.
    expect(normalizeAlbumPath("/")).toBe("/");
  });
});

describe("needsReviewPaths", () => {
  it("keeps only the albums the service left needing review", () => {
    const paths = needsReviewPaths([
      release("/music/A/One", "needs_review"),
      release("/music/A/Two", "healthy"),
      release("/music/A/Three", "needs_tagging"),
      release("/music/A/Four", "failed"),
    ]);
    expect(paths).toEqual(["/music/A/One"]);
  });

  it("reports an album once even when the service classified it twice", () => {
    // The survey and the last run can both write the same album.
    const paths = needsReviewPaths([
      release("/music/A/One", "needs_review"),
      release("/music/A/One/", "needs_review"),
    ]);
    expect(paths).toEqual(["/music/A/One"]);
  });

  it("does not report a failed album as needing review", () => {
    // A failure is the service's problem, not a tagging judgement to review.
    expect(needsReviewPaths([release("/music/A/One", "failed")])).toEqual([]);
  });
});

describe("isNeedsReview", () => {
  it("matches the folder tree's path to the service's path", () => {
    const flagged = new Set(needsReviewPaths([release("/music/A/One", "needs_review")]));
    expect(isNeedsReview(flagged, "/music/A/One")).toBe(true);
    expect(isNeedsReview(flagged, "/music/A/One/")).toBe(true);
    expect(isNeedsReview(flagged, "/music/A/Two")).toBe(false);
  });
});

describe("albumNameOf", () => {
  it("names an album without its parent path", () => {
    expect(albumNameOf("/music/Artist Alpha/Reference Album One")).toBe(
      "Reference Album One",
    );
    expect(albumNameOf("/music/Artist Alpha/Reference Album One/")).toBe(
      "Reference Album One",
    );
  });
});
