#!/usr/bin/env node
/**
 * Aggregate checkpoint batch results into a single JSON report
 * suitable for generate-corruption-report.js.
 *
 * Usage: node scripts/aggregate-checkpoint.js <checkpoint-dir> [output.json]
 */
const fs = require("fs");
const path = require("path");

const checkpointDir = process.argv[2];
const outputFile = process.argv[3] || path.join(__dirname, "../reports", `flac-metadata-report-${new Date().toISOString().replace(/[:.]/g, "-")}.json`);

if (!checkpointDir) {
  console.error("Usage: node scripts/aggregate-checkpoint.js <checkpoint-dir> [output.json]");
  process.exit(1);
}

const manifest = JSON.parse(fs.readFileSync(path.join(checkpointDir, "manifest.json"), "utf8"));
const resultsDir = path.join(checkpointDir, "results");

if (!fs.existsSync(resultsDir)) {
  console.error("No results/ directory in", checkpointDir);
  process.exit(1);
}

const batchFiles = fs.readdirSync(resultsDir)
  .filter(f => f.endsWith(".json"))
  .sort();

const diagnosis = {};
let total = 0, clean = 0, minor = 0, medium = 0, broken = 0;

for (const batchFile of batchFiles) {
  const batch = JSON.parse(fs.readFileSync(path.join(resultsDir, batchFile), "utf8"));
  for (const result of batch.results) {
    const d = result.diagnosis;
    if (!d) continue;

    const artist = d.artist || "Unknown Artist";
    const album = d.album || "Unknown Album";
    const track = d.track || "unknown.flac";
    const bucket = result.bucket || d.bucket || "clean";

    if (!diagnosis[artist]) diagnosis[artist] = {};
    if (!diagnosis[artist][album]) diagnosis[artist][album] = {};

    diagnosis[artist][album][track] = [bucket];

    total++;
    if (bucket === "clean") clean++;
    else if (bucket === "minor") minor++;
    else if (bucket === "medium") medium++;
    else if (bucket === "broken") broken++;
  }
}

const report = {
  generatedAt: new Date().toISOString(),
  mode: manifest.mode || "doctor",
  sourceRoot: manifest.topDir || "",
  summary: { total, clean, minor, medium, broken, fixed: 0, fixFailed: 0 },
  diagnosis,
};

fs.mkdirSync(path.dirname(outputFile), { recursive: true });
fs.writeFileSync(outputFile, JSON.stringify(report, null, 2), "utf8");

console.log(`Aggregated ${batchFiles.length} batches → ${outputFile}`);
console.log(`Total: ${total} | Clean: ${clean} | Minor: ${minor} | Medium: ${medium} | Broken: ${broken}`);
