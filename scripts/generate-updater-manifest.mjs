#!/usr/bin/env node

import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const REPOSITORY = "ydeng11/Soundrobe";

function walk(root) {
  const files = [];
  for (const name of readdirSync(root)) {
    const path = join(root, name);
    if (statSync(path).isDirectory()) files.push(...walk(path));
    else files.push(path);
  }
  return files;
}

function semverFromTag(tag) {
  const match = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.exec(
    tag,
  );
  if (!match) throw new Error("tag must match v<semver>");
  return tag.slice(1);
}

function platformSpecs(version) {
  return [
    ["darwin-aarch64-app", `soundrobe-${version}-macos-arm64.app.tar.gz`],
    ["darwin-x86_64-app", `soundrobe-${version}-macos-intel.app.tar.gz`],
    ["linux-aarch64-appimage", `soundrobe-${version}-linux-arm64.AppImage`],
    ["linux-aarch64-deb", `soundrobe-${version}-linux-arm64.deb`],
    ["linux-x86_64-appimage", `soundrobe-${version}-linux-x64.AppImage`],
    ["linux-x86_64-deb", `soundrobe-${version}-linux-x64.deb`],
    ["windows-x86_64-nsis", `soundrobe-${version}-windows-x64-setup.exe`],
  ];
}

function exactlyOne(files, expected, kind) {
  const matches = files.filter((path) => basename(path) === expected);
  if (matches.length === 0) throw new Error(`missing updater ${kind}: ${expected}`);
  if (matches.length > 1) throw new Error(`duplicate updater ${kind}: ${expected}`);
  return matches[0];
}

export function buildUpdaterManifest({
  assetsDir,
  tag,
  pubDate = null,
  notes = "",
}) {
  const version = semverFromTag(tag);
  const files = walk(resolve(assetsDir));
  const platforms = {};

  for (const [target, artifactName] of platformSpecs(version)) {
    exactlyOne(files, artifactName, "artifact");
    const signaturePath = exactlyOne(files, `${artifactName}.sig`, "signature");
    const signature = readFileSync(signaturePath, "utf8").trim();
    if (!signature) throw new Error(`empty updater signature: ${artifactName}.sig`);
    platforms[target] = {
      url: `https://github.com/${REPOSITORY}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(artifactName)}`,
      signature,
    };
  }

  return {
    version,
    notes,
    ...(pubDate ? { pub_date: pubDate } : {}),
    platforms,
  };
}

function option(args, name, required = true) {
  const index = args.indexOf(name);
  const value = index >= 0 ? args[index + 1] : undefined;
  if (required && !value) throw new Error(`missing required argument ${name}`);
  return value;
}

function main(args) {
  const assetsDir = option(args, "--assets");
  const tag = option(args, "--tag");
  const output = option(args, "--output");
  const manifest = buildUpdaterManifest({
    assetsDir,
    tag,
    pubDate: option(args, "--pub-date", false) ?? null,
    notes: option(args, "--notes", false) ?? "",
  });
  writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
