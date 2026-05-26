import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { writeFileSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import {
  saveAlias,
  getAliases,
  isChineseName,
  setAliasFilePath,
} from "../../electron/handlers/aliases";

let tmpDir: string;
let aliasFile: string;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), "aliases-test-"));
  aliasFile = join(tmpDir, "artist-aliases.json");
  setAliasFilePath(aliasFile);
});

afterEach(() => {
  rmSync(tmpDir, { recursive: true, force: true });
});

describe("saveAlias / getAliases", () => {
  it("returns empty for unknown hint", () => {
    expect(getAliases("Unknown Artist")).toEqual([]);
  });

  it("returns empty for null hint", () => {
    expect(getAliases(null)).toEqual([]);
  });

  it("stores and retrieves an alias", () => {
    saveAlias("蔡健雅", "Tanya Chua");
    const aliases = getAliases("蔡健雅");
    expect(aliases).toContain("Tanya Chua");
  });

  it("is case-insensitive for the hint key", () => {
    saveAlias("britney spears", "Britney");
    expect(getAliases("BRITNEY SPEARS")).toContain("Britney");
  });

  it("deduplicates aliases", () => {
    saveAlias("Beatles", "The Beatles");
    saveAlias("Beatles", "the beatles");
    const aliases = getAliases("Beatles");
    expect(aliases).toHaveLength(1);
    expect(aliases[0]).toBe("The Beatles");
  });

  it("refuses to store identical alias", () => {
    saveAlias("Test", "Test");
    expect(getAliases("Test")).toEqual([]);
  });

  it("persists multiple aliases for one hint (different aliases)", () => {
    saveAlias("久石让", "Joe Hisaishi");
    saveAlias("久石让", "Hisaishi Joe");
    const aliases = getAliases("久石让");
    expect(aliases).toHaveLength(2);
  });
});

describe("isChineseName", () => {
  it("returns true for Chinese name", () => {
    expect(isChineseName("蔡健雅")).toBe(true);
  });

  it("returns true for mixed script", () => {
    expect(isChineseName("Tanya 蔡健雅")).toBe(true);
  });

  it("returns false for Latin-only", () => {
    expect(isChineseName("Tanya Chua")).toBe(false);
  });

  it("returns false for empty string", () => {
    expect(isChineseName("")).toBe(false);
  });
});
