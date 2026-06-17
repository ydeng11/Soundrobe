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
  scoreAlbumTitleMatch,
  ALBUM_TITLE_MATCH_THRESHOLD,
} from "./candidates";
import type { ReleaseCache, ReleaseMeta } from "./cache";

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
  private releaseCache: ReleaseCache | null;

  constructor(options?: string | { baseUrl?: string; releaseCache?: ReleaseCache | null }) {
    if (typeof options === "string") {
      this.baseUrl = options;
      this.releaseCache = null;
    } else {
      this.baseUrl = options?.baseUrl ?? MUSICBRAINZ_BASE;
      this.releaseCache = options?.releaseCache ?? null;
    }
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
   * Look up a release by MusicBrainz release ID (MBID).
   * Calls /ws/2/release/{id}?inc=recordings+artist-credits directly.
   */
  async lookupReleaseById(releaseId: string): Promise<AlbumCandidate | null> {
    const cached = this.releaseCache?.getReleaseDetail("musicbrainz", releaseId);
    if (cached) return cached;

    await musicBrainzRateLimit();

    const url = `${this.baseUrl}/release/${releaseId}?fmt=json&inc=recordings+artist-credits`;

    try {
      const response = await fetch(url, {
        headers: {
          "User-Agent": USER_AGENT,
          Accept: "application/json",
        },
      });

      if (!response.ok) return null;

      const data = (await response.json()) as {
        id?: string;
        title?: string;
        "artist-credit"?: Array<Record<string, unknown>>;
        date?: string;
        media?: Array<Record<string, unknown>>;
      };

      const credit = data["artist-credit"] ?? [];
      const firstCredit = credit[0] as Record<string, unknown> | undefined;
      const artistName = (firstCredit?.name as string) ?? null;
      const mbArtistId = (firstCredit?.artist as Record<string, unknown>)?.id as string ?? null;
      const year = data.date ? data.date.slice(0, 4) : null;

      const tracks = this.parseTracksFromMedia(data.media ?? [], artistName);

      const candidate = makeAlbumCandidate({
        artist: artistName,
        artists: artistName ? [artistName] : [],
        album: data.title ?? null,
        albumArtist: artistName,
        albumArtists: artistName ? [artistName] : [],
        year,
        musicbrainzAlbumId: data.id ?? releaseId,
        musicbrainzArtistId: mbArtistId,
        tracks,
        source: "musicbrainz",
      });
      this.releaseCache?.setReleaseDetail("musicbrainz", data.id ?? releaseId, candidate);
      return candidate;
    } catch {
      return null;
    }
  }

  async lookupArtistReleaseByAlbum(
    artistId: string,
    albumHint: string,
    options?: { yearHint?: string | null },
  ): Promise<AlbumCandidate | null> {
    const MAX_PAGES = 3;
    const LIMIT = 100;

    let bestMatch: ReleaseMeta | null = null;
    let bestScore = 0;

    for (let page = 1; page <= MAX_PAGES; page++) {
      const releases = await this.getArtistReleasePage(artistId, page, LIMIT);
      if (releases.length === 0) break;

      for (const release of releases) {
        const match = await scoreAlbumTitleMatch(albumHint, release.title, {
          localYear: options?.yearHint,
          remoteYear: release.year,
          artistMatches: true,
        });
        if (match.score > bestScore) {
          bestScore = match.score;
          bestMatch = release;
        }
      }

      if (bestScore >= 100) break;
    }

    if (!bestMatch || bestScore < ALBUM_TITLE_MATCH_THRESHOLD) return null;
    return this.lookupReleaseById(bestMatch.id);
  }

  private async getArtistReleasePage(
    artistId: string,
    page: number,
    limit: number,
  ): Promise<ReleaseMeta[]> {
    const cached = this.releaseCache?.getArtistReleaseList("musicbrainz", artistId, page);
    if (cached) return cached;

    await musicBrainzRateLimit();
    const offset = (page - 1) * limit;
    const url = `${this.baseUrl}/release?artist=${encodeURIComponent(artistId)}&limit=${limit}&offset=${offset}&fmt=json&inc=artist-credits`;

    try {
      const response = await fetch(url, {
        headers: {
          "User-Agent": USER_AGENT,
          Accept: "application/json",
        },
      });

      if (!response.ok) return [];

      const data = (await response.json()) as {
        releases?: Array<Record<string, unknown>>;
      };

      const releases = (data.releases ?? [])
        .map((release) => {
          const credit = (release["artist-credit"] as Array<Record<string, unknown>>) ?? [];
          const firstCredit = credit[0] as Record<string, unknown> | undefined;
          const date = (release.date as string) ?? null;
          return {
            id: (release.id as string) ?? "",
            title: (release.title as string) ?? "",
            year: date ? Number(date.slice(0, 4)) : null,
            type: "release" as const,
            artistName: (firstCredit?.name as string) ?? null,
          };
        })
        .filter((release) => release.id && release.title);

      this.releaseCache?.setArtistReleaseList("musicbrainz", artistId, page, releases);
      return releases;
    } catch {
      return [];
    }
  }

  /**
   * Parse TrackCandidate array from release media data.
   */
  private parseTracksFromMedia(
    media: Array<Record<string, unknown>>,
    artistName: string | null,
  ): TrackCandidate[] {
    const tracks: TrackCandidate[] = [];

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
  }

  /**
   * Load tracks for a release by its MBID (separate API call).
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

      return this.parseTracksFromMedia(data.media ?? [], artistName);
    } catch {
      return [];
    }
  }

  /**
   * Search for an artist by name and return artist info with aliases.
   * Used by ArtistIdentityResolver to find English aliases for CJK artists.
   */
  async searchArtistByName(
    artistName: string,
  ): Promise<{ id: string; name: string; aliases?: Array<{ name: string; locale?: string; type?: string }> } | null> {
    await musicBrainzRateLimit();

    const query = `artist:"${escapeQuery(artistName)}"`;
    const url = `${this.baseUrl}/artist/?query=${encodeURIComponent(query)}&fmt=json&limit=5`;

    try {
      const response = await fetch(url, {
        headers: {
          "User-Agent": USER_AGENT,
          Accept: "application/json",
        },
      });

      if (!response.ok) return null;

      const data = (await response.json()) as {
        artists?: Array<{
          id?: string;
          name?: string;
          "sort-name"?: string;
          disambiguation?: string;
          type?: string;
          aliases?: Array<{ name: string; locale?: string; type?: string }>;
        }>;
      };

      const artists = data.artists ?? [];
      if (artists.length === 0) return null;

      // Find best match: exact name, Person type, with useful disambiguation
      let bestMatch = artists[0];
      for (const artist of artists) {
        if (!artist.name || !artist.id) continue;

        // Exact name match is preferred
        if (artist.name === artistName || artist["sort-name"] === artistName) {
          bestMatch = artist;
          break;
        }
      }

      if (!bestMatch?.id || !bestMatch?.name) return null;

      return {
        id: bestMatch.id,
        name: bestMatch.name,
        aliases: bestMatch.aliases,
      };
    } catch {
      return null;
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
