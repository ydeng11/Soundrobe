/**
 * Discogs API client — raw fetch() to the Discogs API for album search.
 * Ported from Python auto_tagger.integrations.discogs_client.
 *
 * Rate limit: 25 req/min unauthenticated, 60 req/min with token.
 */

import {
  type AlbumCandidate,
  type TrackCandidate,
  artistDisplayName,
  makeAlbumCandidate,
  makeTrackCandidate,
  splitArtistNames,
} from "./candidates";

const DISCOGS_BASE = "https://api.discogs.com";

// ── App-wide rate limiter for Discogs API ──────────────────────────
// Sliding-window rate limiter: 25 req / 60s unauthenticated, 60/min with token.
// Created at module level so all DiscogsClient instances share the same budget.

class DiscogsRateLimiter {
  private timestamps: number[] = [];
  private maxReqs: number;
  private windowMs: number;

  constructor(maxReqs = 25, windowMs = 60_000) {
    this.maxReqs = maxReqs;
    this.windowMs = windowMs;
  }

  async wait(): Promise<void> {
    const now = Date.now();
    // Prune expired timestamps
    this.timestamps = this.timestamps.filter((t) => now - t < this.windowMs);

    if (this.timestamps.length >= this.maxReqs) {
      const oldest = this.timestamps[0];
      const waitMs = this.windowMs - (now - oldest) + 100;
      if (waitMs > 0) {
        await new Promise((r) => setTimeout(r, waitMs));
      }
    }

    this.timestamps.push(Date.now());
  }

  /** Update the max requests per window (e.g. when token becomes available). */
  setMaxReqs(n: number): void {
    this.maxReqs = n;
  }
}

/** Shared app-wide Discogs rate limiter. Starts at 25/min (anonymous). */
const sharedDiscogsRateLimiter = new DiscogsRateLimiter(25);

/**
 * Update the shared Discogs rate limit when an API token is available.
 * Called during client construction; safe to call multiple times.
 */
function updateDiscogsRateLimit(hasToken: boolean): void {
  if (hasToken) {
    sharedDiscogsRateLimiter.setMaxReqs(60);
  }
}

export class DiscogsClient {
  private baseUrl: string;
  private token: string | null;
  private userAgent: string;
  private maxCandidates: number;
  private timeoutMs: number;

  constructor(options?: {
    token?: string | null;
    userAgent?: string;
    maxCandidates?: number;
    timeoutSeconds?: number;
  }) {
    this.baseUrl = DISCOGS_BASE;
    this.token = options?.token ?? null;
    this.userAgent = options?.userAgent ?? "auto-tagger/0.1.0";
    this.maxCandidates = options?.maxCandidates ?? 3;
    this.timeoutMs = (options?.timeoutSeconds ?? 20) * 1000;
    // Update shared rate limiter when token is present
    updateDiscogsRateLimit(!!this.token);
  }

  /**
   * Search Discogs for an album by artist and album name.
   */
  async searchAlbum(
    artist: string,
    album: string,
  ): Promise<AlbumCandidate[]> {
    if (!artist && !album) return [];

    const releaseCandidates = await this.searchAlbumByType(artist, album, "release");
    if (releaseCandidates.length > 0) return releaseCandidates;
    return this.searchAlbumByType(artist, album, "master");
  }

  private async searchAlbumByType(
    artist: string,
    album: string,
    searchType: "release" | "master",
  ): Promise<AlbumCandidate[]> {
    const query = `${artist} ${album}`.trim();
    await sharedDiscogsRateLimiter.wait();

    const url = `${this.baseUrl}/database/search?q=${encodeURIComponent(query)}&type=${searchType}&per_page=${this.maxCandidates * 3}`;

    try {
      const response = await fetch(url, {
        headers: this.buildHeaders(),
        signal: AbortSignal.timeout(this.timeoutMs),
      });

      if (!response.ok) {
        return [];
      }

      const data = (await response.json()) as {
        results?: Array<Record<string, unknown>>;
      };

      const results = data.results ?? [];
      const candidates: AlbumCandidate[] = [];

      for (const result of results.slice(0, this.maxCandidates)) {
        const title = result.title as string ?? "";
        const resourceUrl = result.resource_url as string ?? null;

        // Parse title format: "Artist - Album" (Discogs format)
        const artistName = title.includes(" - ")
          ? title.split(" - ")[0].trim()
          : artist;
        const albumName = title.includes(" - ")
          ? title.split(" - ")[1].trim()
          : title;

        // Only include if it looks like the artist matches (or no artist constraint)
        if (artist && !this.artistMatchesHint(artistName, artist)) {
          continue;
        }

        let candidate: AlbumCandidate | null = null;
        if (resourceUrl) {
          candidate = await this.loadReleaseCandidate(
            resourceUrl,
            artistName,
            albumName,
            result.year != null ? String(result.year) : null,
            mergeGenreStyle(result.genre, result.style),
          );
        }
        if (!candidate) {
          const artists = splitArtistNames([artistName]);
          candidate = makeAlbumCandidate({
            artist: artistName,
            artists,
            album: albumName,
            albumArtist: artistName,
            albumArtists: artists,
            year: result.year != null ? String(result.year) : null,
            genre: mergeGenreStyle(result.genre, result.style),
            tracks: [],
            source: "discogs",
          });
        }
        candidates.push(candidate);
      }

      return candidates;
    } catch {
      return [];
    }
  }

  private async loadReleaseCandidate(
    url: string,
    fallbackArtist: string,
    fallbackAlbum: string,
    fallbackYear: string | null,
    fallbackGenre: string | null,
  ): Promise<AlbumCandidate | null> {
    await sharedDiscogsRateLimiter.wait();

    try {
      const response = await fetch(url, {
        headers: this.buildHeaders(),
        signal: AbortSignal.timeout(this.timeoutMs),
      });
      if (!response.ok) return null;

      const data = (await response.json()) as {
        title?: string;
        artists?: Array<Record<string, unknown>>;
        year?: number | string;
        genres?: string[];
        styles?: string[];
        tracklist?: Array<Record<string, unknown>>;
      };

      const artists = parseDiscogsArtists(data.artists, fallbackArtist);
      const albumArtist = artistDisplayName(artists, fallbackArtist);
      const tracks = this.tracksFromRelease(data.tracklist ?? [], artists);
      return makeAlbumCandidate({
        artist: albumArtist,
        artists,
        album: data.title ?? fallbackAlbum,
        albumArtist,
        albumArtists: artists,
        year: data.year != null ? String(data.year) : fallbackYear,
        genre: mergeGenreStyle(data.genres, data.styles) ?? fallbackGenre,
        tracks,
        source: "discogs",
      });
    } catch {
      return null;
    }
  }

  private tracksFromRelease(
    tracklist: Array<Record<string, unknown>>,
    albumArtists: string[],
  ): TrackCandidate[] {
    const tracks = tracklist
        .filter((t) => {
          const pos = t.position as string;
          return pos && pos.trim();
        })
        .map((t, i) => {
          const position = t.position as string;
          const parsed = parseDiscogsPosition(position);
          const artists = parseDiscogsArtists(
            (t.artists as Array<Record<string, unknown>>) ??
              (t.extraartists as Array<Record<string, unknown>>) ??
              [],
            albumArtists[0] ?? null,
          );
          return makeTrackCandidate({
            title: (t.title as string) ?? null,
            artist: artistDisplayName(artists, albumArtists[0] ?? null),
            artists: artists.length > 0 ? artists : albumArtists,
            trackNumber: parsed.trackNumber ?? i + 1,
            discNumber: parsed.discNumber,
            length: parseDuration(t.duration as string),
          });
        });
    for (const track of tracks) track.trackTotal = tracks.length;
    return tracks;
  }

  /**
   * Check if the Discogs artist name matches the hint.
   */
  private artistMatchesHint(
    discogsArtist: string,
    hint: string,
  ): boolean {
    const a = discogsArtist.toLowerCase().trim();
    const b = hint.toLowerCase().trim();
    return a.includes(b) || b.includes(a);
  }

  private buildHeaders(): Record<string, string> {
    const headers: Record<string, string> = {
      "User-Agent": this.userAgent,
    };
    if (this.token) {
      headers["Authorization"] = `Discogs token=${this.token}`;
    }
    return headers;
  }
}

function mergeGenreStyle(genre: unknown, style: unknown): string | null {
  const values = [
    ...(Array.isArray(genre) ? genre : []),
    ...(Array.isArray(style) ? style : []),
  ]
    .map((item) => String(item).trim())
    .filter(Boolean);
  return values.length > 0 ? [...new Set(values)].join(", ") : null;
}

function parseDiscogsArtists(
  rawArtists: Array<Record<string, unknown>> | undefined,
  fallback: string | null,
): string[] {
  const names = rawArtists
    ?.map((artist) => cleanDiscogsArtist(String(artist.name ?? "")))
    .filter(Boolean);
  return splitArtistNames(names && names.length > 0 ? names : [fallback]);
}

function cleanDiscogsArtist(name: string): string {
  return name.replace(/\s+\(\d+\)$/, "").trim();
}

function parseDiscogsPosition(position: string): {
  discNumber: number | null;
  trackNumber: number | null;
} {
  const compact = position.trim();
  const cdMatch = /^CD\s*(\d+)[-. ]*(\d+)$/i.exec(compact);
  if (cdMatch) {
    return {
      discNumber: Number(cdMatch[1]),
      trackNumber: Number(cdMatch[2]),
    };
  }

  const numberMatch = /(\d+)$/.exec(compact);
  return {
    discNumber: null,
    trackNumber: numberMatch ? Number(numberMatch[1]) : null,
  };
}

/**
 * Parse a Discogs duration string (e.g. "4:30" or "4:30:00") to seconds.
 */
function parseDuration(duration: string | null | undefined): number | null {
  if (!duration) return null;
  const parts = duration.split(":").map(Number);
  if (parts.some(isNaN)) return null;
  if (parts.length === 2) return parts[0] * 60 + parts[1];
  if (parts.length === 3) return parts[0] * 3600 + parts[1] * 60 + parts[2];
  return null;
}
