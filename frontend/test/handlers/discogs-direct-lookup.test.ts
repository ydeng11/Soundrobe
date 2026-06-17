import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { DiscogsClient } from "../../electron/handlers/discogs";
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
  });

  it("lookupArtistReleaseByAlbum returns null when no album matches", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response(JSON.stringify({ releases: [] }), { status: 200 }),
    );
    const candidate = await client.lookupArtistReleaseByAlbum("1902728", "幻象波普星");
    expect(candidate).toBeNull();
  });
});
