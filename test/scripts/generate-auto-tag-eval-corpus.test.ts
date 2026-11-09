// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const scriptPath = path.join(repoRoot, "scripts/generate_auto_tag_eval_corpus.py");
const fixturePath = path.join(repoRoot, "test/fixtures/tauri/media-corpus/minimal.wav");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe("generate_auto_tag_eval_corpus.py", () => {
  it.each([
    ["--source-root", (source: string) => ["--source-root", source], {}],
    ["SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT", (source: string) => [], { useEnv: true }],
  ])("accepts %s and writes relative corpus paths", (_label, argsFor, options) => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-generate-corpus-"));
    temporaryRoots.push(root);
    const source = path.join(root, "curated");
    const album = path.join(source, "Ariana Grande", "Test Album");
    fs.mkdirSync(album, { recursive: true });
    fs.copyFileSync(fixturePath, path.join(album, "01.wav"));
    fs.mkdirSync(path.join(root, "test/fixtures/tauri/auto-tag-eval"), { recursive: true });

    execFileSync("python3", [scriptPath, ...argsFor(source)], {
      cwd: root,
      env: options.useEnv
        ? { ...process.env, SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: source }
        : process.env,
      encoding: "utf8",
    });

    const corpus = JSON.parse(fs.readFileSync(
      path.join(root, "test/fixtures/tauri/auto-tag-eval/corpus.json"),
      "utf8",
    ));
    expect(corpus.sourceRoot).toBe(".");
    expect(corpus.cases).toHaveLength(1);
    expect(corpus.cases[0].sourceRelativeFolder).toBe("Ariana Grande/Test Album");
    expect(corpus.cases[0].sourceRelativeFolder).not.toContain("/Users/");
  });

  it("exports selected CJK artists and independent compilation albums without blessing captured tags", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-expanded-corpus-"));
    temporaryRoots.push(root);
    const source = path.join(root, "source");
    for (const folder of ["盧廣仲/Album/CD1", "盧廣仲/Album/CD2", "Compilations/OST A", "Compilations/OST B", "Unselected/Album"]) {
      fs.mkdirSync(path.join(source, folder), { recursive: true });
      fs.copyFileSync(fixturePath, path.join(source, folder, "01.wav"));
    }
    const output = path.join(root, "extension");
    execFileSync("python3", [scriptPath, "--source-root", source,
      "--artist", "盧廣仲", "--compilation-container", "Compilations",
      "--output-dir", output, "--corpus-version", "test.extension-1"], { cwd: root, stdio: "pipe" });
    const corpus = JSON.parse(fs.readFileSync(path.join(output, "corpus.json"), "utf8"));
    const ledger = JSON.parse(fs.readFileSync(path.join(output, "expectations.json"), "utf8"));
    expect(corpus).toMatchObject({ corpusVersion: "test.extension-1", summary: { cases: 4, tracks: 4 } });
    expect(corpus.cases.filter((item: { artist: string }) => item.artist === "Various Artists")).toHaveLength(2);
    expect(corpus.cases.filter((item: { releaseType: string }) => item.releaseType === "compilation")).toHaveLength(2);
    expect(new Set(corpus.cases.filter((item: { artist: string }) => item.artist === "盧廣仲")
      .map((item: { releaseGroupId: string }) => item.releaseGroupId)).size).toBe(1);
    expect(ledger.cases.every((item: { status: string; mapping: unknown[] }) =>
      item.status === "unverified" && item.mapping.length === 0)).toBe(true);
    expect(corpus.providerSnapshots).toEqual([]);
    expect(fs.existsSync(path.join(root, "test/fixtures/tauri/auto-tag-eval/corpus.json"))).toBe(false);
  });

  it("freezes only explicitly sampled albums with their reasons and ordered credits", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-selected-corpus-"));
    temporaryRoots.push(root);
    const source = path.join(root, "source");
    for (const folder of ["Artist/keep/Hard Album", "Artist/keep/Unselected", "Compilations/TV OST/CD1", "Compilations/TV OST/CD2"]) {
      fs.mkdirSync(path.join(source, folder), { recursive: true });
      fs.copyFileSync(fixturePath, path.join(source, folder, "01.wav"));
    }
    execFileSync("python3", ["-c", `
import sys
from mutagen.wave import WAVE
from mutagen.id3 import TXXX
audio = WAVE(sys.argv[1])
audio.add_tags()
for key, value in {"ARTISTS": "Ronald Isley;The Isley Brothers;Beyoncé", "ALBUMARTISTS": "Band;Guest", "Discogs Artist Id": "144910"}.items():
    audio.tags.add(TXXX(encoding=3, desc=key, text=[value]))
audio.save()
`, path.join(source, "Artist/keep/Hard Album/01.wav")]);
    const selection = path.join(root, "selection.json");
    fs.writeFileSync(selection, JSON.stringify({ schemaVersion: 1, folders: [
      { folder: "Artist/keep/Hard Album", artist: "Artist", labels: ["conflicting_titles"], reason: "Observed filename/tag disagreement" },
      { folder: "Compilations/TV OST", artist: "Various Artists", releaseType: "compilation", labels: ["multidisc", "tv_ost"], reason: "Separate discs and performers" },
    ] }));
    const output = path.join(root, "export");
    execFileSync("python3", [scriptPath, "--source-root", source, "--selection", selection,
      "--output-dir", output, "--corpus-version", "test.sample-1"], { stdio: "pipe" });
    const corpus = JSON.parse(fs.readFileSync(path.join(output, "corpus.json"), "utf8"));
    expect(corpus.summary).toMatchObject({ cases: 3, tracks: 3 });
    expect(corpus.cases.find((item: { artist: string }) => item.artist === "Artist")).toMatchObject({
      difficultyLabels: ["conflicting_titles"], selectionRationale: "Observed filename/tag disagreement",
      tracks: [expect.objectContaining({ artists: ["Ronald Isley", "The Isley Brothers", "Beyoncé"],
        albumArtists: ["Band", "Guest"], providerIds: { discogsArtistId: "144910" } })],
    });
    expect(corpus.cases.every((item: { tracks: { artists: unknown[]; albumArtists: unknown[] }[] }) =>
      item.tracks.every((track) => Array.isArray(track.artists) && Array.isArray(track.albumArtists)))).toBe(true);
    expect(corpus.selectionManifest).toBe("selection.json");
    expect(fs.readFileSync(path.join(output, "selection.json"), "utf8")).toBe(fs.readFileSync(selection, "utf8"));
    expect(corpus.cases.every((item: { sourceRelativeFolder: string }) => !item.sourceRelativeFolder.includes("Unselected"))).toBe(true);
    fs.writeFileSync(selection, JSON.stringify({ schemaVersion: 1, folders: [
      { folder: "../outside", artist: "Artist", labels: [], reason: "invalid" },
    ] }));
    expect(() => execFileSync("python3", [scriptPath, "--source-root", source, "--selection", selection,
      "--output-dir", output], { stdio: "pipe" })).toThrow(/escapes source root/);
  });
});
