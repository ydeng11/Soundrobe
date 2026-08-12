// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import { SettingsModal } from "../../src/components/SettingsModal";

afterEach(() => cleanup());

// Global mock for window.api
const defaultMockConfig = {
  llmModel: "mock-model",
  remoteLookupEnabled: true,
  discogsEnabled: true,
  chineseScript: null,
};

beforeEach(() => {
  window.api = {
    appInfo: vi.fn().mockResolvedValue({
      identifier: "com.ihelio.soundrobe",
      version: "0.1.0",
      runtime: "tauri",
      dev: false,
    }),
    getConfig: vi.fn().mockResolvedValue(defaultMockConfig),
    setConfig: vi.fn().mockResolvedValue(undefined),
    setDebugMode: vi.fn().mockResolvedValue(undefined),
    subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
  } as any;
});

function clickTab(label: string) {
  fireEvent.click(screen.getByRole("tab", { name: label }));
}

describe("SettingsModal", () => {
  it("renders nothing when closed", () => {
    const { container } = render(
      <SettingsModal open={false} onClose={() => {}} />,
    );
    expect(container.innerHTML).toBe("");
  });

  it("shows loading state then providers tab content", async () => {
    const mockGetConfig = vi.fn().mockResolvedValue({
      llmModel: "model-v1",
    });

    window.api = {
      getConfig: mockGetConfig,
      setConfig: vi.fn().mockResolvedValue(undefined),
      setDebugMode: vi.fn().mockResolvedValue(undefined),
      subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
    } as any;

    render(<SettingsModal open={true} onClose={() => {}} />);

    // Should show loading initially
    expect(screen.getByText("Loading…")).toBeTruthy();

    // Wait for config to load
    const llmField = await screen.findByDisplayValue("model-v1");
    expect(llmField).toBeTruthy();
    expect(mockGetConfig).toHaveBeenCalledTimes(1);

    // AI & Providers tab is active by default
    expect(screen.getByRole("tab", { name: "AI & Providers" }).getAttribute("aria-selected")).toBe("true");
  });

  it("shows providers content by default with all provider fields", async () => {
    window.api.getConfig = vi.fn().mockResolvedValue({
      llmModel: "test-model",
      remoteLookupEnabled: true,
      discogsEnabled: false,
    });

    render(<SettingsModal open={true} onClose={() => {}} />);

    // Wait for loading to finish
    const llmField = await screen.findByDisplayValue("test-model");
    expect(llmField).toBeTruthy();

    // AI & Providers fields visible
    expect(
      screen.getByPlaceholderText(
        "sk-… (leave blank to keep current)",
      ),
    ).toBeTruthy();
    // Discogs Token + TheAudioDB API Key have the same placeholder, both on AI & Providers tab
    expect(
      screen.getAllByPlaceholderText(
        "(leave blank to keep current)",
      ).length,
    ).toBe(2);
    expect(screen.getByText("Version 0.1.0")).toBeTruthy();

    // Metadata fields should NOT be visible on the default AI & Providers tab
    expect(screen.queryByText("Auto-download Lyrics")).toBeNull();
  });

  it("switches to metadata tab and shows metadata fields", async () => {
    window.api.getConfig = vi.fn().mockResolvedValue({
      llmModel: "model",
      lyricsDownloadEnabled: true,
    });

    render(<SettingsModal open={true} onClose={() => {}} />);

    await screen.findByDisplayValue("model");

    // Switch to Metadata tab
    clickTab("Metadata");
    expect(screen.getByRole("tab", { name: "Metadata" }).getAttribute("aria-selected")).toBe("true");

    // AI & Providers fields hidden, metadata visible
    expect(screen.queryByPlaceholderText("sk-… (leave blank to keep current)")).toBeNull();
    expect(screen.getByText("Auto-download Lyrics")).toBeTruthy();
  });

  it("switches to advanced tab and shows advanced fields", async () => {
    window.api.getConfig = vi.fn().mockResolvedValue({
      llmModel: "model",
      remoteLookupEnabled: true,
      discogsEnabled: false,
      debug: true,
    });

    render(<SettingsModal open={true} onClose={() => {}} />);

    await screen.findByDisplayValue("model");

    // Switch to Advanced tab
    clickTab("Advanced");
    expect(screen.getByRole("tab", { name: "Advanced" }).getAttribute("aria-selected")).toBe("true");

    expect(screen.getByText("Debug Mode")).toBeTruthy();
  });

  it("preserves values when switching tabs and back", async () => {
    window.api.getConfig = vi.fn().mockResolvedValue({
      llmModel: "gpt-4",
    });

    render(<SettingsModal open={true} onClose={() => {}} />);

    await screen.findByDisplayValue("gpt-4");

    // Switch to Metadata and then back to AI & Providers
    clickTab("Metadata");
    clickTab("AI & Providers");

    // Value should still be there
    expect(screen.getByDisplayValue("gpt-4")).toBeTruthy();
  });

  it("loads chineseScript from config on metadata tab", async () => {
    window.api.getConfig = vi.fn().mockResolvedValue({
      llmModel: "model",
      remoteLookupEnabled: true,
      discogsEnabled: false,
      chineseScript: "simplified",
    });

    render(<SettingsModal open={true} onClose={() => {}} />);

    await screen.findByDisplayValue("model");

    // Switch to Metadata tab where Chinese Script lives
    clickTab("Metadata");

    const select = await screen.findByDisplayValue("Simplified Chinese");
    expect(select).toBeTruthy();
    expect((select as HTMLSelectElement).value).toBe("simplified");
  });

  it("keeps settings usable when native version lookup fails", async () => {
    window.api.appInfo = vi.fn().mockRejectedValue(new Error("unavailable"));

    render(<SettingsModal open={true} onClose={() => {}} />);

    expect(await screen.findByDisplayValue("mock-model")).toBeTruthy();
    expect(screen.getByText("Version unavailable")).toBeTruthy();
  });

  it("calls onClose when Cancel is clicked", async () => {
    const onClose = vi.fn();
    render(<SettingsModal open={true} onClose={onClose} />);
    await screen.findByDisplayValue("mock-model");
    const cancelBtn = screen.getByText("Cancel");
    fireEvent.click(cancelBtn);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("saves settings and closes", async () => {
    const onClose = vi.fn();
    const setConfig = vi.fn().mockResolvedValue(undefined);
    window.api = {
      getConfig: vi.fn().mockResolvedValue({
        llmModel: "model",
        remoteLookupEnabled: true,
        discogsEnabled: false,
        chineseScript: null,
      }),
      setConfig,
      setDebugMode: vi.fn().mockResolvedValue(undefined),
      subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
    } as any;

    render(<SettingsModal open={true} onClose={onClose} />);

    // Wait for load
    await screen.findByDisplayValue("model");

    // Enter new API key (on AI & Providers tab, active by default)
    const apiKeyInput = screen.getByPlaceholderText(
      "sk-… (leave blank to keep current)",
    );
    fireEvent.change(apiKeyInput, {
      target: { value: "sk-or-v1-newkey" },
    });

    // Click Save
    const saveBtn = screen.getByText("Save");
    fireEvent.click(saveBtn);

    // Should have called setConfig with the new key
    await waitFor(() => {
      expect(setConfig).toHaveBeenCalledWith(
        "llmApiKey",
        "sk-or-v1-newkey",
      );
    });

    // llmModel should also be saved (unchanged value is also written)
    await waitFor(() => {
      expect(setConfig).toHaveBeenCalledWith("llmModel", "model");
    });

    // chineseScript should be saved as null when unset
    await waitFor(() => {
      expect(setConfig).toHaveBeenCalledWith("chineseScript", null);
    });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("saves chineseScript as simplified when Simplified Chinese is selected", async () => {
    const onClose = vi.fn();
    const setConfig = vi.fn().mockResolvedValue(undefined);
    window.api = {
      getConfig: vi
        .fn()
        .mockResolvedValue({
          llmModel: "model",
          remoteLookupEnabled: true,
          discogsEnabled: false,
          chineseScript: null,
        }),
      setConfig,
      setDebugMode: vi.fn().mockResolvedValue(undefined),
      subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
    } as any;

    render(<SettingsModal open={true} onClose={onClose} />);

    // Wait for load
    await screen.findByDisplayValue("model");

    // Switch to Metadata tab where Chinese Script lives
    clickTab("Metadata");

    // Select Simplified Chinese
    const select = screen.getByDisplayValue(
      "Default (no conversion)",
    ) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "simplified" } });

    // Click Save
    const saveBtn = screen.getByText("Save");
    fireEvent.click(saveBtn);

    // chineseScript should be saved as "simplified"
    await waitFor(() => {
      expect(setConfig).toHaveBeenCalledWith(
        "chineseScript",
        "simplified",
      );
    });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not save masked secrets when user did not edit them", async () => {
    const onClose = vi.fn();
    const setConfig = vi.fn().mockResolvedValue(undefined);
    window.api = {
      getConfig: vi.fn().mockResolvedValue({
        llmModel: "model",
        llmApiKey: "****b7",
        llmApiKeyConfigured: true,
        discogsToken: "****1234",
        theAudioDbApiKey: "****90",
      }),
      setConfig,
      setDebugMode: vi.fn().mockResolvedValue(undefined),
      subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
    } as any;

    render(<SettingsModal open={true} onClose={onClose} />);

    // Wait for load — masked key appears in the input field
    await screen.findByDisplayValue("****b7");

    // Click Save without editing any fields
    const saveBtn = screen.getByText("Save");
    fireEvent.click(saveBtn);

    // Secret fields should NOT be saved because their values are masked
    await waitFor(() => {
      expect(setConfig).not.toHaveBeenCalledWith(
        "llmApiKey",
        expect.any(String),
      );
      expect(setConfig).not.toHaveBeenCalledWith(
        "discogsToken",
        expect.any(String),
      );
      expect(setConfig).not.toHaveBeenCalledWith(
        "theAudioDbApiKey",
        expect.any(String),
      );
    });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("saves a newly entered real API key", async () => {
    const onClose = vi.fn();
    const setConfig = vi.fn().mockResolvedValue(undefined);
    window.api = {
      getConfig: vi.fn().mockResolvedValue({
        llmModel: "model",
        llmApiKey: "****b7",
        llmApiKeyConfigured: true,
        discogsToken: "****1234",
        theAudioDbApiKey: "****90",
      }),
      setConfig,
      setDebugMode: vi.fn().mockResolvedValue(undefined),
      subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
    } as any;

    render(<SettingsModal open={true} onClose={onClose} />);

    // Wait for load
    await screen.findByDisplayValue("****b7");

    // Overwrite with a real key
    const apiKeyInput = screen.getByPlaceholderText(
      "sk-… (leave blank to keep current)",
    );
    fireEvent.change(apiKeyInput, {
      target: { value: "sk-or-v1-newkey" },
    });

    // Click Save
    const saveBtn = screen.getByText("Save");
    fireEvent.click(saveBtn);

    // The real key should be saved
    await waitFor(() => {
      expect(setConfig).toHaveBeenCalledWith(
        "llmApiKey",
        "sk-or-v1-newkey",
      );
    });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("passes empty key to testLlmConnection when displayed key is masked", async () => {
    const testLlmConnection = vi
      .fn()
      .mockResolvedValue({ model: "deepseek/deepseek-v4-flash" });
    window.api = {
      getConfig: vi.fn().mockResolvedValue({
        llmModel: "deepseek/deepseek-v4-flash",
        llmApiKey: "****b7",
        llmApiKeyConfigured: true,
        discogsEnabled: false,
      }),
      setConfig: vi.fn().mockResolvedValue(undefined),
      setDebugMode: vi.fn().mockResolvedValue(undefined),
      testLlmConnection,
      subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
    } as any;

    render(<SettingsModal open={true} onClose={vi.fn()} />);

    // Wait for load — masked key and model are displayed
    await screen.findByDisplayValue("****b7");

    // The test connection button should be visible since model is set
    const testBtn = await screen.findByRole("button", {
      name: /test connection/i,
    });
    expect(testBtn).toBeTruthy();
    fireEvent.click(testBtn);

    // Should invoke testLlmConnection with empty key (masked → "")
    await waitFor(() => {
      expect(testLlmConnection).toHaveBeenCalledWith(
        "",
        "deepseek/deepseek-v4-flash",
        undefined,
        undefined,
      );
    });
  });
});
