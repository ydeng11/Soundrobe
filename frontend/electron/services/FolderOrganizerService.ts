/**
 * FolderOrganizerService — file move and album-title grouping.
 *
 * All destination paths are contained inside the current library root.
 * Creates a reversible manifest for every move.
 */

import fs from "fs";
import path from "path";

export interface FolderGroupInput {
  trackPaths: string[];
  libraryRoot: string;
  albumTitleFn?: (trackPath: string) => string | null;
}

export type FileOrganizeCriterion =
  | "extension"
  | "pattern"
  | "date_created"
  | "size";

export interface FileOrganizeInput {
  sourceDir: string;
  criterion: FileOrganizeCriterion;
  patternString?: string;
  targetDirName: string;
}

export interface MoveAction {
  sourcePath: string;
  destinationPath: string;
  skipReason?: string;
}

export interface FolderOrganizerPlan {
  kind: "folder-move";
  summary: string;
  moves: MoveAction[];
  noops: MoveAction[];
  skipped: MoveAction[];
  affectedTracks: number;
  reversible: boolean;
}

export interface MoveResult {
  sourcePath: string;
  destinationPath: string;
  success: boolean;
  error?: string;
}

type MutablePlanParts = Pick<FolderOrganizerPlan, "moves" | "noops" | "skipped">;

export class FolderOrganizerService {
  private libraryRoot: string | null = null;

  setLibraryRoot(root: string | null): void {
    this.libraryRoot = root;
  }

  /**
   * Plan grouping tracks into album-title folders.
   *
   * Default behavior:
   * 1. Use provided track paths
   * 2. Destination is <libraryRoot>/<sanitized album title>/<original filename>
   * 3. Files already in the correct folder are reported as no-ops
   * 4. Tracks with missing album title are skipped unless albumTitleFn is provided
   */
  planGroupByAlbum(input: FolderGroupInput): FolderOrganizerPlan {
    if (!this.libraryRoot) {
      throw new Error("Library root not set");
    }

    // Default: use filename without extension as album title hint (for testing)
    // In real usage, the caller provides albumTitleFn from track metadata
    const getAlbumTitle = input.albumTitleFn ?? ((trackPath: string) => {
      // Fallback: parent directory name
      return path.basename(path.dirname(trackPath));
    });

    const moves: MoveAction[] = [];
    const noops: MoveAction[] = [];
    const skipped: MoveAction[] = [];

    for (const trackPath of input.trackPaths) {
      const resolved = path.resolve(trackPath);

      // Validate path containment
      if (!this.isInsideLibrary(resolved)) {
        skipped.push({
          sourcePath: trackPath,
          destinationPath: "",
          skipReason: "Path is outside the library root",
        });
        continue;
      }

      const albumTitle = getAlbumTitle(trackPath);
      if (!albumTitle || albumTitle.trim() === "") {
        skipped.push({
          sourcePath: trackPath,
          destinationPath: "",
          skipReason: "Missing album title — cannot determine destination folder",
        });
        continue;
      }

      const sanitizedTitle = this.sanitizeFolderName(albumTitle);
      const filename = path.basename(trackPath);
      const destinationDir = path.join(this.libraryRoot, sanitizedTitle);
      const destinationPath = this.uniqueDestinationPath(
        resolved,
        destinationDir,
        filename,
      );

      // Check if already in correct folder
      const currentDir = path.dirname(resolved);
      if (currentDir === destinationDir) {
        noops.push({
          sourcePath: trackPath,
          destinationPath,
          skipReason: "Already in correct album folder",
        });
        continue;
      }

      moves.push({
        sourcePath: trackPath,
        destinationPath,
      });
    }

    return {
      kind: "folder-move",
      summary: this.planSummary({ moves, noops, skipped }),
      moves,
      noops,
      skipped,
      affectedTracks: moves.length,
      reversible: true,
    };
  }

  /**
   * Plan a composite file organization macro over direct child files.
   *
   * This is intentionally non-recursive: the assistant chooses a single source
   * directory and a high-level criterion, while the service handles scanning,
   * grouping, path safety, conflict handling, and preview details.
   */
  planOrganizeFiles(input: FileOrganizeInput): FolderOrganizerPlan {
    if (!this.libraryRoot) {
      throw new Error("Library root not set");
    }

    const sourceDir = path.resolve(input.sourceDir);
    if (!this.isInsideLibrary(sourceDir)) {
      throw new Error("Source directory is outside the library root");
    }
    if (!fs.existsSync(sourceDir)) {
      throw new Error("Source directory does not exist");
    }
    if (!fs.statSync(sourceDir).isDirectory()) {
      throw new Error("Source path is not a directory");
    }
    if (input.criterion === "pattern" && !input.patternString?.trim()) {
      throw new Error("pattern_string is required when criterion is pattern");
    }

    const targetRoot = path.join(
      sourceDir,
      this.sanitizeFolderName(input.targetDirName),
    );
    const extensionFilters = this.parseExtensionFilters(input.patternString);
    const patternMatcher = input.criterion === "pattern"
      ? this.globMatcher(input.patternString ?? "")
      : null;

    const moves: MoveAction[] = [];
    const noops: MoveAction[] = [];
    const skipped: MoveAction[] = [];

    const entries = fs.readdirSync(sourceDir, { withFileTypes: true });
    for (const entry of entries) {
      const sourcePath = path.join(sourceDir, entry.name);
      const resolved = path.resolve(sourcePath);

      if (!this.isInsideLibrary(resolved)) {
        this.skip(skipped, sourcePath, "Path is outside the library root");
        continue;
      }

      if (entry.name.startsWith(".")) {
        this.skip(skipped, sourcePath, "Hidden file skipped");
        continue;
      }

      if (!entry.isFile()) {
        this.skip(skipped, sourcePath, "Subdirectory skipped");
        continue;
      }

      const destinationDir = this.destinationDirForCriterion({
        sourcePath,
        targetRoot,
        criterion: input.criterion,
        patternMatcher,
        extensionFilters,
      });

      if (!destinationDir) {
        this.skip(skipped, sourcePath, this.skipReasonForCriterion(input.criterion));
        continue;
      }

      if (!this.isInsideLibrary(destinationDir)) {
        this.skip(skipped, sourcePath, "Destination is outside the library root");
        continue;
      }

      const destinationPath = this.uniqueDestinationPath(
        resolved,
        destinationDir,
        entry.name,
      );

      if (path.dirname(resolved) === destinationDir) {
        noops.push({
          sourcePath,
          destinationPath,
          skipReason: "Already in target folder",
        });
        continue;
      }

      moves.push({
        sourcePath,
        destinationPath,
      });
    }

    return {
      kind: "folder-move",
      summary: this.planSummary({ moves, noops, skipped }),
      moves,
      noops,
      skipped,
      affectedTracks: moves.length,
      reversible: true,
    };
  }

  /**
   * Apply a planned folder move.
   * Returns a manifest for reversal.
   */
  async applyMoves(plan: FolderOrganizerPlan): Promise<{
    results: MoveResult[];
    manifest: Array<{ from: string; to: string }>;
  }> {
    const results: MoveResult[] = [];
    const manifest: Array<{ from: string; to: string }> = [];

    for (const move of plan.moves) {
      try {
        const targetDir = path.dirname(move.destinationPath);
        if (!fs.existsSync(targetDir)) {
          fs.mkdirSync(targetDir, { recursive: true });
        }

        await fs.promises.rename(move.sourcePath, move.destinationPath);
        results.push({
          sourcePath: move.sourcePath,
          destinationPath: move.destinationPath,
          success: true,
        });
        manifest.push({ from: move.sourcePath, to: move.destinationPath });
      } catch (error) {
        results.push({
          sourcePath: move.sourcePath,
          destinationPath: move.destinationPath,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }

    return { results, manifest };
  }

  /**
   * Revert a folder move using a manifest.
   */
  async revertMoves(
    manifest: Array<{ from: string; to: string }>,
  ): Promise<MoveResult[]> {
    const results: MoveResult[] = [];

    // Revert in reverse order
    for (const entry of manifest.reverse()) {
      try {
        if (fs.existsSync(entry.to)) {
          await fs.promises.rename(entry.to, entry.from);
          results.push({
            sourcePath: entry.from,
            destinationPath: entry.to,
            success: true,
          });
        }
      } catch (error) {
        results.push({
          sourcePath: entry.from,
          destinationPath: entry.to,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }

    return results;
  }

  /**
   * Sanitize a folder name for filesystem use.
   * Removes characters that are problematic across platforms.
   */
  private sanitizeFolderName(name: string): string {
    return name
      .replace(/[/\\?%*:|"<>]/g, "") // Remove reserved chars
      .replace(/\s+/g, " ")           // Normalize whitespace
      .replace(/^\.+/, "")            // Remove leading dots
      .replace(/[\x00-\x1f]/g, "")    // Remove control chars
      .trim()
      || "Unknown Album"; // Fallback for empty result
  }

  private planSummary(plan: MutablePlanParts): string {
    return plan.moves.length > 0
      ? `Move ${plan.moves.length} file(s), ${plan.noops.length} already in place, ${plan.skipped.length} skipped`
      : "No files to move";
  }

  private skip(skipped: MoveAction[], sourcePath: string, skipReason: string): void {
    skipped.push({
      sourcePath,
      destinationPath: "",
      skipReason,
    });
  }

  private destinationDirForCriterion(input: {
    sourcePath: string;
    targetRoot: string;
    criterion: FileOrganizeCriterion;
    patternMatcher: ((fileName: string) => boolean) | null;
    extensionFilters: Set<string> | null;
  }): string | null {
    const fileName = path.basename(input.sourcePath);

    switch (input.criterion) {
      case "extension": {
        const extension = path.extname(fileName).slice(1).toLowerCase();
        const folder = extension || "no-extension";
        if (input.extensionFilters && !input.extensionFilters.has(folder)) {
          return null;
        }
        return path.join(input.targetRoot, folder);
      }
      case "pattern":
        return input.patternMatcher?.(fileName) ? input.targetRoot : null;
      case "date_created": {
        const stats = fs.statSync(input.sourcePath);
        const createdAt = stats.birthtimeMs > 0 ? stats.birthtime : stats.ctime;
        const year = createdAt.getFullYear();
        const month = String(createdAt.getMonth() + 1).padStart(2, "0");
        return path.join(input.targetRoot, `${year}-${month}`);
      }
      case "size": {
        const size = fs.statSync(input.sourcePath).size;
        return path.join(input.targetRoot, this.sizeBucket(size));
      }
    }
  }

  private skipReasonForCriterion(criterion: FileOrganizeCriterion): string {
    switch (criterion) {
      case "extension":
        return "Extension does not match pattern_string filter";
      case "pattern":
        return "Filename does not match pattern_string";
      case "date_created":
      case "size":
        return "File did not match criterion";
    }
  }

  private parseExtensionFilters(patternString?: string): Set<string> | null {
    if (!patternString?.trim()) return null;

    const filters = patternString
      .split(/[,\s]+/)
      .map((value) => value.trim().toLowerCase().replace(/^\./, ""))
      .filter(Boolean);

    return filters.length > 0 ? new Set(filters) : null;
  }

  private globMatcher(patternString: string): (fileName: string) => boolean {
    const escaped = patternString
      .trim()
      .replace(/[.+^${}()|[\]\\]/g, "\\$&")
      .replace(/\*/g, ".*")
      .replace(/\?/g, ".");
    const regex = new RegExp(`^${escaped}$`, "i");
    return (fileName: string) => regex.test(fileName);
  }

  private sizeBucket(sizeBytes: number): "small" | "medium" | "large" | "huge" {
    const mb = 1024 * 1024;
    const gb = 1024 * mb;
    if (sizeBytes < 10 * mb) return "small";
    if (sizeBytes < 100 * mb) return "medium";
    if (sizeBytes < gb) return "large";
    return "huge";
  }

  private uniqueDestinationPath(
    sourcePath: string,
    destinationDir: string,
    fileName: string,
  ): string {
    const destinationPath = path.join(destinationDir, fileName);
    if (!fs.existsSync(destinationPath) || path.resolve(sourcePath) === path.resolve(destinationPath)) {
      return destinationPath;
    }

    const ext = path.extname(fileName);
    const base = path.basename(fileName, ext);
    let counter = 1;
    let uniquePath = destinationPath;

    while (fs.existsSync(uniquePath)) {
      uniquePath = path.join(destinationDir, `${base}_${counter}${ext}`);
      counter++;
    }

    return uniquePath;
  }

  private isInsideLibrary(targetPath: string): boolean {
    if (!this.libraryRoot) return false;
    const libraryRoot = path.resolve(this.libraryRoot);
    const resolvedTarget = path.resolve(targetPath);
    const relative = path.relative(libraryRoot, resolvedTarget);
    return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
  }
}
