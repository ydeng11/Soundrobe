import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { DiscogsClient } from "../../electron/handlers/discogs";
import { makeTrackCandidate, type AlbumCandidate } from "../../electron/handlers/candidates";
import type { ReleaseMeta } from "../../electron/handlers/cache";

const BASE = "https://api.discogs.com";

describe("DiscogsClient — direct ID lookup", () => {
  let client: DiscogsClient;

  beforeEach(() => {
    client = new DiscogsClient({ token: "test-token" });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("lookupReleaseById calls /releases/{id}", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");

    const mockReleaseResponse = {
      id: 6951078,
      title: "幻象波普星",
      artists: [{ name: "Hedgehog (4)" }],
      year: 2013,
      genres: ["Rock"],
      styles: ["Indie Rock"],
      tracklist: [
        { position: "1", title: "Track 1" },
        { position: "2", title: "Track 2" },
      ],
    };

    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify(mockReleaseResponse), { status: 200 }),
    );

    const candidate = await client.lookupReleaseById("6951078");

    // Verify URL
    const callUrl = fetchSpy.mock.calls[0][0] as string;
    expect(callUrl).toBe(`${BASE}/releases/6951078`);

    // Verify candidate fields
    expect(candidate).not.toBeNull();
    expect(candidate!.album).toBe("幻象波普星");
    expect(candidate!.artist).toBe("Hedgehog");
    expect(candidate!.year).toBe("2013");
    expect(candidate!.source).toBe("discogs");
    expect(candidate!.discogsReleaseId).toBe("6951078");
    expect(candidate!.tracks).toHaveLength(2);
  });

  it("does not treat Discogs extra artist credits as track performers", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response(JSON.stringify({
        id: 28809994,
        title: "情义新歌+精选光耀全纪录",
        artists: [{ name: "任賢齊" }],
        tracklist: [{
          position: "8",
          title: "你是我老婆",
          extraartists: [
            { name: "涂惠源", role: "Composed By" },
            { name: "小虫", role: "Written-By" },
          ],
        }],
      }), { status: 200 }),
    );

    const candidate = await client.lookupReleaseById("28809994");

    expect(candidate?.tracks[0].artist).toBe("任賢齊");
    expect(candidate?.tracks[0].artists).toEqual(["任賢齊"]);
  });

  it("lookupReleaseById returns null on HTTP error", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response(null, { status: 404 }),
    );
    const candidate = await client.lookupReleaseById("999999");
    expect(candidate).toBeNull();
  });

  it("lookupArtistReleaseByAlbum calls /artists/{id}/releases then /releases/{id}", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");

    // Artist releases response
    const mockArtistReleases = {
      releases: [
        {
          id: 2387299,
          title: "Blue Day Dreaming",
          artist: "Hedgehog (4)",
          year: 2009,
        },
        {
          id: 6951078,
          title: "幻象波普星",
          artist: "Hedgehog (4)",
          year: 2013,
        },
      ],
    };

    // Release detail response
    const mockReleaseDetail = {
      id: 6951078,
      title: "幻象波普星",
      artists: [{ name: "Hedgehog (4)" }],
      year: 2013,
      genres: ["Rock"],
      styles: ["Indie Rock"],
      tracklist: [
        { position: "1", title: "Track 1" },
        { position: "2", title: "Track 2" },
      ],
    };

    fetchSpy
      .mockResolvedValueOnce(new Response(JSON.stringify(mockArtistReleases), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify(mockReleaseDetail), { status: 200 }));

    const candidate = await client.lookupArtistReleaseByAlbum("1902728", "幻象波普星");

    // Verify first call: /artists/1902728/releases
    const firstUrl = fetchSpy.mock.calls[0][0] as string;
    expect(firstUrl).toContain(`${BASE}/artists/1902728/releases`);

    // Verify candidate
    expect(candidate).not.toBeNull();
    expect(candidate!.album).toBe("幻象波普星");
    expect(candidate!.discogsReleaseId).toBe("6951078");
  });

  it("lookupArtistReleaseByAlbum matches Traditional Chinese titles through shared normalization", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");

    fetchSpy
      .mockResolvedValueOnce(new Response(JSON.stringify({
        releases: [
          { id: 111, title: "到底有誰能夠告訴我", artist: "Aaron Kwok", year: 1991 },
        ],
      }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({
        id: 111,
        title: "到底有誰能夠告訴我",
        artists: [{ name: "Aaron Kwok" }],
        year: 1991,
        tracklist: [],
      }), { status: 200 }));

    const candidate = await client.lookupArtistReleaseByAlbum("123", "到底有谁能够告诉我");

    expect(candidate).not.toBeNull();
    expect(candidate!.discogsReleaseId).toBe("111");
  });

  it("uses cached artist release pages before fetching Discogs", async () => {
    const cachedReleases: ReleaseMeta[] = [
      {
        id: "6951078",
        title: "幻象波普星",
        year: 2013,
        type: "release",
        artistName: "Hedgehog",
      },
    ];
    const releaseCache = {
      getArtistReleaseList: vi.fn().mockReturnValue(cachedReleases),
      setArtistReleaseList: vi.fn(),
      getReleaseDetail: vi.fn().mockReturnValue(null),
      setReleaseDetail: vi.fn(),
    };
    client = new DiscogsClient({ token: "test-token", releaseCache: releaseCache as never });
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    fetchSpy.mockResolvedValueOnce(new Response(JSON.stringify({
      id: 6951078,
      title: "幻象波普星",
      artists: [{ name: "Hedgehog (4)" }],
      year: 2013,
      tracklist: [],
    }), { status: 200 }));

    const candidate = await client.lookupArtistReleaseByAlbum("1902728", "幻象波普星");

    expect(candidate).not.toBeNull();
    expect(fetchSpy).toHaveBeenCalledTimes(1);
    expect(String(fetchSpy.mock.calls[0][0])).toBe(`${BASE}/releases/6951078`);
    expect(releaseCache.getReleaseDetail).toHaveBeenCalledWith("discogs-v2", "6951078");
    expect(releaseCache.setReleaseDetail).toHaveBeenCalledWith(
      "discogs-v2",
      "6951078",
      expect.objectContaining({ discogsReleaseId: "6951078" }),
    );
  });

  it("uses track-title coverage to choose among shortlisted Discogs artist releases", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    fetchSpy.mockImplementation(async (url) => {
      const textUrl = String(url);
      if (textUrl.includes("/artists/1902728/releases")) {
        return new Response(JSON.stringify({
          releases: [
            { id: 1001, title: "那些女孩教我的事 [FLAC]", artist: "品冠", year: 2008 },
            { id: 1002, title: "那些女孩教我的事", artist: "品冠", year: 2008 },
          ],
        }), { status: 200 });
      }
      if (textUrl.includes("/releases/1001")) {
        return new Response(JSON.stringify({
          id: 1001,
          title: "那些女孩教我的事 [FLAC]",
          artists: [{ name: "品冠" }],
          year: 2008,
          tracklist: [
            { position: "1", title: "Wrong A" },
            { position: "2", title: "Wrong B" },
          ],
        }), { status: 200 });
      }
      if (textUrl.includes("/releases/1002")) {
        return new Response(JSON.stringify({
          id: 1002,
          title: "那些女孩教我的事",
          artists: [{ name: "品冠" }],
          year: 2008,
          tracklist: [
            { position: "1", title: "小白很乖" },
            { position: "2", title: "漂亮" },
          ],
        }), { status: 200 });
      }
      return new Response(null, { status: 404 });
    });

    const candidate = await client.lookupArtistReleaseByAlbum(
      "1902728",
      "那些女孩教我的事 [FLAC]",
      {
        localTracks: [
          makeTrackCandidate({ title: "小白很乖", trackNumber: 1 }),
          makeTrackCandidate({ title: "漂亮", trackNumber: 2 }),
        ],
      },
    );

    expect(candidate?.discogsReleaseId).toBe("1002");
    const urls = fetchSpy.mock.calls.map((call) => String(call[0]));
    expect(urls.some((url) => url.includes("/releases/1001"))).toBe(true);
    expect(urls.some((url) => url.includes("/releases/1002"))).toBe(true);
  });

  it("uses alternate LLM-cleaned titles when scoring Discogs releases", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    fetchSpy.mockImplementation(async (url) => {
      const textUrl = String(url);
      if (textUrl.includes("/artists/1902728/releases")) {
        return new Response(JSON.stringify({ releases: [
          { id: 1001, title: "那些女孩教我的事 [FLAC]", artist: "品冠", year: 2008 },
          { id: 1002, title: "那些女孩教我的事", artist: "品冠", year: 2008 },
        ] }), { status: 200 });
      }
      const good = textUrl.includes("/releases/1002");
      return new Response(JSON.stringify({
        id: good ? 1002 : 1001,
        title: good ? "那些女孩教我的事" : "那些女孩教我的事 [FLAC]",
        artists: [{ name: "品冠" }],
        year: 2008,
        tracklist: [{ position: "1", title: good ? "小白很乖" : "Wrong" }],
      }), { status: 200 });
    });

    const candidate = await client.lookupArtistReleaseByAlbum(
      "1902728",
      "那些女孩教我的事 [FLAC]",
      {
        localTracks: [makeTrackCandidate({ title: "Unknown 01", trackNumber: 1 })],
        alternateTrackTitles: ["小白很乖"],
      },
    );

    expect(candidate?.discogsReleaseId).toBe("1002");
  });

  it("deduplicates main release IDs before applying the shortlist cap", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    fetchSpy.mockImplementation(async (url) => {
      const textUrl = String(url);
      if (textUrl.includes("/artists/1902728/releases")) {
        return new Response(JSON.stringify({ releases: [
          { id: 2001, main_release: 1001, title: "那些女孩教我的事 [FLAC]", artist: "品冠", year: 2008 },
          { id: 2002, main_release: 1001, title: "那些女孩教我的事 [FLAC]", artist: "品冠", year: 2008 },
          { id: 2003, main_release: 1001, title: "那些女孩教我的事 [FLAC]", artist: "品冠", year: 2008 },
          { id: 1002, title: "那些女孩教我的事", artist: "品冠", year: 2008 },
        ] }), { status: 200 });
      }
      const good = textUrl.includes("/releases/1002");
      return new Response(JSON.stringify({
        id: good ? 1002 : 1001,
        title: good ? "那些女孩教我的事" : "那些女孩教我的事 [FLAC]",
        artists: [{ name: "品冠" }],
        year: 2008,
        tracklist: [{ position: "1", title: good ? "小白很乖" : "Wrong" }],
      }), { status: 200 });
    });

    const candidate = await client.lookupArtistReleaseByAlbum(
      "1902728",
      "那些女孩教我的事 [FLAC]",
      { localTracks: [makeTrackCandidate({ title: "小白很乖", trackNumber: 1 })] },
    );

    expect(candidate?.discogsReleaseId).toBe("1002");
    const detailUrls = fetchSpy.mock.calls
      .map((call) => String(call[0]))
      .filter((url) => url.includes("/releases/"));
    expect(detailUrls.filter((url) => url.includes("/releases/1001"))).toHaveLength(1);
    expect(detailUrls.some((url) => url.includes("/releases/1002"))).toBe(true);
  });

  it("does not fetch weak album-name matches for Discogs artist-scoped shortlist", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    fetchSpy.mockImplementation(async (url) => {
      const textUrl = String(url);
      if (textUrl.includes("/artists/1902728/releases")) {
        return new Response(JSON.stringify({
          releases: [
            { id: 1001, title: "Unrelated Album", artist: "品冠", year: 2008 },
            { id: 1002, title: "那些女孩教我的事", artist: "品冠", year: 2008 },
          ],
        }), { status: 200 });
      }
      if (textUrl.includes("/releases/1002")) {
        return new Response(JSON.stringify({
          id: 1002,
          title: "那些女孩教我的事",
          artists: [{ name: "品冠" }],
          year: 2008,
          tracklist: [],
        }), { status: 200 });
      }
      return new Response(null, { status: 404 });
    });

    const candidate = await client.lookupArtistReleaseByAlbum(
      "1902728",
      "那些女孩教我的事",
    );

    expect(candidate?.discogsReleaseId).toBe("1002");
    const urls = fetchSpy.mock.calls.map((call) => String(call[0]));
    expect(urls.some((url) => url.includes("/releases/1001"))).toBe(false);
  });

  it("coalesces concurrent artist release page and release detail requests", async () => {
    const inFlightReleasePages = new Map<string, Promise<ReleaseMeta[]>>();
    const inFlightReleaseDetails = new Map<string, Promise<AlbumCandidate | null>>();
    client = new DiscogsClient({
      token: "test-token",
      inFlightReleasePages,
      inFlightReleaseDetails,
    });
    const secondClient = new DiscogsClient({
      token: "test-token",
      inFlightReleasePages,
      inFlightReleaseDetails,
    });
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    fetchSpy.mockImplementation(async (url) => {
      const textUrl = String(url);
      if (textUrl.includes("/artists/1902728/releases")) {
        await new Promise((resolve) => setTimeout(resolve, 10));
        return new Response(JSON.stringify({
          releases: [
            { id: 6951078, title: "幻象波普星", artist: "Hedgehog", year: 2013 },
          ],
        }), { status: 200 });
      }
      if (textUrl.includes("/releases/6951078")) {
        await new Promise((resolve) => setTimeout(resolve, 10));
        return new Response(JSON.stringify({
          id: 6951078,
          title: "幻象波普星",
          artists: [{ name: "Hedgehog (4)" }],
          year: 2013,
          tracklist: [],
        }), { status: 200 });
      }
      return new Response(null, { status: 404 });
    });

    const [first, second] = await Promise.all([
      client.lookupArtistReleaseByAlbum("1902728", "幻象波普星"),
      secondClient.lookupArtistReleaseByAlbum("1902728", "幻象波普星"),
    ]);

    expect(first?.discogsReleaseId).toBe("6951078");
    expect(second?.discogsReleaseId).toBe("6951078");
    expect(fetchSpy).toHaveBeenCalledTimes(2);
    expect(inFlightReleasePages.size).toBe(0);
    expect(inFlightReleaseDetails.size).toBe(0);
  });

  it("does not keep failed in-flight artist release page requests", async () => {
    const inFlightReleasePages = new Map<string, Promise<ReleaseMeta[]>>();
    client = new DiscogsClient({
      token: "test-token",
      inFlightReleasePages,
    });
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    fetchSpy
      .mockRejectedValueOnce(new Error("temporary outage"))
      .mockResolvedValueOnce(new Response(JSON.stringify({ releases: [] }), { status: 200 }));

    await expect(client.lookupArtistReleaseByAlbum("1902728", "幻象波普星"))
      .resolves.toBeNull();
    await expect(client.lookupArtistReleaseByAlbum("1902728", "幻象波普星"))
      .resolves.toBeNull();
    expect(fetchSpy).toHaveBeenCalledTimes(2);
  });

  it("lookupArtistReleaseByAlbum returns null when no album matches", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response(JSON.stringify({ releases: [] }), { status: 200 }),
    );
    const candidate = await client.lookupArtistReleaseByAlbum("1902728", "幻象波普星");
    expect(candidate).toBeNull();
  });
});
