import { ipcMain, dialog } from "electron";
import { parseFile } from "music-metadata";
import sharp from "sharp";
import fs from "fs";
import path from "path";
import { ArtworkResolverService } from "../services/ArtworkResolverService";
import { loadConfig } from "./auto-tag";
import debug from "./debug";

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
 * Also reads Discogs/MusicBrainz IDs for smarter cover resolution.
 */
async function readFirstTrackMetadata(albumPath: string): Promise<{
  artist: string | null;
  album: string | null;
  musicbrainzAlbumId: string | null;
  discogsArtistId: string | null;
  discogsReleaseId: string | null;
} | null> {
  try {
    const entries = fs.readdirSync(albumPath, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;
      if (!AUDIO_EXTS.includes(path.extname(entry.name).toLowerCase())) continue;

      const filePath = path.join(albumPath, entry.name);
      const metadata = await parseFile(filePath, { duration: false });
      const common = metadata.common;

      // Read Discogs IDs from native tags (Vorbis TXXX or ID3 TXXX)
      const native = metadata.native ?? {};
      let discogsArtistId: string | null = null;
      let discogsReleaseId: string | null = null;

      // Check Vorbis comments (FLAC/OGG)
      const vorbis = native["VORBIS_COMMENT"] as Array<{ id: string; value: string }> | undefined;
      if (vorbis) {
        for (const tag of vorbis) {
          if (tag.id === "DISCOGS_ARTIST_ID") discogsArtistId = tag.value;
          if (tag.id === "DISCOGS_RELEASE_ID") discogsReleaseId = tag.value;
        }
      }

      // Check ID3v2 (MP3) — stored as TXXX frames
      const id3v2 = (native["ID3v2.4"] ?? native["ID3v2.3"]) as
        | Array<{ id: string; value: string }>
        | undefined;
      if (id3v2) {
        for (const tag of id3v2) {
          if (tag.id === "TXXX:Discogs Artist Id") discogsArtistId = tag.value;
          if (tag.id === "TXXX:Discogs Release Id") discogsReleaseId = tag.value;
        }
      }

      return {
        artist: common.artist ?? null,
        album: common.album ?? null,
        musicbrainzAlbumId: common.musicbrainz_albumid?.toString() ?? null,
        discogsArtistId: discogsArtistId ?? (common as any).discogs_artist_id?.toString() ?? null,
        discogsReleaseId: discogsReleaseId ?? (common as any).discogs_release_id?.toString() ?? null,
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
        debug.debug("cover", `cover:data-url for ${albumPath}`);

        // 1. Check for external cover file
        const externalCover = findExternalCover(albumPath);
        if (externalCover) {
          const imageData = fs.readFileSync(externalCover);
          debug.debug("cover", `cover:data-url found external ${externalCover} (${imageData.length} bytes)`);
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
              debug.debug("cover", `cover:data-url found embedded cover in ${entry.name}`);
              return imageToDataUrl(metadata.common.picture[0].data);
            }
          } catch {
            continue;
          }
        }

        debug.debug("cover", `cover:data-url no cover found for ${albumPath}`);
        return null;
      } catch (err) {
        debug.warn("cover", "cover:data-url threw", err);
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
    if (!fs.existsSync(albumPath)) {
      debug.warn("cover", `resolveAndWriteArtwork: path does not exist ${albumPath}`);
      return null;
    }

    const metadata = await readFirstTrackMetadata(albumPath);
    if (!metadata) {
      debug.warn("cover", `resolveAndWriteArtwork: no metadata found for ${albumPath}`);
      return null;
    }
    if (kind === "artist-image" && !metadata.artist) {
      debug.debug("cover", `resolveAndWriteArtwork: artist-image but no artist metadata`);
      return null;
    }

    if (kind === "artist-image") {
      debug.info("cover", `resolveAndWriteArtwork: kind=${kind} artist="${metadata.artist ?? ""}"`);
    } else {
      debug.info("cover", `resolveAndWriteArtwork: kind=${kind} artist="${metadata.artist ?? ""}" album="${metadata.album ?? ""}" mbid=${metadata.musicbrainzAlbumId ?? "null"}`);
    }

    const resolver = getArtworkResolver();
    const ctx = resolver.buildContext(
      kind,
      albumPath,
      metadata.artist,
      metadata.album,
      metadata.musicbrainzAlbumId,
      metadata.discogsArtistId,
      metadata.discogsReleaseId,
    );

    const result = await resolver.resolve(ctx);
    if (!result) {
      debug.info("cover", `resolveAndWriteArtwork: resolve returned null — no artwork found`);
      return null;
    }

    debug.info("cover", `resolveAndWriteArtwork: resolved source=${result.source} url=${result.url ?? "(none)"} bytes=${result.bytes.length}`);

    const normalized = await normalizeDownloadImage(result.bytes);
    if (!normalized) {
      debug.warn("cover", `resolveAndWriteArtwork: normalization failed`);
      return null;
    }

    const savePath = kind === "album-cover"
      ? path.join(albumPath, "cover.jpg")
      : path.join(path.dirname(albumPath), "artist.jpg");

    fs.writeFileSync(savePath, normalized);
    debug.info("cover", `resolveAndWriteArtwork: saved to ${savePath} (${normalized.length} bytes)`);

    return { bytes: normalized, source: result.source, savePath };
  }

  ipcMain.handle(
    "cover:download",
    async (_event, albumPath: string): Promise<string | null> => {
      debug.info("cover", `cover:download for ${albumPath}`);
      try {
        const out = await resolveAndWriteArtwork("album-cover", albumPath);
        if (!out) {
          debug.info("cover", "cover:download failed — no artwork found");
          return null;
        }
        debug.info("cover", `cover:download SUCCESS source=${out.source} bytes=${out.bytes.length}`);
        return `data:image/jpeg;base64,${out.bytes.toString("base64")}`;
      } catch (err) {
        debug.warn("cover", "cover:download threw", err);
        return null;
      }
    }
  );

  ipcMain.handle(
    "cover:download-artist-art",
    async (_event, albumPath: string): Promise<{ path: string; source: string } | null> => {
      debug.info("cover", `cover:download-artist-art for ${albumPath}`);
      try {
        const out = await resolveAndWriteArtwork("artist-image", albumPath);
        if (!out) {
          debug.info("cover", "cover:download-artist-art failed — no artwork found");
          return null;
        }
        debug.info("cover", `cover:download-artist-art SUCCESS source=${out.source} path=${out.savePath}`);
        return { path: out.savePath, source: out.source };
      } catch (err) {
        debug.warn("cover", "cover:download-artist-art threw", err);
        return null;
      }
    }
  );
}
