// @vitest-environment node
import { describe, expect, it, vi } from "vitest";
import {
  ensureElectronAbi,
  getElectronAbi,
  verifyElectronCanLoadBetterSqlite,
} from "../../scripts/ensure-electron-abi.mjs";

function createExecMock() {
  return vi.fn((file: string, args: readonly string[]) => {
    const command = `${file} ${args.join(" ")}`;
    if (command.includes("process.versions.modules")) {
      return "146\n";
    }
    return "";
  });
}

describe("ensure-electron-abi script", () => {
  it("reads the ABI from Electron, not shell Node", () => {
    const execFileSync = createExecMock();

    expect(getElectronAbi({ execFileSync, existsSync: () => true })).toBe("146");

    expect(execFileSync).toHaveBeenCalledWith(
      expect.stringContaining("electron"),
      expect.arrayContaining(["-e", expect.stringContaining("process.versions.modules")]),
      expect.objectContaining({
        env: expect.objectContaining({ ELECTRON_RUN_AS_NODE: "1" }),
      }),
    );
  });

  it("verifies better-sqlite3 by loading it through Electron", () => {
    const execFileSync = createExecMock();

    verifyElectronCanLoadBetterSqlite({ execFileSync, existsSync: () => true });

    expect(execFileSync).toHaveBeenCalledWith(
      expect.stringContaining("electron"),
      expect.arrayContaining(["-e", expect.stringContaining("better_sqlite3.node")]),
      expect.objectContaining({
        env: expect.objectContaining({ ELECTRON_RUN_AS_NODE: "1" }),
      }),
    );
  });

  it("does not rebuild when Electron can load better-sqlite3", () => {
    const execFileSync = createExecMock();

    const result = ensureElectronAbi({
      execFileSync,
      existsSync: () => true,
      stdout: { write: vi.fn() },
      stderr: { write: vi.fn() },
    });

    expect(result.rebuilt).toBe(false);
    expect(result.electronAbi).toBe("146");
    expect(execFileSync).not.toHaveBeenCalledWith(
      expect.stringContaining("electron-rebuild"),
      expect.anything(),
      expect.anything(),
    );
  });

  it("rebuilds once when Electron cannot load better-sqlite3", () => {
    let loadAttempts = 0;
    const execFileSync = vi.fn((file: string, args: readonly string[]) => {
      const command = `${file} ${args.join(" ")}`;
      if (command.includes("process.versions.modules")) {
        return "146\n";
      }
      if (command.includes("better_sqlite3.node")) {
        loadAttempts += 1;
        if (loadAttempts === 1) {
          throw new Error("NODE_MODULE_VERSION 147. This version requires NODE_MODULE_VERSION 146.");
        }
      }
      return "";
    });

    const result = ensureElectronAbi({
      execFileSync,
      existsSync: () => true,
      stdout: { write: vi.fn() },
      stderr: { write: vi.fn() },
    });

    expect(result.rebuilt).toBe(true);
    expect(execFileSync).toHaveBeenCalledWith(
      expect.stringContaining("electron-rebuild"),
      ["-f", "-w", "better-sqlite3"],
      expect.objectContaining({ stdio: "inherit" }),
    );
  });

  it("fails loud when rebuild does not repair the Electron load", () => {
    const execFileSync = vi.fn((file: string, args: readonly string[]) => {
      const command = `${file} ${args.join(" ")}`;
      if (command.includes("process.versions.modules")) {
        return "146\n";
      }
      if (command.includes("better_sqlite3.node")) {
        throw new Error("NODE_MODULE_VERSION 147. This version requires NODE_MODULE_VERSION 146.");
      }
      return "";
    });

    expect(() =>
      ensureElectronAbi({
        execFileSync,
        existsSync: () => true,
        stdout: { write: vi.fn() },
        stderr: { write: vi.fn() },
      }),
    ).toThrow(/Electron ABI 146/);
  });
});
