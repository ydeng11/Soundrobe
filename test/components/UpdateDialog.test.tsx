// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { UpdateDialog } from "../../src/components/UpdateDialog";
import type { AppUpdateInfo } from "../../src/shared/desktop-api";

const update: AppUpdateInfo = {
  currentVersion: "0.1.0",
  availableVersion: "0.2.0",
  date: "2026-08-14T12:00:00Z",
  notes: "Safer updates and release verification.",
};

afterEach(cleanup);

describe("UpdateDialog", () => {
  it("shows release details and lets the user defer before installation", () => {
    const onLater = vi.fn();
    render(
      <UpdateDialog
        update={update}
        busy={false}
        installing={false}
        progress={null}
        error={null}
        onLater={onLater}
        onInstall={vi.fn()}
      />,
    );

    expect(screen.getByText("Soundrobe 0.2.0 is available")).toBeTruthy();
    expect(screen.getByText(update.notes!)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(onLater).toHaveBeenCalledTimes(1);
  });

  it("blocks acceptance while protected app work is busy", () => {
    const onInstall = vi.fn();
    render(
      <UpdateDialog
        update={update}
        busy={true}
        installing={false}
        progress={null}
        error={null}
        onLater={vi.fn()}
        onInstall={onInstall}
      />,
    );

    expect(screen.getByText(/finish the current disk operation/i)).toBeTruthy();
    const install = screen.getByRole("button", { name: "Download and restart" });
    expect((install as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(install);
    expect(onInstall).not.toHaveBeenCalled();
  });

  it("shows determinate progress and cannot be dismissed while installing", () => {
    const onLater = vi.fn();
    render(
      <UpdateDialog
        update={update}
        busy={false}
        installing={true}
        progress={{ phase: "downloading", downloaded: 25, total: 100 }}
        error={null}
        onLater={onLater}
        onInstall={vi.fn()}
      />,
    );

    const progress = screen.getByRole("progressbar") as HTMLProgressElement;
    expect(progress.value).toBe(25);
    expect(progress.max).toBe(100);
    expect(screen.queryByRole("button", { name: "Later" })).toBeNull();
  });

  it("allows retry and deferral after installation fails", () => {
    const onInstall = vi.fn();
    render(
      <UpdateDialog
        update={update}
        busy={false}
        installing={false}
        progress={null}
        error="Signature verification failed"
        onLater={vi.fn()}
        onInstall={onInstall}
      />,
    );

    expect(screen.getByText("Signature verification failed")).toBeTruthy();
    fireEvent.click(
      screen.getByRole("button", { name: "Retry download and restart" }),
    );
    expect(onInstall).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Later" })).toBeTruthy();
  });
});
