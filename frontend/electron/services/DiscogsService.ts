/**
 * Independent Discogs API service used by auto-tag, audit, and artwork resolver.
 *
 * Consolidates all Discogs API access into one place with shared rate limiting,
 * consistent headers, and unified search/lookup methods.
 */

import {
  ALBUM_TITLE_MATCH_THRESHOLD,
  scoreAlbumTitleMatch,
} from "../handlers/candidates";

const DISCOGS_BASE = "https://api.discogs.com";

// ── Rate limiter ────────────────────────────────────────────────────

class DiscogsRateLimiter {
  private timestamps: number[] = [];
  private maxReqs = 25;
  private windowMs = 60_000;
  private tokenSet = false;

  async wait(): Promise<void> {
    const now = Date.now();
    this.timestamps = this.timestamps.filter((t) => now - t < this.windowMs);

    if (this.timestamps.length >= this.maxReqs) {
      const oldest = this.timestamps[0];
      const waitMs = this.windowMs - (now - oldest) + 100;
      if (waitMs > 0) await new Promise((r) => setTimeout(r, waitMs));
    }
    this.timestamps.push(Date.now());
  }

  setTokenPresent(present: boolean): void {
    if (present && !this.tokenSet) {
      this.maxReqs = 60;
      this.tokenSet = true;
    }
  }
}

const sharedLimiter = new DiscogsRateLimiter();

// ── Types ───────────────────────────────────────────────────────────

export interface DiscogsArtistResult {
  title: string;
  artistId: number;
}

export interface DiscogsArtistDetail {
  name: string;
  images: Array<{ type: string; uri: string }>;
}

export interface DiscogsSearchResult {
  title: string;
  id?: number;
  cover_image?: string;
  resource_url?: string;
  year?: number | string;
  genre?: unknown;
  style?: unknown;
}

export interface DiscogsReleaseDetail {
  id?: number;
  title?: string;
  artists?: Array<{ name?: string }>;
  year?: number | string;
  genres?: string[];
  styles?: string[];
  tracklist?: Array<{ position?: string; title?: string; duration?: string; artists?: unknown[]; extraartists?: unknown[] }>;
  images?: Array<{ type: string; uri: string }>;
}

// ── Service ─────────────────────────────────────────────────────────

export class DiscogsService {
  private token: string | null;
  private userAgent: string;
  private timeoutMs: number;

  constructor(options?: {
    token?: string | null;
    userAgent?: string;
    timeoutSeconds?: number;
  }) {
    this.token = options?.token ?? null;
    this.userAgent = options?.userAgent ?? "auto-tagger/0.1.0";
    this.timeoutMs = (options?.timeoutSeconds ?? 15) * 1000;
    if (this.token) sharedLimiter.setTokenPresent(true);
  }

  // ── Public API methods ──────────────────────────────────────────

  /**
   * Search for an artist by name.
   * First tries precise artist=<name> search, then falls back to
   * generic q=<name>&type=artist (catches non-Latin names where the
   * Discogs title is in Latin script).
   * Returns null if precise search succeeds (no alias needed) or if
   * neither search finds anything.
   */
  async searchArtists(artistName: string): Promise<DiscogsArtistResult | null> {
    // Step 1: Precise artist=<name> search
    const preciseUrl = `${DISCOGS_BASE}/database/search?type=artist&artist=${encodeURIComponent(artistName)}&per_page=5`;
    const preciseRes = await this.fetch(preciseUrl);
    if (preciseRes) {
      const data = (await preciseRes.json()) as { results?: DiscogsSearchResult[] };
      const r = (data.results ?? [])[0];
      if (r && r.id != null) {
        return null; // resolves directly, no alias needed
      }
    }

    // Step 2: Generic q=<name>&type=artist search
    const genericUrl = `${DISCOGS_BASE}/database/search?type=artist&q=${encodeURIComponent(artistName)}&per_page=5`;
    const genericRes = await this.fetch(genericUrl);
    if (!genericRes) return null;

    const genericData = (await genericRes.json()) as { results?: DiscogsSearchResult[] };
    const first = (genericData.results ?? [])[0];
    if (!first || !first.id) return null;

    const discogsTitle = first.title ?? "";
    if (!discogsTitle || discogsTitle === artistName) return null;

    return { title: discogsTitle, artistId: first.id };
  }

  /**
   * Search for releases by artist + album name.
   * Returns raw search results (no detail fetch).
   */
  async searchReleases(
    artist: string,
    album: string,
    searchType: "release" | "master" = "release",
    perPage = 10,
  ): Promise<DiscogsSearchResult[]> {
    const query = `${artist} ${album}`.trim();
    const url = `${DISCOGS_BASE}/database/search?q=${encodeURIComponent(query)}&type=${searchType}&per_page=${perPage}`;
    const res = await this.fetch(url);
    if (!res) return [];

    const data = (await res.json()) as { results?: DiscogsSearchResult[] };
    return data.results ?? [];
  }

  /**
   * Get artist detail (name + images).
   * Used by cover downloader to fetch artist images.
   */
  async getArtistDetail(artistId: number): Promise<DiscogsArtistDetail | null> {
    const url = `${DISCOGS_BASE}/artists/${artistId}`;
    const res = await this.fetch(url);
    if (!res) return null;

    const data = (await res.json()) as DiscogsArtistDetail & Record<string, unknown>;
    if (!data.name) return null;

    return {
      name: data.name,
      images: data.images ?? [],
    };
  }

  /**
   * Fetch a release by ID.
   */
  async getReleaseDetail(releaseId: number | string): Promise<DiscogsReleaseDetail | null> {
    const url = `${DISCOGS_BASE}/releases/${releaseId}`;
    const res = await this.fetch(url);
    if (!res) return null;

    return (await res.json()) as DiscogsReleaseDetail;
  }

  /**
   * Fetch an artist's releases, filtered by album title match.
   * Uses the shared album-title scorer so service/artwork matching stays
   * aligned with auto-tag provider lookup behavior.
   */
  async getArtistReleaseByTitle(
    artistId: number | string,
    albumHint: string,
  ): Promise<DiscogsReleaseDetail | null> {
    const MAX_PAGES = 3;
    const PER_PAGE = 50;

    let bestMatch: number | null = null;
    let bestScore = 0;

    for (let page = 1; page <= MAX_PAGES; page++) {

      const url = `${DISCOGS_BASE}/artists/${artistId}/releases?per_page=${PER_PAGE}&page=${page}`;
      const res = await this.fetch(url);
      if (!res) break;

      const data = (await res.json()) as {
        releases?: Array<{ id: number; title: string }>;
        pagination?: { pages: number; items: number };
      };
      if (!data.releases || data.releases.length === 0) break;

      for (const release of data.releases) {
        const match = await scoreAlbumTitleMatch(albumHint, release.title);
        if (match.score > bestScore) {
          bestScore = match.score;
          bestMatch = release.id;
        }
      }

      // Short-circuit: stop if we found a perfect match or exhausted pages
      if (bestScore === 100 || (data.pagination && page >= data.pagination.pages)) break;
    }

    if (!bestMatch || bestScore < ALBUM_TITLE_MATCH_THRESHOLD) return null;
    return this.getReleaseDetail(bestMatch);
  }

  /**
   * Fetch an image URL. Returns bytes + mime type.
   */
  async fetchImage(url: string): Promise<{ bytes: Buffer; mime: string } | null> {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(this.timeoutMs) });
      if (!res.ok) return null;
      const bytes = Buffer.from(await res.arrayBuffer());
      const mime = res.headers.get("content-type") ?? "image/jpeg";
      return { bytes, mime };
    } catch {
      return null;
    }
  }

  // ── Internal helpers ────────────────────────────────────────────

  private buildHeaders(): Record<string, string> {
    const headers: Record<string, string> = { "User-Agent": this.userAgent };
    if (this.token) headers["Authorization"] = `Discogs token=${this.token}`;
    return headers;
  }

  async fetch(url: string): Promise<Response | null> {
    await sharedLimiter.wait();
    try {
      const res = await fetch(url, {
        headers: this.buildHeaders(),
        signal: AbortSignal.timeout(this.timeoutMs),
      });
      return res.ok ? res : null;
    } catch {
      return null;
    }
  }
}
