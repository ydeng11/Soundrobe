import { execFileSync } from "node:child_process";
import path from "node:path";
import type { Options } from "@wdio/types";
import { cleanupE2eWorkspace, prepareE2eWorkspace } from "./e2e-tauri/fixtures";

const workspace = prepareE2eWorkspace();
const authorityPhase = process.env.SOUNDROBE_E2E_AUTHORITY_PHASE;

const binary = process.env.SOUNDROBE_E2E_BINARY ?? path.resolve(
  "src-tauri/target/release",
  process.platform === "win32" ? "soundrobe.exe" : "soundrobe",
);

export const config: Options.Testrunner = {
  runner: "local",
  // Credentialed and real-display smokes are selected explicitly by their
  // `just` recipes; the default CI suite remains deterministic and offline.
  specs: [authorityPhase ? "./e2e-tauri/library-authority.spec.ts" : "./e2e-tauri/workflows.spec.ts"],
  maxInstances: 1,
  services: [
    [
      "@wdio/tauri-service",
      {
        appBinaryPath: binary,
        driverProvider: "embedded",
        captureBackendLogs: true,
        captureFrontendLogs: true,
      },
    ],
  ],
  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": { application: binary },
    },
  ],
  logLevel: "warn",
  bail: 0,
  waitforTimeout: 15_000,
  connectionRetryTimeout: 90_000,
  connectionRetryCount: 1,
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: 60_000 },
  onPrepare: () => {
    if (!authorityPhase && (process.platform === "darwin" || process.platform === "linux")) {
      execFileSync(binary, ["--nominate-library", workspace.manifest.library], {
        stdio: "inherit",
      });
    }
  },
  onComplete: () => {
    if (!authorityPhase) cleanupE2eWorkspace(workspace.root);
  },
};
