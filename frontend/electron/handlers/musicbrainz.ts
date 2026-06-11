/**
 * MusicBrainz API client — raw fetch() to the JSON API.
 * Ported from Python auto_tagger.integrations.beets_client.
 *
 * Rate-limited to 1 req/sec as per MusicBrainz usage guidelines.
 */

import {
  type AlbumCandidate,
  type TrackCandidate,
  makeAlbumCandidate,
  makeTrackCandidate,
  normalizeLookupText,
} from "./candidates";

const MUSICBRAINZ_BASE = "https://musicbrainz.org/ws/2";
const USER_AGENT = "auto-tagger/0.1.0 ( https://github.com/auto-tagger )";

/**
 * App-wide 1-req/sec rate limiter for MusicBrainz API.
 * Module-level state so all MusicBrainzClient instances share the same
 * request budget across the entire application.
 */
let lastMusicBrainzCall = 0;
async function musicBrainzRateLimit(): Promise<void> {
  const now = Date.now();
  const elapsed = now - lastMusicBrainzCall;
  if (elapsed < 1000 && lastMusicBrainzCall > 0) {
    await new Promise((r) => setTimeout(r, 1000 - elapsed));
  }
  lastMusicBrainzCall = Date.now();
}

export class MusicBrainzClient {
  private baseUrl: string;

  constructor(baseUrl?: string) {
    this.baseUrl = baseUrl ?? MUSICBRAINZ_BASE;
  }

  /**
   * Search for an album release by artist name and album title.
   */
  async searchAlbum(
    artistHint: string | null,
    albumHint: string | null,
    maxCandidates = 5,
  ): Promise<AlbumCandidate[]> {
    if (!artistHint || !albumHint) return [];

    const query = `artist:"${escapeQuery(artistHint)}" AND release:"${escapeQuery(albumHint)}"`;

    await musicBrainzRateLimit();

    const url = `${this.baseUrl}/release?query=${encodeURIComponent(query)}&fmt=json&limit=${Math.min(maxCandidates, 25)}`;

    let response: Response;
    try {
      response = await fetch(url, {
        headers: {
          "User-Agent": USER_AGENT,
          Accept: "application/json",
        },
      });
    } catch (err) {
      throw new Error(`MusicBrainz request failed: ${(err as Error).message}`);
    }

    if (!response.ok) {
      throw new Error(`MusicBrainz API error ${response.status}`);
    }

    const data = (await response.json()) as {
      releases?: Array<Record<string, unknown>>;
      "release-count"?: number;
    };

    const releases = data.releases ?? [];
    if (releases.length === 0) return [];

    const candidates: AlbumCandidate[] = [];

    for (const release of releases.slice(0, maxCandidates)) {
      const releaseId = release.id as string;
      const title = (release.title as string) ?? null;
      const credit = (release["artist-credit"] as Array<Record<string, unknown>>) ?? [];
      const firstCredit = credit[0] as Record<string, unknown> | undefined;
      const artistName = (firstCredit?.name as string) ?? null;
      const musicbrainzArtistId = (firstCredit?.artist as Record<string, unknown>)?.id as string ?? null;

      const date = (release.date as string) ?? null;
      const year = date ? date.slice(0, 4) : null;

      // Load tracks for this release
      const tracks = await this.loadTracks(releaseId, artistName);

      candidates.push(
        makeAlbumCandidate({
          artist: artistName,
          artists: artistName ? [artistName] : [],
          album: title,
          albumArtist: artistName,
          albumArtists: artistName ? [artistName] : [],
          year,
          musicbrainzAlbumId: releaseId,
          musicbrainzArtistId,
          tracks,
          source: "musicbrainz",
        }),
      );
    }

    return candidates;
  }

  /**
   * Load tracks for a release by its MBID.
   */
  private async loadTracks(
    releaseId: string,
    artistName: string | null,
  ): Promise<TrackCandidate[]> {
    await musicBrainzRateLimit();

    const url = `${this.baseUrl}/release/${releaseId}?fmt=json&inc=recordings`;

    try {
      const response = await fetch(url, {
        headers: {
          "User-Agent": USER_AGENT,
          Accept: "application/json",
        },
      });

      if (!response.ok) return [];

      const data = (await response.json()) as {
        media?: Array<Record<string, unknown>>;
      };

      const tracks: TrackCandidate[] = [];
      const media = data.media ?? [];

      for (const medium of media) {
        const discNumber = (medium.position as number) ?? null;
        const recordings = (medium.tracks as Array<Record<string, unknown>>) ?? [];

        for (const track of recordings) {
          const recording = track.recording as Record<string, unknown> | undefined;
          tracks.push(
            makeTrackCandidate({
              title: (track.title as string) ?? (recording?.title as string) ?? null,
              artist: artistName,
              artists: artistName ? [artistName] : [],
              trackNumber: (track.number as number) ?? (track.position as number) ?? null,
              discNumber,
              musicbrainzTrackId: recording?.id as string ?? null,
              length: recording?.length as number ?? null,
            }),
          );
        }
      }

      return tracks;
    } catch {
      return [];
    }
  }
}

/**
 * Escape special characters in a MusicBrainz query string.
 */
function escapeQuery(value: string): string {
  return value
    .replace(/"/g, '\\"')
    .replace(/\(/g, "\\(")
    .replace(/\)/g, "\\)");
}
