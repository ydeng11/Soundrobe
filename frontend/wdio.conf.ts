import path from "node:path";
import type { Options } from "@wdio/types";
import { cleanupE2eWorkspace, prepareE2eWorkspace } from "./e2e-tauri/fixtures";

const workspace = prepareE2eWorkspace();

const binary = path.resolve(
  "src-tauri/target/release",
  process.platform === "win32" ? "auto-tagger.exe" : "auto-tagger",
);

export const config: Options.Testrunner = {
  runner: "local",
  specs: ["./e2e-tauri/**/*.spec.ts"],
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
  onComplete: () => cleanupE2eWorkspace(workspace.root),
};
