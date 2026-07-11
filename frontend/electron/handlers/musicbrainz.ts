/**
 * MusicBrainz API client — raw fetch() to the JSON API.
 * Ported from Python auto_tagger.integrations.beets_client.
 *
 * Rate-limited to 1 req/sec as per MusicBrainz usage guidelines.
 */

import {
  type AlbumCandidate,
  type TrackCandidate,
  isAlbumTitleShortlistMatch,
  makeAlbumCandidate,
  makeTrackCandidate,
  normalizeLookupText,
  scoreAlbumTitleMatch,
} from "./candidates";
import type { ReleaseCache, ReleaseMeta } from "./cache";
import {
  scoreRemoteTrackTitleCoverage,
  type TrackTitleCoverageScore,
} from "../services/RemoteTrackMatcher";

const MUSICBRAINZ_BASE = "https://musicbrainz.org/ws/2";
const USER_AGENT = "auto-tagger/0.1.0 ( https://github.com/auto-tagger )";
const ARTIST_RELEASE_SHORTLIST_LIMIT = 3;

/**
 * Cache provider key for release-detail entries. Bump this to invalidate
 * stale cached AlbumCandidate objects (e.g. when parsing logic changes).
 */
const MUSICBRAINZ_RELEASE_DETAIL_CACHE_PROVIDER = "musicbrainz-v3";

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

function compareShortlistedRelease(
  a: { candidate: AlbumCandidate; albumScore: number; titleScore: TrackTitleCoverageScore | null },
  b: { candidate: AlbumCandidate; albumScore: number; titleScore: TrackTitleCoverageScore | null },
  localTrackCount: number,
): number {
  if (localTrackCount > 0 && (a.titleScore || b.titleScore)) {
    const aTitle = a.titleScore?.matched ?? 0;
    const bTitle = b.titleScore?.matched ?? 0;
    if (aTitle !== bTitle) return aTitle - bTitle;

    const aDuration = a.titleScore?.durationMatched ?? 0;
    const bDuration = b.titleScore?.durationMatched ?? 0;
    if (aDuration !== bDuration) return aDuration - bDuration;

    const aDelta = Math.abs(a.candidate.tracks.length - localTrackCount);
    const bDelta = Math.abs(b.candidate.tracks.length - localTrackCount);
    if (aDelta !== bDelta) return bDelta - aDelta;
  }

  return a.albumScore - b.albumScore;
}

export class MusicBrainzClient {
  private baseUrl: string;
  private releaseCache: ReleaseCache | null;
  private inFlightReleasePages: Map<string, Promise<ReleaseMeta[]>> | null;
  private inFlightReleaseDetails: Map<string, Promise<AlbumCandidate | null>> | null;

  constructor(options?: string | {
    baseUrl?: string;
    releaseCache?: ReleaseCache | null;
    inFlightReleasePages?: Map<string, Promise<ReleaseMeta[]>>;
    inFlightReleaseDetails?: Map<string, Promise<AlbumCandidate | null>>;
  }) {
    if (typeof options === "string") {
      this.baseUrl = options;
      this.releaseCache = null;
      this.inFlightReleasePages = null;
      this.inFlightReleaseDetails = null;
    } else {
      this.baseUrl = options?.baseUrl ?? MUSICBRAINZ_BASE;
      this.releaseCache = options?.releaseCache ?? null;
      this.inFlightReleasePages = options?.inFlightReleasePages ?? null;
      this.inFlightReleaseDetails = options?.inFlightReleaseDetails ?? null;
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
    const cached = this.releaseCache?.getReleaseDetail(MUSICBRAINZ_RELEASE_DETAIL_CACHE_PROVIDER, releaseId);
    if (cached) return cached;

    const key = `musicbrainz:release:${releaseId}`;
    const inFlight = this.inFlightReleaseDetails?.get(key);
    if (inFlight) return inFlight;

    const promise = this.fetchReleaseById(releaseId).finally(() => {
      this.inFlightReleaseDetails?.delete(key);
    });
    this.inFlightReleaseDetails?.set(key, promise);
    return promise;
  }

  private async fetchReleaseById(releaseId: string): Promise<AlbumCandidate | null> {
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
      this.releaseCache?.setReleaseDetail(MUSICBRAINZ_RELEASE_DETAIL_CACHE_PROVIDER, data.id ?? releaseId, candidate);
      return candidate;
    } catch {
      return null;
    }
  }

  async lookupArtistReleaseByAlbum(
    artistId: string,
    albumHint: string,
    options?: {
      yearHint?: string | null;
      localTracks?: TrackCandidate[];
      filenames?: string[];
      artistHints?: string[];
      alternateTrackTitles?: Array<string | null | undefined>;
    },
  ): Promise<AlbumCandidate | null> {
    const MAX_PAGES = 3;
    const LIMIT = 100;

    const shortlist: Array<{
      release: ReleaseMeta;
      albumScore: number;
    }> = [];

    for (let page = 1; page <= MAX_PAGES; page++) {
      const releases = await this.getArtistReleasePage(artistId, page, LIMIT);
      if (releases.length === 0) break;

      for (const release of releases) {
        const baseMatch = await scoreAlbumTitleMatch(albumHint, release.title);
        if (!isAlbumTitleShortlistMatch(baseMatch)) continue;

        const rankedMatch = await scoreAlbumTitleMatch(albumHint, release.title, {
          localYear: options?.yearHint,
          remoteYear: release.year,
          artistMatches: true,
        });
        shortlist.push({ release, albumScore: rankedMatch.score });
      }

      if (releases.length < LIMIT) break;
    }

    const detailShortlist = shortlist
      .sort((a, b) => b.albumScore - a.albumScore)
      .slice(0, ARTIST_RELEASE_SHORTLIST_LIMIT);
    if (detailShortlist.length === 0) return null;

    let best: {
      candidate: AlbumCandidate;
      albumScore: number;
      titleScore: TrackTitleCoverageScore | null;
    } | null = null;

    for (const item of detailShortlist) {
      const candidate = await this.lookupReleaseById(item.release.id);
      if (!candidate) continue;
      const titleScore = options?.localTracks && options.localTracks.length > 0
        ? await scoreRemoteTrackTitleCoverage(
            options.localTracks,
            options.filenames ?? [],
            candidate.tracks,
            "musicbrainz",
            {
              artistHints: options.artistHints,
              alternateTrackTitles: options.alternateTrackTitles,
            },
          )
        : null;
      const current = { candidate, albumScore: item.albumScore, titleScore };
      if (!best || compareShortlistedRelease(current, best, options?.localTracks?.length ?? 0) > 0) {
        best = current;
      }
    }

    return best?.candidate ?? null;
  }

  private async getArtistReleasePage(
    artistId: string,
    page: number,
    limit: number,
  ): Promise<ReleaseMeta[]> {
    const cached = this.releaseCache?.getArtistReleaseList("musicbrainz", artistId, page);
    if (cached) return cached;

    const key = `musicbrainz:artist:${artistId}:page:${page}:limit:${limit}`;
    const inFlight = this.inFlightReleasePages?.get(key);
    if (inFlight) return inFlight;

    const promise = this.fetchArtistReleasePage(artistId, page, limit).finally(() => {
      this.inFlightReleasePages?.delete(key);
    });
    this.inFlightReleasePages?.set(key, promise);
    return promise;
  }

  private async fetchArtistReleasePage(
    artistId: string,
    page: number,
    limit: number,
  ): Promise<ReleaseMeta[]> {
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
   * Format a MusicBrainz artist-credit array into a display string.
   * E.g. [{name: "林俊傑", joinphrase: " feat. "}, {name: "MC HotDog", joinphrase: ""}]
   *   → "林俊傑 feat. MC HotDog"
   */
  private formatArtistCredit(
    credit: Array<Record<string, unknown>>,
  ): string {
    return credit
      .map((c) => {
        const name = (c.name as string) ?? "";
        const join = (c.joinphrase as string) ?? "";
        return name + join;
      })
      .join("");
  }

  /**
   * Extract individual artist names from a MusicBrainz artist-credit array.
   * E.g. [{name: "林俊傑", ...}, {name: "MC HotDog", ...}]
   *   → ["林俊傑", "MC HotDog"]
   */
  private artistNamesFromCredit(
    credit: Array<Record<string, unknown>>,
  ): string[] {
    return credit
      .map((c) => (c.name as string) ?? "")
      .filter((n) => n.length > 0);
  }

  private parsePositiveInteger(value: unknown): number | null {
    if (typeof value === "number" && Number.isInteger(value) && value > 0) {
      return value;
    }
    if (typeof value === "string" && /^\d+$/.test(value.trim())) {
      return Number(value);
    }
    return null;
  }

  /**
   * Parse TrackCandidate array from release media data.
   * Per-track artist-credit takes precedence over release-level artistName:
   *   1. track["artist-credit"] (release-track level)
   *   2. recording["artist-credit"] (recording level)
   *   3. release-level artistName (fallback)
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
        const title = (track.title as string) ?? (recording?.title as string) ?? null;
        const recordingTitle = (recording?.title as string) ?? null;

        // Resolve artist credit: track-level → recording-level → release-level
        // Only treat non-empty arrays as authoritative (guard against empty credits)
        const hasCredit = (v: unknown): v is Array<Record<string, unknown>> =>
          Array.isArray(v) && v.length > 0;
        const trackCredit = track["artist-credit"];
        const recordingCredit = recording?.["artist-credit"];
        const resolvedCredit = hasCredit(trackCredit)
          ? trackCredit
          : hasCredit(recordingCredit)
            ? recordingCredit
            : null;

        let trackArtist: string | null;
        let trackArtists: string[];
        if (resolvedCredit) {
          trackArtist = this.formatArtistCredit(resolvedCredit);
          trackArtists = this.artistNamesFromCredit(resolvedCredit);
        } else {
          trackArtist = artistName;
          trackArtists = artistName ? [artistName] : [];
        }

        tracks.push(
          makeTrackCandidate({
            title,
            matchTitles: recordingTitle && recordingTitle !== title ? [recordingTitle] : [],
            artist: trackArtist,
            artists: trackArtists,
            trackNumber: this.parsePositiveInteger(track.number) ?? this.parsePositiveInteger(track.position),
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

    const url = `${this.baseUrl}/release/${releaseId}?fmt=json&inc=recordings+artist-credits`;

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

  async lookupArtistById(
    artistId: string,
  ): Promise<{ id: string; name: string; aliases?: Array<{ name: string; locale?: string; type?: string }> } | null> {
    await musicBrainzRateLimit();

    const url = `${this.baseUrl}/artist/${encodeURIComponent(artistId)}?fmt=json&inc=aliases`;

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
        name?: string;
        aliases?: Array<{ name: string; locale?: string; type?: string }>;
      };

      if (!data.id || !data.name) return null;
      return {
        id: data.id,
        name: data.name,
        aliases: data.aliases,
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
