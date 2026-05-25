import { ipcMain } from "electron";
import { parseFile } from "music-metadata";
import fs from "fs";
import path from "path";
import type { CoverInfo } from "../preload";
import { writeTags, batchWriteTags } from "./writer";
import type { WriteFields } from "./writer";

export interface TrackData {
  path: string;
  title: string | null;
  artist: string | null;
  artists: string[];
  album: string | null;
  albumArtist: string | null;
  albumArtists: string[];
  trackNumber: number | null;
  trackTotal: number | null;
  discNumber: number | null;
  discTotal: number | null;
  year: string | null;
  genre: string | null;
  composer: string | null;
  lyrics: string | null;
  compilation: boolean | null;
  musicbrainzTrackId: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  hasCover: boolean;
  sizeBytes: number;
  bitrate: number | null;
  sampleRate: number | null;
  codec: string;
  duration: number;
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

function detectExternalCover(albumPath: string): string | null {
  const coverNames = [
    "cover",
    "Cover",
    "COVER",
    "front",
    "Front",
    "FRONT",
    "folder",
    "Folder",
    "FOLDER",
    "albumart",
    "AlbumArt",
  ];
  const coverExts = [".jpg", ".jpeg", ".png"];

  for (const name of coverNames) {
    for (const ext of coverExts) {
      const candidate = path.join(albumPath, `${name}${ext}`);
      try {
        if (fs.existsSync(candidate)) {
          return candidate;
        }
      } catch {
        continue;
      }
    }
  }
  return null;
}

export async function readTrackMetadata(filePath: string): Promise<TrackData> {
  const metadata = await parseFile(filePath);
  const { common, format } = metadata;
  const stat = fs.statSync(filePath);

  // Format year as string
  let year: string | null = null;
  if (common.year) {
    year = String(common.year);
  } else if (common.date) {
    year = common.date.slice(0, 4);
  }

  // Determine if track has embedded cover art
  const hasCover = common.picture !== undefined && common.picture.length > 0;

  return {
    path: filePath,
    title: common.title ?? null,
    artist: common.artist ?? null,
    artists: common.artists ?? [],
    album: common.album ?? null,
    albumArtist: common.albumartist ?? null,
    albumArtists: common.albumartist ? [common.albumartist] : [],
    trackNumber: common.track?.no ?? null,
    trackTotal: common.track?.of ?? null,
    discNumber: common.disk?.no ?? null,
    discTotal: common.disk?.of ?? null,
    year,
    genre: common.genre?.[0] ?? null,
    composer: common.composer?.[0] ?? null,
    lyrics: (common.lyrics?.[0] as string | undefined) ?? null,
    compilation: common.compilation ?? null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover,
    sizeBytes: stat.size,
    bitrate: format.bitrate ?? null,
    sampleRate: format.sampleRate ?? null,
    codec: format.codec ?? format.container ?? "unknown",
    duration: format.duration ?? 0,
  };
}

/** Read all track metadata for an album directory. */
export async function readAlbum(
  albumPath: string
): Promise<{
  path: string;
  name: string;
  artistHint: string;
  albumHint: string;
  tracks: TrackData[];
  coverInfo: CoverInfo;
  status: string;
}> {
  const entries = fs.readdirSync(albumPath);
  const audioFiles: string[] = [];

  for (const entry of entries) {
    if (entry.startsWith(".")) continue;
    const fullPath = path.join(albumPath, entry);
    if (fs.statSync(fullPath).isFile() && isAudioFile(fullPath)) {
      audioFiles.push(fullPath);
    }
  }
  audioFiles.sort();

  const tracks: TrackData[] = [];
  let errorCount = 0;

  for (const audioFile of audioFiles) {
    try {
      const track = await readTrackMetadata(audioFile);
      tracks.push(track);
    } catch (err) {
      errorCount++;
      // Push a minimal entry for unreadable files
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

  const dirName = path.basename(albumPath);
  const parentDir = path.basename(path.dirname(albumPath));

  const externalCover = detectExternalCover(albumPath);
  const hasEmbeddedCover = tracks.some((t) => t.hasCover);

  const coverInfo: CoverInfo = {
    path: externalCover,
    source: externalCover
      ? "external"
      : hasEmbeddedCover
        ? "embedded"
        : "missing",
    dataUrl: null,
  };

  const status =
    errorCount === 0 ? "ok" : errorCount < tracks.length ? "warning" : "error";

  return {
    path: albumPath,
    name: dirName,
    artistHint: parentDir,
    albumHint: dirName,
    tracks,
    coverInfo,
    status,
  };
}

export function registerTrackHandlers(): void {
  ipcMain.handle("album:read", async (_event, albumPath: string) => {
    return readAlbum(albumPath);
  });

  ipcMain.handle(
    "track:write",
    async (
      _event,
      trackPath: string,
      fields: Record<string, unknown>
    ): Promise<TrackData> => {
      // Cast fields to WriteFields (unknown keys are safely ignored by writer)
      const writeFields = fields as unknown as WriteFields;
      await writeTags(trackPath, writeFields);
      // Re-read and return updated metadata
      return readTrackMetadata(trackPath);
    }
  );

  ipcMain.handle(
    "tracks:batch-write",
    async (
      _event,
      updates: Array<{ path: string; fields: Record<string, unknown> }>
    ): Promise<TrackData[]> => {
      const writeUpdates = updates.map((u) => ({
        path: u.path,
        fields: u.fields as unknown as WriteFields,
      }));
      await batchWriteTags(writeUpdates);
      // Re-read all updated tracks
      return Promise.all(updates.map((u) => readTrackMetadata(u.path)));
    }
  );
}
