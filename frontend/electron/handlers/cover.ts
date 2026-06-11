import { ipcMain, dialog } from "electron";
import { parseFile } from "music-metadata";
import sharp from "sharp";
import fs from "fs";
import path from "path";
import { ArtworkResolverService } from "../services/ArtworkResolverService";
import { loadConfig } from "./auto-tag";

// ── Download result types ───────────────────────────────────────────

export interface CoverDownloadResult {
  dataUrl: string;
  source: string;
}

export interface ArtistArtDownloadResult {
  path: string;
  source: string;
}

const COVER_NAMES = [
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

const COVER_EXTS = [".jpg", ".jpeg", ".png"];

/** Audio file extensions used for embedded cover scanning and metadata reading. */
const AUDIO_EXTS = [".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus"];

/**
 * Find external cover art file in a directory.
 */
function findExternalCover(albumPath: string): string | null {
  if (!fs.existsSync(albumPath)) return null;

  for (const name of COVER_NAMES) {
    for (const ext of COVER_EXTS) {
      const candidate = path.join(albumPath, `${name}${ext}`);
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }
  return null;
}

/**
 * Convert image bytes to a data URL (JPEG, resized to max 500px).
 */
async function imageToDataUrl(
  imageData: Buffer | Uint8Array,
  maxDimension = 500
): Promise<string | null> {
  try {
    const resized = await sharp(imageData)
      .resize(maxDimension, maxDimension, {
        fit: "inside",
        withoutEnlargement: true,
      })
      .jpeg({ quality: 85 })
      .toBuffer();

    return `data:image/jpeg;base64,${resized.toString("base64")}`;
  } catch {
    return null;
  }
}

function findFirstAudioFile(dirPath: string): string | null {
  try {
    const entries = fs.readdirSync(dirPath, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;
      const fullPath = path.join(dirPath, entry.name);
      if (AUDIO_EXTS.includes(path.extname(entry.name).toLowerCase())) {
        return fullPath;
      }
    }
  } catch {
    // ignore
  }
  return null;
}

// ── Artwork resolver factory ──────────────────────────────────────

let artworkResolverInstance: ArtworkResolverService | null = null;

/**
 * Get or create the artwork resolver service, provisioning credentials
 * from the app config.
 */
function getArtworkResolver(): ArtworkResolverService {
  if (artworkResolverInstance) return artworkResolverInstance;
  const resolver = new ArtworkResolverService();
  try {
    const cfg = loadConfig();
    resolver.setCredentials({
      googleApiKey: (cfg as any).googleImageApiKey ?? null,
      googleSearchEngineId: (cfg as any).googleImageSearchEngineId ?? null,
      theAudioDbApiKey: (cfg as any).theAudioDbApiKey ?? null,
      discogsToken: (cfg as any).discogsToken ?? null,
    });
  } catch {
    // Config not available — resolver runs without credentials
  }
  artworkResolverInstance = resolver;
  return resolver;
}

/**
 * Read metadata from the first audio track in an album directory.
 */
async function readFirstTrackMetadata(albumPath: string): Promise<{
  artist: string | null;
  album: string | null;
  musicbrainzAlbumId: string | null;
} | null> {
  try {
    const entries = fs.readdirSync(albumPath, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;
      if (!AUDIO_EXTS.includes(path.extname(entry.name).toLowerCase())) continue;

      const filePath = path.join(albumPath, entry.name);
      const metadata = await parseFile(filePath, { duration: false });
      const common = metadata.common;

      return {
        artist: common.artist ?? null,
        album: common.album ?? null,
        musicbrainzAlbumId: common.musicbrainz_albumid?.toString() ?? null,
      };
    }
  } catch {
    // Fall through to null
  }
  return null;
}

/**
 * Normalize image bytes through sharp to JPEG, max 1000×1000, quality 90.
 * Uses the same settings as the resolve pipeline.
 */
async function normalizeDownloadImage(bytes: Buffer): Promise<Buffer | null> {
  try {
    return await sharp(bytes)
      .resize(1000, 1000, { fit: "inside", withoutEnlargement: true })
      .jpeg({ quality: 90 })
      .toBuffer();
  } catch {
    return null;
  }
}

export function registerCoverHandlers(): void {
  ipcMain.handle(
    "cover:data-url",
    async (_event, albumPath: string): Promise<string | null> => {
      try {
        // 1. Check for external cover file
        const externalCover = findExternalCover(albumPath);
        if (externalCover) {
          const imageData = fs.readFileSync(externalCover);
          return imageToDataUrl(imageData);
        }

        // 2. Check for embedded cover in first track with cover art
        const entries = fs
          .readdirSync(albumPath, { withFileTypes: true })
          .filter(
            (e) =>
              e.isFile() &&
              !e.name.startsWith(".") &&
              AUDIO_EXTS.includes(
                path.extname(e.name).toLowerCase()
              )
          );

        for (const entry of entries) {
          const filePath = path.join(albumPath, entry.name);
          try {
            const metadata = await parseFile(filePath, { duration: false });
            if (
              metadata.common.picture &&
              metadata.common.picture.length > 0
            ) {
              return imageToDataUrl(metadata.common.picture[0].data);
            }
          } catch {
            continue;
          }
        }

        return null;
      } catch {
        return null;
      }
    }
  );

  /**
   * cover:set — open a native file dialog and copy the chosen image as cover.jpg
   * into the album directory. Returns the new data URL, or null if cancelled.
   */
  ipcMain.handle(
    "cover:set",
    async (_event, albumPath: string): Promise<string | null> => {
      try {
        const result = await dialog.showOpenDialog({
          title: "Choose Cover Artwork",
          defaultPath: albumPath,
          filters: [
            { name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] },
          ],
          properties: ["openFile"],
        });

        if (result.canceled || result.filePaths.length === 0) {
          return null;
        }

        const selectedPath = result.filePaths[0];
        const destPath = path.join(albumPath, "cover.jpg");

        // Copy and convert to JPEG
        const imageData = fs.readFileSync(selectedPath);
        const jpeg = await sharp(imageData)
          .resize(500, 500, { fit: "inside", withoutEnlargement: true })
          .jpeg({ quality: 90 })
          .toBuffer();

        fs.writeFileSync(destPath, jpeg);

        return imageToDataUrl(jpeg);
      } catch {
        return null;
      }
    }
  );

  /**
   * cover:remove — delete external cover files in the album directory.
   */
  ipcMain.handle(
    "cover:remove",
    async (_event, albumPath: string): Promise<boolean> => {
      try {
        const externalCover = findExternalCover(albumPath);
        if (externalCover) {
          fs.unlinkSync(externalCover);
        }
        return true;
      } catch {
        return false;
      }
    }
  );

  /**
   * Shared download logic for both cover art and artist images.
   * Resolves artwork, normalizes, writes to disk, and returns the
   * caller-formatted result.
   */
  async function resolveAndWriteArtwork(
    kind: "album-cover" | "artist-image",
    albumPath: string,
  ): Promise<{ bytes: Buffer; source: string; savePath: string } | null> {
    if (!fs.existsSync(albumPath)) return null;
    const metadata = await readFirstTrackMetadata(albumPath);
    if (!metadata) return null;
    if (kind === "artist-image" && !metadata.artist) return null;

    const resolver = getArtworkResolver();
    const ctx = resolver.buildContext(
      kind,
      albumPath,
      metadata.artist,
      metadata.album,
      metadata.musicbrainzAlbumId,
    );

    const result = await resolver.resolve(ctx);
    if (!result) return null;

    const normalized = await normalizeDownloadImage(result.bytes);
    if (!normalized) return null;

    const savePath = kind === "album-cover"
      ? path.join(albumPath, "cover.jpg")
      : path.join(path.dirname(albumPath), "artist.jpg");

    fs.writeFileSync(savePath, normalized);

    return { bytes: normalized, source: result.source, savePath };
  }

  ipcMain.handle(
    "cover:download",
    async (_event, albumPath: string): Promise<string | null> => {
      try {
        const out = await resolveAndWriteArtwork("album-cover", albumPath);
        if (!out) return null;
        return `data:image/jpeg;base64,${out.bytes.toString("base64")}`;
      } catch {
        return null;
      }
    }
  );

  ipcMain.handle(
    "cover:download-artist-art",
    async (_event, albumPath: string): Promise<{ path: string; source: string } | null> => {
      try {
        const out = await resolveAndWriteArtwork("artist-image", albumPath);
        if (!out) return null;
        return { path: out.savePath, source: out.source };
      } catch {
        return null;
      }
    }
  );
}
