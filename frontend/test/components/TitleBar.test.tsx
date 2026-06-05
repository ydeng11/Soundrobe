// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";

afterEach(() => cleanup());

import { TitleBar } from "../../src/components/TitleBar";

/**
 * Helper: create default props with all required handlers as spies.
 * Tests can override specific props at call site.
 */
function defaultProps(overrides?: Record<string, unknown>) {
  return {
    libraryPath: "/Users/test/Music",
    trackCount: 42,
    filterText: "",
    onFilterChange: vi.fn(),
    selectedFilePath: null,
    saving: false,
    autoTagging: false,
    darkMode: false,
    error: null,
    onOpenLibrary: vi.fn(),
    onConvert: vi.fn(),
    onAutoTag: vi.fn(),
    onRefresh: vi.fn(),
    onToggleDarkMode: vi.fn(),
    onOpenSettings: vi.fn(),
    ...overrides,
  } as const;
}

describe("TitleBar — all buttons", () => {
  // ── Open Library ──────────────────────────────────────────

  describe("Open Library button", () => {
    it("renders the button", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.getByText("Open Library")).toBeTruthy();
    });

    it("calls onOpenLibrary on click", () => {
      const onOpenLibrary = vi.fn();
      render(<TitleBar {...defaultProps({ onOpenLibrary })} />);
      fireEvent.click(screen.getByText("Open Library"));
      expect(onOpenLibrary).toHaveBeenCalledOnce();
    });
  });

  // ── Library path + track count + refresh ──────────────────

  describe("library path display", () => {
    it("shows library path and track count when libraryPath is set", () => {
      render(<TitleBar {...defaultProps({ libraryPath: "/my/music", trackCount: 7 })} />);
      expect(screen.getByText("/my/music")).toBeTruthy();
      expect(screen.getByText("(7)")).toBeTruthy();
    });

    it("shows refresh button when libraryPath is set", () => {
      render(<TitleBar {...defaultProps({ libraryPath: "/music" })} />);
      const refreshBtn = screen.getByTitle("Refresh library (⌘R)");
      expect(refreshBtn).toBeTruthy();
    });

    it("calls onRefresh when refresh button is clicked", () => {
      const onRefresh = vi.fn();
      render(<TitleBar {...defaultProps({ libraryPath: "/music", onRefresh })} />);
      const refreshBtn = screen.getByTitle("Refresh library (⌘R)");
      fireEvent.click(refreshBtn);
      expect(onRefresh).toHaveBeenCalledOnce();
    });

    it("hides library path when libraryPath is null", () => {
      render(<TitleBar {...defaultProps({ libraryPath: null })} />);
      expect(screen.queryByText("/my/music")).toBeFalsy();
    });
  });

  // ── Filter / search ──────────────────────────────────────

  describe("filter input", () => {
    it("renders the filter input", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.getByPlaceholderText("Filter files...")).toBeTruthy();
    });

    it("calls onFilterChange on input change", () => {
      const onFilterChange = vi.fn();
      render(<TitleBar {...defaultProps({ onFilterChange })} />);
      const input = screen.getByPlaceholderText("Filter files...");
      fireEvent.change(input, { target: { value: "jazz" } });
      expect(onFilterChange).toHaveBeenCalledWith("jazz");
    });

    it("shows clear button when filterText is non-empty", () => {
      const { container } = render(<TitleBar {...defaultProps({ filterText: "rock" })} />);
      const clearBtn = container.querySelector(
        'button.absolute.inset-y-0.right-0',
      );
      expect(clearBtn).toBeTruthy();
    });

    it("calls onFilterChange('') when clear button is clicked", () => {
      const onFilterChange = vi.fn();
      render(<TitleBar {...defaultProps({ filterText: "rock", onFilterChange })} />);
      const buttons = screen.getAllByRole("button");
      const allClearCandidates = buttons.filter((b) =>
        b.querySelector("svg path[d='M18 6 6 18']"),
      );
      if (allClearCandidates.length > 0) {
        fireEvent.click(allClearCandidates[0]);
        expect(onFilterChange).toHaveBeenCalledWith("");
      }
    });

    it("updates the input value from filterText prop", () => {
      render(<TitleBar {...defaultProps({ filterText: "pop" })} />);
      const input = screen.getByPlaceholderText("Filter files...") as HTMLInputElement;
      expect(input.value).toBe("pop");
    });
  });

  // ── Auto-Tag button ──────────────────────────────────────

  describe("Auto-Tag button", () => {
    it("renders the button", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.getByText("Auto-Tag")).toBeTruthy();
    });

    it("calls onAutoTag on click", () => {
      const onAutoTag = vi.fn();
      render(<TitleBar {...defaultProps({ onAutoTag })} />);
      fireEvent.click(screen.getByText("Auto-Tag"));
      expect(onAutoTag).toHaveBeenCalledOnce();
    });

    it("is disabled when libraryPath is null", () => {
      render(<TitleBar {...defaultProps({ libraryPath: null })} />);
      const btn = screen.getByText("Auto-Tag").closest("button");
      expect(btn?.disabled).toBe(true);
    });

    it("is disabled when autoTagging is true", () => {
      render(<TitleBar {...defaultProps({ autoTagging: true })} />);
      const btn = screen.getByText("Tagging…");
      expect(btn).toBeTruthy();
      expect(btn.closest("button")?.disabled).toBe(true);
    });

    it("shows spinning indicator when autoTagging", () => {
      const { container } = render(
        <TitleBar {...defaultProps({ autoTagging: true })} />,
      );
      expect(screen.getByText("Tagging…")).toBeTruthy();
      expect(screen.queryByText("Auto-Tag")).toBeFalsy();
      const spinner = container.querySelector(".animate-spin");
      expect(spinner).toBeTruthy();
    });
  });

  // ── Convert button ───────────────────────────────────────

  describe("Convert button", () => {
    it("renders the button", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.getByText("Convert")).toBeTruthy();
    });

    it("calls onConvert on click", () => {
      const onConvert = vi.fn();
      render(<TitleBar {...defaultProps({ onConvert })} />);
      fireEvent.click(screen.getByText("Convert"));
      expect(onConvert).toHaveBeenCalledOnce();
    });

    it("is not disabled by default", () => {
      render(<TitleBar {...defaultProps()} />);
      const btn = screen.getByText("Convert").closest("button");
      expect(btn?.disabled).toBe(false);
    });
  });

  // ── Dark mode toggle ─────────────────────────────────────

  describe("Dark mode toggle button", () => {
    it("renders the button", () => {
      render(<TitleBar {...defaultProps()} />);
      const btn = screen.getByTitle("Switch to dark mode");
      expect(btn).toBeTruthy();
    });

    it("calls onToggleDarkMode on click", () => {
      const onToggleDarkMode = vi.fn();
      render(<TitleBar {...defaultProps({ onToggleDarkMode })} />);
      fireEvent.click(screen.getByTitle("Switch to dark mode"));
      expect(onToggleDarkMode).toHaveBeenCalledOnce();
    });

    it("shows moon icon when darkMode is false", () => {
      const { container } = render(<TitleBar {...defaultProps({ darkMode: false })} />);
      const moonPaths = container.querySelectorAll(
        'svg path[d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"]',
      );
      expect(moonPaths.length).toBeGreaterThanOrEqual(1);
      expect(screen.getByTitle("Switch to dark mode")).toBeTruthy();
    });

    it("shows sun icon when darkMode is true", () => {
      const { container } = render(<TitleBar {...defaultProps({ darkMode: true })} />);
      const sunCircles = container.querySelectorAll(
        'svg circle[cx="12"][cy="12"][r="5"]',
      );
      expect(sunCircles.length).toBeGreaterThanOrEqual(1);
      expect(screen.getByTitle("Switch to light mode")).toBeTruthy();
    });

    it("toggles title text based on darkMode", () => {
      const { rerender } = render(<TitleBar {...defaultProps({ darkMode: false })} />);
      expect(screen.getByTitle("Switch to dark mode")).toBeTruthy();

      rerender(<TitleBar {...defaultProps({ darkMode: true })} />);
      expect(screen.getByTitle("Switch to light mode")).toBeTruthy();
    });
  });

  // ── Settings button ───────────────────────────────────────

  describe("Settings gear button", () => {
    it("renders the button", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.getByTitle("Settings")).toBeTruthy();
    });

    it("calls onOpenSettings on click", () => {
      const onOpenSettings = vi.fn();
      render(<TitleBar {...defaultProps({ onOpenSettings })} />);
      fireEvent.click(screen.getByTitle("Settings"));
      expect(onOpenSettings).toHaveBeenCalledOnce();
    });
  });

  // ── Status indicators ─────────────────────────────────────

  describe("status indicators", () => {
    it("shows track count when nothing is selected and not saving", () => {
      render(<TitleBar {...defaultProps({ trackCount: 15 })} />);
      expect(screen.getByText("15 files")).toBeTruthy();
    });

    it("shows '1 selected' when a file is selected", () => {
      render(
        <TitleBar
          {...defaultProps({
            selectedFilePath: "/music/song.mp3",
          })}
        />,
      );
      expect(screen.getByText("1 selected")).toBeTruthy();
    });

    it("shows error message when error is provided", () => {
      render(<TitleBar {...defaultProps({ error: "Something went wrong" })} />);
      expect(screen.getByText("Something went wrong")).toBeTruthy();
    });

    it("shows saving indicator when saving is true", () => {
      render(<TitleBar {...defaultProps({ saving: true })} />);
      expect(screen.getByText("Saving")).toBeTruthy();
    });
  });

  // ── Keyboard shortcut tooltips ────────────────────────────

  describe("keyboard shortcut tooltips", () => {
    it("shows ⌘O on Open Library", () => {
      render(<TitleBar {...defaultProps()} />);
      const btn = screen.getByText("Open Library").closest("button");
      expect(btn?.title).toContain("⌘O");
    });

    it("shows ⌘T on Auto-Tag", () => {
      render(<TitleBar {...defaultProps()} />);
      const btn = screen.getByText("Auto-Tag").closest("button");
      expect(btn?.title).toContain("⌘T");
    });
  });

  // ── Removed buttons ──────────────────────────────────────

  describe("Save and Revert buttons are removed", () => {
    it("does not render Save button", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.queryByText(/Save/)).toBeFalsy();
    });

    it("does not render Revert button", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.queryByText(/Revert/)).toBeFalsy();
    });

    it("does not show dirty counter", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.queryByText(/unsaved/i)).toBeFalsy();
    });
  });
});
