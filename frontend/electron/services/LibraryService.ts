/**
 * LibraryService — extracted reusable library operations.
 *
 * Shared between IPC handlers and the assistant runtime.
 * All filesystem reads, writes, and moves are resolved and validated
 * inside the selected library root.
 */

import fs from "fs";
import path from "path";
import { scanDirectory, parseArtistAlbumHint } from "../handlers/library";
import type { AlbumInfo } from "../preload";
import { readAlbum as readAlbumTracks, readTrackMetadata } from "../handlers/tracks";
import type { TrackData } from "../handlers/tracks";
import type { AlbumDetail } from "../preload";

export interface LibrarySummary {
  albumCount: number;
  trackCount: number;
  totalSizeBytes: number;
  totalDurationSeconds: number;
  artistCount: number;
  genreCount: number;
  missingAlbum: number;
  missingArtist: number;
  missingTitle: number;
  missingYear: number;
  missingGenre: number;
  byCodec: Record<string, number>;
}

export interface AssistantAppContext {
  libraryPath: string | null;
  activeAlbumPath: string | null;
  selectedTrackPaths: string[];
  visibleTrackCount: number;
  selectedTrackSummaries: TrackSummary[];
  activeAlbumSummary: AlbumSummary | null;
  assistantAutonomous: boolean;
}

export interface TrackSummary {
  path: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  albumArtist: string | null;
  trackNumber: number | null;
  year: string | null;
  genre: string | null;
  codec: string;
  duration: number;
  hasCover: boolean;
}

export interface AlbumSummary {
  path: string;
  name: string;
  artistHint: string;
  albumHint: string;
  trackCount: number;
  hasCover: boolean;
}

export class LibraryService {
  private libraryPath: string | null = null;

  /**
   * Set the current library root path.
   */
  setLibraryPath(libraryPath: string | null): void {
    this.libraryPath = libraryPath;
  }

  /**
   * Get the current library root path.
   */
  getLibraryPath(): string | null {
    return this.libraryPath;
  }

  /**
   * Assert that a target path is inside the current library root.
   * Returns the resolved canonical path.
   * Throws if outside or if library root is not set.
   */
  assertInsideLibrary(targetPath: string): string {
    if (!this.libraryPath) {
      throw new Error("No library selected");
    }

    const resolvedLib = path.resolve(this.libraryPath);
    const resolvedTarget = path.resolve(targetPath);

    if (!resolvedTarget.startsWith(resolvedLib + path.sep) && resolvedTarget !== resolvedLib) {
      throw new Error(
        `Path "${targetPath}" is outside the current library "${this.libraryPath}"`,
      );
    }

    return resolvedTarget;
  }

  /**
   * Scan the library and return all discovered albums.
   */
  scanLibrary(libraryPath: string): AlbumInfo[] {
    const { albums } = scanDirectory(libraryPath);
    return Array.from(albums.values());
  }

  /**
   * Read complete album detail including tracks.
   */
  async readAlbum(albumPath: string): Promise<AlbumDetail> {
    this.assertInsideLibrary(albumPath);
    return readAlbumTracks(albumPath);
  }

  /**
   * Read tracks for a set of album paths.
   */
  async readTracksForAlbums(albumPaths: string[]): Promise<TrackData[]> {
    const allTracks: TrackData[] = [];
    for (const albumPath of albumPaths) {
      this.assertInsideLibrary(albumPath);
      const album = await readAlbumTracks(albumPath);
      allTracks.push(...album.tracks);
    }
    return allTracks;
  }

  /**
   * Read metadata for a specific track.
   */
  async readTrack(filePath: string): Promise<TrackData> {
    this.assertInsideLibrary(filePath);
    return readTrackMetadata(filePath);
  }

  /**
   * Build a compact library summary from pre-scanned album infos and track data.
   */
  summarizeLibrary(albums: AlbumInfo[], tracks: TrackData[]): LibrarySummary {
    const codecs: Record<string, number> = {};
    const artists = new Set<string>();
    const genres = new Set<string>();

    let totalSizeBytes = 0;
    let totalDurationSeconds = 0;
    let missingAlbum = 0;
    let missingArtist = 0;
    let missingTitle = 0;
    let missingYear = 0;
    let missingGenre = 0;

    for (const track of tracks) {
      totalSizeBytes += track.sizeBytes;
      totalDurationSeconds += track.duration;

      codecs[track.codec] = (codecs[track.codec] ?? 0) + 1;

      const artist = this.nonBlank(track.artist);
      const albumArtist = this.nonBlank(track.albumArtist);
      const genre = this.nonBlank(track.genre);

      if (artist) artists.add(artist);
      if (albumArtist) artists.add(albumArtist);
      if (genre) genres.add(genre);

      if (!track.album || track.album.trim() === "") missingAlbum++;
      if (!track.artist || track.artist.trim() === "") missingArtist++;
      if (!track.title || track.title.trim() === "") missingTitle++;
      if (!track.year || track.year.trim() === "") missingYear++;
      if (!track.genre || track.genre.trim() === "") missingGenre++;
    }

    return {
      albumCount: albums.length,
      trackCount: tracks.length,
      totalSizeBytes,
      totalDurationSeconds,
      artistCount: artists.size,
      genreCount: genres.size,
      missingAlbum,
      missingArtist,
      missingTitle,
      missingYear,
      missingGenre,
      byCodec: codecs,
    };
  }

  /**
   * Build a compact context object for the assistant.
   */
  buildAppContext(input: {
    libraryPath: string | null;
    activeAlbumPath: string | null;
    selectedTrackPaths: string[];
    tracks: TrackData[];
    albums: AlbumInfo[];
    assistantAutonomous: boolean;
  }): AssistantAppContext {
    const selectedSummaries = input.selectedTrackPaths
      .map((p) => input.tracks.find((t) => t.path === p))
      .filter((t): t is TrackData => t !== undefined)
      .map(this.trackToSummary);

    let activeAlbumSummary: AlbumSummary | null = null;
    if (input.activeAlbumPath) {
      const albumInfo = input.albums.find((a) => a.path === input.activeAlbumPath);
      const albumTracks = input.tracks.filter((t) =>
        this.isInsideDirectory(t.path, input.activeAlbumPath!),
      );
      activeAlbumSummary = {
        path: input.activeAlbumPath,
        name: albumInfo?.name ?? path.basename(input.activeAlbumPath),
        artistHint: albumInfo?.artistHint ?? "",
        albumHint: albumInfo?.albumHint ?? "",
        trackCount: albumTracks.length,
        hasCover: albumTracks.some((t) => t.hasCover),
      };
    }

    return {
      libraryPath: input.libraryPath,
      activeAlbumPath: input.activeAlbumPath,
      selectedTrackPaths: input.selectedTrackPaths,
      visibleTrackCount: input.tracks.length,
      selectedTrackSummaries: selectedSummaries,
      activeAlbumSummary,
      assistantAutonomous: input.assistantAutonomous,
    };
  }

  /**
   * Convert a TrackData to a compact summary.
   */
  private trackToSummary(track: TrackData): TrackSummary {
    return {
      path: track.path,
      title: track.title,
      artist: track.artist,
      album: track.album,
      albumArtist: track.albumArtist,
      trackNumber: track.trackNumber,
      year: track.year,
      genre: track.genre,
      codec: track.codec,
      duration: track.duration,
      hasCover: track.hasCover,
    };
  }

  private isInsideDirectory(filePath: string, directoryPath: string): boolean {
    const relative = path.relative(path.resolve(directoryPath), path.resolve(filePath));
    return relative !== "" && !relative.startsWith("..") && !path.isAbsolute(relative);
  }

  private nonBlank(value: string | null | undefined): string | null {
    if (!value) return null;
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : null;
  }
}
