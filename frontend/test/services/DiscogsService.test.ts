import { describe, it, expect, vi, beforeEach } from "vitest";
import { DiscogsService } from "../../electron/services/DiscogsService";

const BASE = "https://api.discogs.com";

describe("DiscogsService", () => {
  let service: DiscogsService;

  beforeEach(() => {
    service = new DiscogsService({ token: "test-token" });
  });

  describe("searchArtists", () => {
    it("returns null when precise search succeeds (no alias needed)", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
        new Response(JSON.stringify({ results: [{ title: "Nirvana", id: 123 }] }), { status: 200 }),
      );
      const result = await service.searchArtists("Nirvana");
      expect(result).toBeNull();
    });

    it("returns alias + artistId when only generic search matches", async () => {
      const spy = vi.spyOn(globalThis, "fetch");
      spy.mockResolvedValueOnce(new Response(JSON.stringify({ results: [] }), { status: 200 }));
      spy.mockResolvedValueOnce(
        new Response(JSON.stringify({ results: [{ title: "Hedgehog (4)", id: 1902728 }] }), { status: 200 }),
      );
      const result = await service.searchArtists("刺猬");
      expect(result).toEqual({ title: "Hedgehog (4)", artistId: 1902728 });
    });

    it("returns null when neither search finds anything", async () => {
    const spy = vi.spyOn(globalThis, "fetch");
    spy.mockImplementationOnce(() => Promise.resolve(
      new Response(JSON.stringify({ results: [] }), { status: 200 }),
    ));
    spy.mockImplementationOnce(() => Promise.resolve(
      new Response(JSON.stringify({ results: [] }), { status: 200 }),
    ));
      const result = await service.searchArtists("NonExistent123");
      expect(result).toBeNull();
    });
  });

  describe("searchReleases", () => {
    it("finds releases by artist + album", async () => {
      const mockResults = [{ title: "Nirvana - Nevermind", id: 123 }];
      vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
        new Response(JSON.stringify({ results: mockResults }), { status: 200 }),
      );
      const results = await service.searchReleases("Nirvana", "Nevermind");
      expect(results).toHaveLength(1);
      expect(results[0].id).toBe(123);
    });

    it("returns empty array on HTTP error", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(new Response(null, { status: 403 }));
      const results = await service.searchReleases("Nirvana", "Nevermind");
      expect(results).toEqual([]);
    });
  });

  describe("getArtistDetail", () => {
    it("returns artist name, real name, and images", async () => {
      const mockDetail = {
        name: "Hedgehog (4)",
        realname: "刺猬",
        images: [{ type: "primary", uri: "https://example.com/img.jpg" }],
      };
      vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
        new Response(JSON.stringify(mockDetail), { status: 200 }),
      );
      const result = await service.getArtistDetail(1902728);
      expect(result).not.toBeNull();
      expect(result!.name).toBe("Hedgehog (4)");
      expect(result!.realname).toBe("刺猬");
      expect(result!.images).toHaveLength(1);
    });
  });

  describe("getReleaseDetail", () => {
    it("fetches release by ID", async () => {
      const mockRelease = { id: 6951078, title: "幻象波普星", artists: [{ name: "Hedgehog" }] };
      vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
        new Response(JSON.stringify(mockRelease), { status: 200 }),
      );
      const result = await service.getReleaseDetail(6951078);
      expect(result).not.toBeNull();
      expect(result!.title).toBe("幻象波普星");
    });
  });

  describe("getArtistReleaseByTitle", () => {
    it("matches release by album title", async () => {
      const spy = vi.spyOn(globalThis, "fetch");
      spy.mockResolvedValueOnce(
        new Response(JSON.stringify({
          releases: [{ id: 2387299, title: "Blue Day Dreaming" }, { id: 6951078, title: "幻象波普星" }],
        }), { status: 200 }),
      );
      spy.mockResolvedValueOnce(
        new Response(JSON.stringify({ id: 6951078, title: "幻象波普星" }), { status: 200 }),
      );
      const result = await service.getArtistReleaseByTitle(1902728, "幻象波普星");
      expect(result).not.toBeNull();
      expect(result!.id).toBe(6951078);
    });
  });

  describe("fetchImage", () => {
    it("downloads image bytes", async () => {
      const imgBuffer = Buffer.from([0xff, 0xd8, 0xff]);
      vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
        new Response(imgBuffer, { status: 200, headers: { "content-type": "image/jpeg" } }),
      );
      const result = await service.fetchImage("https://example.com/img.jpg");
      expect(result).not.toBeNull();
      expect(result!.bytes).toEqual(imgBuffer);
      expect(result!.mime).toBe("image/jpeg");
    });
  });
});
