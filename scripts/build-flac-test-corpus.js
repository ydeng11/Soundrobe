#!/usr/bin/env node
/**
 * Build a local, reproducible FLAC test corpus from a real music library.
 *
 * Copies files only into a disposable destination such as
 * /private/tmp/soundrobe-flac-corpus, preserving artist/album paths.
 */
const fs = require("fs");
const path = require("path");

const DEFAULT_SOURCE = "/Volumes/downloads/music";
const DEFAULT_DEST = "/private/tmp/soundrobe-flac-corpus";
const DEFAULT_COUNT = 200;
const DEFAULT_SEED = "flac-bucket-v1";
const DEFAULT_REPORTS_DIR = path.resolve(process.cwd(), "reports");
const DEST_BASENAME = "soundrobe-flac-corpus";

function parseArgs(argv) {
  const options = {
    source: DEFAULT_SOURCE,
    dest: DEFAULT_DEST,
    count: DEFAULT_COUNT,
    seed: DEFAULT_SEED,
    reportsDir: DEFAULT_REPORTS_DIR,
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const next = () => {
      if (i + 1 >= argv.length) throw new Error(`Missing value for ${arg}`);
      return argv[++i];
    };

    if (arg === "--source") options.source = next();
    else if (arg.startsWith("--source=")) options.source = arg.slice("--source=".length);
    else if (arg === "--dest") options.dest = next();
    else if (arg.startsWith("--dest=")) options.dest = arg.slice("--dest=".length);
    else if (arg === "--count") options.count = Number(next());
    else if (arg.startsWith("--count=")) options.count = Number(arg.slice("--count=".length));
    else if (arg === "--seed") options.seed = next();
    else if (arg.startsWith("--seed=")) options.seed = arg.slice("--seed=".length);
    else if (arg === "--reports-dir") options.reportsDir = next();
    else if (arg.startsWith("--reports-dir=")) options.reportsDir = arg.slice("--reports-dir=".length);
    else if (arg === "--help" || arg === "-h") {
      printUsage();
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  if (!Number.isInteger(options.count) || options.count < 1) {
    throw new Error("--count must be a positive integer");
  }

  options.source = path.resolve(options.source);
  options.dest = path.resolve(options.dest);
  options.reportsDir = path.resolve(options.reportsDir);
  return options;
}

function printUsage() {
  console.log("Usage: node scripts/build-flac-test-corpus.js [--source DIR] [--dest DIR] [--count N] [--seed TEXT] [--reports-dir DIR]");
}

function isInside(parent, candidate) {
  const rel = path.relative(parent, candidate);
  return rel === "" || (!!rel && !rel.startsWith("..") && !path.isAbsolute(rel));
}

function assertSafePaths(source, dest) {
  if (!fs.existsSync(source) || !fs.statSync(source).isDirectory()) {
    throw new Error(`Source directory does not exist: ${source}`);
  }
  if (isInside(source, dest)) {
    throw new Error(`Refusing to write destination inside source: ${dest}`);
  }
  if (path.basename(dest) !== DEST_BASENAME) {
    throw new Error(`Refusing to recreate destination unless basename is ${DEST_BASENAME}: ${dest}`);
  }
}

function walkFlac(dir, root = dir, files = []) {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walkFlac(full, root, files);
    } else if (entry.isFile() && /\.flac$/i.test(entry.name)) {
      files.push(toPosix(path.relative(root, full)));
    }
  }
  return files;
}

function toPosix(value) {
  return value.split(path.sep).join("/");
}

function findLatestReport(reportsDir) {
  if (!fs.existsSync(reportsDir)) return null;
  const reports = fs.readdirSync(reportsDir)
    .filter((file) => /^flac-metadata-report-.*\.txt$/.test(file))
    .sort()
    .reverse();
  return reports.length > 0 ? path.join(reportsDir, reports[0]) : null;
}

function addTarget(targets, relativePath, tags) {
  if (!relativePath || !/\.flac$/i.test(relativePath)) return;
  const normalized = relativePath.replace(/^[/\\]+/, "").split(/[\\/]+/).join("/");
  const existing = targets.get(normalized);
  if (existing) {
    for (const tag of tags) existing.tags.add(tag);
  } else {
    targets.set(normalized, { relativePath: normalized, tags: new Set(tags) });
  }
}

function parseReportTargets(reportPath) {
  const targets = new Map();
  if (!reportPath || !fs.existsSync(reportPath)) return [];

  const lines = fs.readFileSync(reportPath, "utf8").split(/\r?\n/);
  for (const line of lines) {
    let match = line.match(/^  ALERT_STRICT_DECODE_FAIL\s+(.+?)\s+\(/);
    if (match) {
      addTarget(targets, match[1], ["strict-alert"]);
      continue;
    }

    match = line.match(/^  WARNING_STRICT_DECODE\s+(.+?)\s+\(/);
    if (match) {
      addTarget(targets, match[1], ["strict-warning"]);
      continue;
    }

    match = line.match(/^  METADATA_GAP\s+(.+?)\s+\(/);
    if (match) {
      addTarget(targets, match[1], ["metadata-gap"]);
      continue;
    }

    match = line.match(/^  VORBIS_MISMATCH\s+(.+?)\s*$/);
    if (match) {
      addTarget(targets, match[1], ["vorbis-mismatch"]);
      continue;
    }

    match = line.match(/^  AUDIO_CORRUPTED\s+(.+?)\s+\(/);
    if (match) {
      addTarget(targets, match[1], ["audio-sync"]);
      continue;
    }

    match = line.match(/^  CORRUPTED_PICTURE\s+(.+?)\s+\(/);
    if (match) {
      addTarget(targets, match[1], ["corrupted-picture"]);
      continue;
    }

    match = line.match(/^  BROKEN_CHAIN\s+(.+?)\s+\(/);
    if (match) {
      addTarget(targets, match[1], ["broken-chain"]);
    }
  }

  return [...targets.values()].map((target) => ({
    relativePath: target.relativePath,
    tags: [...target.tags].sort(),
  }));
}

function hashString(value) {
  let hash = 2166136261;
  for (let i = 0; i < value.length; i++) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function seededRandom(seed) {
  let state = hashString(seed) || 1;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

function deterministicShuffle(values, seed) {
  const shuffled = [...values];
  const random = seededRandom(seed);
  for (let i = shuffled.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]];
  }
  return shuffled;
}

function copyEntry(sourceRoot, destRoot, relativePath, reason, tags) {
  const sourcePath = path.join(sourceRoot, ...relativePath.split("/"));
  const destPath = path.join(destRoot, ...relativePath.split("/"));
  fs.mkdirSync(path.dirname(destPath), { recursive: true });
  fs.copyFileSync(sourcePath, destPath);
  return {
    relativePath,
    sourcePath,
    destPath,
    reason,
    tags,
    sizeBytes: fs.statSync(destPath).size,
  };
}

function buildCorpus(options) {
  assertSafePaths(options.source, options.dest);
  const reportPath = findLatestReport(options.reportsDir);
  const targetCandidates = parseReportTargets(reportPath);
  const selected = new Set();
  const entries = [];
  const missingTargets = [];

  fs.rmSync(options.dest, { recursive: true, force: true });
  fs.mkdirSync(options.dest, { recursive: true });

  for (const target of targetCandidates) {
    if (entries.length >= options.count) break;
    const sourcePath = path.join(options.source, ...target.relativePath.split("/"));
    if (!fs.existsSync(sourcePath) || !fs.statSync(sourcePath).isFile()) {
      missingTargets.push(target);
      continue;
    }
    if (selected.has(target.relativePath)) continue;
    selected.add(target.relativePath);
    entries.push(copyEntry(options.source, options.dest, target.relativePath, "targeted", target.tags));
  }

  if (entries.length < options.count) {
    const allFlacs = walkFlac(options.source).sort();
    for (const relativePath of deterministicShuffle(allFlacs, options.seed)) {
      if (entries.length >= options.count) break;
      if (selected.has(relativePath)) continue;
      selected.add(relativePath);
      entries.push(copyEntry(options.source, options.dest, relativePath, "random", []));
    }
  }

  const manifest = {
    createdAt: new Date().toISOString(),
    sourceRoot: options.source,
    destRoot: options.dest,
    requestedCount: options.count,
    copiedCount: entries.length,
    seed: options.seed,
    reportPath,
    entries,
    missingTargets,
  };

  fs.writeFileSync(path.join(options.dest, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n", "utf8");
  return manifest;
}

function main() {
  try {
    const options = parseArgs(process.argv.slice(2));
    const manifest = buildCorpus(options);
    console.log(`Corpus: ${manifest.destRoot}`);
    console.log(`Copied: ${manifest.copiedCount}/${manifest.requestedCount}`);
    console.log(`Targeted: ${manifest.entries.filter((entry) => entry.reason === "targeted").length}`);
    console.log(`Random: ${manifest.entries.filter((entry) => entry.reason === "random").length}`);
    if (manifest.missingTargets.length > 0) {
      console.log(`Missing targeted files: ${manifest.missingTargets.length}`);
    }
    console.log(`Manifest: ${path.join(manifest.destRoot, "manifest.json")}`);
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
}

main();
