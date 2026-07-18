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
  it("uses Tauri as the desktop development and distribution runtime", () => {
    const { scripts } = readPackageJson();

    expect(scripts.dev).toBe("tauri dev");
    expect(scripts.build).toBe("tauri build");
    expect(scripts.dist).toBe("tauri build");
  });

  it("keeps the renderer build separate for Tauri lifecycle hooks", () => {
    const { scripts } = readPackageJson();

    expect(scripts["dev:web"]).toBe("vite");
    expect(scripts["build:web"]).toBe("tsc && vite build");
    expect(scripts["ensure:electron-abi"]).toBeUndefined();
    expect(scripts["rebuild:electron"]).toBeUndefined();
    expect(scripts.postinstall).toBeUndefined();
  });

  it("runs both renderer and Rust tests without native Node rebuilds", () => {
    const { scripts } = readPackageJson();

    expect(scripts.test).toBe("npm run test:web && npm run test:rust");
    expect(scripts["test:rust"]).toBe("cargo test --manifest-path src-tauri/Cargo.toml");
    expect(scripts["test:native-node"]).toBeUndefined();
  });

  it("declares every required unsigned Tauri bundle target", () => {
    const { scripts } = readPackageJson();
    const tauriConfig = JSON.parse(
      readFileSync(resolve(__dirname, "../src-tauri/tauri.conf.json"), "utf8"),
    ) as { bundle: { category: string } };

    expect(scripts["dist:mac"]).toBe("tauri build --bundles app,dmg");
    expect(scripts["dist:win"]).toBe("tauri build --bundles nsis");
    expect(scripts["dist:linux"]).toBe("tauri build --bundles appimage,deb");
    expect(tauriConfig.bundle.category).toBe("Music");
  });

  it("builds each platform bundle in CI", () => {
    const workflow = readFileSync(
      resolve(__dirname, "../../.github/workflows/tauri.yml"),
      "utf8",
    );

    expect(workflow).toContain("macos-latest");
    expect(workflow).toContain("windows-latest");
    expect(workflow).toContain("ubuntu-22.04");
    expect(workflow).toContain("npm run dist:mac");
    expect(workflow).toContain("npm run dist:win");
    expect(workflow).toContain("npm run dist:linux");
  });
});
