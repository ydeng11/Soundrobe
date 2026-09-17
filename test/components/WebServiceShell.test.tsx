// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { WebServiceShell } from "../../src/components/WebServiceShell";

afterEach(cleanup);

describe("WebServiceShell", () => {
  it("renders the library landing content and exposes accessible primary navigation", () => {
    render(
      <WebServiceShell activeView="library">
        <div data-testid="library-landing">Choose a library</div>
      </WebServiceShell>,
    );

    expect(screen.getByRole("navigation", { name: "Primary navigation" })).toBeTruthy();
    expect(screen.getByTestId("library-landing")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Library" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("button", { name: "Activity" }).getAttribute("aria-current")).toBeNull();
    expect(screen.getByRole("button", { name: "Settings" }).getAttribute("aria-current")).toBeNull();
  });

  it("reports navigation intent without owning routes or server state", () => {
    const onViewChange = vi.fn();
    render(
      <WebServiceShell activeView="library" onViewChange={onViewChange}>
        <p>Landing</p>
      </WebServiceShell>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Activity" }));
    expect(onViewChange).toHaveBeenCalledWith("activity");
    expect(screen.getByText("Landing")).toBeTruthy();
  });

  it("constrains the shell to the viewport and leaves the content region scrollable", () => {
    render(
      <WebServiceShell>
        <div>Long landing content</div>
      </WebServiceShell>,
    );

    const shell = screen.getByTestId("web-service-shell");
    const main = screen.getByRole("main");
    expect(shell.className).toMatch(/\bh-screen\b/);
    expect(shell.className).toMatch(/\boverflow-hidden\b/);
    expect(main.className).toMatch(/\bmin-h-0\b/);
    expect(main.className).toMatch(/\boverflow-y-auto\b/);
  });
});
