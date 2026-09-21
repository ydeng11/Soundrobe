// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { WebApp } from "../../src/web/WebApp";
import { WebLibraryPicker } from "../../src/components/WebLibraryPicker";

afterEach(cleanup);

describe("WebApp", () => {
  it("opens the browser workspace without a login request or sign-in screen", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ releases: [] }), { status: 200 }),
    );
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([]),
    } as unknown as Window["api"];

    render(<WebApp api={api} />);

    expect(await screen.findByTestId("workspace-no-root")).toBeTruthy();
    expect(screen.queryByText(/Sign in to Soundrobe/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull();
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes("/auth/"))).toBe(false);
    fetchMock.mockRestore();
  });
});

describe("WebLibraryPicker", () => {
  it("lists mounted roots and selects the requested root", () => {
    const onSelect = vi.fn();
    render(
      <WebLibraryPicker
        roots={[
          { id: "jazz", name: "jazz", path: "/libraries/jazz" },
          { id: "classical", name: "classical", path: "/libraries/classical" },
        ]}
        onSelect={onSelect}
      />,
    );

    expect(screen.getByText("Choose a library")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /jazz/ }));
    expect(onSelect).toHaveBeenCalledWith("/libraries/jazz");
  });

  it("shows a distinct loading state without offering stale selections", () => {
    render(
      <WebLibraryPicker
        roots={[{ id: "jazz", name: "jazz", path: "/libraries/jazz" }]}
        loading
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByRole("status").textContent).toContain("Loading libraries…");
    expect(screen.queryByRole("button", { name: /jazz/ })).toBeNull();
  });

  it("shows a root-loading error and does not expose selections", () => {
    render(
      <WebLibraryPicker
        roots={[{ id: "jazz", name: "jazz", path: "/libraries/jazz" }]}
        error="The library service is unavailable"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("The library service is unavailable");
    expect(screen.queryByRole("button", { name: /jazz/ })).toBeNull();
  });

  it("explains when the service has no mounted roots", () => {
    render(<WebLibraryPicker roots={[]} onSelect={vi.fn()} />);

    expect(screen.getByText(/No libraries are mounted\./)).toBeTruthy();
  });
});
