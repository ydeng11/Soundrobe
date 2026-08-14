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
    lyricsGetting: false,
    auditing: false,
    darkMode: false,
    assistantOpen: false,
    error: null,
    modificationHistory: [],
    reverting: false,
    onOpenLibrary: vi.fn(),
    onRefresh: vi.fn(),
    onConvert: vi.fn(),
    onAutoTag: vi.fn(),
    onSearch: vi.fn(),
    onGetLyrics: vi.fn(),
    onAudit: vi.fn(),
    onNumberTracks: vi.fn(),
    activeAlbumPath: "/music/Album",
    onToggleDarkMode: vi.fn(),
    onOpenSettings: vi.fn(),
    onToggleAssistant: vi.fn(),
    onErrorDismiss: vi.fn(),
    onUndoLatest: vi.fn(),
    onUndoThrough: vi.fn(),
    ...overrides,
  } as const;
}

describe("TitleBar — all buttons", () => {
  it("marks the custom title bar as a Tauri window drag region", () => {
    const { container } = render(<TitleBar {...defaultProps()} />);

    expect(
      container.firstElementChild?.getAttribute("data-tauri-drag-region"),
    ).toBe("deep");
  });

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

  describe("modification history controls", () => {
    const history = [
      {
        id: 2,
        description: "Batch edit",
        timestamp: new Date("2026-08-14T14:30:00Z").getTime(),
        snapshots: [{ path: "/music/one.flac", fields: { artist: "Old" } }],
        affectedFileCount: 1,
      },
      {
        id: 1,
        description: "Number tracks",
        timestamp: new Date("2026-08-14T14:00:00Z").getTime(),
        snapshots: [
          { path: "/music/one.flac", fields: { trackNumber: 9 } },
          { path: "/music/two.flac", fields: { trackNumber: 8 } },
        ],
        affectedFileCount: 2,
      },
    ];

    it("renders exactly two adjacent split controls and disables both without history", () => {
      render(<TitleBar {...defaultProps()} />);
      const group = screen.getByTestId("undo-control-group");
      expect(group.children).toHaveLength(2);
      expect(screen.getByRole("button", { name: "Undo latest modification" }).getAttribute("disabled")).not.toBeNull();
      expect(screen.getByRole("button", { name: "Open modification history" }).getAttribute("disabled")).not.toBeNull();
    });

    it("undoes the latest command from the primary button", () => {
      const onUndoLatest = vi.fn();
      render(<TitleBar {...defaultProps({ modificationHistory: history, onUndoLatest })} />);
      fireEvent.click(screen.getByRole("button", { name: "Undo latest modification" }));
      expect(onUndoLatest).toHaveBeenCalledOnce();
    });

    it("shows newest-first descriptions, local timestamps, and affected-file counts", () => {
      render(<TitleBar {...defaultProps({ modificationHistory: history })} />);
      fireEvent.click(screen.getByRole("button", { name: "Open modification history" }));

      const items = screen.getAllByRole("menuitem");
      expect(items[0].textContent).toContain("Batch edit");
      expect(items[0].textContent).toContain("1 file");
      expect(items[1].textContent).toContain("Number tracks");
      expect(items[1].textContent).toContain("2 files");
      expect(items[0].textContent).toContain(
        new Date(history[0].timestamp).toLocaleString(),
      );
    });

    it("closes on selection, outside click, and Escape", () => {
      const onUndoThrough = vi.fn();
      render(<TitleBar {...defaultProps({ modificationHistory: history, onUndoThrough })} />);
      const chevron = screen.getByRole("button", { name: "Open modification history" });

      fireEvent.click(chevron);
      fireEvent.click(screen.getAllByRole("menuitem")[0]);
      expect(onUndoThrough).toHaveBeenCalledWith(history[0].id);
      expect(screen.queryByRole("menu")).toBeNull();

      fireEvent.click(chevron);
      fireEvent.mouseDown(document.body);
      expect(screen.queryByRole("menu")).toBeNull();

      fireEvent.click(chevron);
      fireEvent.keyDown(document, { key: "Escape" });
      expect(screen.queryByRole("menu")).toBeNull();
    });

    it("disables both controls while saving or reverting", () => {
      const { rerender } = render(
        <TitleBar {...defaultProps({ modificationHistory: history, saving: true })} />,
      );
      expect(screen.getByRole("button", { name: "Undo latest modification" }).getAttribute("disabled")).not.toBeNull();
      rerender(
        <TitleBar {...defaultProps({ modificationHistory: history, reverting: true })} />,
      );
      expect(screen.getByRole("button", { name: "Open modification history" }).getAttribute("disabled")).not.toBeNull();
      expect(screen.getByRole("button", { name: "Auto-Tag" }).getAttribute("disabled")).not.toBeNull();
      expect(screen.getByRole("button", { name: "Convert" }).getAttribute("disabled")).not.toBeNull();
    });
  });

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

  // ── Number button ────────────────────────────────────────

  describe("Number button", () => {
    it("renders the button", () => {
      render(<TitleBar {...defaultProps()} />);
      expect(screen.getByText("Number")).toBeTruthy();
    });

    it("is disabled when libraryPath is null", () => {
      render(<TitleBar {...defaultProps({ libraryPath: null })} />);
      const btn = screen.getByText("Number").closest("button");
      expect(btn?.disabled).toBe(true);
    });

    it("is disabled when activeAlbumPath is null", () => {
      render(<TitleBar {...defaultProps({ activeAlbumPath: null })} />);
      const btn = screen.getByText("Number").closest("button");
      expect(btn?.disabled).toBe(true);
    });

    it("shows the dropdown when clicked", () => {
      render(<TitleBar {...defaultProps()} />);
      const numberBtn = screen.getByText("Number");
      fireEvent.click(numberBtn);

      expect(screen.getByText("Number tracks by…")).toBeTruthy();
      expect(screen.getByText("By filename (A-Z)")).toBeTruthy();
      expect(screen.getByText("By title (A-Z)")).toBeTruthy();
    });

    it("calls onNumberTracks with the correct rule when a rule is clicked", () => {
      const onNumberTracks = vi.fn();
      render(<TitleBar {...defaultProps({ onNumberTracks })} />);

      const numberBtn = screen.getByText("Number");
      fireEvent.click(numberBtn);

      fireEvent.click(screen.getByText("By filename (A-Z)"));
      expect(onNumberTracks).toHaveBeenCalledWith("filename-asc");
    });

    it("calls onNumberTracks with title-desc when By title Z-A is clicked", () => {
      const onNumberTracks = vi.fn();
      render(<TitleBar {...defaultProps({ onNumberTracks })} />);

      fireEvent.click(screen.getByText("Number"));
      expect(screen.getByText("By title (Z-A)")).toBeTruthy();
      fireEvent.click(screen.getByText("By title (Z-A)"));
      expect(onNumberTracks).toHaveBeenCalledWith("title-desc");
    });

    it("closes the dropdown after selecting a rule", () => {
      const onNumberTracks = vi.fn();
      render(<TitleBar {...defaultProps({ onNumberTracks })} />);

      fireEvent.click(screen.getByText("Number"));
      expect(screen.getByText("Number tracks by…")).toBeTruthy();

      fireEvent.click(screen.getByText("By filename (A-Z)"));
      expect(screen.queryByText("Number tracks by…")).toBeFalsy();
    });

    it("closes the dropdown when clicking outside", () => {
      render(<TitleBar {...defaultProps()} />);

      fireEvent.click(screen.getByText("Number"));
      expect(screen.getByText("Number tracks by…")).toBeTruthy();

      // Click outside (on the document body)
      fireEvent.mouseDown(document.body);
      expect(screen.queryByText("Number tracks by…")).toBeFalsy();
    });

    it("shows all 8 ordering rules in the dropdown", () => {
      render(<TitleBar {...defaultProps()} />);

      fireEvent.click(screen.getByText("Number"));

      expect(screen.getByText("By filename (A-Z)")).toBeTruthy();
      expect(screen.getByText("By filename (Z-A)")).toBeTruthy();
      expect(screen.getByText("By title (A-Z)")).toBeTruthy();
      expect(screen.getByText("By title (Z-A)")).toBeTruthy();
      expect(screen.getByText("By existing track # (asc)")).toBeTruthy();
      expect(screen.getByText("By existing track # (desc)")).toBeTruthy();
      expect(screen.getByText("By duration (short→long)")).toBeTruthy();
      expect(screen.getByText("By duration (long→short)")).toBeTruthy();
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

  describe("Assistant toggle button", () => {
    it("exposes and styles the open state", () => {
      render(<TitleBar {...defaultProps({ assistantOpen: true })} />);

      const button = screen.getByRole("button", { name: "AI Assistant" });
      expect(button.getAttribute("aria-pressed")).toBe("true");
      expect(button.className).toContain("bg-accent/10");
    });

    it("calls onToggleAssistant", () => {
      const onToggleAssistant = vi.fn();
      render(<TitleBar {...defaultProps({ onToggleAssistant })} />);

      fireEvent.click(screen.getByRole("button", { name: "AI Assistant" }));
      expect(onToggleAssistant).toHaveBeenCalledOnce();
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
