// @vitest-environment node
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));

interface PackageJson {
  scripts: Record<string, string>;
}

function readPackageJson(): PackageJson {
  const packageJsonPath = resolve(__dirname, "../package.json");
  return JSON.parse(readFileSync(packageJsonPath, "utf8")) as PackageJson;
}

describe("package scripts", () => {
  it("starts dev without forcing a native module rebuild", () => {
    const { scripts } = readPackageJson();

    expect(scripts.dev).toBe("vite");
    expect(scripts.dev).not.toContain("rebuild:electron");
  });

  it("keeps explicit Electron native rebuild commands available", () => {
    const { scripts } = readPackageJson();

    expect(scripts["dev:rebuild"]).toBe("npm run rebuild:electron && vite");
    expect(scripts["rebuild:electron"]).toBe("electron-rebuild -f -w better-sqlite3");
    expect(scripts.postinstall).toBe("npm run rebuild:electron");
  });
});
