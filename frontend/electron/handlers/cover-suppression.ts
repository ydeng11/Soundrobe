import fs from "fs";
import path from "path";

const COVER_REMOVED_MARKER = ".auto-tagger-cover-removed";

function coverRemovedMarkerPath(albumPath: string): string {
  return path.join(albumPath, COVER_REMOVED_MARKER);
}

export function isAlbumCoverSuppressed(albumPath: string): boolean {
  return fs.existsSync(coverRemovedMarkerPath(albumPath));
}

export function suppressAlbumCover(albumPath: string): void {
  fs.writeFileSync(coverRemovedMarkerPath(albumPath), "", "utf-8");
}

export function clearAlbumCoverSuppression(albumPath: string): void {
  const markerPath = coverRemovedMarkerPath(albumPath);
  if (fs.existsSync(markerPath)) {
    fs.unlinkSync(markerPath);
  }
}
