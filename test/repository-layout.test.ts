// @vitest-environment node
import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";

const repositoryRoot = execFileSync("git", ["rev-parse", "--show-toplevel"], {
  encoding: "utf8",
}).trim();

describe("repository layout", () => {
  it("keeps the active Tauri application on the repository root golden path", () => {
    const requiredRootPaths = [
      "package.json",
      "package-lock.json",
      "index.html",
      "vite.config.ts",
      "tsconfig.json",
      "src",
      "src-tauri",
      "test",
      "e2e-tauri",
      "data/artist-aliases.json",
    ];

    for (const requiredPath of requiredRootPaths) {
      expect(existsSync(resolve(repositoryRoot, requiredPath)), requiredPath).toBe(true);
    }

    expect(existsSync(resolve(repositoryRoot, "frontend"))).toBe(false);
  });

  it("keeps Tauri and Vite paths aligned with the root layout", () => {
    const tauriConfig = JSON.parse(
      readFileSync(resolve(repositoryRoot, "src-tauri/tauri.conf.json"), "utf8"),
    ) as { build: { frontendDist: string } };
    const viteConfig = readFileSync(resolve(repositoryRoot, "vite.config.ts"), "utf8");

    expect(tauriConfig.build.frontendDist).toBe("../dist");
    expect(viteConfig).toContain('ignored: ["**/src-tauri/**"]');
  });

  it("keeps root utility scripts in their existing CommonJS runtime", () => {
    const scriptsPackage = JSON.parse(
      readFileSync(resolve(repositoryRoot, "scripts/package.json"), "utf8"),
    ) as { type: string };

    expect(scriptsPackage.type).toBe("commonjs");
  });

  it("keeps atomic replacement available to review recovery on every platform", () => {
    const mutations = readFileSync(
      resolve(repositoryRoot, "src-tauri/src/commands/mutations.rs"),
      "utf8",
    );
    const definitions = mutations.match(
      /pub\(crate\) fn replace_file_atomic\(source: &Path, destination: &Path\)/g,
    );

    // Review recovery is a sibling command module and imports this helper on
    // every target, so both the Unix and Windows implementations must retain
    // crate visibility.
    expect(definitions).toHaveLength(2);
  });
});
