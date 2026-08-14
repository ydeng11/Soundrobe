import { describe, it, expect } from "vitest";
import {
  UndoManager,
  revertHistoryThrough,
} from "../../src/state/UndoManager";

describe("UndoManager", () => {
  it("starts with canUndo = false", () => {
    const um = new UndoManager();
    expect(um.canUndo).toBe(false);
    expect(um.length).toBe(0);
    expect(um.currentDescription).toBeNull();
  });

  it("push adds an operation and canUndo becomes true", () => {
    const um = new UndoManager();
    um.push("Edit title", [
      { path: "/music/song.mp3", fields: { title: "Old Title" } },
    ]);
    expect(um.canUndo).toBe(true);
    expect(um.length).toBe(1);
    expect(um.currentDescription).toBe("Edit title");
  });

  it("clear empties the stack", () => {
    const um = new UndoManager();
    um.push("Edit 1", [{ path: "/one", fields: { title: "One" } }]);
    um.push("Edit 2", [{ path: "/two", fields: { title: "Two" } }]);
    um.clear();
    expect(um.length).toBe(0);
    expect(um.canUndo).toBe(false);
  });

  it("exposes newest-first history and caps the session at 20 commands", () => {
    const um = new UndoManager();
    for (let index = 1; index <= 21; index++) {
      um.push(`Op ${index}`, [
        { path: `/music/${index}.flac`, fields: { title: `Old ${index}` } },
      ]);
    }

    expect(um.length).toBe(20);
    expect(um.history[0].description).toBe("Op 21");
    expect(um.history[0].affectedFileCount).toBe(1);
    expect(um.history.at(-1)?.description).toBe("Op 2");
  });

  it("reverts a selected older command and every newer command newest-first", async () => {
    const um = new UndoManager();
    um.push("Oldest", [{ path: "/oldest", fields: { title: "a" } }]);
    um.push("Middle", [{ path: "/middle", fields: { title: "b" } }]);
    um.push("Newest", [{ path: "/newest", fields: { title: "c" } }]);
    const calls: string[] = [];

    const result = await revertHistoryThrough(
      um,
      um.history[1].id,
      async (snapshot) => {
        calls.push(snapshot.path);
        return null;
      },
    );

    expect(calls).toEqual(["/newest", "/middle"]);
    expect(result.manager.history.map((operation) => operation.description)).toEqual([
      "Oldest",
    ]);
    expect(result.failures).toEqual([]);
  });

  it("retains failed snapshots as the latest retryable remainder and stops before older commands", async () => {
    const um = new UndoManager();
    um.push("Older", [{ path: "/older", fields: { title: "a" } }]);
    um.push("Newest", [
      { path: "/success", fields: { title: "b" } },
      { path: "/failure", fields: { title: "c", genre: "Pop" } },
    ]);

    const result = await revertHistoryThrough(
      um,
      um.history[1].id,
      async (snapshot) =>
        snapshot.path === "/failure"
          ? {
              snapshot: { path: snapshot.path, fields: { genre: "Pop" } },
              error: "disk full",
            }
          : null,
    );

    expect(result.manager.history.map((operation) => operation.description)).toEqual([
      "Newest",
      "Older",
    ]);
    expect(result.manager.history[0].snapshots).toEqual([
      { path: "/failure", fields: { genre: "Pop" } },
    ]);
    expect(result.failures).toEqual([{ path: "/failure", error: "disk full" }]);
  });

  it("preserves snapshot data in exposed history", () => {
    const um = new UndoManager();
    const snapshot = {
      path: "/music/song.mp3",
      fields: {
        title: "Old Title",
        artist: "Old Artist",
        album: "Old Album",
        year: "2020",
      },
    };

    um.push("Edit multiple", [snapshot]);
    const op = um.history[0];
    expect(op!.snapshots).toHaveLength(1);
    expect(op!.snapshots[0].path).toBe("/music/song.mp3");
    expect(op!.snapshots[0].fields.title).toBe("Old Title");
    expect(op!.snapshots[0].fields.artist).toBe("Old Artist");
    expect(op!.snapshots[0].fields.album).toBe("Old Album");
    expect(op!.snapshots[0].fields.year).toBe("2020");
  });

  it("supports multiple snapshots per operation", () => {
    const um = new UndoManager();
    um.push("Batch edit", [
      { path: "/music/track1.mp3", fields: { title: "T1 Old" } },
      { path: "/music/track2.mp3", fields: { title: "T2 Old" } },
      { path: "/music/track3.mp3", fields: { title: "T3 Old" } },
    ]);

    const op = um.history[0];
    expect(op!.snapshots).toHaveLength(3);
    expect(op!.description).toBe("Batch edit");
  });
});
