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
};

beforeEach(() => {
  window.api = {
    getConfig: vi.fn().mockResolvedValue(defaultMockConfig),
    setConfig: vi.fn().mockResolvedValue(undefined),
    setDebugMode: vi.fn().mockResolvedValue(undefined),
    subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
  } as any;
});

describe("SettingsModal", () => {
  it("renders nothing when closed", () => {
    const { container } = render(
      <SettingsModal open={false} onClose={() => {}} />,
    );
    expect(container.innerHTML).toBe("");
  });

  it("renders settings form when open (after loading)", async () => {
    window.api.getConfig = vi.fn().mockResolvedValue({
      llmModel: "test-model",
      remoteLookupEnabled: true,
      discogsEnabled: false,
    });

    render(<SettingsModal open={true} onClose={() => {}} />);

    // Wait for loading to finish and fields to appear
    expect(screen.getByText("Loading…")).toBeTruthy();
    const llmField = await screen.findByDisplayValue("test-model");
    expect(llmField).toBeTruthy();

    // Labels use uppercase with tracking — use find by role or placeholder
    expect(
      screen.getByPlaceholderText(
        "sk-or-v1-… (leave blank to keep current)",
      ),
    ).toBeTruthy();
    expect(
      screen.getAllByPlaceholderText(
        "(leave blank to keep current)",
      ).length,
    ).toBe(2); // Discogs Token + TheAudioDB API Key
  });

  it("shows loading state then fields", async () => {
    const mockGetConfig = vi.fn().mockResolvedValue({
      llmModel: "model-v1",
      remoteLookupEnabled: true,
      discogsEnabled: true,
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
      }),
      setConfig,
      setDebugMode: vi.fn().mockResolvedValue(undefined),
      subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
    } as any;

    render(<SettingsModal open={true} onClose={onClose} />);

    // Wait for load
    await screen.findByDisplayValue("model");

    // Enter new API key
    const apiKeyInput = screen.getByPlaceholderText(
      "sk-or-v1-… (leave blank to keep current)",
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

    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
