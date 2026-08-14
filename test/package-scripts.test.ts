// @vitest-environment node
import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));

interface PackageJson {
  name: string;
  author: string;
  homepage: string;
  scripts: Record<string, string>;
}

function readPackageJson(): PackageJson {
  const packageJsonPath = resolve(__dirname, "../package.json");
  return JSON.parse(readFileSync(packageJsonPath, "utf8")) as PackageJson;
}

describe("package scripts", () => {
  it("keeps the Soundrobe identity synchronized across app manifests", () => {
    const packageJson = readPackageJson();
    const tauriConfig = JSON.parse(
      readFileSync(resolve(__dirname, "../src-tauri/tauri.conf.json"), "utf8"),
    ) as {
      productName: string;
      identifier: string;
      app: { windows: Array<{ title: string }> };
      bundle: { publisher: string; longDescription: string };
    };
    const cargoToml = readFileSync(
      resolve(__dirname, "../src-tauri/Cargo.toml"),
      "utf8",
    );

    expect(packageJson.name).toBe("soundrobe");
    expect(packageJson.author).toBe("Soundrobe Contributors");
    expect(packageJson.homepage).toBe("https://github.com/ydeng11/Soundrobe");
    expect(tauriConfig.productName).toBe("Soundrobe");
    expect(tauriConfig.identifier).toBe("com.ihelio.soundrobe");
    expect(tauriConfig.app.windows[0]?.title).toBe("Soundrobe");
    expect(tauriConfig.bundle.publisher).toBe("Soundrobe Contributors");
    expect(tauriConfig.bundle.longDescription).toMatch(/^Soundrobe is /);
    expect(cargoToml).toMatch(/^name = "soundrobe"$/m);
    expect(cargoToml).toMatch(/^name = "soundrobe_lib"$/m);
    expect(cargoToml).toMatch(/^authors = \["Soundrobe Contributors"\]$/m);
    expect(cargoToml).toMatch(/^homepage = "https:\/\/github\.com\/ydeng11\/Soundrobe"$/m);
  });

  it("keeps Tauri as the only application and packaging backend", () => {
    const legacyPaths = [
      "tests",
      "pyproject.toml",
      "uv.lock",
      "packaging/homebrew",
      "electron",
      "electron-builder.yml",
      "dist-electron",
      "e2e",
    ];

    for (const legacyPath of legacyPaths) {
      expect(existsSync(resolve(__dirname, "..", legacyPath)), legacyPath).toBe(false);
    }
  });

  it("keeps the app version synchronized across Tauri, Rust, and npm", () => {
    const packageJson = readFileSync(resolve(__dirname, "../package.json"), "utf8");
    const { version } = JSON.parse(packageJson) as { version: string };
    const tauriConfig = JSON.parse(
      readFileSync(resolve(__dirname, "../src-tauri/tauri.conf.json"), "utf8"),
    ) as { version: string };
    const cargoToml = readFileSync(
      resolve(__dirname, "../src-tauri/Cargo.toml"),
      "utf8",
    );
    const providers = readFileSync(
      resolve(__dirname, "../src-tauri/src/state/providers.rs"),
      "utf8",
    );
    const lyrics = readFileSync(
      resolve(__dirname, "../src-tauri/src/commands/lyrics.rs"),
      "utf8",
    );

    expect(version).toMatch(/^\d+\.\d+\.\d+$/);
    expect(tauriConfig.version).toBe(version);
    expect(cargoToml).toMatch(
      new RegExp(`^version = "${version.replaceAll(".", "\\.")}"$`, "m"),
    );
    expect(providers).toContain('env!("CARGO_PKG_VERSION")');
    expect(lyrics).toContain('env!("CARGO_PKG_VERSION")');
  });

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
    const justfile = readFileSync(resolve(__dirname, "../Justfile"), "utf8");

    expect(justfile).toContain('set dotenv-path := ".env.local"');
    for (const recipe of ["install", "dev", "build", "test", "typecheck", "check", "dist"]) {
      expect(justfile).toMatch(new RegExp(`^${recipe}(?: [^:]*)?:`, "m"));
      expect(justfile).toContain(`DEPRECATED: use 'just ${recipe}'`);
    }
    expect(justfile).toContain("smoke-openrouter:");
    expect(justfile).toContain("fe-smoke-openrouter:");
    expect(justfile).toContain("live_openrouter_returns_schema_constrained_json");
    expect(justfile).toContain("fe-smoke-assistant:");
    expect(justfile).toContain("live-openrouter.spec.ts");
    expect(justfile).toContain("fe-smoke-cover-picker:");
    expect(justfile).toContain("live-cover-picker.spec.ts");
  });

  it("declares every required unsigned Tauri bundle target", () => {
    const { scripts } = readPackageJson();
    const justfile = readFileSync(resolve(__dirname, "../Justfile"), "utf8");
    const tauriConfig = JSON.parse(
      readFileSync(resolve(__dirname, "../src-tauri/tauri.conf.json"), "utf8"),
    ) as { bundle: { category: string } };

    expect(scripts["dist:mac"]).toBe("tauri build --bundles app,dmg");
    expect(scripts["dist:win"]).toBe("tauri build --bundles nsis");
    expect(scripts["dist:linux"]).toBe("tauri build --bundles appimage,deb");
    expect(justfile).toContain("fe-dist-mac-intel:");
    expect(justfile).toContain("--target x86_64-apple-darwin");
    expect(tauriConfig.bundle.category).toBe("Music");
  });

  it("keeps updater signing release-only and renderer access behind app commands", () => {
    const tauriConfig = JSON.parse(
      readFileSync(resolve(__dirname, "../src-tauri/tauri.conf.json"), "utf8"),
    ) as {
      bundle: { createUpdaterArtifacts?: boolean };
      plugins: { updater: { endpoints: string[]; pubkey: string } };
    };
    const releaseConfig = JSON.parse(
      readFileSync(
        resolve(__dirname, "../src-tauri/tauri.updater.conf.json"),
        "utf8",
      ),
    ) as { bundle: { createUpdaterArtifacts: boolean } };
    const capability = readFileSync(
      resolve(__dirname, "../src-tauri/capabilities/default.json"),
      "utf8",
    );
    const cargoToml = readFileSync(
      resolve(__dirname, "../src-tauri/Cargo.toml"),
      "utf8",
    );

    expect(tauriConfig.bundle.createUpdaterArtifacts).toBeUndefined();
    expect(releaseConfig.bundle.createUpdaterArtifacts).toBe(true);
    expect(tauriConfig.plugins.updater.endpoints).toEqual([
      "https://github.com/ydeng11/Soundrobe/releases/latest/download/latest.json",
    ]);
    const temporaryKeyMarker = ["__SOUNDROBE", "UPDATER_PUBLIC_KEY__"].join("_");
    expect(tauriConfig.plugins.updater.pubkey).not.toContain(temporaryKeyMarker);
    expect(tauriConfig.plugins.updater.pubkey.length).toBeGreaterThan(80);
    expect(cargoToml).toMatch(/^tauri-plugin-updater = "2\.10\.1"$/m);
    expect(capability).not.toContain("updater:");
  });

  it("keeps pull request status checks limited to tests", () => {
    const workflow = readFileSync(
      resolve(__dirname, "../.github/workflows/tests.yml"),
      "utf8",
    );

    expect(workflow).toMatch(/^name: Tests$/m);
    expect(workflow).toContain("macOS ARM");
    expect(workflow).toContain("macos-latest");
    expect(workflow).toContain("windows-latest");
    expect(workflow).toContain("ubuntu-22.04");
    expect(workflow).toContain("npm test && npm run typecheck");
    expect(workflow).toContain("npm run test:e2e");
    expect(workflow).toContain("name: Required tests");
    expect(workflow).not.toContain("npm run dist:");
    expect(workflow).not.toContain("createCommitStatus");
  });

  it("checks nightly for a new app version before publishing release bundles", () => {
    const releaseWorkflow = readFileSync(
      resolve(__dirname, "../.github/workflows/release.yml"),
      "utf8",
    );

    expect(releaseWorkflow).toMatch(/^name: Release$/m);
    expect(releaseWorkflow).toContain('tags:\n      - "v*.*.*"');
    expect(releaseWorkflow).toContain("schedule:");
    expect(releaseWorkflow).toContain('cron: "');
    expect(releaseWorkflow).toContain("workflow_dispatch:");
    expect(releaseWorkflow).toContain("contents: write");
    expect(releaseWorkflow).toContain("PUSHED_TAG");
    expect(releaseWorkflow).toContain("package_version");
    expect(releaseWorkflow).toContain("gh release view");
    expect(releaseWorkflow).toContain("release_needed=true");
    expect(releaseWorkflow).toContain("needs.version.outputs.release_needed == 'true'");
    expect(releaseWorkflow).toContain("macos-arm64");
    expect(releaseWorkflow).toContain("macos-intel");
    expect(releaseWorkflow).toContain("windows-x64");
    expect(releaseWorkflow).toContain("linux-x64");
    expect(releaseWorkflow).toContain("linux-arm64");
    expect(releaseWorkflow).toContain("release_os: macos");
    expect(releaseWorkflow).toContain("release_os: linux");
    expect(releaseWorkflow).toContain("release_arch: arm64");
    expect(releaseWorkflow).toContain("release_arch: intel");
    expect(releaseWorkflow).toContain("release_arch: x64");
    expect(releaseWorkflow).toContain("ubuntu-24.04-arm");
    expect(releaseWorkflow).toContain("Rename macOS release bundles");
    expect(releaseWorkflow).toContain("Rename Windows release bundles");
    expect(releaseWorkflow).toContain("Rename Linux release bundles");
    expect(releaseWorkflow).toContain(
      "soundrobe-${version}-${{ matrix.release_os }}-${{ matrix.release_arch }}.dmg",
    );
    expect(releaseWorkflow).toContain(
      "soundrobe-${version}-${{ matrix.release_os }}-${{ matrix.release_arch }}.AppImage",
    );
    expect(releaseWorkflow).toContain(
      "soundrobe-${version}-${{ matrix.release_os }}-${{ matrix.release_arch }}.deb",
    );
    expect(releaseWorkflow).toContain("release-${{ matrix.artifact }}");
    expect(releaseWorkflow).toContain("TAURI_SIGNING_PRIVATE_KEY");
    expect(releaseWorkflow).toContain('TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ""');
    expect(releaseWorkflow).toContain("tauri.updater.conf.json");
    expect(releaseWorkflow).toContain('APPLE_SIGNING_IDENTITY: "-"');
    expect(releaseWorkflow).toContain("codesign --verify --deep --strict");
    expect(releaseWorkflow).toContain("actions/download-artifact@v4");
    expect(releaseWorkflow).toContain("generate-updater-manifest.mjs");
    expect(releaseWorkflow).toContain("release-assets/latest.json");
    expect(releaseWorkflow).toContain("softprops/action-gh-release@v2");
    expect(releaseWorkflow).toContain("release-assets/**/*.dmg");
    expect(releaseWorkflow).toContain("release-assets/**/*-setup.exe");
    expect(releaseWorkflow).toContain("release-assets/**/*.AppImage");
    expect(releaseWorkflow).toContain("release-assets/**/*.deb");
    expect(releaseWorkflow).toContain("release-assets/**/*.sig");
    expect(releaseWorkflow).toContain("release-assets/**/*.app.tar.gz");
  });

  it("runs test-only embedded WebdriverIO coverage on every desktop platform", () => {
    const { scripts } = readPackageJson();
    const workflow = readFileSync(
      resolve(__dirname, "../.github/workflows/tests.yml"),
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
    expect(workflow).toContain("Desktop (${{ matrix.platform }})");
    expect(workflow).toContain("macOS ARM");
    expect(workflow).toContain("macOS Intel");
    expect(workflow).toContain("Windows");
    expect(workflow).toContain("Linux");
    expect(workflow).not.toContain("Smoke macOS app bundle and DMG");
    expect(workflow).not.toContain("Smoke Windows NSIS installer");
    expect(workflow).not.toContain("Smoke Linux AppImage bundle");
    expect(workflow).not.toContain("Smoke Linux deb installer");
  });
});
