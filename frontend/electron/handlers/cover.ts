import { ipcMain, dialog } from "electron";
import { parseFile } from "music-metadata";
import sharp from "sharp";
import fs from "fs";
import path from "path";

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
      if (
        [".mp3", ".flac", ".m4a", ".mp4", ".ogg", ".opus"].includes(
          path.extname(entry.name).toLowerCase()
        )
      ) {
        return fullPath;
      }
    }
  } catch {
    // ignore
  }
  return null;
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
              [".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus"].includes(
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
}
