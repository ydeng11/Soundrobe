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

  it("loads the local dotenv file for the credentialed OpenRouter release gate", () => {
    const justfile = readFileSync(resolve(__dirname, "../../Justfile"), "utf8");

    expect(justfile).toContain('set dotenv-path := ".env.local"');
    expect(justfile).toContain("fe-smoke-openrouter:");
    expect(justfile).toContain("live_openrouter_returns_schema_constrained_json");
    expect(justfile).toContain("fe-smoke-assistant:");
    expect(justfile).toContain("live-openrouter.spec.ts");
    expect(justfile).toContain("fe-smoke-cover-picker:");
    expect(justfile).toContain("live-cover-picker.spec.ts");
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
    expect(workflow).toMatch(
      /name: Build unsigned bundles\n\s+env:\n\s+CI: "true"/,
    );
  });

  it("runs test-only embedded WebdriverIO coverage on every desktop platform", () => {
    const { scripts } = readPackageJson();
    const workflow = readFileSync(
      resolve(__dirname, "../../.github/workflows/tauri.yml"),
      "utf8",
    );
    const wdioConfig = readFileSync(resolve(__dirname, "../wdio.conf.ts"), "utf8");
    const workflowSpec = readFileSync(
      resolve(__dirname, "../e2e-tauri/workflows.spec.ts"),
      "utf8",
    );

    expect(scripts["build:e2e"]).toContain("--features wdio");
    expect(scripts["test:e2e"]).toBe("npm run build:e2e && wdio run wdio.conf.ts");
    expect(wdioConfig).toContain("driverProvider: \"embedded\"");
    expect(wdioConfig).toContain('specs: ["./e2e-tauri/workflows.spec.ts"]');
    expect(wdioConfig).toContain("prepareE2eWorkspace");
    expect(workflowSpec).toContain("reveals the native main window after renderer boot");
    expect(workflowSpec).toContain("preserves absolute paths through the native library pipeline");
    expect(workflowSpec).toContain("previews and applies deterministic assistant organization");
    expect(workflowSpec).toContain("audits and applies deterministic metadata fixes");
    expect(workflowSpec).toContain("auto-tags an album through the offline native task pipeline");
    expect(workflowSpec).toContain("converts a title into artist and title tags through the renderer");
    expect(workflowSpec).toContain("numbers tracks through the renderer and native batch writer");
    expect(workflow).toContain("npm run test:e2e");
    expect(workflow).toContain("wdio-macos");
    expect(workflow).toContain("wdio-windows");
    expect(workflow).toContain("wdio-linux");
    expect(workflow).toContain("Smoke macOS app bundle and DMG");
    expect(workflow).toContain("Smoke Windows NSIS installer");
    expect(workflow).toContain("Smoke Linux AppImage bundle");
    expect(workflow).toContain("Smoke Linux deb installer");
  });
});
