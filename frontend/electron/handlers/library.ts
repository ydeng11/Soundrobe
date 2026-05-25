import { ipcMain } from "electron";
import fs from "fs";
import path from "path";
import { readAlbum } from "./tracks";

export interface AlbumInfo {
  path: string;
  name: string;
  artistHint: string;
  albumHint: string;
  trackCount: number;
}

const SUPPORTED_EXTENSIONS = new Set([
  ".mp3",
  ".flac",
  ".m4a",
  ".mp4",
  ".wav",
  ".ogg",
  ".opus",
  ".aiff",
]);

export function isAudioFile(filePath: string): boolean {
  const ext = path.extname(filePath).toLowerCase();
  return SUPPORTED_EXTENSIONS.has(ext);
}

function isHiddenDir(dirName: string): boolean {
  return dirName.startsWith(".");
}

/**
 * Guess the artist hint from a path segment.
 * Common patterns: "Artist/Album", "Artist - Album", "Artist/Year - Album"
 */
export function parseArtistAlbumHint(
  dirPath: string,
  parentDir: string
): { artistHint: string; albumHint: string } {
  const dirName = path.basename(dirPath);

  // Parent directory is likely the artist
  // The directory name is likely the album
  // But handle "Artist - Album" flat pattern
  const dashMatch = dirName.match(/^(.+?)\s*-\s*(.+)$/);
  if (dashMatch) {
    const artistCandidate = dashMatch[1].trim();
    const isYear = /^\d{4}$/.test(artistCandidate);
    if (!isYear) {
      // Plausible "Artist - Album" — use dash parsing
      return { artistHint: artistCandidate, albumHint: dashMatch[2].trim() };
    }
    // Looks like "2025 - Album" — fall through to use parent dir
  }

  // Standard hierarchy: parent = artist, dir = album
  return {
    artistHint: parentDir || "",
    albumHint: dirName,
  };
}

interface ScanResult {
  albums: Map<string, AlbumInfo>;
  albumAudioFiles: Map<string, string[]>;
}

/**
 * Walk a directory tree, group audio files by album directory,
 * return album metadata + per-album file lists.
 */
export function scanDirectory(libraryPath: string): ScanResult {
  const albums = new Map<string, AlbumInfo>();
  const albumAudioFiles = new Map<string, string[]>();

  if (!fs.existsSync(libraryPath)) {
    throw new Error(`Library path not found: ${libraryPath}`);
  }

  const stat = fs.statSync(libraryPath);
  if (stat.isFile()) {
    // Single file — wrap as one "album"
    const parentDir = path.dirname(libraryPath);
    const parentName = path.basename(parentDir);
    const grandParent = path.dirname(parentDir);
    const grandName = path.basename(grandParent);

    const { artistHint, albumHint } = parseArtistAlbumHint(
      parentDir,
      grandName
    );

    albums.set(parentDir, {
      path: parentDir,
      name: albumHint,
      artistHint,
      albumHint,
      trackCount: 1,
    });
    albumAudioFiles.set(parentDir, [libraryPath]);
    return { albums, albumAudioFiles };
  }

  // Read top-level entries
  const entries = fs.readdirSync(libraryPath, { withFileTypes: true });

  const rootAudioFiles: string[] = [];

  for (const entry of entries) {
    if (entry.name.startsWith(".")) continue;

    const fullPath = path.join(libraryPath, entry.name);

    if (entry.isDirectory()) {
      // Strategy 1: This directory is an album (audio files at 1 level)
      // Strategy 2: This directory is an artist (subdirs contain audio files)
      let audioFiles = collectAudioFiles(fullPath);
      let albumDir = fullPath;
      let parentDir = path.basename(libraryPath);

      if (audioFiles.length === 0) {
        // No audio files at 1 level — check subdirectories (artist/album)
        const subEntries = fs.readdirSync(fullPath, { withFileTypes: true });
        for (const sub of subEntries) {
          if (sub.name.startsWith(".") || !sub.isDirectory()) continue;
          const subPath = path.join(fullPath, sub.name);
          const subAudioFiles = collectAudioFiles(subPath);
          if (subAudioFiles.length > 0) {
            const { artistHint, albumHint } = parseArtistAlbumHint(
              subPath,
              entry.name
            );
            albums.set(subPath, {
              path: subPath,
              name: albumHint,
              artistHint,
              albumHint,
              trackCount: subAudioFiles.length,
            });
            albumAudioFiles.set(subPath, subAudioFiles);
          }
        }
        continue;
      }

      // Direct album directory
      const { artistHint, albumHint } = parseArtistAlbumHint(
        albumDir,
        parentDir
      );
      albums.set(albumDir, {
        path: albumDir,
        name: albumHint,
        artistHint,
        albumHint,
        trackCount: audioFiles.length,
      });
      albumAudioFiles.set(albumDir, audioFiles);

    } else if (isAudioFile(fullPath)) {
      rootAudioFiles.push(fullPath);
    }
  }

  // Group root-level audio files as a single "album" entry
  if (rootAudioFiles.length > 0) {
    albums.set(libraryPath, {
      path: libraryPath,
      name: path.basename(libraryPath),
      artistHint: "",
      albumHint: path.basename(libraryPath),
      trackCount: rootAudioFiles.length,
    });
    albumAudioFiles.set(libraryPath, rootAudioFiles);
  }

  return { albums, albumAudioFiles };
}

/**
 * Recursively collect audio files in a directory (1 level deep).
 */
export function collectAudioFiles(dirPath: string): string[] {
  const result: string[] = [];
  try {
    const entries = fs.readdirSync(dirPath, { withFileTypes: true });
    for (const entry of entries) {
      if (isHiddenDir(entry.name)) continue;
      const fullPath = path.join(dirPath, entry.name);
      if (entry.isFile() && isAudioFile(fullPath)) {
        result.push(fullPath);
      }
    }
  } catch {
    // Permission errors, skip
  }
  result.sort();
  return result;
}

export function registerLibraryHandlers(): void {
  ipcMain.handle(
    "library:scan",
    async (_event, dirPath: string): Promise<AlbumInfo[]> => {
      const { albums } = scanDirectory(dirPath);
      return Array.from(albums.values());
    }
  );

  ipcMain.handle(
    "album:refresh",
    async (_event, albumPath: string) => {
      // Re-read full track metadata from disk
      return readAlbum(albumPath);
    }
  );
}
