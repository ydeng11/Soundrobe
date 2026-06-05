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
  describe("SET_LIBRARY", () => {
    it("sets libraryPath and clears selection", () => {
      const state = { ...initialAppState, selectedTrackPath: "/old/track.mp3" };
      const next = appReducer(state, {
        type: "SET_LIBRARY",
        path: "/music",
      });
      expect(next.libraryPath).toBe("/music");
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

  describe("SET_TRACKS", () => {
    it("refreshes the selected track object when reloaded tracks include it", () => {
      const selectedPath = "/music/song.mp3";
      const state = {
        ...initialAppState,
        selectedTrackPath: selectedPath,
        selectedTrack: makeTrack(selectedPath, { title: "Old Title" }),
      };

      const next = appReducer(state, {
        type: "SET_TRACKS",
        tracks: [makeTrack(selectedPath, { title: "New Title" })],
      });

      expect(next.selectedTrack?.title).toBe("New Title");
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

    it("POP_UNDO triggers a re-render without modifying the undo stack", () => {
      const um = new UndoManager();
      um.push("Edit", [
        { path: "/music/s.mp3", fields: { title: "Old" } },
      ]);
      const state = { ...initialAppState, undoManager: um };
      const afterPop = appReducer(state, {
        type: "POP_UNDO",
      });
      // POP_UNDO just returns the same state (triggers re-render).
      // The actual pop is done by handleRevert before dispatching POP_UNDO.
      expect(afterPop.undoManager.canUndo).toBe(true);
      expect(afterPop.undoManager.length).toBe(1);
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

  describe("SET_ACTIVE_ALBUM", () => {
    it("sets activeAlbumPath and clears selection when track is outside scope", () => {
      const state = {
        ...initialAppState,
        selectedTrackPath: "/music/track.mp3",
        selectedTrack: {} as any,
        coverDataUrl: "data:,",
      };
      const next = appReducer(state, {
        type: "SET_ACTIVE_ALBUM",
        path: "/music/My Album",
      });
      expect(next.activeAlbumPath).toBe("/music/My Album");
      expect(next.selectedTrackPath).toBeNull();
      expect(next.selectedTrack).toBeNull();
      expect(next.coverDataUrl).toBeNull();
    });

    it("preserves selection when selected track is inside the new album scope", () => {
      const track = makeTrack("/music/Album A/song.mp3", { title: "Keep Me" });
      const state = {
        ...initialAppState,
        tracks: [track],
        selectedTrackPath: "/music/Album A/song.mp3",
        selectedTrack: track,
        coverDataUrl: "data:image/jpeg;base64,keep",
      };
      const next = appReducer(state, {
        type: "SET_ACTIVE_ALBUM",
        path: "/music/Album A",
      });
      expect(next.activeAlbumPath).toBe("/music/Album A");
      expect(next.selectedTrackPath).toBe("/music/Album A/song.mp3");
      expect(next.selectedTrack).toBe(track);
      expect(next.coverDataUrl).toBe("data:image/jpeg;base64,keep");
    });

    it("preserves selection when navigating to null (show all)", () => {
      const track = makeTrack("/music/Album B/song.mp3");
      const state = {
        ...initialAppState,
        tracks: [track],
        selectedTrackPath: "/music/Album B/song.mp3",
        selectedTrack: track,
        coverDataUrl: "data:image/png;base64,keep",
      };
      const next = appReducer(state, {
        type: "SET_ACTIVE_ALBUM",
        path: null,
      });
      expect(next.activeAlbumPath).toBeNull();
      // null scope means show all — selection is always in scope
      expect(next.selectedTrackPath).toBe("/music/Album B/song.mp3");
      expect(next.coverDataUrl).toBe("data:image/png;base64,keep");
    });

    it("does not modify tracks array", () => {
      const tracks = [
        makeTrack("/music/Album A/s1.mp3"),
        makeTrack("/music/Album B/s2.mp3"),
      ];
      const state = { ...initialAppState, tracks };
      const next = appReducer(state, {
        type: "SET_ACTIVE_ALBUM",
        path: "/music/Album A",
      });
      expect(next.tracks).toBe(tracks);
      expect(next.tracks).toHaveLength(2);
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

  describe("TOGGLE_SETTINGS", () => {
    it("shows the settings modal", () => {
      const next = appReducer(initialAppState, {
        type: "TOGGLE_SETTINGS",
        show: true,
      });
      expect(next.showSettings).toBe(true);
    });

    it("hides the settings modal", () => {
      const state = { ...initialAppState, showSettings: true };
      const next = appReducer(state, {
        type: "TOGGLE_SETTINGS",
        show: false,
      });
      expect(next.showSettings).toBe(false);
    });
  });

  describe("CLEAR_ALL", () => {
    it("resets to initial state", () => {
      const state = {
        ...initialAppState,
        libraryPath: "/music",
        tracks: [makeTrack("/music/s.mp3")],
        error: "Some error",
      };
      const next = appReducer(state, { type: "CLEAR_ALL" });
      expect(next.libraryPath).toBeNull();
      expect(next.tracks).toHaveLength(0);
      expect(next.error).toBeNull();
    });
  });
});
