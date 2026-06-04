/**
 * Integration test: organize_files assistant macro preview and apply path.
 */

import { describe, it, expect } from "vitest";
import fs from "fs";
import os from "os";
import path from "path";
import { AssistantRuntime } from "../../electron/services/AssistantRuntime";
import { AssistantToolRegistry } from "../../electron/services/AssistantToolRegistry";
import { FolderOrganizerService } from "../../electron/services/FolderOrganizerService";

function touchSync(filePath: string): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, "dummy content", "utf-8");
}

describe("assistant organize_files integration", () => {
  it("creates a folder-move preview batch and applies planned moves", async () => {
    const libraryRoot = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-macro-"));
    try {
      const folderOrganizer = new FolderOrganizerService();
      folderOrganizer.setLibraryRoot(libraryRoot);
      const runtime = new AssistantRuntime(
        { onApiCall: () => () => {} } as never,
        new AssistantToolRegistry(),
        false,
      );

      touchSync(path.join(libraryRoot, "incoming", "cover.jpg"));
      touchSync(path.join(libraryRoot, "incoming", "cover.png"));
      touchSync(path.join(libraryRoot, "incoming", "song.mp3"));

      const plan = folderOrganizer.planOrganizeFiles({
        sourceDir: path.join(libraryRoot, "incoming"),
        criterion: "pattern",
        patternString: "cover.*",
        targetDirName: "Artwork",
      });

      const batch = runtime.createActionBatch({
        kind: "folder-move",
        title: `Organize ${plan.moves.length} file(s) by pattern`,
        summary: plan.summary,
        riskLevel: "medium",
        actions: [
          ...plan.moves.map((move) => ({
            sourcePath: move.sourcePath,
            destinationPath: move.destinationPath,
            description: "move",
          })),
          ...plan.skipped.map((move) => ({
            sourcePath: move.sourcePath,
            skipReason: move.skipReason ?? "Skipped",
            description: "skip",
          })),
        ],
        reversible: plan.reversible,
      });

      expect(batch.kind).toBe("folder-move");
      expect(batch.status).toBe("pending");
      expect(batch.actions.filter((action) => action.description === "move")).toHaveLength(2);
      expect(batch.actions.filter((action) => action.description === "skip")).toHaveLength(1);

      const moves = batch.actions
        .filter((action) => action.sourcePath && action.destinationPath && !action.skipReason)
        .map((action) => ({
          sourcePath: action.sourcePath!,
          destinationPath: action.destinationPath!,
        }));

      const { results } = await folderOrganizer.applyMoves({
        kind: "folder-move",
        summary: `Move ${moves.length} files`,
        moves,
        noops: [],
        skipped: [],
        affectedTracks: moves.length,
        reversible: true,
      });

      expect(results.every((result) => result.success)).toBe(true);
      expect(fs.existsSync(path.join(libraryRoot, "incoming", "Artwork", "cover.jpg"))).toBe(true);
      expect(fs.existsSync(path.join(libraryRoot, "incoming", "Artwork", "cover.png"))).toBe(true);
      expect(fs.existsSync(path.join(libraryRoot, "incoming", "song.mp3"))).toBe(true);
    } finally {
      fs.rmSync(libraryRoot, { recursive: true, force: true });
    }
  });
});
