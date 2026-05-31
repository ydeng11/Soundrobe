import { ipcMain } from "electron";
import fs from "fs";
import path from "path";
import { isAudioFile, readTrackMetadata } from "./tracks";

export interface SortByAlbumResult {
  /** The source directory that was processed. */
  sourceDir: string;
  /** Per-album results. */
  albums: SortByAlbumEntry[];
  /** Total number of files processed. */
  totalFiles: number;
  /** Number of files that were skipped due to errors. */
  skippedFiles: number;
}

export interface SortByAlbumEntry {
  /** The album name from metadata. */
  albumName: string;
  /** The destination directory created for this album. */
  destDir: string;
  /** Files that were copied into this album folder. */
  files: SortByAlbumFile[];
}

export interface SortByAlbumFile {
  /** Source path of the file. */
  sourcePath: string;
  /** Destination path of the file. */
  destPath: string;
  /** Whether the operation succeeded. */
  success: boolean;
  /** Error message if failed. */
  error?: string;
}

/**
 * Scan all audio files in a directory, group them by their album metadata tag,
 * and copy each group into a subdirectory named after the album.
 */
export async function sortByAlbum(
  sourceDir: string,
  options: { copy?: boolean } = {}
): Promise<SortByAlbumResult> {
  const doCopy = options.copy ?? true; // Default to copy, not move

  // Collect all audio files recursively
  const audioFiles = collectAllAudioFiles(sourceDir);
  const result: SortByAlbumResult = {
    sourceDir,
    albums: [],
    totalFiles: audioFiles.length,
    skippedFiles: 0,
  };

  if (audioFiles.length === 0) return result;

  // Read album tag for each file, group by album name
  const albumGroups = new Map<string, string[]>();

  for (const filePath of audioFiles) {
    try {
      const metadata = await readTrackMetadata(filePath);
      const albumName = metadata.album?.trim() || "Unknown Album";

      // Sanitize album name for use as a directory name
      const sanitized = sanitizeDirName(albumName);

      if (!albumGroups.has(sanitized)) {
        albumGroups.set(sanitized, []);
      }
      albumGroups.get(sanitized)!.push(filePath);
    } catch {
      result.skippedFiles++;
    }
  }

  // Process each album group
  for (const [sanitizedAlbumName, files] of albumGroups) {
    const destDir = path.join(sourceDir, sanitizedAlbumName);

    // Ensure destination directory exists
    if (!fs.existsSync(destDir)) {
      fs.mkdirSync(destDir, { recursive: true });
    }

    const albumFiles: SortByAlbumFile[] = [];

    for (const sourcePath of files) {
      const fileName = path.basename(sourcePath);
      const destPath = path.join(destDir, fileName);

      try {
        if (doCopy) {
          await fs.promises.copyFile(sourcePath, destPath);
        } else {
          await fs.promises.rename(sourcePath, destPath);
        }
        albumFiles.push({ sourcePath, destPath, success: true });
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        albumFiles.push({ sourcePath, destPath, success: false, error: message });
        result.skippedFiles++;
      }
    }

    result.albums.push({
      albumName: sanitizedAlbumName,
      destDir,
      files: albumFiles,
    });
  }

  return result;
}

/**
 * Recursively collect all audio files from a directory tree.
 */
function collectAllAudioFiles(dirPath: string): string[] {
  const results: string[] = [];

  if (!fs.existsSync(dirPath)) return results;

  const entries = fs.readdirSync(dirPath, { withFileTypes: true });

  for (const entry of entries) {
    if (entry.name.startsWith(".")) continue;

    const fullPath = path.join(dirPath, entry.name);

    if (entry.isDirectory()) {
      // Recurse into subdirectories
      results.push(...collectAllAudioFiles(fullPath));
    } else if (entry.isFile() && isAudioFile(fullPath)) {
      results.push(fullPath);
    }
  }

  results.sort();
  return results;
}

/**
 * Sanitize a string for use as a directory name.
 * Replaces characters that are problematic on Windows/macOS/Linux.
 */
function sanitizeDirName(name: string): string {
  // Characters not allowed on Windows: \ / : * ? " < > |
  // Also strip leading/trailing spaces and dots, and control characters
  let sanitized = name.replace(/[<>:"/\\|?*]/g, "_");
  sanitized = sanitized.replace(/[\x00-\x1f\x7f]/g, "");
  sanitized = sanitized.trim();
  sanitized = sanitized.replace(/^\.+/, "_");
  sanitized = sanitized.replace(/\.+$/, "_");

  // Collapse multiple underscores/spaces
  sanitized = sanitized.replace(/[_ ]+/g, " ").trim();

  // Avoid empty directory name
  if (!sanitized || sanitized.length === 0) {
    sanitized = "Unknown Album";
  }

  return sanitized;
}

export function registerOrganizerHandlers(): void {
  ipcMain.handle(
    "files:sort-by-album",
    async (
      _event,
      sourceDir: string,
      options?: { copy?: boolean }
    ): Promise<SortByAlbumResult> => {
      return sortByAlbum(sourceDir, options ?? {});
    }
  );
}
