import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  existsSync,
  readFileSync,
  writeFileSync,
  mkdirSync,
  rmSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { loadConfig, saveConfig } from "../../electron/handlers/auto-tag";

describe("saveConfig", () => {
  const originalHome = process.env.HOME;
  let tempHome: string;

  beforeEach(() => {
    // Create a temp HOME dir
    tempHome = join(tmpdir(), `auto-tag-test-home-${Date.now()}`);
    mkdirSync(tempHome, { recursive: true });
    process.env.HOME = tempHome;
  });

  afterEach(() => {
    process.env.HOME = originalHome;
    try {
      rmSync(tempHome, { recursive: true, force: true });
    } catch {
      // ignore
    }
  });

  it("creates config file when none exists", () => {
    saveConfig("llmModel", "test-model");

    const configPath = join(tempHome, ".auto-tagger", "config.yaml");
    expect(existsSync(configPath)).toBe(true);
    const content = readFileSync(configPath, "utf-8");
    expect(content).toContain("llm_model: test-model");
  });

  it("updates an existing key in config file", () => {
    // Create initial config
    const configPath = join(tempHome, ".auto-tagger", "config.yaml");
    mkdirSync(join(tempHome, ".auto-tagger"), { recursive: true });
    writeFileSync(
      configPath,
      "llm_model: old-model\nremote_lookup_enabled: true\n",
      "utf-8",
    );

    // Update
    saveConfig("llmModel", "new-model");

    const content = readFileSync(configPath, "utf-8");
    expect(content).toContain("llm_model: new-model");
    expect(content).toContain("remote_lookup_enabled: true");
  });

  it("maps JS camelCase to YAML snake_case keys", () => {
    saveConfig("llmApiKey", "sk-or-v1-test");
    saveConfig("discogsToken", "discogs-token");
    saveConfig("remoteLookupEnabled", false);
    saveConfig("discogsEnabled", false);

    const configPath = join(tempHome, ".auto-tagger", "config.yaml");
    const content = readFileSync(configPath, "utf-8");
    expect(content).toContain("llm_api_key: sk-or-v1-test");
    expect(content).toContain("discogs_token: discogs-token");
    expect(content).toContain("remote_lookup_enabled: false");
    expect(content).toContain("discogs_enabled: false");
  });

  it("does not write unknown keys", () => {
    saveConfig("unknownKey", "value");
    const configPath = join(tempHome, ".auto-tagger", "config.yaml");
    // File should not be created since only unknown keys were written
    expect(existsSync(configPath)).toBe(false);
  });

  it("persists values that can be read back by loadConfig", () => {
    saveConfig("llmModel", "persist-test");
    saveConfig("remoteLookupEnabled", false);

    delete process.env.LLM_MODEL;
    delete process.env.AUTO_TAG_REMOTE_LOOKUP;

    const cfg = loadConfig();
    expect(cfg.llmModel).toBe("persist-test");
    expect(cfg.remoteLookupEnabled).toBe(false);
  });

  it("can write and read boolean values", () => {
    saveConfig("remoteLookupEnabled", false);
    saveConfig("discogsEnabled", false);

    delete process.env.AUTO_TAG_REMOTE_LOOKUP;
    delete process.env.AUTO_TAG_DISCOGS_ENABLED;

    const cfg = loadConfig();
    expect(cfg.remoteLookupEnabled).toBe(false);
    expect(cfg.discogsEnabled).toBe(false);

    // Toggle back
    saveConfig("remoteLookupEnabled", true);
    saveConfig("discogsEnabled", true);

    delete process.env.AUTO_TAG_REMOTE_LOOKUP;
    delete process.env.AUTO_TAG_DISCOGS_ENABLED;

    const cfg2 = loadConfig();
    expect(cfg2.remoteLookupEnabled).toBe(true);
    expect(cfg2.discogsEnabled).toBe(true);
  });
});
