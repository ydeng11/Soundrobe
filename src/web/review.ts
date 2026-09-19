/**
 * Which albums background tagging flagged for manual review.
 *
 * The service classifies release health for every path it has seen, and the
 * workspace only cares about the ones that need a person. That classification
 * is the service's, not the browser's: nothing here infers review state from
 * tag contents, because a folder can look complete and still have been left
 * alone deliberately.
 *
 * Pure helpers only, so the matching rule is testable without a server.
 */

import type { MonitoredRelease } from "../shared/web-adapter";

/**
 * Trailing separators are stripped before comparison. The service reports
 * directory paths as it walked them and the folder tree asks by the path it
 * was handed, which can differ by a trailing slash.
 */
export function normalizeAlbumPath(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  return trimmed === "" ? path : trimmed;
}

/** Albums the last classification left needing review, in stable order. */
export function needsReviewPaths(releases: MonitoredRelease[]): string[] {
  const paths = new Set<string>();
  for (const release of releases) {
    if (release.status === "needs_review") {
      paths.add(normalizeAlbumPath(release.path));
    }
  }
  return [...paths].sort((left, right) => left.localeCompare(right));
}

/** Membership test for the folder tree, which asks one path at a time. */
export function isNeedsReview(
  reviewPaths: ReadonlySet<string>,
  path: string,
): boolean {
  return reviewPaths.has(normalizeAlbumPath(path));
}

/** The trailing path segment, for showing an album by name rather than by path. */
export function albumNameOf(path: string): string {
  const segments = normalizeAlbumPath(path).split(/[/\\]/);
  return segments[segments.length - 1] ?? path;
}
