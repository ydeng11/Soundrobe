import { describe, it, expect, vi, afterAll, beforeAll } from "vitest";
import { MusicBrainzClient } from "../../electron/handlers/musicbrainz";

describe("MusicBrainzClient", () => {
  const originalFetch = globalThis.fetch;

  afterAll(() => {
    globalThis.fetch = originalFetch;
  });

  const mockReleaseResponse = (releases: unknown[]) => ({
    ok: true,
    json: async () => ({
      releases,
      "release-count": releases.length,
    }),
  });

  const mockTrackResponse = (media: unknown[]) => ({
    ok: true,
    json: async () => ({ media }),
  });

  it("returns candidates for valid search", async () => {
    let callCount = 0;
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      callCount++;
      // First call: release search
      if (url.includes("/release?")) {
        return mockReleaseResponse([
          {
            id: "mb-1",
            title: "Abbey Road",
            date: "1969-09-26",
            status: "official",
            "artist-credit": [
              {
                name: "The Beatles",
                artist: { id: "mb-artist-1", name: "The Beatles" },
              },
            ],
          },
          {
            id: "mb-2",
            title: "Abbey Road (2019)",
            date: "2019-09-27",
            status: "official",
            "artist-credit": [
              {
                name: "The Beatles",
                artist: { id: "mb-artist-1", name: "The Beatles" },
              },
            ],
          },
        ]);
      }
      // Second/third call: track lookup
      if (url.includes("/release/")) {
        return mockTrackResponse([
          {
            position: 1,
            title: "Come Together",
            tracks: [
              {
                number: 1,
                title: "Come Together",
                recording: { id: "rec-1", title: "Come Together", length: 259000 },
              },
            ],
          },
          {
            position: 1,
            title: "Something",
            tracks: [
              {
                number: 2,
                title: "Something",
                recording: { id: "rec-2", title: "Something", length: 182000 },
              },
            ],
          },
        ]);
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    const results = await client.searchAlbum("The Beatles", "Abbey Road");

    expect(results).toHaveLength(2);
    expect(results[0].artist).toBe("The Beatles");
    expect(results[0].album).toBe("Abbey Road");
    expect(results[0].year).toBe("1969");
    expect(results[0].source).toBe("musicbrainz");
  });

  it("returns empty array for no results", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      mockReleaseResponse([]),
    );

    const client = new MusicBrainzClient();
    const results = await client.searchAlbum("Nonexistent", "Unknown");
    expect(results).toHaveLength(0);
  });

  it("returns empty for missing artist or album", async () => {
    const client = new MusicBrainzClient();
    expect(await client.searchAlbum(null, "Album")).toHaveLength(0);
    expect(await client.searchAlbum("Artist", null)).toHaveLength(0);
  });

  it("handles API error gracefully", async () => {
    globalThis.fetch = vi.fn().mockRejectedValue(new Error("Network error"));

    const client = new MusicBrainzClient();
    await expect(
      client.searchAlbum("The Beatles", "Abbey Road"),
    ).rejects.toThrow("Network error");
  });
});
