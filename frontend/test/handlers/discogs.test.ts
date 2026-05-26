import { describe, it, expect, vi, afterAll } from "vitest";
import { DiscogsClient } from "../../electron/handlers/discogs";

describe("DiscogsClient", () => {
  const originalFetch = globalThis.fetch;

  afterAll(() => {
    globalThis.fetch = originalFetch;
  });

  it("returns candidates from search results", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/database/search")) {
        return {
          ok: true,
          json: async () => ({
            results: [
              {
                id: 123,
                title: "The Beatles - Abbey Road",
                year: 1969,
                genre: ["Rock", "Pop"],
                resource_url: "https://api.discogs.com/masters/123",
              },
            ],
          }),
        };
      }
      if (url.includes("/masters/") || url.includes("/releases/")) {
        return {
          ok: true,
          json: async () => ({
            tracklist: [
              { position: "1", title: "Come Together", duration: "4:19" },
              { position: "2", title: "Something", duration: "3:02" },
            ],
          }),
        };
      }
      return { ok: false };
    });

    const client = new DiscogsClient({ token: null });
    const results = await client.searchAlbum("The Beatles", "Abbey Road");

    expect(results.length).toBeGreaterThanOrEqual(1);
    expect(results[0].artist).toBe("The Beatles");
    expect(results[0].album).toBe("Abbey Road");
    expect(results[0].year).toBe("1969");
    expect(results[0].source).toBe("discogs");
  });

  it("returns tracks with correct durations", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/database/search")) {
        return {
          ok: true,
          json: async () => ({
            results: [
              {
                id: 1,
                title: "Artist - Album",
                year: 2000,
                resource_url: "https://api.discogs.com/masters/1",
              },
            ],
          }),
        };
      }
      return {
        ok: true,
        json: async () => ({
          tracklist: [
            { position: "1", title: "Track 1", duration: "4:30" },
            { position: "2", title: "Track 2", duration: "3:15" },
          ],
        }),
      };
    });

    const client = new DiscogsClient({ token: null });
    const results = await client.searchAlbum("Artist", "Album");

    expect(results[0].tracks).toHaveLength(2);
    expect(results[0].tracks[0].title).toBe("Track 1");
    expect(results[0].tracks[0].length).toBe(270); // 4:30 = 270s
    expect(results[0].tracks[1].title).toBe("Track 2");
    expect(results[0].tracks[1].length).toBe(195); // 3:15 = 195s
  });

  it("returns empty for no results", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ results: [] }),
    });

    const client = new DiscogsClient({ token: null });
    const results = await client.searchAlbum("Nonexistent", "Unknown");
    expect(results).toHaveLength(0);
  });

  it("handles API error gracefully", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 429,
    });

    const client = new DiscogsClient({ token: null });
    const results = await client.searchAlbum("Artist", "Album");
    expect(results).toHaveLength(0);
  });

  it("skips non-matching artists", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        results: [
          {
            id: 1,
            title: "Some Other Artist - Album Name",
            year: 2020,
          },
        ],
      }),
    });

    const client = new DiscogsClient({ token: null });
    const results = await client.searchAlbum("My Artist", "Album Name");
    expect(results).toHaveLength(0);
  });

  it("uses token when provided", async () => {
    let authHeader = "";
    globalThis.fetch = vi.fn().mockImplementation(async (url: string, opts?: any) => {
      authHeader = opts?.headers?.Authorization ?? "";
      return {
        ok: true,
        json: async () => ({ results: [] }),
      };
    });

    const client = new DiscogsClient({ token: "my-token" });
    await client.searchAlbum("Test", "Album");
    expect(authHeader).toContain("my-token");
  });
});
