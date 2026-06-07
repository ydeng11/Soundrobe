#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(__dirname, "..");

function binName(name) {
  return process.platform === "win32" ? `${name}.cmd` : name;
}

function run(command, args) {
  return spawnSync(command, args, {
    cwd: projectRoot,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
}

const vitestArgs = process.argv.slice(2);
const vitest = run(join(projectRoot, "node_modules", ".bin", binName("vitest")), vitestArgs);
const vitestStatus = vitest.status ?? 1;

const restore = run(process.execPath, [join("scripts", "ensure-electron-abi.mjs")]);
if ((restore.status ?? 1) !== 0) {
  process.exit(restore.status ?? 1);
}

process.exit(vitestStatus);
