// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { BatchEditor } from "../../src/components/BatchEditor";
import type { TrackData } from "../../src/shared/desktop-api";

afterEach(() => cleanup());

function makeTrack(path: string, overrides?: Partial<TrackData>): TrackData {
  return {
    path,
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

describe("BatchEditor", () => {
  it("renders standard batch fields", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByText("Batch Edit")).toBeTruthy();
    expect(screen.getByText("2 files selected")).toBeTruthy();
    expect(screen.getByText("Artist")).toBeTruthy();
    expect(screen.getByText("Album")).toBeTruthy();
    expect(screen.getByText("Genre")).toBeTruthy();
    expect(screen.getByText("Year")).toBeTruthy();
  });

  it("does not render an Extra Tags button in the Batch Edit panel", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    expect(screen.queryByText("Extra Tags")).toBeNull();
  });

  it("shows saving indicator when saving is true", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={true}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByText("Saving")).toBeTruthy();
  });

  it("shows unsaved indicator when a field is edited", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    fireEvent.change(artistInput, { target: { value: "New Artist" } });

    expect(screen.getByText(/Unsaved/i)).toBeTruthy();
  });

  it("calls onSave with filled fields when focus leaves the panel", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    const albumInput = screen.getByPlaceholderText("Common album…");
    fireEvent.change(artistInput, { target: { value: "New Artist" } });
    fireEvent.change(albumInput, { target: { value: "New Album" } });

    // Simulate focus leaving the panel
    const container = artistInput.closest('[class*="flex flex-col h-full overflow-y-auto"]');
    expect(container).toBeTruthy();
    fireEvent.blur(container!, { relatedTarget: null });

    expect(onSave).toHaveBeenCalledWith({
      artist: "New Artist",
      album: "New Album",
    });
  });

  it("does not call onSave when focus moves between fields within the panel", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    const albumInput = screen.getByPlaceholderText("Common album…");

    fireEvent.change(artistInput, { target: { value: "New Artist" } });

    // Moving focus from artist to album (both inside the panel)
    fireEvent.blur(artistInput, { relatedTarget: albumInput });

    expect(onSave).not.toHaveBeenCalled();
  });

  it("does not call onSave when all fields are empty on blur", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const container = screen.getByText("Batch Edit").closest('[class*="flex flex-col h-full overflow-y-auto"]');
    expect(container).toBeTruthy();
    fireEvent.blur(container!, { relatedTarget: null });

    expect(onSave).not.toHaveBeenCalled();
  });

  it("shows mixed indicator when tracks have differing values for a field", () => {
    const tracks = [
      makeTrack("/music/a.mp3", { artist: "Artist A" }),
      makeTrack("/music/b.mp3", { artist: "Artist B" }),
    ];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const mixedBadges = screen.getAllByText("mixed");
    expect(mixedBadges.length).toBeGreaterThanOrEqual(1);
  });

  it("displays cover art when coverDataUrl is provided", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl="data:image/jpeg;base64,abc123"
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const img = screen.getByAltText("Cover art");
    expect(img).toBeTruthy();
    expect(img.getAttribute("src")).toBe("data:image/jpeg;base64,abc123");
  });

  it("applies a genre edit explicitly to the selected tracks", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText("Common genre…"), {
      target: { value: "Jazz" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

    expect(onSave).toHaveBeenCalledWith({ genre: "Jazz" });
  });

  it("allows a batch field to be cleared explicitly", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const genre = screen.getByPlaceholderText("Common genre…");
    fireEvent.change(genre, { target: { value: "Jazz" } });
    fireEvent.change(genre, { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply changes" }));

    expect(onSave).toHaveBeenCalledWith({ genre: "" });
  });

  it("exposes the normal artwork actions in batch mode", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onChangeCover = vi.fn();
    const onRemoveCover = vi.fn();
    const onDownloadCover = vi.fn();
    const onDownloadArtistArt = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl="data:image/jpeg;base64,abc123"
        saving={false}
        onSave={vi.fn()}
        onChangeCover={onChangeCover}
        onRemoveCover={onRemoveCover}
        onDownloadCover={onDownloadCover}
        onDownloadArtistArt={onDownloadArtistArt}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Change cover" }));
    fireEvent.click(screen.getByRole("button", { name: "Download cover" }));
    fireEvent.click(screen.getByRole("button", { name: "Download artist image" }));
    fireEvent.click(screen.getByRole("button", { name: "Remove cover" }));

    expect(onChangeCover).toHaveBeenCalledOnce();
    expect(onDownloadCover).toHaveBeenCalledOnce();
    expect(onDownloadArtistArt).toHaveBeenCalledOnce();
    expect(onRemoveCover).toHaveBeenCalledOnce();
  });

  it("clears unsaved indicator after blur triggers save", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    fireEvent.change(artistInput, { target: { value: "New Artist" } });
    expect(screen.getByText(/Unsaved/i)).toBeTruthy();

    // Blur the panel
    const container = artistInput.closest('[class*="flex flex-col h-full overflow-y-auto"]');
    expect(container).toBeTruthy();
    fireEvent.blur(container!, { relatedTarget: null });

    // Indicator should be gone
    expect(screen.queryByText(/Unsaved/i)).toBeFalsy();
  });

  // ── Custom listbox (replaces native <datalist>) ──────────────

  it("does not render a native datalist or list attribute", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const { container } = render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    expect(document.querySelector("datalist")).toBeNull();

    const inputs = container.querySelectorAll('input[list]');
    expect(inputs.length).toBe(0);
  });

  it("shows the suggestion listbox on focus when suggestions exist", () => {
    const tracks = [makeTrack("/music/a.mp3", { artist: "Test Artist" })];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    expect(screen.queryByRole("listbox")).toBeNull();

    fireEvent.focus(artistInput);

    const listbox = screen.getByRole("listbox");
    expect(listbox).toBeTruthy();

    const options = screen.getAllByRole("option");
    expect(options.length).toBe(1);
    expect(options[0].textContent).toBe("Test Artist");
  });

  it("listbox has black background and white text", () => {
    const tracks = [makeTrack("/music/a.mp3", { artist: "Artist A" })];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    fireEvent.focus(artistInput);

    const listbox = screen.getByRole("listbox");
    expect(listbox.classList.contains("bg-[#000]")).toBe(true);

    const option = screen.getByRole("option");
    expect(option.classList.contains("text-white")).toBe(true);
  });

  it("selects a suggestion on click", () => {
    const tracks = [makeTrack("/music/a.mp3", { artist: "Selected Artist" })];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    fireEvent.focus(artistInput);

    const option = screen.getByRole("option");
    fireEvent.mouseDown(option);

    expect((artistInput as HTMLInputElement).value).toBe("Selected Artist");
    // Listbox should close after selection
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("selects a suggestion on click with multiple suggestions", () => {
    const tracks = [
      makeTrack("/music/a.mp3", { artist: "First" }),
      makeTrack("/music/b.mp3", { artist: "Second" }),
    ];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const inputs = screen.getAllByRole("combobox");
    const artistInput = inputs[0];
    fireEvent.focus(artistInput);

    const options = screen.getAllByRole("option");
    fireEvent.mouseDown(options[1]);

    expect((artistInput as HTMLInputElement).value).toBe("Second");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("selects a suggestion on Enter", () => {
    const tracks = [makeTrack("/music/a.mp3", { artist: "Entered Artist" })];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    fireEvent.focus(artistInput);

    // ArrowDown to activate the first option
    fireEvent.keyDown(artistInput, { key: "ArrowDown" });
    // Press Enter to select
    fireEvent.keyDown(artistInput, { key: "Enter" });

    expect((artistInput as HTMLInputElement).value).toBe("Entered Artist");
    // Listbox should close after selection
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("closes listbox on Escape", () => {
    const tracks = [makeTrack("/music/a.mp3", { artist: "Artist" })];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    fireEvent.focus(artistInput);
    expect(screen.getByRole("listbox")).toBeTruthy();

    fireEvent.keyDown(artistInput, { key: "Escape" });

    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("filters suggestions as the user types", () => {
    // Same artist across tracks — avoids the "mixed" placeholder
    const tracks = [
      makeTrack("/music/a.mp3", { artist: "Alpha" }),
      makeTrack("/music/b.mp3", { artist: "Beta" }),
      makeTrack("/music/c.mp3", { artist: "Gamma" }),
    ];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const inputs = screen.getAllByRole("combobox");
    const artistInput = inputs[0];
    fireEvent.focus(artistInput);
    let options = screen.getAllByRole("option");
    expect(options.length).toBe(3);

    // Type "al" — should match "Alpha" only
    fireEvent.change(artistInput, { target: { value: "al" } });
    options = screen.getAllByRole("option");
    expect(options.length).toBe(1);
    expect(options[0].textContent).toBe("Alpha");
  });

  it("ArrowDown opens listbox and navigates", () => {
    // Same artist across tracks — avoids the "mixed" placeholder
    const tracks = [
      makeTrack("/music/a.mp3", { artist: "Same Artist" }),
      makeTrack("/music/b.mp3", { artist: "Same Artist" }),
    ];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const inputs = screen.getAllByRole("combobox");
    const artistInput = inputs[0];
    // ArrowDown opens the listbox and activates first option
    fireEvent.keyDown(artistInput, { key: "ArrowDown" });

    const listbox = screen.getByRole("listbox");
    expect(listbox).toBeTruthy();

    const options = screen.getAllByRole("option");
    expect(options.length).toBe(1);
    expect(options[0].getAttribute("aria-selected")).toBe("true");
  });
});
