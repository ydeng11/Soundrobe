import { describe, it, expect } from "vitest";
import { UndoManager } from "../../src/state/UndoManager";

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

  it("pop returns the most recent operation", () => {
    const um = new UndoManager();
    um.push("Edit title", [
      { path: "/music/song.mp3", fields: { title: "Old Title" } },
    ]);
    um.push("Edit artist", [
      { path: "/music/song.mp3", fields: { artist: "Old Artist" } },
    ]);

    const op = um.pop();
    expect(op).not.toBeNull();
    expect(op!.description).toBe("Edit artist");
    expect(op!.snapshots).toHaveLength(1);
    expect(op!.snapshots[0].fields.artist).toBe("Old Artist");
    expect(um.length).toBe(1);
  });

  it("pop returns null when stack is empty", () => {
    const um = new UndoManager();
    expect(um.pop()).toBeNull();
  });

  it("clear empties the stack", () => {
    const um = new UndoManager();
    um.push("Edit 1", []);
    um.push("Edit 2", []);
    um.clear();
    expect(um.length).toBe(0);
    expect(um.canUndo).toBe(false);
  });

  it("respects max depth by dropping oldest operations", () => {
    const um = new UndoManager(3);
    um.push("Op 1", []);
    um.push("Op 2", []);
    um.push("Op 3", []);
    um.push("Op 4", []);

    expect(um.length).toBe(3);
    // The oldest (Op 1) should be gone
    expect(um.currentDescription).toBe("Op 4");
    um.pop(); // Op 4
    um.pop(); // Op 3
    expect(um.currentDescription).toBe("Op 2");
    um.pop(); // Op 2
    expect(um.canUndo).toBe(false);
  });

  it("preserves snapshot data through push/pop cycle", () => {
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
    const op = um.pop();
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

    const op = um.pop();
    expect(op!.snapshots).toHaveLength(3);
    expect(op!.description).toBe("Batch edit");
  });
});
