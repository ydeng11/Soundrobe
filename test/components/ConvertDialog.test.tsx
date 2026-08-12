// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ConvertDialog } from "../../src/components/ConvertDialog";
import type { TrackPreviewData } from "../../src/components/ConvertDialog";

afterEach(() => cleanup());

const track: TrackPreviewData = {
  filename: "01 - Old Title.flac",
  title: "New Artist - New Title",
  artist: "Old Artist",
  album: "Album",
  year: "2026",
  track: 1,
  genre: "Pop",
  albumArtist: "Album Artist",
  composer: "Composer",
  comment: "Comment",
  discNumber: 1,
};

describe("ConvertDialog", () => {
  it("renders readable direction labels without escaped unicode codes", () => {
    render(
      <ConvertDialog
        open
        tracks={[track]}
        onClose={vi.fn()}
        onConvert={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Filename -> Tags" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Tag -> Tags" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Tags -> Filename" })).toBeTruthy();
    expect(screen.queryByText(/u2192/)).toBeNull();
  });

  it("lets users edit a preset pattern directly", () => {
    render(
      <ConvertDialog
        open
        tracks={[track]}
        onClose={vi.fn()}
        onConvert={vi.fn()}
      />,
    );

    const pattern = screen.getByDisplayValue("%{track}% %{title}%");
    expect(pattern.getAttribute("readonly")).toBeNull();
    fireEvent.change(pattern, { target: { value: "%{track}% - %{artist}% - %{title}%" } });

    expect(screen.getByText("No match - pattern does not fit this value")).toBeTruthy();
  });

  it("removes Regex and Custom modes from the preset list", () => {
    render(
      <ConvertDialog
        open
        tracks={[track]}
        onClose={vi.fn()}
        onConvert={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: "Regex" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Custom" })).toBeNull();
  });

  it("converts one source tag into multiple writable tags", () => {
    const onConvert = vi.fn();
    const onClose = vi.fn();
    render(
      <ConvertDialog
        open
        tracks={[track]}
        onClose={onClose}
        onConvert={onConvert}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Tag -> Tags" }));
    expect(screen.getByText("Artist=New Artist, Title=New Title")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Convert" }));

    expect(onConvert).toHaveBeenCalledWith({
      direction: "tag-to-tags",
      pattern: "%{artist}% - %{title}%",
      presetLabel: "Title has Artist - Title",
      writeFields: {
        artist: "New Artist",
        title: "New Title",
      },
      sourceFilename: undefined,
      sourceTag: "title",
      sourceValue: "New Artist - New Title",
    });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("shows track count badge for multiple tracks", () => {
    render(
      <ConvertDialog
        open
        tracks={[track, track]}
        onClose={vi.fn()}
        onConvert={vi.fn()}
      />,
    );

    expect(screen.getByText("2 tracks")).toBeTruthy();
    expect(screen.getByText("Convert (2 tracks)")).toBeTruthy();
  });

  it("shows disabled message when tracks array is empty", () => {
    render(
      <ConvertDialog
        open
        tracks={[]}
        onClose={vi.fn()}
        onConvert={vi.fn()}
      />,
    );

    expect(screen.getByText("Select a file to convert")).toBeTruthy();
  });
});
