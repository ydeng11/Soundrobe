import { describe, it, expect, vi, afterAll } from "vitest";
import { MusicBrainzClient } from "../../electron/handlers/musicbrainz";
import { makeTrackCandidate, type AlbumCandidate } from "../../electron/handlers/candidates";
import type { ReleaseMeta } from "../../electron/handlers/cache";

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
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
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

  it("looks up an artist name by MusicBrainz artist ID", async () => {
    globalThis.fetch = vi.fn().mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        id: "mb-artist-1",
        name: "黄绮珊",
        aliases: [{ name: "Susan Huang", locale: "en" }],
      }),
    });

    const client = new MusicBrainzClient();
    const artist = await client.lookupArtistById("mb-artist-1");

    expect(artist).toEqual({
      id: "mb-artist-1",
      name: "黄绮珊",
      aliases: [{ name: "Susan Huang", locale: "en" }],
    });
    expect(String(vi.mocked(globalThis.fetch).mock.calls[0][0])).toContain("/artist/mb-artist-1");
  });

  it("handles API error gracefully", async () => {
    globalThis.fetch = vi.fn().mockRejectedValue(new Error("Network error"));

    const client = new MusicBrainzClient();
    await expect(
      client.searchAlbum("The Beatles", "Abbey Road"),
    ).rejects.toThrow("Network error");
  });

  it("browses releases by MusicBrainz artist ID and fetches only the matched release detail", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?artist=mb-artist-1")) {
        return mockReleaseResponse([
          {
            id: "mb-wrong",
            title: "A Different Album",
            date: "2001-01-01",
            "artist-credit": [{ name: "Artist", artist: { id: "mb-artist-1" } }],
          },
          {
            id: "mb-good",
            title: "到底有誰能夠告訴我",
            date: "1991-01-01",
            "artist-credit": [{ name: "Artist", artist: { id: "mb-artist-1" } }],
          },
        ]);
      }
      if (url.includes("/release/mb-good")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-good",
            title: "到底有誰能夠告訴我",
            date: "1991-01-01",
            "artist-credit": [{ name: "Artist", artist: { id: "mb-artist-1" } }],
            media: [],
          }),
        };
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    const candidate = await client.lookupArtistReleaseByAlbum(
      "mb-artist-1",
      "到底有谁能够告诉我",
    );

    expect(candidate).not.toBeNull();
    expect(candidate!.musicbrainzAlbumId).toBe("mb-good");
    const urls = vi.mocked(globalThis.fetch).mock.calls.map((call) => String(call[0]));
    expect(urls.some((url) => url.includes("/release/mb-wrong"))).toBe(false);
  });

  it("keeps MusicBrainz recording titles as match-only alternatives", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?artist=mb-artist-1")) {
        return mockReleaseResponse([
          {
            id: "mb-need-u-most",
            title: "Need U Most（最需要妳）",
            date: "2007-10-05",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
          },
        ]);
      }
      if (url.includes("/release/mb-need-u-most")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-need-u-most",
            title: "Need U Most（最需要妳）",
            date: "2007-10-05",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
            media: [{
              position: 1,
              tracks: [{
                number: "12",
                title: "站在世界之巔",
                recording: {
                  id: "df2eeddb-4c12-432a-a3b3-c3b170222a15",
                  title: "Top of the World（我站上全世界的屋頂）",
                  length: 232693,
                },
              }],
            }],
          }),
        };
      }
      return { ok: false };
    });

    const candidate = await new MusicBrainzClient().lookupArtistReleaseByAlbum(
      "mb-artist-1",
      "Need U Most 最需要你 K歌情人",
    );

    expect(candidate?.musicbrainzAlbumId).toBe("mb-need-u-most");
    expect(candidate?.tracks[0].title).toBe("站在世界之巔");
    expect(candidate?.tracks[0].matchTitles).toEqual([
      "Top of the World（我站上全世界的屋頂）",
    ]);
  });

  it("uses cached MusicBrainz artist release pages before fetching", async () => {
    const cachedReleases: ReleaseMeta[] = [
      {
        id: "mb-good",
        title: "Abbey Road",
        year: 1969,
        type: "release",
        artistName: "The Beatles",
      },
    ];
    const releaseCache = {
      getArtistReleaseList: vi.fn().mockReturnValue(cachedReleases),
      setArtistReleaseList: vi.fn(),
      getReleaseDetail: vi.fn().mockReturnValue(null),
      setReleaseDetail: vi.fn(),
    };
    globalThis.fetch = vi.fn().mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        id: "mb-good",
        title: "Abbey Road",
        date: "1969-09-26",
        "artist-credit": [{ name: "The Beatles", artist: { id: "mb-artist-1" } }],
        media: [],
      }),
    });

    const client = new MusicBrainzClient({ releaseCache: releaseCache as never });
    const candidate = await client.lookupArtistReleaseByAlbum("mb-artist-1", "Abbey Road");

    expect(candidate).not.toBeNull();
    expect(vi.mocked(globalThis.fetch)).toHaveBeenCalledTimes(1);
    expect(String(vi.mocked(globalThis.fetch).mock.calls[0][0])).toContain("/release/mb-good");
    expect(releaseCache.getReleaseDetail).toHaveBeenCalledWith("musicbrainz-v3", "mb-good");
    expect(releaseCache.setReleaseDetail).toHaveBeenCalledWith(
      "musicbrainz-v3",
      "mb-good",
      expect.any(Object),
    );
  });

  it("uses track-title coverage to choose among shortlisted artist releases", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?artist=mb-artist-1")) {
        return mockReleaseResponse([
          {
            id: "mb-exact-wrong",
            title: "那些女孩教我的事 [FLAC]",
            date: "2008-06-01",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
          },
          {
            id: "mb-contained-good",
            title: "那些女孩教我的事",
            date: "2008-06-01",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
          },
        ]);
      }
      if (url.includes("/release/mb-exact-wrong")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-exact-wrong",
            title: "那些女孩教我的事 [FLAC]",
            date: "2008-06-01",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
            media: [
              {
                position: 1,
                tracks: [
                  { number: "1", title: "Wrong A", recording: { id: "wrong-a", title: "Wrong A" } },
                  { number: "2", title: "Wrong B", recording: { id: "wrong-b", title: "Wrong B" } },
                ],
              },
            ],
          }),
        };
      }
      if (url.includes("/release/mb-contained-good")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-contained-good",
            title: "那些女孩教我的事",
            date: "2008-06-01",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
            media: [
              {
                position: 1,
                tracks: [
                  { number: "1", title: "小白很乖", recording: { id: "good-a", title: "小白很乖" } },
                  { number: "2", title: "漂亮", recording: { id: "good-b", title: "漂亮" } },
                ],
              },
            ],
          }),
        };
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    const candidate = await client.lookupArtistReleaseByAlbum(
      "mb-artist-1",
      "那些女孩教我的事 [FLAC]",
      {
        localTracks: [
          makeTrackCandidate({ title: "小白很乖", trackNumber: 1 }),
          makeTrackCandidate({ title: "漂亮", trackNumber: 2 }),
        ],
      },
    );

    expect(candidate?.musicbrainzAlbumId).toBe("mb-contained-good");
    const urls = vi.mocked(globalThis.fetch).mock.calls.map((call) => String(call[0]));
    expect(urls.some((url) => url.includes("/release/mb-exact-wrong"))).toBe(true);
    expect(urls.some((url) => url.includes("/release/mb-contained-good"))).toBe(true);
  });

  it("uses alternate LLM-cleaned track titles when scoring shortlisted releases", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?artist=mb-artist-1")) {
        return mockReleaseResponse([
          { id: "mb-wrong", title: "那些女孩教我的事 [FLAC]", date: "2008-06-01" },
          { id: "mb-good", title: "那些女孩教我的事", date: "2008-06-01" },
        ]);
      }
      const good = url.includes("/release/mb-good");
      return {
        ok: true,
        json: async () => ({
          id: good ? "mb-good" : "mb-wrong",
          title: good ? "那些女孩教我的事" : "那些女孩教我的事 [FLAC]",
          date: "2008-06-01",
          media: [{
            position: 1,
            tracks: [{
              number: "1",
              title: good ? "小白很乖" : "Wrong",
              recording: { id: good ? "good" : "wrong", title: good ? "小白很乖" : "Wrong" },
            }],
          }],
        }),
      };
    });

    const candidate = await new MusicBrainzClient().lookupArtistReleaseByAlbum(
      "mb-artist-1",
      "那些女孩教我的事 [FLAC]",
      {
        localTracks: [makeTrackCandidate({ title: "Unknown 01", trackNumber: 1 })],
        alternateTrackTitles: ["小白很乖"],
      },
    );

    expect(candidate?.musicbrainzAlbumId).toBe("mb-good");
  });

  it("does not fetch weak album-name matches for artist-scoped shortlist", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?artist=mb-artist-1")) {
        return mockReleaseResponse([
          {
            id: "mb-weak",
            title: "Unrelated Album",
            date: "2008-06-01",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
          },
          {
            id: "mb-good",
            title: "那些女孩教我的事",
            date: "2008-06-01",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
          },
        ]);
      }
      if (url.includes("/release/mb-good")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-good",
            title: "那些女孩教我的事",
            date: "2008-06-01",
            "artist-credit": [{ name: "品冠", artist: { id: "mb-artist-1" } }],
            media: [],
          }),
        };
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    const candidate = await client.lookupArtistReleaseByAlbum(
      "mb-artist-1",
      "那些女孩教我的事",
    );

    expect(candidate?.musicbrainzAlbumId).toBe("mb-good");
    const urls = vi.mocked(globalThis.fetch).mock.calls.map((call) => String(call[0]));
    expect(urls.some((url) => url.includes("/release/mb-weak"))).toBe(false);
  });

  it("coalesces concurrent artist release page and release detail requests", async () => {
    const inFlightReleasePages = new Map<string, Promise<ReleaseMeta[]>>();
    const inFlightReleaseDetails = new Map<string, Promise<AlbumCandidate | null>>();
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?")) {
        await new Promise((resolve) => setTimeout(resolve, 10));
        return mockReleaseResponse([
          {
            id: "mb-good",
            title: "看我72变",
            date: "2003-03-07",
            "artist-credit": [{ name: "蔡依林", artist: { id: "mb-artist-1" } }],
          },
        ]);
      }
      if (url.includes("/release/mb-good")) {
        await new Promise((resolve) => setTimeout(resolve, 10));
        return {
          ok: true,
          json: async () => ({
            id: "mb-good",
            title: "看我72变",
            date: "2003-03-07",
            "artist-credit": [{ name: "蔡依林", artist: { id: "mb-artist-1" } }],
            media: [],
          }),
        };
      }
      return { ok: false };
    });

    const client1 = new MusicBrainzClient({
      inFlightReleasePages,
      inFlightReleaseDetails,
    });
    const client2 = new MusicBrainzClient({
      inFlightReleasePages,
      inFlightReleaseDetails,
    });

    const [first, second] = await Promise.all([
      client1.lookupArtistReleaseByAlbum("mb-artist-1", "看我72变"),
      client2.lookupArtistReleaseByAlbum("mb-artist-1", "看我72变"),
    ]);

    expect(first?.musicbrainzAlbumId).toBe("mb-good");
    expect(second?.musicbrainzAlbumId).toBe("mb-good");
    expect(vi.mocked(globalThis.fetch)).toHaveBeenCalledTimes(2);
    expect(inFlightReleasePages.size).toBe(0);
    expect(inFlightReleaseDetails.size).toBe(0);
  });

  it("does not keep failed in-flight artist release page requests", async () => {
    const inFlightReleasePages = new Map<string, Promise<ReleaseMeta[]>>();
    globalThis.fetch = vi.fn()
      .mockRejectedValueOnce(new Error("temporary outage"))
      .mockResolvedValueOnce(mockReleaseResponse([]));

    const client = new MusicBrainzClient({ inFlightReleasePages });

    await expect(client.lookupArtistReleaseByAlbum("mb-artist-1", "Album"))
      .resolves.toBeNull();
    await expect(client.lookupArtistReleaseByAlbum("mb-artist-1", "Album"))
      .resolves.toBeNull();
    expect(vi.mocked(globalThis.fetch)).toHaveBeenCalledTimes(2);
  });

  it("uses per-track artist-credit when present (featured artist)", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?")) {
        return mockReleaseResponse([
          {
            id: "mb-album",
            title: "100天",
            date: "2009-12-18",
            "artist-credit": [
              { name: "林俊傑", artist: { id: "mb-artist-1" } },
            ],
          },
        ]);
      }
      if (url.includes("/release/mb-album")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-album",
            title: "100天",
            date: "2009-12-18",
            "artist-credit": [
              { name: "林俊傑", artist: { id: "mb-artist-1" } },
            ],
            media: [
              {
                position: 1,
                tracks: [
                  {
                    number: "3",
                    title: "加油!",
                    "artist-credit": [
                      { name: "林俊傑", joinphrase: " feat. ", artist: { id: "mb-artist-1" } },
                      { name: "MC HotDog", joinphrase: "", artist: { id: "mb-artist-2" } },
                    ],
                    recording: {
                      id: "rec-1",
                      title: "加油!",
                      length: 227000,
                    },
                  },
                ],
              },
            ],
          }),
        };
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    const results = await client.searchAlbum("林俊傑", "100天");

    expect(results).toHaveLength(1);
    const tracks = results[0].tracks;
    expect(tracks).toHaveLength(1);
    expect(tracks[0].trackNumber).toBe(3);
    expect(tracks[0].artist).toBe("林俊傑 feat. MC HotDog");
    expect(tracks[0].artists).toEqual(["林俊傑", "MC HotDog"]);
    // Album-level artist stays as release-level
    expect(results[0].artist).toBe("林俊傑");
  });

  it("falls back to recording artist-credit when track has none", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?")) {
        return mockReleaseResponse([
          {
            id: "mb-album",
            title: "Test Album",
            date: "2020-01-01",
            "artist-credit": [
              { name: "Main Artist", artist: { id: "mb-artist-1" } },
            ],
          },
        ]);
      }
      if (url.includes("/release/mb-album")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-album",
            title: "Test Album",
            date: "2020-01-01",
            "artist-credit": [
              { name: "Main Artist", artist: { id: "mb-artist-1" } },
            ],
            media: [
              {
                position: 1,
                tracks: [
                  {
                    number: 1,
                    title: "Track With Recording Credit",
                    // No track-level artist-credit
                    recording: {
                      id: "rec-1",
                      title: "Track With Recording Credit",
                      length: 180000,
                      "artist-credit": [
                        { name: "Main Artist", joinphrase: " feat. ", artist: { id: "mb-artist-1" } },
                        { name: "Guest Artist", joinphrase: "", artist: { id: "mb-artist-3" } },
                      ],
                    },
                  },
                ],
              },
            ],
          }),
        };
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    const results = await client.searchAlbum("Main Artist", "Test Album");

    expect(results).toHaveLength(1);
    const tracks = results[0].tracks;
    expect(tracks).toHaveLength(1);
    expect(tracks[0].artist).toBe("Main Artist feat. Guest Artist");
    expect(tracks[0].artists).toEqual(["Main Artist", "Guest Artist"]);
  });

  it("falls back to recording artist-credit when track credit is empty array", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?")) {
        return mockReleaseResponse([
          {
            id: "mb-album",
            title: "Test Album",
            date: "2020-01-01",
            "artist-credit": [
              { name: "Main Artist", artist: { id: "mb-artist-1" } },
            ],
          },
        ]);
      }
      if (url.includes("/release/mb-album")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-album",
            title: "Test Album",
            date: "2020-01-01",
            "artist-credit": [
              { name: "Main Artist", artist: { id: "mb-artist-1" } },
            ],
            media: [
              {
                position: 1,
                tracks: [
                  {
                    number: 1,
                    title: "Track With Empty Credit",
                    "artist-credit": [], // empty — should not be treated as authoritative
                    recording: {
                      id: "rec-1",
                      title: "Track With Empty Credit",
                      length: 180000,
                      "artist-credit": [
                        { name: "Main Artist", joinphrase: " feat. ", artist: { id: "mb-artist-1" } },
                        { name: "Guest", joinphrase: "", artist: { id: "mb-artist-3" } },
                      ],
                    },
                  },
                ],
              },
            ],
          }),
        };
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    const results = await client.searchAlbum("Main Artist", "Test Album");

    expect(results).toHaveLength(1);
    const tracks = results[0].tracks;
    expect(tracks).toHaveLength(1);
    // Should use recording-level credit, not the empty track credit
    expect(tracks[0].artist).toBe("Main Artist feat. Guest");
    expect(tracks[0].artists).toEqual(["Main Artist", "Guest"]);
  });

  it("falls back to release-level artist when neither track nor recording has credit", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?")) {
        return mockReleaseResponse([
          {
            id: "mb-album",
            title: "Simple Album",
            date: "2020-01-01",
            "artist-credit": [
              { name: "Solo Artist", artist: { id: "mb-artist-1" } },
            ],
          },
        ]);
      }
      if (url.includes("/release/mb-album")) {
        return {
          ok: true,
          json: async () => ({
            id: "mb-album",
            title: "Simple Album",
            date: "2020-01-01",
            "artist-credit": [
              { name: "Solo Artist", artist: { id: "mb-artist-1" } },
            ],
            media: [
              {
                position: 1,
                tracks: [
                  {
                    number: 1,
                    title: "Simple Track",
                    // No artist-credit anywhere
                    recording: {
                      id: "rec-1",
                      title: "Simple Track",
                      length: 200000,
                    },
                  },
                ],
              },
            ],
          }),
        };
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    const results = await client.searchAlbum("Solo Artist", "Simple Album");

    expect(results).toHaveLength(1);
    const tracks = results[0].tracks;
    expect(tracks).toHaveLength(1);
    expect(tracks[0].artist).toBe("Solo Artist");
    expect(tracks[0].artists).toEqual(["Solo Artist"]);
  });

  it("loadTracks requests artist-credits in inc param", async () => {
    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      if (url.includes("/release?")) {
        return mockReleaseResponse([
          {
            id: "mb-album",
            title: "Test",
            date: "2020-01-01",
            "artist-credit": [
              { name: "Artist", artist: { id: "mb-artist-1" } },
            ],
          },
        ]);
      }
      if (url.includes("/release/mb-album")) {
        return {
          ok: true,
          json: async () => ({
            media: [
              {
                position: 1,
                tracks: [
                  {
                    number: 1,
                    title: "Track",
                    recording: { id: "rec-1", title: "Track", length: 100000 },
                  },
                ],
              },
            ],
          }),
        };
      }
      return { ok: false };
    });

    const client = new MusicBrainzClient();
    await client.searchAlbum("Artist", "Test");

    // Find the release-detail URL (not the search URL)
    const urls = vi.mocked(globalThis.fetch).mock.calls.map((c) => String(c[0]));
    const detailUrl = urls.find((u) => u.includes("/release/mb-album"));
    expect(detailUrl).toBeDefined();
    expect(detailUrl).toContain("inc=recordings+artist-credits");
  });

  it("shares rate limiter across instances (app-wide 1 req/sec)", async () => {
    // Verify that two MusicBrainzClient instances share the same
    // module-level rate limiter by measuring the time between sequential
    // requests issued from different instances.
    let callCount = 0;

    globalThis.fetch = vi.fn().mockImplementation(async (url: string) => {
      callCount++;
      if (url.includes("/release?")) {
        // Add a small delay so the second instance's rate limiter
        // triggers its 1-second wait
        await new Promise((r) => setTimeout(r, 5));
        return mockReleaseResponse([]);
      }
      return { ok: false };
    });

    const client1 = new MusicBrainzClient();
    const client2 = new MusicBrainzClient();

    const t0 = Date.now();
    // Make concurrent requests from both instances
    await Promise.all([
      client1.searchAlbum("Test", "Album1"),
      client2.searchAlbum("Test", "Album2"),
    ]);
    const elapsed = Date.now() - t0;

    // Both requests should have been made
    expect(callCount).toBe(2);
    // With shared rate limiter, second request waits ~1000ms
    // If they used separate limiters, both would complete in ~5ms.
    expect(elapsed).toBeGreaterThanOrEqual(900);
  });
});
