import { ipcMain } from "electron";
import fs from "fs";
import path from "path";
import { isAudioFile, readTrackMetadata } from "./tracks";

export interface DirEntry {
  name: string;
  path: string;
  isDirectory: boolean;
}

export interface DirTreeData {
  entries: DirEntry[];
  parentPath: string | null;
}

/** List subdirectories and count audio files in a given directory. */
function listDirectoryEntries(dirPath: string): DirEntry[] {
  const results: DirEntry[] = [];

  if (!fs.existsSync(dirPath)) return results;

  try {
    const entries = fs.readdirSync(dirPath, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;
      const fullPath = path.join(dirPath, entry.name);
      if (entry.isDirectory()) {
        results.push({ name: entry.name, path: fullPath, isDirectory: true });
      }
    }
  } catch {
    // Permission errors — skip
  }

  results.sort((a, b) => a.name.localeCompare(b.name));
  return results;
}

/**
 * Read a directory: return subdirectories + all audio files with full metadata.
 */
export async function readDirectory(dirPath: string) {
  const subdirs = listDirectoryEntries(dirPath);
  const audioFiles: string[] = [];

  try {
    const entries = fs.readdirSync(dirPath, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.name.startsWith(".") || entry.isDirectory()) continue;
      const fullPath = path.join(dirPath, entry.name);
      if (isAudioFile(fullPath)) {
        audioFiles.push(fullPath);
      }
    }
  } catch {
    // pass
  }

  audioFiles.sort();

  const tracks = [];
  for (const audioFile of audioFiles) {
    try {
      const track = await readTrackMetadata(audioFile);
      tracks.push(track);
    } catch {
      const fileStat = fs.statSync(audioFile);
      tracks.push({
        path: audioFile,
        title: path.basename(audioFile),
        artist: null,
        artists: [],
        album: null,
        albumArtist: null,
        albumArtists: [],
        trackNumber: null,
        trackTotal: null,
        discNumber: null,
        discTotal: null,
        year: null,
        genre: null,
        composer: null,
        lyrics: null,
        compilation: null,
        musicbrainzTrackId: null,
        musicbrainzAlbumId: null,
        musicbrainzArtistId: null,
        hasCover: false,
        sizeBytes: fileStat.size,
        bitrate: null,
        sampleRate: null,
        codec: "unknown",
        duration: 0,
      });
    }
  }

  const dirName = path.basename(dirPath);

  return {
    path: dirPath,
    name: dirName,
    subdirs,
    tracks,
    audioCount: audioFiles.length,
  };
}

export function registerDirectoryHandlers(): void {
  ipcMain.handle(
    "directory:list",
    async (_event, dirPath: string): Promise<DirEntry[]> => {
      return listDirectoryEntries(dirPath);
    }
  );

  ipcMain.handle(
    "directory:read",
    async (_event, dirPath: string) => {
      return readDirectory(dirPath);
    }
  );
}
