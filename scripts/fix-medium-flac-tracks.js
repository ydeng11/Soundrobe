#!/usr/bin/env node
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

function usage() {
  console.error("Usage: fix-medium-flac-tracks.js <library> --from-report <report.json> [--work-dir <dir>] [--include-warnings] [--apply --yes] [--skip-post-doctor]");
}

function parseArgs(argv) {
  const args = {
    library: argv[2],
    reportPath: null,
    workDir: null,
    includeWarnings: false,
    apply: false,
    yes: false,
    skipPostDoctor: false,
  };

  for (let i = 3; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--from-report") args.reportPath = argv[++i];
    else if (arg === "--work-dir") args.workDir = argv[++i];
    else if (arg === "--include-warnings") args.includeWarnings = true;
    else if (arg === "--apply") args.apply = true;
    else if (arg === "--yes") args.yes = true;
    else if (arg === "--skip-post-doctor") args.skipPostDoctor = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }

  if (!args.library || !args.reportPath) {
    usage();
    process.exit(2);
  }
  if (args.apply && !args.yes) {
    throw new Error("Refusing to apply without --yes");
  }

  args.library = path.resolve(args.library);
  args.reportPath = path.resolve(args.reportPath);
  args.workDir = path.resolve(args.workDir || path.join(os.tmpdir(), "soundrobe-medium-flac-fix"));
  return args;
}

function readReport(reportPath, library) {
  const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
  if (path.resolve(report.sourceRoot) !== library) {
    throw new Error("report sourceRoot does not match selected library");
  }
  return report;
}

function candidateEntries(report, includeWarnings) {
  return Object.values(report.details || {}).filter((entry) => {
    if (entry.bucket !== "medium") return false;
    const issues = entry.issues || [];
    if (issues.includes("strict-decode-invalid-playable")) return true;
    return includeWarnings && issues.includes("strict-decode-warning");
  });
}

function writeJson(filePath, data) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, JSON.stringify(data, null, 2));
}

function writePlan(args, candidates) {
  const plan = {
    generatedAt: new Date().toISOString(),
    apply: args.apply,
    sourceRoot: args.library,
    candidates: candidates.map((candidate) => ({
      relativePath: candidate.relativePath,
      issues: candidate.issues || [],
    })),
  };
  writeJson(path.join(args.workDir, "medium-fix-plan.json"), plan);
  return plan;
}

function recoverOne(library, workDir, candidate) {
  const source = path.join(library, candidate.relativePath);
  const temp = path.join(workDir, `${candidate.relativePath.replace(/[\\/]/g, "__")}.recovered.flac`);
  fs.mkdirSync(path.dirname(temp), { recursive: true });

  execFileSync("ffmpeg", ["-y", "-i", source, "-map", "0:a:0", "-c:a", "flac", temp], { stdio: "ignore" });
  execFileSync("flac", ["-t", "-w", temp], { stdio: "ignore" });
  fs.copyFileSync(temp, source);
}

function main() {
  const args = parseArgs(process.argv);
  const report = readReport(args.reportPath, args.library);
  const candidates = candidateEntries(report, args.includeWarnings);
  const plan = writePlan(args, candidates);

  console.log(`Mode: ${args.apply ? "apply" : "dry-run"}`);
  console.log(`Candidates: ${plan.candidates.length}`);
  for (const candidate of plan.candidates) {
    console.log(candidate.relativePath);
  }

  const results = { recovered: [], failed: [] };
  if (args.apply) {
    for (const candidate of candidates) {
      try {
        recoverOne(args.library, args.workDir, candidate);
        results.recovered.push({ relativePath: candidate.relativePath });
      } catch (error) {
        results.failed.push({
          relativePath: candidate.relativePath,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }
    writeJson(path.join(args.workDir, "medium-fix-results.json"), results);
    console.log(`Recovered: ${results.recovered.length}`);
    console.log(`Failed: ${results.failed.length}`);
  }
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
