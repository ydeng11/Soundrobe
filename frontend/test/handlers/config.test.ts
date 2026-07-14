import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  existsSync,
  readFileSync,
  writeFileSync,
  mkdirSync,
  rmSync,
} from "node:fs";
import { join, dirname } from "node:path";
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

/**
 * Normalized Electron-vs-Rust redaction fixture. The expected JSON is the
 * cross-runtime contract for `config:get`'s response shape; the matching Rust
 * test (`src-tauri/src/state/config.rs` redacted_fixture_matches_normalized_
 * contract) asserts `ConfigState::redacted()` produces the SAME object from the
 * SAME on-disk file. Both runtimes must agree exactly; if this drifts from the
 * Rust fixture, one side of `config:get` parity is broken.
 */
describe("redaction fixture (normalized vs Rust)", () => {
  const EXPECTED_REDACTED = {
    llmApiKey: "****7890",
    llmModel: "gpt-4",
    discogsToken: "****1234",
    remoteLookupEnabled: true,
    discogsEnabled: true,
    debug: true,
    lyricsDownloadEnabled: false,
    lyricsApiUrl: "https://lr.example/api",
    theAudioDbApiKey: null,
    chineseScript: "traditional",
  } satisfies Record<string, unknown>;

  // Electron's `getConfig()` redaction formula, mirrored 1:1 so the fixture
  // is exercised against the same logic the handler uses.
  const mask = (v: string | undefined) =>
    v ? "****" + v.slice(-4) : null;

  it("Electron loadConfig + getConfig formula matches the normalized fixture", () => {
    const originalHome = process.env.HOME;
    const stash = ["LLM_API_KEY", "LLM_MODEL", "AUTO_TAG_DISCOGS_TOKEN",
      "THEAUDIODB_API_KEY", "AUTO_TAG_DEBUG",
      "AUTO_TAG_LYRICS_DOWNLOAD_ENABLED", "AUTO_TAG_LYRICS_API_URL",
      "AUTO_TAG_CHINESE_SCRIPT"];
    const stashVals: Record<string, string | undefined> = {};
    for (const k of stash) {
      stashVals[k] = process.env[k];
      delete process.env[k];
    }
    const home = tempHomeFixture();
    process.env.HOME = home;
    try {
      const configPath = join(home, ".auto-tagger", "config.yaml");
      mkdirSync(dirname(configPath), { recursive: true });
      writeFileSync(
        configPath,
        "llm_api_key: sk-or-v1-1234567890\n" +
          "llm_model: gpt-4\n" +
          "discogs_token: mytoken1234\n" +
          "debug: true\n" +
          "lyrics_api_url: https://lr.example/api\n" +
          "chinese_script: traditional\n",
        "utf-8",
      );

      const cfg = loadConfig();
      const redacted = {
        llmApiKey: mask(cfg.llmApiKey ?? undefined),
        llmModel: cfg.llmModel ?? null,
        discogsToken: mask(cfg.discogsToken ?? undefined),
        remoteLookupEnabled: cfg.remoteLookupEnabled,
        discogsEnabled: cfg.discogsEnabled,
        debug: cfg.debug ?? false,
        lyricsDownloadEnabled: cfg.lyricsDownloadEnabled ?? false,
        lyricsApiUrl: cfg.lyricsApiUrl ?? null,
        theAudioDbApiKey: mask(cfg.theAudioDbApiKey ?? undefined),
        chineseScript: cfg.chineseScript ?? null,
      };
      expect(redacted).toEqual(EXPECTED_REDACTED);
    } finally {
      process.env.HOME = originalHome;
      for (const k of stash) {
        if (stashVals[k] !== undefined) process.env[k] = stashVals[k]!;
        else delete process.env[k];
      }
    }
  });

  function tempHomeFixture(): string {
    return join(tmpdir(), `auto-tag-redact-${Date.now()}-${Math.random().toString(36).slice(2)}`);
  }
});
