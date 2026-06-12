import { describe, it, expect, vi, beforeEach } from "vitest";
import { resolveDiscogsArtistAlias, suggestDiscogsAliases } from "../../electron/handlers/audit";

const DISCOGS_BASE = "https://api.discogs.com";

describe("resolveDiscogsArtistAlias", () => {
  it("returns null when artist resolves directly on Discogs (no alias needed)", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      new Response(JSON.stringify({
        results: [{ title: "Nirvana", id: 123 }],
      }), { status: 200 }),
    );

    const result = await resolveDiscogsArtistAlias("Nirvana", "test-token");
    expect(result).toBeNull(); // resolves directly, no alias needed
  });

  it("returns Discogs title alias when Chinese name finds nothing but generic q search finds it", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    // First: precise artist search returns nothing
    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify({ results: [] }), { status: 200 }),
    );
    // Second: generic q search finds the artist with English title
    fetchSpy.mockResolvedValueOnce(
      new Response(JSON.stringify({
        results: [{ title: "Hedgehog (4)", id: 1902728 }],
      }), { status: 200 }),
    );

    const result = await resolveDiscogsArtistAlias("刺猬", "test-token");
    expect(result).toBe("Hedgehog (4)");
  });

  it("returns null when neither search finds anything", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ results: [] }), { status: 200 }),
    );

    const result = await resolveDiscogsArtistAlias("NonExistentArtist123456", "test-token");
    expect(result).toBeNull();
  });

  it("returns null on HTTP error", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(null, { status: 403 }),
    );

    const result = await resolveDiscogsArtistAlias("刺猬", "test-token");
    expect(result).toBeNull();
  });

  it("returns null when no token configured", async () => {
    const result = await resolveDiscogsArtistAlias("刺猬", undefined);
    expect(result).toBeNull();
  });
});

describe("suggestDiscogsAliases", () => {
  it("returns empty array for non-CJK artist names", async () => {
    const result = await suggestDiscogsAliases("Nirvana", { completeJson: vi.fn() } as any);
    expect(result).toEqual([]);
  });

  it("returns aliases from LLM for CJK artist names", async () => {
    const mockClient = {
      completeJson: vi.fn().mockResolvedValue({
        data: { aliases: ["Hedgehog", "Hedgehog (band)"] },
      }),
    };

    const result = await suggestDiscogsAliases("刺猬", mockClient as any);
    expect(result).toEqual(["Hedgehog", "Hedgehog (band)"]);
    expect(mockClient.completeJson).toHaveBeenCalled();
  });

  it("returns empty array when LLM returns no aliases", async () => {
    const mockClient = {
      completeJson: vi.fn().mockResolvedValue({
        data: { aliases: [] },
      }),
    };

    const result = await suggestDiscogsAliases("刺猬", mockClient as any);
    expect(result).toEqual([]);
  });

  it("filters out invalid alias suggestions", async () => {
    const mockClient = {
      completeJson: vi.fn().mockResolvedValue({
        data: { aliases: ["", " ", null, "Hedgehog"] },
      }),
    };

    const result = await suggestDiscogsAliases("刺猬", mockClient as any);
    expect(result).toEqual(["Hedgehog"]);
  });
});
