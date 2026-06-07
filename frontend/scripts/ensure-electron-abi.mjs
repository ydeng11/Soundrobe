#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(__dirname, "..");

function binName(name) {
  return process.platform === "win32" ? `${name}.cmd` : name;
}

function defaultElectronBin() {
  return join(projectRoot, "node_modules", ".bin", binName("electron"));
}

function defaultElectronRebuildBin() {
  return join(projectRoot, "node_modules", ".bin", binName("electron-rebuild"));
}

function defaultNativePath() {
  return join(
    projectRoot,
    "node_modules",
    "better-sqlite3",
    "build",
    "Release",
    "better_sqlite3.node",
  );
}

function runElectronNodeExpression(expression, options = {}) {
  const run = options.execFileSync ?? execFileSync;
  return run(options.electronBin ?? defaultElectronBin(), ["-e", expression], {
    cwd: options.projectRoot ?? projectRoot,
    encoding: "utf8",
    timeout: options.timeoutMs ?? 30_000,
    env: {
      ...process.env,
      ELECTRON_RUN_AS_NODE: "1",
      ...(options.env ?? {}),
    },
    stdio: options.stdio ?? "pipe",
  });
}

export function getElectronAbi(options = {}) {
  return String(runElectronNodeExpression("console.log(process.versions.modules)", options)).trim();
}

export function verifyElectronCanLoadBetterSqlite(options = {}) {
  const nativePath = options.nativePath ?? defaultNativePath();
  runElectronNodeExpression(`require(${JSON.stringify(nativePath)})`, options);
}

export function rebuildBetterSqliteForElectron(options = {}) {
  const run = options.execFileSync ?? execFileSync;
  run(options.electronRebuildBin ?? defaultElectronRebuildBin(), ["-f", "-w", "better-sqlite3"], {
    cwd: options.projectRoot ?? projectRoot,
    stdio: "inherit",
    timeout: options.timeoutMs ?? 120_000,
  });
}

function formatError(error) {
  if (error && typeof error === "object" && "stderr" in error && error.stderr) {
    const stderr = String(error.stderr);
    const abiStart = stderr.indexOf("Error: The module");
    if (abiStart >= 0) {
      return stderr.slice(abiStart).split("\n").slice(0, 7).join("\n");
    }
    const lines = stderr.trim().split("\n").filter(Boolean);
    if (lines.length > 0) return lines.slice(-8).join("\n");
  }
  if (error instanceof Error) return error.message;
  return String(error);
}

export function ensureElectronAbi(options = {}) {
  const stdout = options.stdout ?? process.stdout;
  const stderr = options.stderr ?? process.stderr;
  const nativePath = options.nativePath ?? defaultNativePath();
  const electronAbi = getElectronAbi(options);
  const shellAbi = process.versions.modules;

  const hasNativeBinary = options.existsSync ?? existsSync;

  if (!hasNativeBinary(nativePath)) {
    stderr.write(
      `[native-abi] better-sqlite3 binary is missing; rebuilding for Electron ABI ${electronAbi}.\n`,
    );
    rebuildBetterSqliteForElectron(options);
    verifyElectronCanLoadBetterSqlite(options);
    stdout.write(`[native-abi] better-sqlite3 is ready for Electron ABI ${electronAbi}.\n`);
    return { rebuilt: true, electronAbi, shellAbi };
  }

  try {
    verifyElectronCanLoadBetterSqlite(options);
    stdout.write(
      `[native-abi] better-sqlite3 is already compatible with Electron ABI ${electronAbi} (shell Node ABI ${shellAbi}).\n`,
    );
    return { rebuilt: false, electronAbi, shellAbi };
  } catch (firstError) {
    stderr.write(
      `[native-abi] better-sqlite3 is not compatible with Electron ABI ${electronAbi} (shell Node ABI ${shellAbi}).\n`,
    );
    stderr.write(`[native-abi] Load failure: ${formatError(firstError)}\n`);
  }

  rebuildBetterSqliteForElectron(options);

  try {
    verifyElectronCanLoadBetterSqlite(options);
    stdout.write(`[native-abi] Rebuilt better-sqlite3 for Electron ABI ${electronAbi}.\n`);
    return { rebuilt: true, electronAbi, shellAbi };
  } catch (secondError) {
    throw new Error(
      `Could not load better-sqlite3 after rebuilding for Electron ABI ${electronAbi}. ` +
        `Shell Node ABI is ${shellAbi}. Last error: ${formatError(secondError)}`,
    );
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try {
    ensureElectronAbi();
  } catch (error) {
    process.stderr.write(`[native-abi] ${formatError(error)}\n`);
    process.exit(1);
  }
}
