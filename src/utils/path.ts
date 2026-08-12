/**
 * Cross-platform path utilities for the renderer process.
 *
 * Electron paths from the main process use the OS-native separator
 * (backslash on Windows, forward slash on POSIX). These helpers
 * normalize to forward slashes so splitting logic works everywhere.
 */

/** Convert a path to use forward slashes only. */
export function toPosixPath(p: string): string {
  return p.replace(/\\/g, "/");
}

/** Get the parent directory path (normalized to forward slashes). */
export function dirname(p: string): string {
  return toPosixPath(p).split("/").slice(0, -1).join("/");
}

/** Get the last segment of a path (normalized to forward slashes). */
export function basename(p: string): string {
  const segments = toPosixPath(p).split("/").filter(Boolean);
  return segments.at(-1) ?? p;
}

/** Get the last N segments of a path joined by "/". Default depth = 4. */
export function shortPath(p: string, depth = 4): string {
  return toPosixPath(p).split("/").filter(Boolean).slice(-depth).join("/");
}

/** Check whether `filePath` is inside `directoryPath` (path-boundary safe). */
export function isInsideDirectory(filePath: string, directoryPath: string): boolean {
  const normalized = toPosixPath(filePath);
  const dir = toPosixPath(directoryPath).replace(/\/+$/, "");
  return normalized.startsWith(dir + "/") || normalized === dir;
}
