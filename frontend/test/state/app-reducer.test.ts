import { describe, it, expect } from "vitest";
import { appReducer, initialAppState } from "../../src/state/AppState";
import type { TrackData } from "../../electron/preload";
import { UndoManager } from "../../src/state/UndoManager";

function makeTrack(path: string, overrides?: Partial<TrackData>): TrackData {
  return {
    path,
    title: null,
    artist: null,
    artists: [],
    album: null,
    albumArtist: null,
    albumArtists: [],
    trackNumber: null,
    trackTotal: null,
    discNumber: null,
    discTotal: null,
    year: null,
    genre: null,
    composer: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover: false,
    sizeBytes: 0,
    bitrate: null,
    sampleRate: null,
    codec: "mp3",
    duration: 0,
    ...overrides,
  };
}

describe("appReducer", () => {
  describe("SET_DIRECTORY", () => {
    it("sets currentDir and clears selection", () => {
      const state = { ...initialAppState, selectedTrackPath: "/old/track.mp3" };
      const next = appReducer(state, {
        type: "SET_DIRECTORY",
        path: "/music",
        name: "music",
      });
      expect(next.currentDir).toBe("/music");
      expect(next.currentDirName).toBe("music");
      expect(next.selectedTrackPath).toBeNull();
      expect(next.selectedTrack).toBeNull();
      expect(next.coverDataUrl).toBeNull();
      expect(next.error).toBeNull();
    });
  });

  describe("SELECT_TRACK", () => {
    it("stores the selected track and path", () => {
      const track = makeTrack("/music/song.mp3", { title: "Song Title" });
      const next = appReducer(initialAppState, {
        type: "SELECT_TRACK",
        path: "/music/song.mp3",
        track,
      });
      expect(next.selectedTrackPath).toBe("/music/song.mp3");
      expect(next.selectedTrack?.title).toBe("Song Title");
    });
  });

  describe("CLEAR_SELECTION", () => {
    it("clears selection and cover", () => {
      const state = {
        ...initialAppState,
        selectedTrackPath: "/music/song.mp3",
        selectedTrack: makeTrack("/music/song.mp3"),
        coverDataUrl: "data:image/jpeg;base64,abc",
      };
      const next = appReducer(state, { type: "CLEAR_SELECTION" });
      expect(next.selectedTrackPath).toBeNull();
      expect(next.selectedTrack).toBeNull();
      expect(next.coverDataUrl).toBeNull();
    });
  });

  describe("UPDATE_TRACK", () => {
    it("replaces track data for the matching path", () => {
      const track1 = makeTrack("/music/s1.mp3", { title: "Song 1" });
      const track2 = makeTrack("/music/s2.mp3", { title: "Song 2" });
      const state = {
        ...initialAppState,
        tracks: [track1, track2],
        selectedTrack: track1,
        selectedTrackPath: "/music/s1.mp3",
      };

      const updated = makeTrack("/music/s1.mp3", { title: "Updated Title" });
      const next = appReducer(state, {
        type: "UPDATE_TRACK",
        path: "/music/s1.mp3",
        track: updated,
      });

      expect(next.tracks[0].title).toBe("Updated Title");
      // Other track unchanged
      expect(next.tracks[1].title).toBe("Song 2");
      // Selected track also updates
      expect(next.selectedTrack?.title).toBe("Updated Title");
    });

    it("marks the track path as dirty", () => {
      const track = makeTrack("/music/s1.mp3");
      const state = { ...initialAppState, tracks: [track] };
      const updated = makeTrack("/music/s1.mp3", { title: "New" });
      const next = appReducer(state, {
        type: "UPDATE_TRACK",
        path: "/music/s1.mp3",
        track: updated,
      });
      expect(next.dirtyTracks.has("/music/s1.mp3")).toBe(true);
    });

    it("does not update selectedTrack when a different track is updated", () => {
      const t1 = makeTrack("/music/s1.mp3");
      const t2 = makeTrack("/music/s2.mp3", { title: "Keep" });
      const state = {
        ...initialAppState,
        tracks: [t1, t2],
        selectedTrack: t2,
        selectedTrackPath: "/music/s2.mp3",
      };
      const next = appReducer(state, {
        type: "UPDATE_TRACK",
        path: "/music/s1.mp3",
        track: makeTrack("/music/s1.mp3", { title: "Changed" }),
      });
      expect(next.selectedTrack?.title).toBe("Keep");
    });
  });

  describe("PUSH_UNDO / POP_UNDO", () => {
    it("push adds to the undo stack", () => {
      const state = { ...initialAppState, undoManager: new UndoManager() };
      const afterPush = appReducer(state, {
        type: "PUSH_UNDO",
        description: "Edit title",
        snapshots: [{ path: "/music/s.mp3", fields: { title: "Old" } }],
      });
      expect(afterPush.undoManager.canUndo).toBe(true);
      expect(afterPush.undoManager.length).toBe(1);
    });

    it("pop removes from the undo stack", () => {
      const um = new UndoManager();
      um.push("Edit", [
        { path: "/music/s.mp3", fields: { title: "Old" } },
      ]);
      const state = { ...initialAppState, undoManager: um };
      const afterPop = appReducer(state, {
        type: "POP_UNDO",
        snapshots: null,
      });
      expect(afterPop.undoManager.canUndo).toBe(false);
      expect(afterPop.undoManager.length).toBe(0);
    });
  });

  describe("SET_DIRTY / CLEAR_DIRTY", () => {
    it("SET_DIRTY adds paths to dirty set", () => {
      const next = appReducer(initialAppState, {
        type: "SET_DIRTY",
        paths: ["/music/s1.mp3", "/music/s2.mp3"],
      });
      expect(next.dirtyTracks.has("/music/s1.mp3")).toBe(true);
      expect(next.dirtyTracks.has("/music/s2.mp3")).toBe(true);
      expect(next.dirtyTracks.size).toBe(2);
    });

    it("CLEAR_DIRTY removes a path from dirty set", () => {
      const state = {
        ...initialAppState,
        dirtyTracks: new Set(["/music/s1.mp3", "/music/s2.mp3"]),
      };
      const next = appReducer(state, {
        type: "CLEAR_DIRTY",
        path: "/music/s1.mp3",
      });
      expect(next.dirtyTracks.has("/music/s1.mp3")).toBe(false);
      expect(next.dirtyTracks.has("/music/s2.mp3")).toBe(true);
      expect(next.dirtyTracks.size).toBe(1);
    });
  });

  describe("SET_SAVING", () => {
    it("sets the saving flag", () => {
      const next = appReducer(initialAppState, { type: "SET_SAVING", saving: true });
      expect(next.saving).toBe(true);
      const next2 = appReducer(next, { type: "SET_SAVING", saving: false });
      expect(next2.saving).toBe(false);
    });
  });

  describe("SET_FILTER", () => {
    it("updates the filter text", () => {
      const next = appReducer(initialAppState, { type: "SET_FILTER", filter: "test" });
      expect(next.filterText).toBe("test");
    });
  });

  describe("SET_COVER_URL", () => {
    it("sets the cover data URL", () => {
      const next = appReducer(initialAppState, {
        type: "SET_COVER_URL",
        url: "data:image/png;base64,abc123",
      });
      expect(next.coverDataUrl).toBe("data:image/png;base64,abc123");
    });

    it("sets to null", () => {
      const state = { ...initialAppState, coverDataUrl: "data:..." };
      const next = appReducer(state, { type: "SET_COVER_URL", url: null });
      expect(next.coverDataUrl).toBeNull();
    });
  });

  describe("SET_SCANNING / SET_LOADED", () => {
    it("SET_SCANNING sets the scanning flag", () => {
      const next = appReducer(initialAppState, { type: "SET_SCANNING", scanning: true });
      expect(next.scanning).toBe(true);
    });

    it("SET_LOADED sets the loaded flag", () => {
      const next = appReducer(initialAppState, { type: "SET_LOADED", loaded: true });
      expect(next.loaded).toBe(true);
    });
  });

  describe("TOGGLE_DIR", () => {
    it("adds a dir path when not expanded", () => {
      const next = appReducer(initialAppState, {
        type: "TOGGLE_DIR",
        path: "/music",
      });
      expect(next.expandedDirs.has("/music")).toBe(true);
    });

    it("removes a dir path when already expanded", () => {
      const state = {
        ...initialAppState,
        expandedDirs: new Set(["/music", "/other"]),
      };
      const next = appReducer(state, { type: "TOGGLE_DIR", path: "/music" });
      expect(next.expandedDirs.has("/music")).toBe(false);
      expect(next.expandedDirs.has("/other")).toBe(true);
    });
  });

  describe("SET_ERROR", () => {
    it("sets the error message", () => {
      const next = appReducer(initialAppState, {
        type: "SET_ERROR",
        error: "Something went wrong",
      });
      expect(next.error).toBe("Something went wrong");
    });

    it("clears the error with null", () => {
      const state = { ...initialAppState, error: "Old error" };
      const next = appReducer(state, { type: "SET_ERROR", error: null });
      expect(next.error).toBeNull();
    });
  });

  describe("CLEAR_ALL", () => {
    it("resets to initial state", () => {
      const state = {
        ...initialAppState,
        currentDir: "/music",
        tracks: [makeTrack("/music/s.mp3")],
        error: "Some error",
      };
      const next = appReducer(state, { type: "CLEAR_ALL" });
      expect(next.currentDir).toBeNull();
      expect(next.tracks).toHaveLength(0);
      expect(next.error).toBeNull();
    });
  });
});
