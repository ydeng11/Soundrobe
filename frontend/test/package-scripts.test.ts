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
  it("starts dev behind the Electron native ABI guard", () => {
    const { scripts } = readPackageJson();

    expect(scripts.dev).toBe("npm run ensure:electron-abi && vite");
  });

  it("keeps explicit Electron native rebuild commands available", () => {
    const { scripts } = readPackageJson();

    expect(scripts["dev:rebuild"]).toBe("npm run rebuild:electron && npm run dev");
    expect(scripts["ensure:electron-abi"]).toBe("node scripts/ensure-electron-abi.mjs");
    expect(scripts["rebuild:electron"]).toBe("electron-rebuild -f -w better-sqlite3");
    expect(scripts.postinstall).toBe("npm run rebuild:electron");
  });

  it("does not rebuild better-sqlite3 for shell Node in the normal test flow", () => {
    const { scripts } = readPackageJson();

    expect(scripts.test).toBe("node scripts/run-vitest-with-electron-restore.mjs run");
    expect(scripts.test).not.toContain("rebuild:node");
    expect(scripts.test).not.toContain("npm rebuild better-sqlite3");
    expect(scripts["test:native-node"]).toBe(
      "npm run rebuild:node && node scripts/run-vitest-with-electron-restore.mjs run test/handlers/cache.test.ts test/handlers/dataset.test.ts test/handlers/conversation-logger.test.ts",
    );
  });
});
