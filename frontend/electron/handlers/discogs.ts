/**
 * Discogs API client — raw fetch() to the Discogs API for album search.
 * Ported from Python auto_tagger.integrations.discogs_client.
 *
 * Rate limit: 25 req/min unauthenticated, 60 req/min with token.
 */

import {
  type AlbumCandidate,
  type TrackCandidate,
  makeAlbumCandidate,
  makeTrackCandidate,
} from "./candidates";

const DISCOGS_BASE = "https://api.discogs.com";

// Sliding-window rate limiter: 25 req / 60s unauthenticated
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
}

export class DiscogsClient {
  private baseUrl: string;
  private token: string | null;
  private userAgent: string;
  private maxCandidates: number;
  private timeoutMs: number;
  private rateLimiter: DiscogsRateLimiter;

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
    this.rateLimiter = new DiscogsRateLimiter(this.token ? 60 : 25);
  }

  /**
   * Search Discogs for an album by artist and album name.
   */
  async searchAlbum(
    artist: string,
    album: string,
  ): Promise<AlbumCandidate[]> {
    if (!artist && !album) return [];

    const query = `${artist} ${album}`.trim();
    await this.rateLimiter.wait();

    const url = `${this.baseUrl}/database/search?q=${encodeURIComponent(query)}&type=master&per_page=${this.maxCandidates * 3}`;

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
        const year = result.year != null ? String(result.year) : null;
        const genre = ((result.genre as string[]) ?? []).join(", ") || null;
        const resourceUrl = result.resource_url as string ?? null;
        const masterId = result.id as number ?? null;

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

        // Load tracklist from master release
        let tracks: TrackCandidate[] = [];
        if (resourceUrl) {
          tracks = await this.loadTracklist(resourceUrl);
        }

        candidates.push(
          makeAlbumCandidate({
            artist: artistName,
            artists: [artistName],
            album: albumName,
            albumArtist: artistName,
            albumArtists: [artistName],
            year,
            genre,
            tracks,
            source: "discogs",
          }),
        );
      }

      return candidates;
    } catch {
      return [];
    }
  }

  /**
   * Load tracklist from a release or master URL.
   */
  private async loadTracklist(url: string): Promise<TrackCandidate[]> {
    await this.rateLimiter.wait();

    try {
      const response = await fetch(url, {
        headers: this.buildHeaders(),
        signal: AbortSignal.timeout(this.timeoutMs),
      });

      if (!response.ok) return [];

      const data = (await response.json()) as {
        tracklist?: Array<Record<string, unknown>>;
      };

      const tracklist = data.tracklist ?? [];
      return tracklist
        .filter((t) => {
          const pos = t.position as string;
          // Skip non-track entries like "CD1-1", "CD1" or empty positions
          return pos && pos.trim() && !pos.includes("-");
        })
        .map((t, i) => {
          const position = t.position as string;
          const num = parseInt(position, 10);
          return makeTrackCandidate({
            title: (t.title as string) ?? null,
            trackNumber: isNaN(num) ? i + 1 : num,
            length: parseDuration(t.duration as string),
          });
        });
    } catch {
      return [];
    }
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
