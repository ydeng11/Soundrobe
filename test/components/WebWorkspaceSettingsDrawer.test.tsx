// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { WebWorkspaceSettingsDrawer } from "../../src/components/WebWorkspaceSettingsDrawer";

afterEach(cleanup);

describe("WebWorkspaceSettingsDrawer", () => {
  it("applies the draft detail preference only when Apply is chosen", () => {
    const onApply = vi.fn();
    const onClose = vi.fn();
    render(<WebWorkspaceSettingsDrawer detailsVisible visibleColumns={["filename", "title"]} onApply={onApply} onClose={onClose} />);

    expect(screen.getByRole("dialog", { name: "Workspace settings" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Show file details" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByText(/WebDAV/i)).toBeNull();

    fireEvent.click(screen.getByRole("switch", { name: "Show file details" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onApply).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("commits the detail preference through Apply", () => {
    const onApply = vi.fn();
    render(<WebWorkspaceSettingsDrawer detailsVisible visibleColumns={["filename", "title"]} onApply={onApply} onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("switch", { name: "Show file details" }));
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(onApply).toHaveBeenCalledWith(false, ["filename", "title"]);
  });

  it("applies selected file columns while keeping at least one visible", () => {
    const onApply = vi.fn();
    render(<WebWorkspaceSettingsDrawer detailsVisible visibleColumns={["filename", "title"]} onApply={onApply} onClose={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: "Remove Title" }));
    expect(screen.getByRole("button", { name: "Remove Location" }).hasAttribute("disabled")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(onApply).toHaveBeenCalledWith(true, ["filename"]);
  });
});
