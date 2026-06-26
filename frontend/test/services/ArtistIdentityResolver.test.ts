/**
 * Tests for ArtistIdentityResolver — centralized artist identity resolution.
 *
 * Tests the flow:
 * 1. Cache hit (fast path)
 * 2. Discogs exact match (single API call)
 * 3. MusicBrainz alias lookup (MB + Discogs)
 * 4. Negative caching (no repeated lookups)
 */

import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  findArtistIdentity,
  getEnglishAliases,
  getDiscogsArtistId,
  clearCache,
} from "../../electron/services/ArtistIdentityResolver";

// Mock MusicBrainzClient
vi.mock("../../electron/handlers/musicbrainz", () => ({
  MusicBrainzClient: vi.fn().mockImplementation(() => ({
    searchArtistByName: vi.fn(),
  })),
}));

// Mock DiscogsService
vi.mock("../../electron/services/DiscogsService", () => ({
  DiscogsService: vi.fn().mockImplementation(() => ({
    fetch: vi.fn(),
  })),
}));

// Mock aliases
vi.mock("../../electron/handlers/aliases", () => ({
  saveAlias: vi.fn(),
  getAliases: vi.fn().mockReturnValue([]),
  isChineseName: vi.fn((name: string) => /[\u4e00-\u9fff]/.test(name)),
}));

describe("ArtistIdentityResolver", () => {
  beforeEach(() => {
    clearCache();
    vi.clearAllMocks();
  });

  describe("cache", () => {
    it("returns cached result on second call", async () => {
      const { MusicBrainzClient } = await import("../../electron/handlers/musicbrainz");
      const mbClient = {
        searchArtistByName: vi.fn().mockResolvedValue(null),
      };
      vi.mocked(MusicBrainzClient).mockImplementation(() => mbClient as any);

      const { DiscogsService } = await import("../../electron/services/DiscogsService");
      const fetchMock = vi.fn().mockResolvedValue(null);
      vi.mocked(DiscogsService).mockImplementation(() => ({
        fetch: fetchMock,
      } as any));

      // First call - should hit Discogs API (2 calls: precise + generic search)
      const result1 = await findArtistIdentity("单依纯", { skipDiscogs: false });
      expect(result1.source).toBe("none");
      expect(result1.discogsArtistId).toBeNull();
      const firstCallCount = fetchMock.mock.calls.length;
      expect(firstCallCount).toBe(2); // precise + generic search

      // Second call - should hit cache (no API calls)
      const result2 = await findArtistIdentity("单依纯", { skipDiscogs: false });
      expect(result2.source).toBe("cache");

      // Fetch should not be called again (cache hit)
      expect(fetchMock).toHaveBeenCalledTimes(firstCallCount);
    });

    it("does not reuse a Discogs-only cache entry when MusicBrainz is requested later", async () => {
      const { DiscogsService } = await import("../../electron/services/DiscogsService");
      const dgService = {
        fetch: vi.fn().mockResolvedValue({
          json: () =>
            Promise.resolve({
              results: [{ title: "F.I.R.", id: 2539109 }],
            }),
        }),
      };
      vi.mocked(DiscogsService).mockImplementation(() => dgService as any);

      const discogsOnly = await findArtistIdentity("F.I.R.", { skipMusicBrainz: true });
      expect(discogsOnly.discogsArtistId).toBe("2539109");
      expect(discogsOnly.musicbrainzArtistId).toBeNull();

      const { MusicBrainzClient } = await import("../../electron/handlers/musicbrainz");
      const mbClient = {
        searchArtistByName: vi.fn().mockResolvedValue({
          id: "a8251b7f-2ea9-4661-89c1-0950b5867034",
          name: "F.I.R.",
          aliases: [],
        }),
      };
      vi.mocked(MusicBrainzClient).mockImplementation(() => mbClient as any);

      const mbOnly = await findArtistIdentity("F.I.R.", { skipDiscogs: true });

      expect(mbClient.searchArtistByName).toHaveBeenCalledWith("F.I.R.");
      expect(mbOnly.musicbrainzArtistId).toBe("a8251b7f-2ea9-4661-89c1-0950b5867034");
    });
  });

  describe("Discogs exact match", () => {
    it("finds artist with exact Chinese name on Discogs", async () => {
      const { DiscogsService } = await import("../../electron/services/DiscogsService");
      const dgService = {
        fetch: vi.fn().mockResolvedValue({
          json: () =>
            Promise.resolve({
              results: [{ title: "单依纯", id: 13111254 }],
            }),
        }),
      };
      vi.mocked(DiscogsService).mockImplementation(() => dgService as any);

      const result = await findArtistIdentity("单依纯", { skipMusicBrainz: true });

      expect(result.discogsArtistId).toBe("13111254");
      expect(result.source).toBe("discogs-exact");
      expect(result.englishAliases).toEqual([]);
    });

    it("rejects non-exact Discogs results for Chinese names", async () => {
      const { DiscogsService } = await import("../../electron/services/DiscogsService");
      const dgService = {
        fetch: vi.fn().mockResolvedValue({
          json: () =>
            Promise.resolve({
              results: [{ title: "郭静颖", id: 10725637 }], // Wrong artist
            }),
        }),
      };
      vi.mocked(DiscogsService).mockImplementation(() => dgService as any);

      const result = await findArtistIdentity("郭静", { skipMusicBrainz: true });

      expect(result.discogsArtistId).toBeNull();
      expect(result.source).toBe("none");
    });
  });

  describe("MusicBrainz alias lookup", () => {
    it("finds English alias and resolves Discogs ID", async () => {
      const { MusicBrainzClient } = await import("../../electron/handlers/musicbrainz");
      const mbClient = {
        searchArtistByName: vi.fn().mockResolvedValue({
          id: "eb7fb713-82d5-4417-9c80-65bc7da3233b",
          name: "郭靜",
          aliases: [
            { name: "Claire Kuo", locale: "en", type: "Artist name" },
            { name: "郭伯瑜", locale: "zh", type: "Birth name" },
          ],
        }),
      };
      vi.mocked(MusicBrainzClient).mockImplementation(() => mbClient as any);

      const { DiscogsService } = await import("../../electron/services/DiscogsService");
      let callCount = 0;
      const dgService = {
        fetch: vi.fn().mockImplementation(() => {
          callCount++;
          // First call: search for 郭静 (no exact match)
          // Second call: search for Claire Kuo (exact match)
          if (callCount === 1) {
            return Promise.resolve({
              json: () => Promise.resolve({ results: [] }),
            });
          }
          return Promise.resolve({
            json: () =>
              Promise.resolve({
                results: [{ title: "Claire Kuo", id: 6363042 }],
              }),
          });
        }),
      };
      vi.mocked(DiscogsService).mockImplementation(() => dgService as any);

      const result = await findArtistIdentity("郭静");

      expect(result.musicbrainzArtistId).toBe("eb7fb713-82d5-4417-9c80-65bc7da3233b");
      expect(result.discogsArtistId).toBe("6363042");
      expect(result.englishAliases).toContain("Claire Kuo");
      expect(result.source).toBe("musicbrainz");
    });

    it("returns MB ID even without Discogs match", async () => {
      const { MusicBrainzClient } = await import("../../electron/handlers/musicbrainz");
      const mbClient = {
        searchArtistByName: vi.fn().mockResolvedValue({
          id: "eb7fb713-82d5-4417-9c80-65bc7da3233b",
          name: "郭靜",
          aliases: [{ name: "Claire Kuo", locale: "en" }],
        }),
      };
      vi.mocked(MusicBrainzClient).mockImplementation(() => mbClient as any);

      const { DiscogsService } = await import("../../electron/services/DiscogsService");
      const dgService = {
        fetch: vi.fn().mockResolvedValue({
          json: () => Promise.resolve({ results: [] }),
        }),
      };
      vi.mocked(DiscogsService).mockImplementation(() => dgService as any);

      const result = await findArtistIdentity("郭静");

      expect(result.musicbrainzArtistId).toBe("eb7fb713-82d5-4417-9c80-65bc7da3233b");
      expect(result.discogsArtistId).toBeNull();
      expect(result.source).toBe("musicbrainz");
    });
  });

  describe("convenience functions", () => {
    it("getEnglishAliases returns aliases only", async () => {
      const { MusicBrainzClient } = await import("../../electron/handlers/musicbrainz");
      const mbClient = {
        searchArtistByName: vi.fn().mockResolvedValue({
          id: "eb7fb713-82d5-4417-9c80-65bc7da3233b",
          name: "郭靜",
          aliases: [{ name: "Claire Kuo", locale: "en" }],
        }),
      };
      vi.mocked(MusicBrainzClient).mockImplementation(() => mbClient as any);

      const { DiscogsService } = await import("../../electron/services/DiscogsService");
      const dgService = {
        fetch: vi.fn().mockResolvedValue({
          json: () => Promise.resolve({ results: [] }),
        }),
      };
      vi.mocked(DiscogsService).mockImplementation(() => dgService as any);

      const aliases = await getEnglishAliases("郭静");
      expect(aliases).toContain("Claire Kuo");
    });

    it("getDiscogsArtistId returns ID only", async () => {
      const { DiscogsService } = await import("../../electron/services/DiscogsService");
      const dgService = {
        fetch: vi.fn().mockResolvedValue({
          json: () =>
            Promise.resolve({
              results: [{ title: "单依纯", id: 13111254 }],
            }),
        }),
      };
      vi.mocked(DiscogsService).mockImplementation(() => dgService as any);

      const id = await getDiscogsArtistId("单依纯");
      expect(id).toBe("13111254");
    });
  });
});
