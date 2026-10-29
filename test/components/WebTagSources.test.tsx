// @vitest-environment jsdom
/**
 * Tag-source configuration tests.
 *
 * These encode the safety intent, not just the behaviour. The dangerous
 * failure mode is a save that clears a stored credential, so the redaction
 * rules are asserted directly: a redacted or empty secret must never be sent
 * back to the service, and a connection test must let the service resolve the
 * stored key rather than sending a placeholder.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import {
  EMPTY_TAG_SOURCES,
  TagSourcesView,
  buildTagSourceWrites,
  connectionTestKey,
  isRedacted,
  type TagSourceSettings,
} from "../../src/web/TagSourcesView";

afterEach(cleanup);

const REDACTED = "****abcd";

function settings(overrides: Partial<TagSourceSettings> = {}): TagSourceSettings {
  return { ...EMPTY_TAG_SOURCES, ...overrides };
}

describe("tag source secret handling", () => {
  it("recognises redacted placeholders", () => {
    expect(isRedacted(REDACTED)).toBe(true);
    expect(isRedacted("real-token")).toBe(false);
    expect(isRedacted("")).toBe(false);
  });

  it("never writes a redacted secret back", () => {
    const writes = buildTagSourceWrites(
      settings({
        discogsToken: REDACTED,
        theAudioDbApiKey: REDACTED,
        llmApiKey: REDACTED,
      }),
    );
    const keys = writes.map(([key]) => key);

    expect(keys).not.toContain("discogsToken");
    expect(keys).not.toContain("theAudioDbApiKey");
    expect(keys).not.toContain("llmApiKey");
  });

  it("never writes an empty secret, so saving cannot erase a credential", () => {
    const keys = buildTagSourceWrites(settings()).map(([key]) => key);

    expect(keys).not.toContain("discogsToken");
    expect(keys).not.toContain("theAudioDbApiKey");
    expect(keys).not.toContain("llmApiKey");
  });

  it("writes a newly entered secret", () => {
    const writes = buildTagSourceWrites(settings({ discogsToken: "new-token" }));

    expect(writes).toContainEqual(["discogsToken", "new-token"]);
  });

  it("clears optional non-secret fields with null instead of an empty string", () => {
    const writes = buildTagSourceWrites(settings({ llmProvider: "", llmModel: "" }));
    const byKey = Object.fromEntries(writes);

    expect(byKey.llmProvider).toBeNull();
    expect(byKey.llmModel).toBeNull();
    expect(byKey.llmBaseUrl).toBeNull();
  });

  it("sends a blank key for a redacted credential so the service resolves the stored one", () => {
    expect(connectionTestKey(REDACTED)).toBe("");
    expect(connectionTestKey("live-key")).toBe("live-key");
  });
});

describe("tag sources view", () => {
  function fakeApi(config: Record<string, unknown> = {}) {
    return {
      getConfig: vi.fn(async () => config),
      setConfig: vi.fn(async () => undefined),
      testLlmConnection: vi.fn(async () => undefined),
    };
  }

  it("lists the metadata sources and the AI fallback", async () => {
    render(<TagSourcesView api={fakeApi() as unknown as Window["api"]} />);

    expect(await screen.findByText("MusicBrainz")).toBeTruthy();
    expect(screen.getByText("Discogs")).toBeTruthy();
    expect(screen.getByText("TheAudioDB")).toBeTruthy();
    expect(screen.getByText("AI fallback")).toBeTruthy();
    expect(screen.getByText("Always on")).toBeTruthy();
  });

  it("reports which sources are already configured", async () => {
    render(
      <TagSourcesView
        api={fakeApi({ discogsToken: REDACTED, theAudioDbApiKey: "" }) as unknown as Window["api"]}
      />,
    );

    await waitFor(() => expect(screen.getAllByText("Configured")).toHaveLength(1));
    expect(screen.getAllByText("Not configured")).toHaveLength(1);
  });

  it("loads redacted credentials without exposing a usable value", async () => {
    render(<TagSourcesView api={fakeApi({ llmApiKey: REDACTED }) as unknown as Window["api"]} />);

    const key = (await screen.findByLabelText("AI API key")) as HTMLInputElement;
    expect(key.value).toBe(REDACTED);
  });

  it("saves only the changed fields, keeping the stored secrets intact", async () => {
    const api = fakeApi({ discogsToken: REDACTED, llmProvider: "openai", llmModel: "gpt-4o" });
    render(<TagSourcesView api={api as unknown as Window["api"]} />);

    const model = (await screen.findByLabelText("AI model")) as HTMLInputElement;
    fireEvent.change(model, { target: { value: "gpt-4o-mini" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(api.setConfig).toHaveBeenCalled());
    const writtenKeys = api.setConfig.mock.calls.map((call) => call[0]);
    expect(writtenKeys).not.toContain("discogsToken");
    expect(writtenKeys).toContain("llmModel");
    expect(api.setConfig).toHaveBeenCalledWith("llmModel", "gpt-4o-mini");
  });

  it("surfaces a save failure instead of reporting success", async () => {
    const api = fakeApi({ llmModel: "gpt-4o" });
    api.setConfig.mockRejectedValueOnce(new Error("config write failed"));
    render(<TagSourcesView api={api as unknown as Window["api"]} />);

    await screen.findByLabelText("AI model");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect((await screen.findByRole("alert")).textContent).toContain("config write failed");
    expect(screen.queryByText("Tag sources saved")).toBeNull();
  });

  it("tests the connection with a blank key when the key is redacted", async () => {
    const api = fakeApi({ llmApiKey: REDACTED, llmModel: "gpt-4o", llmProvider: "openai" });
    render(<TagSourcesView api={api as unknown as Window["api"]} />);

    await screen.findByLabelText("AI model");
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));

    await waitFor(() => expect(api.testLlmConnection).toHaveBeenCalled());
    expect(api.testLlmConnection.mock.calls[0][0]).toBe("");
  });

  it("disables the connection test until a model is chosen", async () => {
    render(<TagSourcesView api={fakeApi() as unknown as Window["api"]} />);

    const button = (await screen.findByRole("button", {
      name: "Test connection",
    })) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });

  it("reports a connection failure with the service message", async () => {
    const api = fakeApi({ llmModel: "gpt-4o" });
    api.testLlmConnection.mockRejectedValueOnce(new Error("401 unauthorized"));
    render(<TagSourcesView api={api as unknown as Window["api"]} />);

    await screen.findByLabelText("AI model");
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));

    expect((await screen.findByText("401 unauthorized")).textContent).toBe("401 unauthorized");
  });
});
