import { afterEach, describe, expect, it, vi } from "vitest";
import { SafeApiRequestService } from "../../electron/services/SafeApiRequestService";

describe("SafeApiRequestService", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("rejects lyrics hosts outside the known safe API allowlist", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const service = new SafeApiRequestService();
    service.setLyricsHost("https://example.com");

    const result = await service.execute({
      preset: "lyricsSearch",
      params: { artist: "Artist", title: "Song" },
    });

    expect(result.ok).toBe(false);
    expect(result.error).toBe("UNSUPPORTED_PRESET");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("allows lyrics requests to a known lyrics API host", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ lyrics: "line one\nline two" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const service = new SafeApiRequestService();
    service.setLyricsHost("https://lrclib.net");

    const result = await service.execute({
      preset: "lyricsSearch",
      params: { artist: "Artist", title: "Song" },
    });

    expect(result.ok).toBe(true);
    expect(result.summary).toBe("Found lyrics (2 lines)");
    expect(fetchMock).toHaveBeenCalledOnce();
  });
});
