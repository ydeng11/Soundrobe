import { describe, it, expect } from "vitest";
import fs from "fs";
import os from "os";
import path from "path";
import { FolderOrganizerService } from "../../electron/services/FolderOrganizerService";

function withTempLibrary(testFn: (libraryRoot: string) => void): void {
  const libraryRoot = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-organizer-"));
  try {
    testFn(libraryRoot);
  } finally {
    fs.rmSync(libraryRoot, { recursive: true, force: true });
  }
}

function touch(filePath: string, size = 1): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.closeSync(fs.openSync(filePath, "w"));
  fs.truncateSync(filePath, size);
}

describe("FolderOrganizerService", () => {
  describe("planGroupByAlbum", () => {
    it("plans moves based on album title function", () => {
      const service = new FolderOrganizerService();
      service.setLibraryRoot("/lib");

      const plan = service.planGroupByAlbum({
        trackPaths: ["/lib/artist/album/track1.flac", "/lib/artist/album/track2.flac"],
        libraryRoot: "/lib",
        albumTitleFn: () => "Best Of",
      });

      expect(plan.moves).toHaveLength(2);
      expect(plan.moves[0].destinationPath).toBe("/lib/Best Of/track1.flac");
      expect(plan.moves[1].destinationPath).toBe("/lib/Best Of/track2.flac");
      expect(plan.summary).toContain("Move 2 file(s)");
    });

    it("reports no-ops for files already in correct folder", () => {
      const service = new FolderOrganizerService();
      service.setLibraryRoot("/lib");

      const plan = service.planGroupByAlbum({
        trackPaths: ["/lib/Best Of/track1.flac"],
        libraryRoot: "/lib",
        albumTitleFn: () => "Best Of",
      });

      expect(plan.moves).toHaveLength(0);
      expect(plan.noops).toHaveLength(1);
      expect(plan.noops[0].skipReason).toContain("Already in correct album folder");
    });

    it("skips tracks with missing album title", () => {
      const service = new FolderOrganizerService();
      service.setLibraryRoot("/lib");

      const plan = service.planGroupByAlbum({
        trackPaths: ["/lib/artist/track1.flac"],
        libraryRoot: "/lib",
        albumTitleFn: () => null,
      });

      expect(plan.moves).toHaveLength(0);
      expect(plan.skipped).toHaveLength(1);
      expect(plan.skipped[0].skipReason).toContain("Missing album title");
    });

    it("rejects paths outside the library root", () => {
      const service = new FolderOrganizerService();
      service.setLibraryRoot("/lib");

      const plan = service.planGroupByAlbum({
        trackPaths: ["/outside/track1.flac"],
        libraryRoot: "/lib",
        albumTitleFn: () => "Album",
      });

      expect(plan.moves).toHaveLength(0);
      expect(plan.skipped).toHaveLength(1);
      expect(plan.skipped[0].skipReason).toContain("outside the library root");
    });

    it("sanitizes folder names", () => {
      const service = new FolderOrganizerService();
      service.setLibraryRoot("/lib");

      const plan = service.planGroupByAlbum({
        trackPaths: ["/lib/artist/track1.flac"],
        libraryRoot: "/lib",
        albumTitleFn: () => "Album: Best Of / 2024",
      });

      expect(plan.moves[0].destinationPath).toBe("/lib/Album Best Of 2024/track1.flac");
    });

    it("uses fallback name when sanitized result is empty", () => {
      const service = new FolderOrganizerService();
      service.setLibraryRoot("/lib");

      const plan = service.planGroupByAlbum({
        trackPaths: ["/lib/artist/track1.flac"],
        libraryRoot: "/lib",
        albumTitleFn: () => "...",
      });

      expect(plan.moves[0].destinationPath).toContain("Unknown Album");
    });
  });

  describe("path containment", () => {
    it("throws when library root is not set", () => {
      const service = new FolderOrganizerService();
      expect(() =>
        service.planGroupByAlbum({
          trackPaths: ["/test/track.flac"],
          libraryRoot: "",
          albumTitleFn: () => "Album",
        }),
      ).toThrow("Library root not set");
    });
  });

  describe("planOrganizeFiles", () => {
    it("groups direct child files by extension", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);
        touch(path.join(libraryRoot, "a.mp3"));
        touch(path.join(libraryRoot, "b.FLAC"));
        touch(path.join(libraryRoot, "no-extension"));

        const plan = service.planOrganizeFiles({
          sourceDir: libraryRoot,
          criterion: "extension",
          targetDirName: "By Type",
        });

        const destinations = plan.moves.map((move) =>
          path.relative(libraryRoot, move.destinationPath),
        );
        expect(destinations).toContain(path.join("By Type", "mp3", "a.mp3"));
        expect(destinations).toContain(path.join("By Type", "flac", "b.FLAC"));
        expect(destinations).toContain(path.join("By Type", "no-extension", "no-extension"));
      });
    });

    it("filters extension grouping by pattern_string", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);
        touch(path.join(libraryRoot, "a.mp3"));
        touch(path.join(libraryRoot, "b.flac"));

        const plan = service.planOrganizeFiles({
          sourceDir: libraryRoot,
          criterion: "extension",
          patternString: ".mp3",
          targetDirName: "Audio",
        });

        expect(plan.moves).toHaveLength(1);
        expect(path.basename(plan.moves[0].sourcePath)).toBe("a.mp3");
        expect(plan.skipped[0].skipReason).toContain("Extension does not match");
      });
    });

    it("moves glob pattern matches into the target folder", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);
        touch(path.join(libraryRoot, "cover.jpg"));
        touch(path.join(libraryRoot, "cover.png"));
        touch(path.join(libraryRoot, "track.mp3"));

        const plan = service.planOrganizeFiles({
          sourceDir: libraryRoot,
          criterion: "pattern",
          patternString: "cover.*",
          targetDirName: "Artwork",
        });

        expect(plan.moves).toHaveLength(2);
        expect(plan.moves.map((move) => path.basename(move.sourcePath))).toEqual([
          "cover.jpg",
          "cover.png",
        ]);
        expect(plan.skipped.find((move) => path.basename(move.sourcePath) === "track.mp3")?.skipReason)
          .toContain("Filename does not match");
      });
    });

    it("requires pattern_string for pattern criterion", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);

        expect(() =>
          service.planOrganizeFiles({
            sourceDir: libraryRoot,
            criterion: "pattern",
            targetDirName: "Matches",
          }),
        ).toThrow("pattern_string is required");
      });
    });

    it("groups files by created month", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);
        const filePath = path.join(libraryRoot, "song.mp3");
        touch(filePath);
        const stats = fs.statSync(filePath);
        const createdAt = stats.birthtimeMs > 0 ? stats.birthtime : stats.ctime;
        const expectedFolder = `${createdAt.getFullYear()}-${String(createdAt.getMonth() + 1).padStart(2, "0")}`;

        const plan = service.planOrganizeFiles({
          sourceDir: libraryRoot,
          criterion: "date_created",
          targetDirName: "By Date",
        });

        expect(path.relative(libraryRoot, plan.moves[0].destinationPath)).toBe(
          path.join("By Date", expectedFolder, "song.mp3"),
        );
      });
    });

    it("groups files by size bucket", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);
        const mb = 1024 * 1024;
        touch(path.join(libraryRoot, "small.bin"), 1);
        touch(path.join(libraryRoot, "medium.bin"), 10 * mb);
        touch(path.join(libraryRoot, "large.bin"), 100 * mb);
        touch(path.join(libraryRoot, "huge.bin"), 1024 * mb);

        const plan = service.planOrganizeFiles({
          sourceDir: libraryRoot,
          criterion: "size",
          targetDirName: "By Size",
        });

        const destinations = plan.moves.map((move) =>
          path.relative(libraryRoot, move.destinationPath),
        );
        expect(destinations).toContain(path.join("By Size", "small", "small.bin"));
        expect(destinations).toContain(path.join("By Size", "medium", "medium.bin"));
        expect(destinations).toContain(path.join("By Size", "large", "large.bin"));
        expect(destinations).toContain(path.join("By Size", "huge", "huge.bin"));
      });
    });

    it("sanitizes the target folder name", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);
        touch(path.join(libraryRoot, "song.mp3"));

        const plan = service.planOrganizeFiles({
          sourceDir: libraryRoot,
          criterion: "pattern",
          patternString: "*.mp3",
          targetDirName: "Bad: Folder / Name",
        });

        expect(path.relative(libraryRoot, plan.moves[0].destinationPath)).toBe(
          path.join("Bad Folder Name", "song.mp3"),
        );
      });
    });

    it("skips subdirectories and hidden files without recursive scanning", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);
        touch(path.join(libraryRoot, ".hidden.mp3"));
        touch(path.join(libraryRoot, "nested", "child.mp3"));
        touch(path.join(libraryRoot, "visible.mp3"));

        const plan = service.planOrganizeFiles({
          sourceDir: libraryRoot,
          criterion: "extension",
          targetDirName: "By Type",
        });

        expect(plan.moves).toHaveLength(1);
        expect(path.basename(plan.moves[0].sourcePath)).toBe("visible.mp3");
        expect(plan.skipped.map((move) => move.skipReason)).toEqual(
          expect.arrayContaining(["Hidden file skipped", "Subdirectory skipped"]),
        );
      });
    });

    it("adds deterministic suffixes for destination conflicts", () => {
      withTempLibrary((libraryRoot) => {
        const service = new FolderOrganizerService();
        service.setLibraryRoot(libraryRoot);
        touch(path.join(libraryRoot, "cover.jpg"));
        touch(path.join(libraryRoot, "Artwork", "cover.jpg"));

        const plan = service.planOrganizeFiles({
          sourceDir: libraryRoot,
          criterion: "pattern",
          patternString: "cover.*",
          targetDirName: "Artwork",
        });

        expect(path.relative(libraryRoot, plan.moves[0].destinationPath)).toBe(
          path.join("Artwork", "cover_1.jpg"),
        );
      });
    });

    it("rejects a source directory outside the library root", () => {
      withTempLibrary((libraryRoot) => {
        const outside = fs.mkdtempSync(path.join(os.tmpdir(), "outside-organizer-"));
        try {
          const service = new FolderOrganizerService();
          service.setLibraryRoot(libraryRoot);

          expect(() =>
            service.planOrganizeFiles({
              sourceDir: outside,
              criterion: "extension",
              targetDirName: "By Type",
            }),
          ).toThrow("outside the library root");
        } finally {
          fs.rmSync(outside, { recursive: true, force: true });
        }
      });
    });
  });
});
