import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { MatchCache } from "../../electron/handlers/cache";
import {
  makeAlbumCandidate,
  makeLookupRequest,
  makeTrackCandidate,
} from "../../electron/handlers/candidates";

let tmpDir: string;
let cache: MatchCache;

function canUseNativeCacheInShellNode(): boolean {
  const probeDir = mkdtempSync(join(tmpdir(), "cache-probe-"));
  try {
    const probe = new MatchCache(join(probeDir, "cache.db"));
    probe.close();
    return true;
  } catch {
    return false;
  } finally {
    rmSync(probeDir, { recursive: true, force: true });
  }
}

const describeMatchCache = canUseNativeCacheInShellNode() ? describe : describe.skip;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), "cache-test-"));
  cache = new MatchCache(join(tmpDir, "cache.db"));
});

afterEach(() => {
  cache?.close();
  if (tmpDir) {
    rmSync(tmpDir, { recursive: true, force: true });
  }
});

describeMatchCache("MatchCache — lookup cache", () => {
  it("starts empty", () => {
    const req = makeLookupRequest({ artistHint: "A", albumHint: "B" });
    expect(cache.get(req)).toBeNull();
  });

  it("stores and retrieves candidates", () => {
    const req = makeLookupRequest({ artistHint: "Beatles", albumHint: "Abbey Road" });
    const candidates = [
      makeAlbumCandidate({
        artist: "The Beatles",
        album: "Abbey Road",
        source: "musicbrainz",
        year: "1969",
      }),
    ];
    cache.set(req, candidates);

    const retrieved = cache.get(req);
    expect(retrieved).not.toBeNull();
    expect(retrieved).toHaveLength(1);
    expect(retrieved![0].artist).toBe("The Beatles");
    expect(retrieved![0].album).toBe("Abbey Road");
    expect(retrieved![0].source).toBe("musicbrainz");
    expect(retrieved![0].year).toBe("1969");
  });

  it("does not store empty candidates list", () => {
    const req = makeLookupRequest({ artistHint: "X", albumHint: "Y" });
    cache.set(req, []);
    expect(cache.get(req)).toBeNull();
  });

  it("returns different results for different requests", () => {
    const req1 = makeLookupRequest({ artistHint: "A", albumHint: "1" });
    const req2 = makeLookupRequest({ artistHint: "B", albumHint: "2" });
    cache.set(req1, [makeAlbumCandidate({ artist: "A", album: "1" })]);
    expect(cache.get(req2)).toBeNull();
  });

  it("persists across same cache instance", () => {
    const req = makeLookupRequest({ artistHint: "Persist", albumHint: "Test" });
    cache.set(req, [makeAlbumCandidate({ artist: "P", album: "T", year: "2000" })]);
    const retrieved = cache.get(req);
    expect(retrieved![0].year).toBe("2000");
  });
});

describeMatchCache("MatchCache — album state", () => {
  it("returns null for unknown album", () => {
    expect(cache.getAlbumState("/nonexistent")).toBeNull();
  });

  it("stores and retrieves album state", () => {
    cache.setAlbumState("/music/Artist/Album", "tagged_ok", 1);
    const state = cache.getAlbumState("/music/Artist/Album");
    expect(state).not.toBeNull();
    expect(state!.status).toBe("tagged_ok");
    expect(state!.discCount).toBe(1);
    expect(state!.error).toBeNull();
  });

  it("updates existing album state", () => {
    cache.setAlbumState("/music/A/B", "pending");
    cache.setAlbumState("/music/A/B", "error", 0, "Something went wrong");
    const state = cache.getAlbumState("/music/A/B");
    expect(state!.status).toBe("error");
    expect(state!.error).toBe("Something went wrong");
  });

  it("rejects invalid status", () => {
    expect(() => cache.setAlbumState("/x", "invalid_status")).toThrow(
      "Invalid album status",
    );
  });

  it("clears album state", () => {
    cache.setAlbumState("/music/A/B", "pending");
    cache.clearAlbumState("/music/A/B");
    expect(cache.getAlbumState("/music/A/B")).toBeNull();
  });
});

describeMatchCache("MatchCache — LLM extraction cache", () => {
  it("returns null for unknown folder", () => {
    expect(cache.getLlmExtraction("unknown-folder")).toBeNull();
  });

  it("stores and retrieves LLM extraction", () => {
    const extraction = {
      artist: "Tanya Chua",
      album: "Goodbye & Hello",
      year: "2007",
    };
    cache.setLlmExtraction("2007-Goodbye-&-Hello", extraction);
    const retrieved = cache.getLlmExtraction("2007-Goodbye-&-Hello");
    expect(retrieved).not.toBeNull();
    expect(retrieved!.artist).toBe("Tanya Chua");
    expect(retrieved!.album).toBe("Goodbye & Hello");
    expect(retrieved!.year).toBe("2007");
  });

  it("does not interfere with album state", () => {
    cache.setAlbumState("/music/Artist/Album", "pending");
    cache.setLlmExtraction("some-folder", { artist: "X", album: "Y" });
    const llm = cache.getLlmExtraction("some-folder");
    expect(llm).not.toBeNull();
    // Album state still intact
    const state = cache.getAlbumState("/music/Artist/Album");
    expect(state!.status).toBe("pending");
  });
});

describeMatchCache("MatchCache — edge cases", () => {
  it("handles concurrent lookups from different temp db", () => {
    const tmpDir2 = mkdtempSync(join(tmpdir(), "cache-test-2-"));
    const cache2 = new MatchCache(join(tmpDir2, "cache.db"));
    const req = makeLookupRequest({ artistHint: "Isolated", albumHint: "Test" });
    cache2.set(req, [makeAlbumCandidate({ artist: "I", album: "T" })]);
    expect(cache.get(req)).toBeNull(); // different DB
    expect(cache2.get(req)).toHaveLength(1);
    cache2.close();
    rmSync(tmpDir2, { recursive: true, force: true });
  });

  it("stores tracks in candidates", () => {
    const req = makeLookupRequest({ artistHint: "A", albumHint: "B" });
    const candidates = [
      makeAlbumCandidate({
        artist: "A",
        album: "B",
        tracks: [
          makeTrackCandidate({ title: "Track 1", trackNumber: 1 }),
          makeTrackCandidate({ title: "Track 2", trackNumber: 2 }),
        ],
      }),
    ];
    cache.set(req, candidates);
    const retrieved = cache.get(req);
    expect(retrieved![0].tracks).toHaveLength(2);
    expect(retrieved![0].tracks[0].title).toBe("Track 1");
  });
});
