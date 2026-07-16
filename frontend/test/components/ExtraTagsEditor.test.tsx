// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import { ExtraTagsEditor } from "../../src/components/ExtraTagsEditor";
import type { TrackData } from "../../src/shared/desktop-api";

afterEach(() => cleanup());

function makeTrack(overrides?: Partial<TrackData>): TrackData {
  return {
    path: "/music/song.mp3",
    title: "Song",
    artist: "Artist",
    artists: [],
    album: "Album",
    albumArtist: null,
    albumArtists: [],
    trackNumber: 1,
    trackTotal: 10,
    discNumber: null,
    discTotal: null,
    year: "2024",
    genre: "Pop",
    composer: null,
    comment: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover: false,
    sizeBytes: 123,
    bitrate: null,
    sampleRate: null,
    codec: "MP3",
    duration: 60,
    ...overrides,
  };
}

describe("ExtraTagsEditor", () => {
  beforeEach(() => {
    window.api = {
      readExtraTags: vi.fn().mockResolvedValue([
        { key: "MOOD", value: "Bright", source: "Vorbis" },
      ]),
    } as unknown as Window["api"];
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  it("loads and displays existing extra tags", async () => {
    render(
      <ExtraTagsEditor
        track={makeTrack()}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    expect(await screen.findByDisplayValue("MOOD")).toBeTruthy();
    expect(screen.getByDisplayValue("Bright")).toBeTruthy();
  });

  it("adds a custom tag and saves non-deleted rows", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <ExtraTagsEditor
        track={makeTrack()}
        saving={false}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    await screen.findByDisplayValue("MOOD");
    fireEvent.click(screen.getByText("Add Custom Tag"));

    const keyInputs = screen.getAllByPlaceholderText("Tag key");
    const valueInputs = screen.getAllByPlaceholderText("Value");
    fireEvent.change(keyInputs[keyInputs.length - 1], {
      target: { value: "CATALOGNUMBER" },
    });
    fireEvent.change(valueInputs[valueInputs.length - 1], {
      target: { value: "ABC-123" },
    });
    fireEvent.click(screen.getByText("Save Changes"));

    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith([
        { key: "MOOD", value: "Bright" },
        { key: "CATALOGNUMBER", value: "ABC-123" },
      ]),
    );
  });

  it("allows multiple ARTISTS rows to be saved as extra tags", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    window.api = {
      readExtraTags: vi.fn().mockResolvedValue([]),
    } as unknown as Window["api"];

    render(
      <ExtraTagsEditor
        track={makeTrack({ path: "/music/song.flac", codec: "FLAC" })}
        saving={false}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    await screen.findByText("No extra tags");

    fireEvent.click(screen.getByText("Add Custom Tag"));
    fireEvent.click(screen.getByText("Add Custom Tag"));

    const keyInputs = screen.getAllByPlaceholderText("Tag key");
    const valueInputs = screen.getAllByPlaceholderText("Value");
    fireEvent.change(keyInputs[0], { target: { value: "ARTISTS" } });
    fireEvent.change(valueInputs[0], { target: { value: "foo" } });
    fireEvent.change(keyInputs[1], { target: { value: "ARTISTS" } });
    fireEvent.change(valueInputs[1], { target: { value: "bar" } });

    fireEvent.click(screen.getByText("Save Changes"));

    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith([
        { key: "ARTISTS", value: "foo" },
        { key: "ARTISTS", value: "bar" },
      ]),
    );
  });

  it("marks deleted rows and asks before discarding dirty changes", async () => {
    const onClose = vi.fn();
    render(
      <ExtraTagsEditor
        track={makeTrack()}
        saving={false}
        onClose={onClose}
        onSave={vi.fn()}
      />,
    );

    await screen.findByDisplayValue("MOOD");
    fireEvent.click(screen.getByLabelText("Delete tag"));
    fireEvent.click(screen.getByText("Cancel"));

    expect(window.confirm).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });
});
