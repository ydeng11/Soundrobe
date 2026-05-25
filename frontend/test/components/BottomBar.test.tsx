// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";

afterEach(() => cleanup());
import { BottomBar } from "../../src/components/BottomBar";

describe("BottomBar", () => {
  const defaultProps = {
    filterText: "",
    onFilterChange: vi.fn(),
    totalFiles: 10,
    selectedFilePath: null,
    dirtyCount: 0,
    canUndo: false,
    saving: false,
    error: null,
    onSave: vi.fn(),
    onRevert: vi.fn(),
    onConvert: vi.fn(),
    onAutonumber: vi.fn(),
    onRename: vi.fn(),
  };

  it("shows total file count", () => {
    render(<BottomBar {...defaultProps} totalFiles={5} />);
    expect(screen.getByText("5 files")).toBeTruthy();
  });

  it("shows singular 'file' for count of 1", () => {
    render(<BottomBar {...defaultProps} totalFiles={1} />);
    expect(screen.getByText("1 file")).toBeTruthy();
  });

  it("shows '1 selected' when a file is selected and no dirty", () => {
    render(
      <BottomBar
        {...defaultProps}
        selectedFilePath="/music/song.mp3"
        dirtyCount={0}
      />
    );
    expect(screen.getByText("1 selected")).toBeTruthy();
  });

  it("shows unsaved count when there are dirty tracks", () => {
    render(
      <BottomBar
        {...defaultProps}
        selectedFilePath="/music/song.mp3"
        dirtyCount={3}
      />
    );
    expect(screen.getByText("3 unsaved")).toBeTruthy();
    // "1 selected" should NOT appear when dirty
    expect(screen.queryByText("1 selected")).toBeFalsy();
  });

  it("disables Save button when no dirty tracks", () => {
    render(<BottomBar {...defaultProps} dirtyCount={0} />);
    const saveBtn = screen.getByText("Save").closest("button");
    expect(saveBtn?.disabled).toBe(true);
  });

  it("enables Save button when there are dirty tracks", () => {
    render(<BottomBar {...defaultProps} dirtyCount={2} />);
    const saveBtn = screen.getByText("Save").closest("button");
    expect(saveBtn?.disabled).toBe(false);
  });

  it("shows 'Saving…' and disables Save when saving", () => {
    render(<BottomBar {...defaultProps} dirtyCount={2} saving={true} />);
    expect(screen.getByText("Saving…")).toBeTruthy();
    const saveBtn = screen.getByText("Saving…").closest("button");
    expect(saveBtn?.disabled).toBe(true);
  });

  it("disables Revert button when canUndo is false", () => {
    render(<BottomBar {...defaultProps} canUndo={false} />);
    const revertBtn = screen.getByText("Revert").closest("button");
    expect(revertBtn?.disabled).toBe(true);
  });

  it("enables Revert button when canUndo is true", () => {
    render(<BottomBar {...defaultProps} canUndo={true} />);
    const revertBtn = screen.getByText("Revert").closest("button");
    expect(revertBtn?.disabled).toBe(false);
  });

  it("calls onSave when Save is clicked", () => {
    const onSave = vi.fn();
    render(<BottomBar {...defaultProps} dirtyCount={1} onSave={onSave} />);
    fireEvent.click(screen.getByText("Save"));
    expect(onSave).toHaveBeenCalledOnce();
  });

  it("calls onRevert when Revert is clicked", () => {
    const onRevert = vi.fn();
    render(<BottomBar {...defaultProps} canUndo={true} onRevert={onRevert} />);
    fireEvent.click(screen.getByText("Revert"));
    expect(onRevert).toHaveBeenCalledOnce();
  });

  it("calls onConvert when Convert is clicked", () => {
    const onConvert = vi.fn();
    render(<BottomBar {...defaultProps} onConvert={onConvert} />);
    fireEvent.click(screen.getByText("Convert"));
    expect(onConvert).toHaveBeenCalledOnce();
  });

  it("calls onAutonumber when Autonumber is clicked", () => {
    const onAutonumber = vi.fn();
    render(<BottomBar {...defaultProps} onAutonumber={onAutonumber} />);
    fireEvent.click(screen.getByText("Autonumber"));
    expect(onAutonumber).toHaveBeenCalledOnce();
  });

  it("calls onRename when Rename is clicked", () => {
    const onRename = vi.fn();
    render(<BottomBar {...defaultProps} onRename={onRename} />);
    fireEvent.click(screen.getByText("Rename"));
    expect(onRename).toHaveBeenCalledOnce();
  });

  it("displays error message when error is set", () => {
    render(<BottomBar {...defaultProps} error="Something went wrong" />);
    expect(screen.getByText(/Something went wrong/i)).toBeTruthy();
    // File count should not show when there's an error
    expect(screen.queryByText("10 files")).toBeFalsy();
  });

  it("updates filter text on input change", () => {
    const onFilterChange = vi.fn();
    render(<BottomBar {...defaultProps} onFilterChange={onFilterChange} />);
    const input = screen.getByPlaceholderText("Filter files...");
    fireEvent.change(input, { target: { value: "test" } });
    expect(onFilterChange).toHaveBeenCalledWith("test");
  });

  it("renders all action buttons", () => {
    render(<BottomBar {...defaultProps} />);
    expect(screen.getByText("Save")).toBeTruthy();
    expect(screen.getByText("Revert")).toBeTruthy();
    expect(screen.getByText("Convert")).toBeTruthy();
    expect(screen.getByText("Autonumber")).toBeTruthy();
    expect(screen.getByText("Rename")).toBeTruthy();
    expect(screen.getByText("Help")).toBeTruthy();
  });
});
