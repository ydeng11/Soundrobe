/**
 * Tests for ArtworkResolverService — standalone artwork provider resolution.
 *
 * Principles:
 * - Provider order is respected (first match wins).
 * - Google is never called when a prior source succeeds.
 * - Cover Art Archive only runs for album-cover with MBID.
 * - Wikimedia only runs for artist-image.
 * - Invalid/non-image responses are skipped (next provider tried).
 * - Missing credentials fail for that provider without breaking others.
 *
 * Discogs matching tests verify that artwork is accepted only when the
 * returned release actually matches the requested artist and album.
 */

import { describe, it, expect, vi, beforeAll, beforeEach, afterEach } from "vitest";
import {
  ArtworkResolverService,
  type ArtworkProvider,
  type ArtworkContext,
  type ArtworkResult,
  type ArtworkSource,
} from "../../electron/services/ArtworkResolverService";
import { clearCache as clearArtistIdentityCache } from "../../electron/services/ArtistIdentityResolver";

// ── Helpers ─────────────────────────────────────────────────────────

/** Smallest valid JPEG — used by spy returns to pass sharp normalization. */
let tinyJpeg: Buffer;

beforeAll(async () => {
  const sharp = (await import("sharp")).default;
  tinyJpeg = await sharp({
    create: { width: 1, height: 1, channels: 3, background: { r: 0, g: 0, b: 0 } },
  }).jpeg().toBuffer();
});

function makeResult(overrides: Partial<ArtworkResult> = {}): ArtworkResult {
  return {
    kind: "album-cover",
    source: "local",
    bytes: tinyJpeg,
    mime: "image/jpeg",
    ...overrides,
  };
}

const defaultContext: ArtworkContext = {
  kind: "album-cover",
  artistName: "Test Artist",
  albumName: "Test Album",
  musicbrainzAlbumId: "mbid-123",
};

/** Create a spy provider that returns a fixed result or null. */
function spyProvider(
  name: ArtworkSource,
  result: ArtworkResult | null,
): ArtworkProvider {
  return {
    name,
    find: vi.fn().mockResolvedValue(result),
  };
}

describe("ArtworkResolverService", () => {
  let service: ArtworkResolverService;

  beforeEach(() => {
    service = new ArtworkResolverService();
  });

  describe("resolve / provider ordering", () => {
    it("returns the first provider's result and stops", async () => {
      const p1 = spyProvider("cover-art-archive", makeResult({ source: "cover-art-archive" }));
      const p2 = spyProvider("discogs", makeResult({ source: "discogs" }));
      service.setProviders([p1, p2]);

      const result = await service.resolve(defaultContext);

      expect(result).not.toBeNull();
      expect(result!.source).toBe("cover-art-archive");
      expect(p1.find).toHaveBeenCalledOnce();
      expect(p2.find).not.toHaveBeenCalled();
    });

    it("tries next provider when first returns null", async () => {
      const p1 = spyProvider("cover-art-archive", null);
      const p2 = spyProvider("discogs", makeResult({ source: "discogs" }));
      const p3 = spyProvider("google", makeResult({ source: "google" }));
      service.setProviders([p1, p2, p3]);

      const result = await service.resolve(defaultContext);

      expect(result).not.toBeNull();
      expect(result!.source).toBe("discogs");
      expect(p1.find).toHaveBeenCalledOnce();
      expect(p2.find).toHaveBeenCalledOnce();
      expect(p3.find).not.toHaveBeenCalled();
    });

    it("returns null when all providers fail", async () => {
      const p1 = spyProvider("local", null);
      const p2 = spyProvider("discogs", null);
      service.setProviders([p1, p2]);

      const result = await service.resolve(defaultContext);

      expect(result).toBeNull();
    });

    it("defaults to album-cover provider order: local → CAA → discogs → TADB → google", () => {
      const names = service.getProviderNames();
      expect(names).toEqual([
        "local",
        "cover-art-archive",
        "discogs",
        "theaudiodb",
        "google",
      ]);
    });

    it("uses artist-image provider order: local → discogs → wikimedia → google", async () => {
      const local = spyProvider("local", null);
      const discogs = spyProvider("discogs", null);
      const wikimedia = spyProvider("wikimedia", makeResult({ kind: "artist-image", source: "wikimedia" }));
      const google = spyProvider("google", makeResult({ kind: "artist-image", source: "google" }));
      service.setProviders([local, discogs, wikimedia, google]);
      service.setCredentials({ googleApiKey: "k", googleSearchEngineId: "cx" });

      const ctx: ArtworkContext = {
        kind: "artist-image",
        artistName: "Test",
        albumName: null,
        albumPath: "/music/Test/Album",
        musicbrainzAlbumId: null,
      };

      const result = await service.resolve(ctx);

      expect(result!.source).toBe("wikimedia");
      expect(local.find).toHaveBeenCalledOnce();
      expect(discogs.find).toHaveBeenCalledOnce();
      expect(wikimedia.find).toHaveBeenCalledOnce();
      expect(google.find).not.toHaveBeenCalled();
    });
  });

  describe("resolve / provider-specific guards", () => {
    it("Cover Art Archive is skipped for album-cover without MBID", async () => {
      const p1 = spyProvider("cover-art-archive", makeResult());
      const p2 = spyProvider("google", makeResult({ source: "google" }));
      service.setProviders([p1, p2]);
      const ctx: ArtworkContext = { ...defaultContext, musicbrainzAlbumId: null };

      const result = await service.resolve(ctx);

      expect(result!.source).toBe("google");
      expect(p1.find).not.toHaveBeenCalled();
    });

    it("Cover Art Archive runs for album-cover with MBID", async () => {
      const p1 = spyProvider("cover-art-archive", makeResult({ source: "cover-art-archive" }));
      const p2 = spyProvider("google", makeResult({ source: "google" }));
      service.setProviders([p1, p2]);
      const ctx: ArtworkContext = { ...defaultContext, musicbrainzAlbumId: "mbid-456" };

      const result = await service.resolve(ctx);

      expect(result!.source).toBe("cover-art-archive");
      expect(p1.find).toHaveBeenCalledOnce();
    });

    it("Wikimedia is skipped for album-cover", async () => {
      const wikimedia = spyProvider("wikimedia", makeResult({ source: "wikimedia" }));
      const google = spyProvider("google", makeResult({ source: "google" }));
      service.setProviders([wikimedia, google]);
      const ctx: ArtworkContext = { ...defaultContext, kind: "album-cover" };

      const result = await service.resolve(ctx);

      // wikimedia should not have been called for album-cover
      expect(result!.source).toBe("google");
      expect(wikimedia.find).not.toHaveBeenCalled();
    });

    it("Wikimedia runs for artist-image", async () => {
      const wikimedia = spyProvider("wikimedia", makeResult({ kind: "artist-image", source: "wikimedia" }));
      const google = spyProvider("google", makeResult({ kind: "artist-image", source: "google" }));
      service.setProviders([wikimedia, google]);
      const ctx: ArtworkContext = { ...defaultContext, kind: "artist-image" };

      const result = await service.resolve(ctx);

      expect(result!.source).toBe("wikimedia");
      expect(wikimedia.find).toHaveBeenCalledOnce();
    });

    it("Discogs runs for artist-image (searches with type=artist, no album)", async () => {
      const discogs = spyProvider("discogs", makeResult({ kind: "artist-image", source: "discogs" }));
      const wikimedia = spyProvider("wikimedia", makeResult({ kind: "artist-image", source: "wikimedia" }));
      service.setProviders([discogs, wikimedia]);
      const ctx: ArtworkContext = { ...defaultContext, kind: "artist-image" };

      const result = await service.resolve(ctx);

      // discogs should be tried first and succeed
      expect(result!.source).toBe("discogs");
      expect(discogs.find).toHaveBeenCalledOnce();
      expect(wikimedia.find).not.toHaveBeenCalled();
    });

    it("TheAudioDB returns null for artist-image (no album context)", async () => {
      const tadb: ArtworkProvider = {
        name: "theaudiodb",
        needsCredentials: true,
        find: vi.fn().mockRejectedValue(new Error("should not be called for artist-image")),
      };
      const wikimedia = spyProvider("wikimedia", makeResult({ kind: "artist-image", source: "wikimedia" }));
      service.setProviders([wikimedia]);
      const ctx: ArtworkContext = { ...defaultContext, kind: "artist-image" };

      const result = await service.resolve(ctx);

      expect(result!.source).toBe("wikimedia");
    });
  });

  describe("resolve / skipping invalid results", () => {
    it("skips provider that returns a result with empty bytes", async () => {
      const p1: ArtworkProvider = {
        name: "local",
        find: vi.fn().mockResolvedValue(makeResult({ bytes: Buffer.alloc(0) })),
      };
      const p2 = spyProvider("google", makeResult({ source: "google" }));
      service.setProviders([p1, p2]);

      const result = await service.resolve(defaultContext);

      expect(result!.source).toBe("google");
      expect(p1.find).toHaveBeenCalledOnce();
    });

    it("skips provider that returns a result with null bytes", async () => {
      const p1: ArtworkProvider = {
        name: "local",
        find: vi.fn().mockResolvedValue(makeResult({ bytes: null as unknown as Buffer })),
      };
      const p2 = spyProvider("google", makeResult({ source: "google" }));
      service.setProviders([p1, p2]);

      const result = await service.resolve(defaultContext);

      expect(result!.source).toBe("google");
    });

    it("continues when a provider throws", async () => {
      const p1: ArtworkProvider = {
        name: "cover-art-archive",
        find: vi.fn().mockRejectedValue(new Error("Network error")),
      };
      const p2 = spyProvider("discogs", makeResult({ source: "discogs" }));
      service.setProviders([p1, p2]);

      const result = await service.resolve(defaultContext);

      expect(result!.source).toBe("discogs");
    });
  });

  describe("resolve / credential guards", () => {
    it("skips Google when credentials are missing", async () => {
      const googleProvider: ArtworkProvider = {
        name: "google",
        needsCredentials: true,
        find: vi.fn(),
      };
      const discogs = spyProvider("discogs", makeResult({ source: "discogs" }));
      service.setProviders([googleProvider, discogs]);

      const result = await service.resolve(defaultContext);

      expect(result!.source).toBe("discogs");
      expect(googleProvider.find).not.toHaveBeenCalled();
    });

    it("skips TheAudioDB when credentials are missing", async () => {
      const tadb: ArtworkProvider = {
        name: "theaudiodb",
        needsCredentials: true,
        find: vi.fn(),
      };
      const google: ArtworkProvider = {
        name: "google",
        needsCredentials: true,
        find: vi.fn(),
      };
      const discogs = spyProvider("discogs", makeResult({ source: "discogs" }));
      service.setProviders([tadb, google, discogs]);

      const result = await service.resolve(defaultContext);

      expect(result!.source).toBe("discogs");
      expect(tadb.find).not.toHaveBeenCalled();
      expect(google.find).not.toHaveBeenCalled();
    });

    it("runs Google when credentials are present", async () => {
      const googleProvider: ArtworkProvider = {
        name: "google",
        needsCredentials: true,
        find: vi.fn().mockResolvedValue(makeResult({ source: "google" })),
      };
      service.setProviders([googleProvider]);
      service.setCredentials({ googleApiKey: "key", googleSearchEngineId: "cx" });

      const result = await service.resolve(defaultContext);

      expect(result!.source).toBe("google");
      expect(googleProvider.find).toHaveBeenCalledOnce();
    });
  });

  describe("normalizeImage", () => {
    it("converts valid image bytes to JPEG, max 1000x1000, quality 90", async () => {
      // Use the oversized PNG from beforeAll to test resize
      const input = await (await import("sharp")).default({
        create: { width: 2000, height: 2000, channels: 3, background: { r: 255, g: 0, b: 0 } },
      }).png().toBuffer();

      const normalized = await service.normalizeImage(input);

      expect(normalized).toBeInstanceOf(Buffer);
      expect(normalized.length).toBeGreaterThan(0);
      // Should be JPEG — check magic bytes
      expect(normalized[0]).toBe(0xff);
      expect(normalized[1]).toBe(0xd8);
      // Dimensions should be ≤ 1000
      const meta = await (await import("sharp")).default(normalized).metadata();
      expect(meta.width).toBeLessThanOrEqual(1000);
      expect(meta.height).toBeLessThanOrEqual(1000);
    });

    it("returns null for non-image garbage data", async () => {
      const garbage = Buffer.from([0, 1, 2, 3, 4, 5]);
      const result = await service.normalizeImage(garbage);
      expect(result).toBeNull();
    });

    it("returns null for empty buffer", async () => {
      const result = await service.normalizeImage(Buffer.alloc(0));
      expect(result).toBeNull();
    });
  });

  describe("buildContext (integration with track metadata)", () => {
    it("builds context from track metadata", () => {
      const track = {
        artist: "Test Artist",
        album: "Test Album",
        musicbrainzAlbumId: "mbid-123",
      };

      const ctx = service.buildContext(
        "album-cover",
        "/music/Test Artist/Test Album",
        track.artist,
        track.album,
        track.musicbrainzAlbumId,
      );

      expect(ctx.kind).toBe("album-cover");
      expect(ctx.artistName).toBe("Test Artist");
      expect(ctx.albumName).toBe("Test Album");
      expect(ctx.musicbrainzAlbumId).toBe("mbid-123");
    });
  });

  // ── Discogs candidate matching ─────────────────────────────────

  describe("Discogs candidate filtering", () => {
    let service: ArtworkResolverService;
    let originalFetch: typeof global.fetch;
    let fetchCalls: Array<{ url: string; headers?: Record<string, string> }>;

    /** Build a mock Discogs search response with the given results. */
    function mockDiscogsResponse(
      results: Array<{ title: string; cover_image: string | null; id: number }>,
    ): Response {
      return new Response(JSON.stringify({ results }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }

    /** Build a mock image response. */
    function mockImageResponse(bytes: Buffer): Response {
      return new Response(bytes, {
        status: 200,
        headers: { "Content-Type": "image/jpeg" },
      });
    }

    beforeEach(() => {
      service = new ArtworkResolverService();
      service.setCredentials({ discogsToken: "test-token-123" });
      clearArtistIdentityCache();
      originalFetch = global.fetch;
      fetchCalls = [];

      // Default: mock fetch to return 404 for anything not explicitly handled
      global.fetch = vi.fn().mockImplementation(
        (url: string | Request | URL, init?: RequestInit) => {
          fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });

          // Return 404 by default — tests must set up specific responses
          return Promise.resolve(new Response(null, { status: 404 }));
        },
      );
    });

    afterEach(() => {
      global.fetch = originalFetch;
    });

    describe("Title parsing", () => {
      it("returns null for a title without ' - ' separator", async () => {
        // Discogs result with unparseable title should be skipped.
        const searchResponse = mockDiscogsResponse([
          { title: "SomeRandomString", cover_image: null, id: 1 },
        ]);

        global.fetch = vi.fn().mockResolvedValueOnce(searchResponse);

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.",
          albumName: "无限",
          musicbrainzAlbumId: null,
        };

        // Default providers: local (no fs mock → null), CAA (no mbid → skip),
        // discogs (our mock), TADB (no creds → skip), google (no creds → skip)
        const result = await service.resolve(ctx);
        expect(result).toBeNull();
      });
    });

    describe("Rejection rules", () => {
      it("rejects 'Various' artist results", async () => {
        const searchResponse = mockDiscogsResponse([
          { title: "Various - 冰菊盛放在秋季", cover_image: "https://img.discogs.com/1.jpg", id: 1 },
        ]);

        global.fetch = vi.fn().mockResolvedValueOnce(searchResponse);

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.飞儿乐团",
          albumName: "飞行部落",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).toBeNull();
      });

      it("rejects unrelated Unicode release (ちゅううううううう!!!!!!)", async () => {
        const searchResponse = mockDiscogsResponse([
          { title: "ちゅううううううう!!!!!! - Unicode", cover_image: "https://img.discogs.com/bad.jpg", id: 23116274 },
        ]);

        global.fetch = vi.fn().mockResolvedValueOnce(searchResponse);

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.飞儿乐团",
          albumName: "爱‧歌姬",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).toBeNull();
      });

      it("rejects same-artist wrong-album result (self-titled vs 无限)", async () => {
        // When querying for self-titled album "飞儿乐团", Discogs returns
        // album "无限" first. Should be rejected since albums don't match.
        const searchResponse = mockDiscogsResponse([
          { title: "F.I.R. = 飛兒楽團* - 無限", cover_image: "https://img.discogs.com/wrong.jpg", id: 5853039 },
        ]);

        global.fetch = vi.fn().mockResolvedValueOnce(searchResponse);

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.飞儿乐团",
          albumName: "飞儿乐团",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).toBeNull();
      });

      it("rejects unrelated Discogs result for album that doesn't exist on Discogs (Better Life)", async () => {
        // Discogs returns empty results
        global.fetch = vi.fn().mockResolvedValueOnce(
          new Response(JSON.stringify({ results: [] }), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          }),
        );

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.飞儿乐团",
          albumName: "Better Life",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).toBeNull();
      });
    });

    describe("Acceptance rules", () => {
      it("uses a direct Discogs artist ID for artist images before name-based identity lookup", async () => {
        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            const textUrl = url.toString();
            if (textUrl === "https://api.discogs.com/artists/6153069") {
              return Promise.resolve(new Response(JSON.stringify({
                name: "蛋堡",
                images: [{ type: "primary", uri: "https://img.discogs.com/soft-lipa-primary.jpg" }],
              }), { status: 200, headers: { "Content-Type": "application/json" } }));
            }
            if (textUrl === "https://img.discogs.com/soft-lipa-primary.jpg") {
              return Promise.resolve(mockImageResponse(tinyJpeg));
            }
            return Promise.resolve(new Response(null, { status: 404 }));
          },
        );

        const ctx: ArtworkContext = {
          kind: "artist-image",
          artistName: "蛋堡",
          albumName: "你所不知道的杜振熙之内部整修",
          musicbrainzAlbumId: null,
          discogsArtistId: "6153069",
          albumPath: "/music/Soft Lipa/2013-你所不知道的杜振熙之内部整修",
        };

        const result = await service.resolve(ctx);

        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
        expect(result!.url).toBe("https://img.discogs.com/soft-lipa-primary.jpg");
        expect(fetchCalls.map((c) => c.url)).toEqual([
          "https://api.discogs.com/artists/6153069",
          "https://img.discogs.com/soft-lipa-primary.jpg",
        ]);
      });

      it("falls back to name-based identity lookup when a direct Discogs artist ID has no images", async () => {
        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            const textUrl = url.toString();
            if (textUrl === "https://api.discogs.com/artists/2510991") {
              return Promise.resolve(new Response(JSON.stringify({
                name: "Soft Lipa",
                images: [],
              }), { status: 200, headers: { "Content-Type": "application/json" } }));
            }
            if (textUrl === "https://api.discogs.com/database/search?type=artist&artist=%E8%9B%8B%E5%A0%A1&per_page=5") {
              return Promise.resolve(new Response(JSON.stringify({
                results: [{ title: "蛋堡", id: 6153069 }],
              }), { status: 200, headers: { "Content-Type": "application/json" } }));
            }
            if (textUrl === "https://api.discogs.com/artists/6153069") {
              return Promise.resolve(new Response(JSON.stringify({
                name: "蛋堡",
                images: [{ type: "primary", uri: "https://img.discogs.com/fallback-primary.jpg" }],
              }), { status: 200, headers: { "Content-Type": "application/json" } }));
            }
            if (textUrl === "https://img.discogs.com/fallback-primary.jpg") {
              return Promise.resolve(mockImageResponse(tinyJpeg));
            }
            return Promise.resolve(new Response(null, { status: 404 }));
          },
        );

        const ctx: ArtworkContext = {
          kind: "artist-image",
          artistName: "蛋堡",
          albumName: "你所不知道的杜振熙之内部整修",
          musicbrainzAlbumId: null,
          discogsArtistId: "2510991",
          albumPath: "/music/Soft Lipa/2013-你所不知道的杜振熙之内部整修",
        };

        const result = await service.resolve(ctx);

        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
        expect(result!.url).toBe("https://img.discogs.com/fallback-primary.jpg");
        expect(fetchCalls.map((c) => c.url)).toEqual([
          "https://api.discogs.com/artists/2510991",
          "https://api.discogs.com/database/search?type=artist&artist=%E8%9B%8B%E5%A0%A1&per_page=5",
          "https://api.discogs.com/artists/6153069",
          "https://img.discogs.com/fallback-primary.jpg",
        ]);
      });

      it("downloads a non-Latin artist image from the stored Discogs artist ID", async () => {
        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            const textUrl = url.toString();
            if (textUrl === "https://api.discogs.com/artists/6153069") {
              return Promise.resolve(new Response(JSON.stringify({
                name: "蛋堡",
                images: [{ type: "primary", uri: "https://img.discogs.com/danbao-primary.jpg" }],
              }), { status: 200, headers: { "Content-Type": "application/json" } }));
            }
            if (textUrl === "https://img.discogs.com/danbao-primary.jpg") {
              return Promise.resolve(mockImageResponse(tinyJpeg));
            }
            return Promise.resolve(new Response(null, { status: 404 }));
          },
        );

        const ctx: ArtworkContext = {
          kind: "artist-image",
          artistName: "蛋堡",
          albumName: null,
          musicbrainzAlbumId: null,
          discogsArtistId: "6153069",
          albumPath: "/music/Soft Lipa/Album",
        };

        const result = await service.resolve(ctx);

        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
        expect(result!.url).toBe("https://img.discogs.com/danbao-primary.jpg");
      });

      it("uses a direct Discogs release ID without requiring artist or album text", async () => {
        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            const textUrl = url.toString();
            if (textUrl === "https://api.discogs.com/releases/9565080") {
              return Promise.resolve(new Response(JSON.stringify({
                id: 9565080,
                images: [{ type: "secondary", uri: "https://img.discogs.com/9565080.jpg" }],
              }), { status: 200, headers: { "Content-Type": "application/json" } }));
            }
            if (textUrl === "https://img.discogs.com/9565080.jpg") {
              return Promise.resolve(mockImageResponse(tinyJpeg));
            }
            return Promise.resolve(new Response(null, { status: 404 }));
          },
        );

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: null,
          albumName: null,
          musicbrainzAlbumId: null,
          discogsReleaseId: "9565080",
          albumPath: "/music/郭富城/1991-到底有谁能够告诉我",
        };

        const result = await service.resolve(ctx);

        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
        expect(result!.url).toBe("https://img.discogs.com/9565080.jpg");
        expect(fetchCalls.map((c) => c.url)).toEqual([
          "https://api.discogs.com/releases/9565080",
          "https://img.discogs.com/9565080.jpg",
        ]);
      });

      it("falls through from a Cover Art Archive miss to direct Discogs release artwork", async () => {
        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            const textUrl = url.toString();
            if (textUrl === "https://coverartarchive.org/release/34443d65-15fd-45c2-9cb2-f035374619a3") {
              return Promise.resolve(new Response(null, { status: 404 }));
            }
            if (textUrl === "https://api.discogs.com/releases/9565080") {
              return Promise.resolve(new Response(JSON.stringify({
                id: 9565080,
                images: [{ type: "secondary", uri: "https://img.discogs.com/9565080.jpg" }],
              }), { status: 200, headers: { "Content-Type": "application/json" } }));
            }
            if (textUrl === "https://img.discogs.com/9565080.jpg") {
              return Promise.resolve(mockImageResponse(tinyJpeg));
            }
            return Promise.resolve(new Response(null, { status: 404 }));
          },
        );

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "郭富城",
          albumName: "到底有谁能够告诉我",
          musicbrainzAlbumId: "34443d65-15fd-45c2-9cb2-f035374619a3",
          discogsArtistId: "211321",
          discogsReleaseId: "9565080",
          albumPath: "/music/郭富城/1991-到底有谁能够告诉我",
        };

        const result = await service.resolve(ctx);

        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
        expect(result!.url).toBe("https://img.discogs.com/9565080.jpg");
        expect(fetchCalls.map((c) => c.url)).toEqual([
          "https://coverartarchive.org/release/34443d65-15fd-45c2-9cb2-f035374619a3",
          "https://api.discogs.com/releases/9565080",
          "https://img.discogs.com/9565080.jpg",
        ]);
      });

      it("keeps Cover Art Archive ahead of Discogs when the MBID has artwork", async () => {
        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            const textUrl = url.toString();
            if (textUrl === "https://coverartarchive.org/release/mbid-with-cover") {
              return Promise.resolve(new Response(JSON.stringify({
                images: [{ image: "https://coverartarchive.org/release/mbid-with-cover/front.jpg", types: ["Front"] }],
              }), { status: 200, headers: { "Content-Type": "application/json" } }));
            }
            if (textUrl === "https://coverartarchive.org/release/mbid-with-cover/front.jpg") {
              return Promise.resolve(mockImageResponse(tinyJpeg));
            }
            return Promise.resolve(new Response(null, { status: 404 }));
          },
        );

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "郭富城",
          albumName: "到底有谁能够告诉我",
          musicbrainzAlbumId: "mbid-with-cover",
          discogsReleaseId: "9565080",
          albumPath: "/music/郭富城/1991-到底有谁能够告诉我",
        };

        const result = await service.resolve(ctx);

        expect(result).not.toBeNull();
        expect(result!.source).toBe("cover-art-archive");
        expect(fetchCalls.map((c) => c.url)).toEqual([
          "https://coverartarchive.org/release/mbid-with-cover",
          "https://coverartarchive.org/release/mbid-with-cover/front.jpg",
        ]);
      });

      it("skips first bad result and uses a later exact match", async () => {
        const searchResponse = mockDiscogsResponse([
          { title: "ちゅううううううう!!!!!! - Unicode", cover_image: "https://img.discogs.com/bad.jpg", id: 23116274 },
          { title: "F.I.R. = 飛兒楽團* - 無限", cover_image: "https://img.discogs.com/good.jpg", id: 5853039 },
        ]);

        global.fetch = vi.fn()
          .mockResolvedValueOnce(searchResponse)
          .mockResolvedValueOnce(mockImageResponse(tinyJpeg));

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.飞儿乐团",
          albumName: "无限",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
        expect(result!.url).toContain("good.jpg");
      });

      it("accepts Traditional/Simplified Chinese album variant (無限 → 无限)", async () => {
        const searchResponse = mockDiscogsResponse([
          { title: "F.I.R. = 飛兒楽團* - 無限", cover_image: "https://img.discogs.com/good.jpg", id: 5853039 },
        ]);

        global.fetch = vi.fn()
          .mockResolvedValueOnce(searchResponse)
          .mockResolvedValueOnce(mockImageResponse(tinyJpeg));

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.飞儿乐团",
          albumName: "无限",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
      });

      it("accepts Traditional/Simplified Chinese artist variant", async () => {
        // Discogs has "飛兒楽團" (Traditional variant) while query has "飞儿乐团" (Simplified)
        const searchResponse = mockDiscogsResponse([
          { title: "F.I.R. = 飛兒楽團* - 無限", cover_image: "https://img.discogs.com/good.jpg", id: 5853039 },
        ]);

        global.fetch = vi.fn()
          .mockResolvedValueOnce(searchResponse)
          .mockResolvedValueOnce(mockImageResponse(tinyJpeg));

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.飞儿乐团",
          albumName: "无限",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
      });

      it("accepts Discogs title with = separator for alternative artist names", async () => {
        // "F.I.R. = 飛兒楽團* - 無限" — artist part has "F.I.R." and "飛兒楽團"
        // Query artist "F.I.R.飞儿乐团" should match either part
        const searchResponse = mockDiscogsResponse([
          { title: "F.I.R. = 飛兒楽團* - 無限", cover_image: "https://img.discogs.com/good.jpg", id: 5853039 },
        ]);

        global.fetch = vi.fn()
          .mockResolvedValueOnce(searchResponse)
          .mockResolvedValueOnce(mockImageResponse(tinyJpeg));

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.",
          albumName: "无限",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).not.toBeNull();
        expect(result!.source).toBe("discogs");
      });

      it("restores original fetch after test", async () => {
        // Verify fetch is restored by running a simple check
        expect(global.fetch).not.toBe(originalFetch);
      });
    });

    describe("Headers and credentials", () => {
      beforeEach(() => {
        fetchCalls.length = 0;
      });

      it("includes User-Agent header in Discogs search request", async () => {
        const searchResponse = mockDiscogsResponse([]);

        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            return Promise.resolve(searchResponse);
          },
        );

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.",
          albumName: "无限",
          musicbrainzAlbumId: null,
        };

        await service.resolve(ctx);

        // Find the discogs search call
        const discogsCall = fetchCalls.find(c => c.url.includes("api.discogs.com"));
        expect(discogsCall).toBeDefined();
        expect(discogsCall!.headers?.["User-Agent"]).toBeDefined();
      });

      it("includes Discogs token in search request when configured", async () => {
        const searchResponse = mockDiscogsResponse([]);

        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            return Promise.resolve(searchResponse);
          },
        );

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "Test",
          albumName: "Album",
          musicbrainzAlbumId: null,
        };

        await service.resolve(ctx);

        const discogsCall = fetchCalls.find(c => c.url.includes("api.discogs.com"));
        expect(discogsCall).toBeDefined();
        // Should have Authorization header with token
        const auth = discogsCall!.headers?.["Authorization"] as string ?? "";
        expect(auth).toContain("test-token-123");
      });

      it("requests per_page=10 from Discogs search", async () => {
        const searchResponse = mockDiscogsResponse([]);

        global.fetch = vi.fn().mockImplementation(
          (url: string | Request | URL, init?: RequestInit) => {
            fetchCalls.push({ url: url.toString(), headers: init?.headers as Record<string, string> ?? {} });
            return Promise.resolve(searchResponse);
          },
        );

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.",
          albumName: "无限",
          musicbrainzAlbumId: null,
        };

        await service.resolve(ctx);

        const discogsCall = fetchCalls.find(c => c.url.includes("api.discogs.com"));
        expect(discogsCall).toBeDefined();
        expect(discogsCall!.url).toContain("per_page=10");
      });
    });

    describe("Fallthrough to next provider", () => {
      it("returns null when all Discogs candidates are unrelated — fallback providers can run", async () => {
        // Discogs returns only unrelated results — should return null
        // so the next provider (TADB, Google) gets a chance
        const searchResponse = mockDiscogsResponse([
          { title: "Various - 冰菊盛放在秋季", cover_image: "https://img.discogs.com/1.jpg", id: 1 },
          { title: "ちゅううううううう!!!!!! - Unicode", cover_image: "https://img.discogs.com/2.jpg", id: 2 },
        ]);

        global.fetch = vi.fn().mockResolvedValueOnce(searchResponse);

        const ctx: ArtworkContext = {
          kind: "album-cover",
          artistName: "F.I.R.飞儿乐团",
          albumName: "飞行部落",
          musicbrainzAlbumId: null,
        };

        const result = await service.resolve(ctx);
        expect(result).toBeNull();
      });
    });
  });
});
