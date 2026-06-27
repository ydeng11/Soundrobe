// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import React from "react";
import { cleanup, render, screen } from "@testing-library/react";
import { AuditPanel } from "../../src/components/AuditPanel";

afterEach(() => cleanup());

describe("AuditPanel", () => {
  it("renders audit source, confidence, and auto-fixed state for enriched results", () => {
    render(
      <AuditPanel
        albumName="Album"
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
            autoFixed: true,
          },
          {
            trackIndex: 1,
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

    expect(screen.getByText("2 issue(s) in Album")).toBeTruthy();
    expect(screen.getByText("Fixed")).toBeTruthy();
    expect(screen.getByText("deterministic 98%")).toBeTruthy();
    expect(screen.getByText("llm 61%")).toBeTruthy();
    expect(screen.getByText("Suggestion: Song")).toBeTruthy();
    expect(screen.getByText("Genre needs semantic review.")).toBeTruthy();
  });

  it("does not render an empty audit panel", () => {
    const { container } = render(<AuditPanel albumName="Album" results={[]} />);

    expect(container.firstChild).toBeNull();
  });
});
