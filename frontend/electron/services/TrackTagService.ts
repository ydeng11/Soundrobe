/**
 * TrackTagService — reusable standard tag operations.
 *
 * Shared between IPC handlers and the assistant runtime.
 * Provides plan-update (preview) and apply flows.
 * Tag updates are reversible through existing undo snapshots.
 */

import { writeTags, batchWriteTags } from "../handlers/writer";
import type { WriteFields } from "../handlers/writer";
import { readTrackMetadata } from "../handlers/tracks";
import type { TrackData } from "../handlers/tracks";

export interface TagUpdateInstruction {
  trackPath: string;
  fields: WriteFields;
}

export interface PlannedTagAction {
  trackPath: string;
  field: string;
  oldValue: string | null | undefined;
  newValue: string | null | undefined;
}

export interface PlannedActionBatch {
  kind: "tag-update" | "extra-tag-update" | "folder-move";
  summary: string;
  actions: PlannedTagAction[];
  affectedTracks: number;
  reversible: boolean;
}

export interface TagUpdateResult {
  trackPath: string;
  success: boolean;
  error?: string;
  updatedTrack?: TrackData;
}

export class TrackTagService {
  /**
   * Plan tag updates by reading current metadata and computing diffs.
   * Does NOT write anything.
   */
  async planTagUpdates(
    instructions: TagUpdateInstruction[],
  ): Promise<PlannedActionBatch> {
    const actions: PlannedTagAction[] = [];

    for (const instruction of instructions) {
      const currentTrack = await readTrackMetadata(instruction.trackPath);

      for (const [field, newValue] of Object.entries(instruction.fields)) {
        const currentValue = this.getCurrentField(currentTrack, field);
        const newVal = newValue as string | null | undefined;

        // Skip if values are the same
        if (String(currentValue ?? "") === String(newVal ?? "")) continue;

        actions.push({
          trackPath: instruction.trackPath,
          field,
          oldValue: currentValue,
          newValue: newVal,
        });
      }
    }

    const affectedPaths = new Set(actions.map((a) => a.trackPath));

    return {
      kind: "tag-update",
      summary: actions.length > 0
        ? `Update ${actions.length} tag fields across ${affectedPaths.size} track(s)`
        : "No changes needed",
      actions,
      affectedTracks: affectedPaths.size,
      reversible: true,
    };
  }

  /**
   * Apply tag updates to disk and return updated metadata.
   */
  async applyTagUpdates(
    updates: TagUpdateInstruction[],
  ): Promise<TagUpdateResult[]> {
    // Write tags
    const writeUpdates = updates.map((u) => ({
      path: u.trackPath,
      fields: u.fields,
    }));
    await batchWriteTags(writeUpdates);

    // Re-read all updated tracks
    const results: TagUpdateResult[] = [];
    for (const update of updates) {
      try {
        const updatedTrack = await readTrackMetadata(update.trackPath);
        results.push({
          trackPath: update.trackPath,
          success: true,
          updatedTrack,
        });
      } catch (error) {
        results.push({
          trackPath: update.trackPath,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }

    return results;
  }

  /**
   * Build undo snapshots from tag updates for the undo manager.
   * Each snapshot stores the current metadata before writes.
   */
  async buildUndoSnapshots(
    updates: TagUpdateInstruction[],
  ): Promise<Array<{ path: string; metadata: Record<string, unknown> }>> {
    const snapshots: Array<{ path: string; metadata: Record<string, unknown> }> = [];

    for (const update of updates) {
      const track = await readTrackMetadata(update.trackPath);
      snapshots.push({
        path: update.trackPath,
        metadata: {
          title: track.title,
          artist: track.artist,
          artists: track.artists,
          album: track.album,
          albumArtist: track.albumArtist,
          albumArtists: track.albumArtists,
          year: track.year,
          genre: track.genre,
          composer: track.composer,
          comment: track.comment,
          description: track.description,
          trackNumber: track.trackNumber,
          trackTotal: track.trackTotal,
          discNumber: track.discNumber,
          discTotal: track.discTotal,
          lyrics: track.lyrics,
          compilation: track.compilation,
          musicbrainzTrackId: track.musicbrainzTrackId,
          musicbrainzAlbumId: track.musicbrainzAlbumId,
          musicbrainzArtistId: track.musicbrainzArtistId,
        },
      });
    }

    return snapshots;
  }

  private getCurrentField(
    track: TrackData,
    field: string,
  ): string | null | undefined {
    switch (field) {
      case "title": return track.title;
      case "artist": return track.artist;
      case "artists": return track.artists.join("; ");
      case "album": return track.album;
      case "albumArtist": return track.albumArtist;
      case "albumArtists": return track.albumArtists.join("; ");
      case "year": return track.year;
      case "genre": return track.genre;
      case "composer": return track.composer;
      case "comment": return track.comment;
      case "description": return track.description;
      case "lyrics": return track.lyrics;
      case "trackNumber": return track.trackNumber?.toString();
      case "trackTotal": return track.trackTotal?.toString();
      case "discNumber": return track.discNumber?.toString();
      case "discTotal": return track.discTotal?.toString();
      case "compilation": return track.compilation?.toString();
      case "musicbrainzTrackId": return track.musicbrainzTrackId;
      case "musicbrainzAlbumId": return track.musicbrainzAlbumId;
      case "musicbrainzArtistId": return track.musicbrainzArtistId;
      default: return undefined;
    }
  }
}
