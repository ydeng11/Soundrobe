// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { WebSettingsPage } from "../../src/components/WebSettingsPage";

afterEach(() => cleanup());

const defaultConfig = {
  llmProvider: "openrouter",
  llmModel: "openai/gpt-4o-mini",
  llmBaseUrl: "",
  llmApiKey: "****7f",
  llmApiKeyConfigured: true,
  discogsToken: "****21",
  theAudioDbApiKey: "",
  lyricsApiUrl: "https://lrclib.net/api",
  lyricsDownloadEnabled: true,
  chineseScript: "simplified",
  assistantAutonomous: false,
  debug: true,
};

beforeEach(() => {
  window.api = {
    appInfo: vi.fn().mockResolvedValue({
      identifier: "com.ihelio.soundrobe",
      version: "0.1.11",
      runtime: "web",
      dev: false,
    }),
    getConfig: vi.fn().mockResolvedValue(defaultConfig),
    setConfig: vi.fn().mockResolvedValue(undefined),
    setDebugMode: vi.fn().mockResolvedValue(undefined),
    testLlmConnection: vi.fn().mockResolvedValue({ model: "openai/gpt-4o-mini" }),
  } as any;
});

describe("WebSettingsPage", () => {
  it("renders the inline template settings page before a library is mounted", async () => {
    render(<WebSettingsPage />);

    expect(screen.getByText("CONFIGURATION")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Settings" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "General" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "AI & Providers" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Lyrics & metadata" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save settings" })).toBeTruthy();

    await screen.findByDisplayValue("openai/gpt-4o-mini");
    expect(screen.getByText("Version 0.1.11")).toBeTruthy();
  });

  it("keeps the selected settings section visible in the browser tabs", async () => {
    render(<WebSettingsPage />);

    await screen.findByDisplayValue("openai/gpt-4o-mini");
    const general = screen.getByRole("link", { name: "General" });
    const providers = screen.getByRole("link", { name: "AI & Providers" });
    expect(general.getAttribute("aria-current")).toBe("page");
    expect(providers.getAttribute("aria-current")).toBeNull();

    fireEvent.click(providers);
    expect(general.getAttribute("aria-current")).toBeNull();
    expect(providers.getAttribute("aria-current")).toBe("page");
  });

  it("keeps redacted secrets intact and saves only supported web settings", async () => {
    const setConfig = vi.fn().mockResolvedValue(undefined);
    window.api.setConfig = setConfig;
    render(<WebSettingsPage />);

    await screen.findByDisplayValue("openai/gpt-4o-mini");
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "new-model" } });
    fireEvent.change(screen.getByLabelText("Lyrics API URL"), {
      target: { value: "https://lyrics.example/api" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));

    await waitFor(() => expect(setConfig).toHaveBeenCalled());
    expect(setConfig).toHaveBeenCalledWith("llmModel", "new-model");
    expect(setConfig).toHaveBeenCalledWith("lyricsApiUrl", "https://lyrics.example/api");
    expect(setConfig).not.toHaveBeenCalledWith("llmApiKey", "****7f");
    expect(setConfig).not.toHaveBeenCalledWith("discogsToken", "****21");
    expect(setConfig).not.toHaveBeenCalledWith("llmApiKey", "");
  });

  it("tests the configured connection without exposing a raw key", async () => {
    const testLlmConnection = vi.fn().mockResolvedValue({ model: "openai/gpt-4o-mini" });
    window.api.testLlmConnection = testLlmConnection;
    render(<WebSettingsPage />);

    await screen.findByDisplayValue("openai/gpt-4o-mini");
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));

    await waitFor(() => expect(testLlmConnection).toHaveBeenCalledTimes(1));
    expect(testLlmConnection).toHaveBeenCalledWith(
      "",
      "openai/gpt-4o-mini",
      "openrouter",
      undefined,
    );
    expect(screen.getByText("Connection successful")).toBeTruthy();
  });

  it("keeps Save disabled after a failed load and allows retrying the load", async () => {
    const getConfig = vi.fn()
      .mockRejectedValueOnce(new Error("service unavailable"))
      .mockResolvedValueOnce(defaultConfig);
    const setConfig = vi.fn().mockResolvedValue(undefined);
    window.api.getConfig = getConfig;
    window.api.setConfig = setConfig;
    render(<WebSettingsPage />);

    await screen.findByRole("alert");
    const saveButton = screen.getByRole("button", { name: "Save settings" }) as HTMLButtonElement;
    expect(saveButton.disabled).toBe(true);

    fireEvent.click(saveButton);
    expect(setConfig).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByDisplayValue("openai/gpt-4o-mini");
    expect(getConfig).toHaveBeenCalledTimes(2);
    expect(saveButton.disabled).toBe(false);
  });

  it("tests a protected connection with a blank key after the masked field is cleared", async () => {
    const testLlmConnection = vi.fn().mockResolvedValue({ model: "openai/gpt-4o-mini" });
    window.api.testLlmConnection = testLlmConnection;
    render(<WebSettingsPage />);

    await screen.findByDisplayValue("openai/gpt-4o-mini");
    fireEvent.change(screen.getByLabelText("LLM API key"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));

    await waitFor(() => expect(testLlmConnection).toHaveBeenCalledTimes(1));
    expect(testLlmConnection).toHaveBeenCalledWith(
      "",
      "openai/gpt-4o-mini",
      "openrouter",
      undefined,
    );
  });

  it("does not offer native update controls", async () => {
    render(<WebSettingsPage />);
    await screen.findByDisplayValue("openai/gpt-4o-mini");

    expect(screen.queryByRole("button", { name: /update/i })).toBeNull();
    expect(screen.queryByText(/application updates/i)).toBeNull();
  });

  it("uses the browser detail layout without unsupported storage controls", async () => {
    render(<WebSettingsPage />);
    await screen.findByDisplayValue("openai/gpt-4o-mini");

    expect(screen.getByTestId("web-settings-detail")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "General" })).toBeTruthy();
    expect(screen.queryByText(/WebDAV/i)).toBeNull();
    expect(screen.queryByText(/recursive scan/i)).toBeNull();
    expect(screen.queryByText(/directory root/i)).toBeNull();
  });

  it("uses the same protected settings lifecycle in the workspace drawer and only closes after Apply succeeds", async () => {
    const onClose = vi.fn();
    const setConfig = vi.fn().mockResolvedValue(undefined);
    window.api.setConfig = setConfig;
    render(<WebSettingsPage presentation="drawer" onClose={onClose} />);

    const dialog = await screen.findByRole("dialog", { name: "Workspace settings" });
    expect(dialog).toBeTruthy();
    expect(screen.getByRole("button", { name: "Apply" })).toBeTruthy();
    expect(screen.queryByText(/WebDAV/i)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(setConfig).toHaveBeenCalled());
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(setConfig).not.toHaveBeenCalledWith("llmApiKey", "****7f");
  });

  it("keeps the workspace drawer open when Apply fails and discards its draft on Cancel", async () => {
    const onClose = vi.fn();
    window.api.setConfig = vi.fn().mockRejectedValue(new Error("save unavailable"));
    render(<WebSettingsPage presentation="drawer" onClose={onClose} />);

    await screen.findByRole("dialog", { name: "Workspace settings" });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect((await screen.findByRole("alert")).textContent).toContain("save unavailable");
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("dismisses the workspace drawer with Escape without saving", async () => {
    const onClose = vi.fn();
    render(<WebSettingsPage presentation="drawer" onClose={onClose} />);

    const dialog = await screen.findByRole("dialog", { name: "Workspace settings" });
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(window.api.setConfig).not.toHaveBeenCalled();
  });
});
