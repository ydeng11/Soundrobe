// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { AuditPanel, SelectedTrackAuditFindings } from "../../src/components/AuditPanel";

afterEach(() => cleanup());

describe("AuditPanel", () => {
  it("groups audit findings by track and presents a fix plan for approval", () => {
    const onApplyFixes = vi.fn();
    render(
      <AuditPanel
        albumName="Album"
        onApplyFixes={onApplyFixes}
        results={[
          {
            trackIndex: 0,
            field: "title",
            status: "error",
            message: "Title does not match filename.",
            suggestion: "Song",
            source: "deterministic",
            confidence: 0.98,
            autoFixEligible: true,
            autoFixed: false,
            corrected: { title: "Song" },
          },
          {
            trackIndex: 0,
            field: "genre",
            status: "warning",
            message: "Genre needs semantic review.",
            suggestion: "Rock",
            source: "llm",
            confidence: 0.61,
            autoFixEligible: false,
            autoFixed: false,
          },
        ]}
      />,
    );

    expect(screen.getByText("2 issue(s) across 1 track(s) in Album")).toBeTruthy();
    expect(screen.getByText("Fix Plan")).toBeTruthy();
    expect(screen.getByText("1 fixable field(s), 1 manual-review field(s)")).toBeTruthy();
    expect(screen.getByText("Track 1")).toBeTruthy();
    expect(screen.getByText("1 will fix, 1 needs manual review")).toBeTruthy();
    expect(screen.getByText("Will fix")).toBeTruthy();
    expect(screen.getByText("deterministic 98%")).toBeTruthy();
    expect(screen.getByText("llm 61%")).toBeTruthy();
    expect(screen.getByText("Suggestion: Song")).toBeTruthy();
    expect(screen.getByText("Will write: Song")).toBeTruthy();
    expect(screen.getByText("Genre needs semantic review.")).toBeTruthy();
    fireEvent.click(screen.getByText("Apply Audit Fixes"));
    expect(onApplyFixes).toHaveBeenCalledTimes(1);
  });

  it("does not render an empty audit panel", () => {
    const { container } = render(<AuditPanel albumName="Album" results={[]} />);

    expect(container.firstChild).toBeNull();
  });

  it("renders compact selected-track audit details with fix approval", () => {
    const onApplyFixes = vi.fn();
    render(
      <SelectedTrackAuditFindings
        onApplyFixes={onApplyFixes}
        results={[
          {
            trackIndex: 0,
            field: "albumArtist",
            status: "warning",
            message: "Album artist needs review.",
            suggestion: "Artist",
            autoFixEligible: true,
            autoFixed: false,
            corrected: { albumArtist: "Artist" },
          },
        ]}
      />,
    );

    expect(screen.getByText("Audit Findings")).toBeTruthy();
    expect(screen.getByText("Fix plan: 1 field(s) can be applied after approval.")).toBeTruthy();
    expect(screen.getByText("albumArtist")).toBeTruthy();
    expect(screen.getByText("Will write: Artist")).toBeTruthy();
    expect(screen.getByText("Album artist needs review.")).toBeTruthy();
    fireEvent.click(screen.getByText("Apply Audit Fixes"));
    expect(onApplyFixes).toHaveBeenCalledTimes(1);
  });
});
