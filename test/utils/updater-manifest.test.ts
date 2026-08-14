// @vitest-environment node
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
// @ts-expect-error The release script is plain ESM so GitHub Actions can run it directly.
import {
  buildUpdaterManifest,
  releaseNotesFromChangelog,
} from "../../scripts/generate-updater-manifest.mjs";

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function fixtureRoot(): string {
  const root = mkdtempSync(join(tmpdir(), "soundrobe-updater-manifest-"));
  roots.push(root);
  return root;
}

function writeArtifacts(root: string, version = "1.2.3") {
  const files = [
    `soundrobe-${version}-macos-arm64.app.tar.gz`,
    `soundrobe-${version}-macos-intel.app.tar.gz`,
    `soundrobe-${version}-windows-x64-setup.exe`,
    `soundrobe-${version}-linux-x64.AppImage`,
    `soundrobe-${version}-linux-x64.deb`,
    `soundrobe-${version}-linux-arm64.AppImage`,
    `soundrobe-${version}-linux-arm64.deb`,
  ];
  for (const [index, file] of files.entries()) {
    writeFileSync(join(root, file), `artifact-${index}`);
    writeFileSync(join(root, `${file}.sig`), `signature-${index}\n`);
  }
}

describe("updater manifest generator", () => {
  it("extracts plain-text notes from the exact versioned changelog section", () => {
    const changelog = `# Changelog

## [Unreleased]

- Future work

## [1.2.3] - 2026-08-14

### Added

- **Signed updates** — Install from [GitHub](https://github.com/example).
- Uses \`latest.json\`.

## [1.2.2] - 2026-08-01

- Previous release
`;

    expect(releaseNotesFromChangelog(changelog, "1.2.3")).toBe(
      "Added\n\n- Signed updates — Install from GitHub.\n- Uses latest.json.",
    );
    expect(() => releaseNotesFromChangelog(changelog, "1.2.4")).toThrow(
      /missing changelog section.*1\.2\.4/i,
    );
  });

  it("builds deterministic installer-aware targets with tag-specific URLs and inline signatures", () => {
    const root = fixtureRoot();
    writeArtifacts(root);

    const manifest = buildUpdaterManifest({
      assetsDir: root,
      tag: "v1.2.3",
      pubDate: "2026-08-14T12:00:00Z",
      notes: "Release notes",
    });

    expect(Object.keys(manifest.platforms)).toEqual([
      "darwin-aarch64-app",
      "darwin-x86_64-app",
      "linux-aarch64-appimage",
      "linux-aarch64-deb",
      "linux-x86_64-appimage",
      "linux-x86_64-deb",
      "windows-x86_64-nsis",
    ]);
    expect(manifest.version).toBe("1.2.3");
    expect(manifest.platforms["darwin-aarch64-app"]).toEqual({
      url: "https://github.com/ydeng11/Soundrobe/releases/download/v1.2.3/soundrobe-1.2.3-macos-arm64.app.tar.gz",
      signature: "signature-0",
    });
    expect(manifest.platforms["linux-x86_64-deb"].url).toMatch(
      /\/soundrobe-1\.2\.3-linux-x64\.deb$/,
    );
    expect(manifest.platforms["windows-x86_64-nsis"].signature).toBe(
      "signature-2",
    );
  });

  it("fails loudly when an updater artifact or signature is missing", () => {
    const root = fixtureRoot();
    writeArtifacts(root);
    rmSync(join(root, "soundrobe-1.2.3-linux-arm64.deb.sig"));

    expect(() =>
      buildUpdaterManifest({ assetsDir: root, tag: "v1.2.3" }),
    ).toThrow(/missing updater signature.*linux-arm64\.deb/i);
  });

  it("fails loudly when downloaded workflow artifacts contain a duplicate", () => {
    const root = fixtureRoot();
    writeArtifacts(root);
    const duplicateDir = join(root, "duplicate");
    mkdirSync(duplicateDir);
    writeFileSync(
      join(duplicateDir, "soundrobe-1.2.3-macos-arm64.app.tar.gz"),
      "duplicate",
    );

    expect(() =>
      buildUpdaterManifest({ assetsDir: root, tag: "v1.2.3" }),
    ).toThrow(/duplicate updater artifact.*macos-arm64/i);
  });

  it("rejects a tag that is not an exact SemVer release", () => {
    const root = fixtureRoot();
    expect(() =>
      buildUpdaterManifest({ assetsDir: root, tag: "latest" }),
    ).toThrow(/tag must match v<semver>/i);
  });

  it("rejects prerelease and build-metadata tags so latest stays stable", () => {
    const root = fixtureRoot();

    expect(() =>
      buildUpdaterManifest({ assetsDir: root, tag: "v1.2.3-beta.1" }),
    ).toThrow(/stable release/i);
    expect(() =>
      buildUpdaterManifest({ assetsDir: root, tag: "v1.2.3+nightly" }),
    ).toThrow(/stable release/i);
  });
});
