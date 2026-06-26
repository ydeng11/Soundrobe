/**
 * Centralized artist identity resolution service.
 *
 * Resolves artist names to their MusicBrainz and Discogs IDs by:
 * 1. Checking local cache (fast path)
 * 2. Searching Discogs with original name (exact match only)
 * 3. Searching MusicBrainz for English aliases
 * 4. Searching Discogs with English aliases
 *
 * Used by: ArtworkResolverService, auto-tag pipeline, audit flow.
 */

import { MusicBrainzClient } from "../handlers/musicbrainz";
import { DiscogsService } from "./DiscogsService";
import { saveAlias, getAliases, isChineseName, convertScript } from "../handlers/aliases";

// ── Types ──────────────────────────────────────────────────────────

export interface ArtistIdentity {
  /** MusicBrainz artist ID (if found) */
  musicbrainzArtistId: string | null;
  /** Discogs artist ID (if found) */
  discogsArtistId: string | null;
  /** English/Latin aliases discovered (e.g., ["Claire Kuo"]) */
  englishAliases: string[];
  /** Source of resolution */
  source: "cache" | "discogs-exact" | "musicbrainz" | "none";
}

interface CacheEntry {
  identity: ArtistIdentity;
  timestamp: number;
}

// ── Cache ──────────────────────────────────────────────────────────

const CACHE_TTL_MS = 24 * 60 * 60 * 1000; // 24 hours
const memoryCache = new Map<string, CacheEntry>();

function getCacheKey(
  artistName: string,
  options?: { skipDiscogs?: boolean; skipMusicBrainz?: boolean },
): string {
  const useMusicBrainz = options?.skipMusicBrainz === true ? "no-mb" : "mb";
  const useDiscogs = options?.skipDiscogs === true ? "no-discogs" : "discogs";
  return `${artistName.toLowerCase().trim()}|${useMusicBrainz}|${useDiscogs}`;
}

function getCached(
  artistName: string,
  options?: { skipDiscogs?: boolean; skipMusicBrainz?: boolean },
): ArtistIdentity | null {
  const key = getCacheKey(artistName, options);
  const entry = memoryCache.get(key);
  if (!entry) return null;
  if (Date.now() - entry.timestamp > CACHE_TTL_MS) {
    memoryCache.delete(key);
    return null;
  }
  return entry.identity;
}

function setCache(
  artistName: string,
  identity: ArtistIdentity,
  options?: { skipDiscogs?: boolean; skipMusicBrainz?: boolean },
): void {
  const key = getCacheKey(artistName, options);
  memoryCache.set(key, { identity, timestamp: Date.now() });
}

// ── Normalization ──────────────────────────────────────────────────

function normalizeForMatch(name: string): string {
  return name.toLowerCase().trim().replace(/\s+/g, " ");
}

function namesMatch(a: string, b: string): boolean {
  return normalizeForMatch(a) === normalizeForMatch(b);
}

/**
 * Check if Discogs title is an exact match for the artist name.
 * Handles cases like "胡彦斌" vs "胡彥斌" (simplified vs traditional).
 */
async function isExactDiscogsMatch(discogsTitle: string, searchName: string): Promise<boolean> {
  if (namesMatch(discogsTitle, searchName)) return true;

  // Check if titles are the same after removing common variations
  const clean = (s: string) => s.replace(/[\s\-_.]/g, "").toLowerCase();
  if (clean(discogsTitle) === clean(searchName)) return true;

  // Handle simplified vs traditional Chinese variations
  if (isChineseName(searchName) && isChineseName(discogsTitle)) {
    const searchVariants = await convertScript(searchName);
    const titleVariants = await convertScript(discogsTitle);
    
    for (const sv of searchVariants) {
      for (const tv of titleVariants) {
        if (namesMatch(sv, tv)) return true;
        if (clean(sv) === clean(tv)) return true;
      }
    }
  }

  return false;
}

/**
 * Check if a Discogs search result is high-confidence.
 * Rejects generic first results that don't match well.
 */
async function isHighConfidenceDiscogsResult(
  result: { title?: string; id?: number },
  searchName: string,
): Promise<boolean> {
  if (!result.title || !result.id) return false;

  // Exact match is always high confidence
  if (await isExactDiscogsMatch(result.title, searchName)) return true;

  // For non-Latin names, reject if Discogs title is completely different
  if (isChineseName(searchName)) {
    const titleNorm = normalizeForMatch(result.title);
    const nameNorm = normalizeForMatch(searchName);

    // Reject if title doesn't contain the search name at all
    if (!titleNorm.includes(nameNorm) && !nameNorm.includes(titleNorm)) {
      return false;
    }
  }

  return false;
}

// ── Main resolver ──────────────────────────────────────────────────

/**
 * Find artist identity (MB ID, Discogs ID, English aliases).
 *
 * Flow:
 * 1. Check local cache
 * 2. Search Discogs with original name (exact match only)
 * 3. Search MusicBrainz for English aliases
 * 4. Search Discogs with English aliases
 * 5. Cache and persist results
 */
export async function findArtistIdentity(
  artistName: string,
  options?: {
    discogsToken?: string | null;
    skipDiscogs?: boolean;
    skipMusicBrainz?: boolean;
  },
): Promise<ArtistIdentity> {
  const { discogsToken, skipDiscogs = false, skipMusicBrainz = false } = options ?? {};

  // 1. Check cache
  const cached = getCached(artistName, { skipDiscogs, skipMusicBrainz });
  if (cached) {
    return { ...cached, source: "cache" };
  }

  // Initialize services
  const mbClient = new MusicBrainzClient();
  const discogsService = new DiscogsService({ token: discogsToken });

  // 2. Try Discogs with original name (exact match only)
  if (!skipDiscogs) {
    const discogsResult = await searchDiscogsExact(discogsService, artistName);
    if (discogsResult) {
      const identity: ArtistIdentity = {
        musicbrainzArtistId: null,
        discogsArtistId: discogsResult.artistId,
        englishAliases: [],
        source: "discogs-exact",
      };
      setCache(artistName, identity, { skipDiscogs, skipMusicBrainz });
      return identity;
    }
  }

  // 3. Search MusicBrainz for English aliases
  if (!skipMusicBrainz) {
    const mbResult = await searchMusicBrainz(mbClient, artistName);
    if (mbResult) {
      // 4. Try Discogs with English aliases
      if (!skipDiscogs && mbResult.englishAliases.length > 0) {
        for (const alias of mbResult.englishAliases) {
          const discogsResult = await searchDiscogsExact(discogsService, alias);
          if (discogsResult) {
            const identity: ArtistIdentity = {
              musicbrainzArtistId: mbResult.artistId,
              discogsArtistId: discogsResult.artistId,
              englishAliases: mbResult.englishAliases,
              source: "musicbrainz",
            };
            setCache(artistName, identity, { skipDiscogs, skipMusicBrainz });
            saveAlias(artistName, alias);
            return identity;
          }
        }
      }

      // MB found but no Discogs match
      const identity: ArtistIdentity = {
        musicbrainzArtistId: mbResult.artistId,
        discogsArtistId: null,
        englishAliases: mbResult.englishAliases,
        source: "musicbrainz",
      };
      setCache(artistName, identity, { skipDiscogs, skipMusicBrainz });

      // Save discovered aliases even without Discogs match
      for (const alias of mbResult.englishAliases) {
        saveAlias(artistName, alias);
      }

      return identity;
    }
  }

  // 5. No results found
  const identity: ArtistIdentity = {
    musicbrainzArtistId: null,
    discogsArtistId: null,
    englishAliases: [],
    source: "none",
  };
  setCache(artistName, identity, { skipDiscogs, skipMusicBrainz });
  return identity;
}

// ── Internal search functions ──────────────────────────────────────

/**
 * Search Discogs with exact name match only.
 * Rejects generic first results that don't match well.
 */
async function searchDiscogsExact(
  service: DiscogsService,
  name: string,
): Promise<{ title: string; artistId: string } | null> {
  try {
    // Try precise artist=<name> search first
    const preciseUrl = `https://api.discogs.com/database/search?type=artist&artist=${encodeURIComponent(name)}&per_page=5`;
    const preciseRes = await service.fetch(preciseUrl);
    if (preciseRes) {
      const data = (await preciseRes.json()) as { results?: Array<{ title?: string; id?: number }> };
      const results = data.results ?? [];

      // Look for exact match
      for (const r of results) {
        if (await isHighConfidenceDiscogsResult(r, name)) {
          return { title: r.title!, artistId: String(r.id) };
        }
      }
    }

    // Fall back to generic q=<name> search
    const genericUrl = `https://api.discogs.com/database/search?type=artist&q=${encodeURIComponent(name)}&per_page=5`;
    const genericRes = await service.fetch(genericUrl);
    if (!genericRes) return null;

    const genericData = (await genericRes.json()) as { results?: Array<{ title?: string; id?: number }> };
    const results = genericData.results ?? [];

    // Look for exact match only
    for (const r of results) {
      if (await isHighConfidenceDiscogsResult(r, name)) {
        return { title: r.title!, artistId: String(r.id) };
      }
    }

    return null;
  } catch {
    return null;
  }
}

interface MusicBrainzArtistResult {
  artistId: string;
  englishAliases: string[];
}

/**
 * Search MusicBrainz for artist and extract English/Latin aliases.
 */
async function searchMusicBrainz(
  client: MusicBrainzClient,
  name: string,
): Promise<MusicBrainzArtistResult | null> {
  try {
    const result = await client.searchArtistByName(name);
    if (!result) return null;

    // Extract English/Latin aliases
    const englishAliases = (result.aliases ?? [])
      .filter((a: { name?: string; locale?: string; type?: string }) => {
        // Keep Latin-script aliases
        const aliasName = a.name ?? "";
        return /^[\x00-\x7F]+$/.test(aliasName);
      })
      .map((a: { name: string }) => a.name);

    return {
      artistId: result.id,
      englishAliases,
    };
  } catch {
    return null;
  }
}

// ── Convenience functions ──────────────────────────────────────────

/**
 * Get English aliases for an artist (from cache or API).
 * Useful when you only need the English name, not full identity.
 */
export async function getEnglishAliases(
  artistName: string,
  options?: { discogsToken?: string | null },
): Promise<string[]> {
  const identity = await findArtistIdentity(artistName, options);
  return identity.englishAliases;
}

/**
 * Get Discogs artist ID for an artist.
 * Useful for cover art resolution and tag writing.
 */
export async function getDiscogsArtistId(
  artistName: string,
  options?: { discogsToken?: string | null },
): Promise<string | null> {
  const identity = await findArtistIdentity(artistName, options);
  return identity.discogsArtistId;
}

/**
 * Get MusicBrainz artist ID for an artist.
 * Useful for album lookup and tag writing.
 */
export async function getMusicBrainzArtistId(
  artistName: string,
): Promise<string | null> {
  const identity = await findArtistIdentity(artistName, { skipDiscogs: true });
  return identity.musicbrainzArtistId;
}

/**
 * Clear the in-memory cache. Useful for testing.
 */
export function clearCache(): void {
  memoryCache.clear();
}
