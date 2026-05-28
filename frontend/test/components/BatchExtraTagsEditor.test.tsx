// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import { BatchExtraTagsEditor } from "../../src/components/BatchExtraTagsEditor";
import type { TrackData } from "../../electron/preload";

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

describe("BatchExtraTagsEditor", () => {
  beforeEach(() => {
    window.confirm = vi.fn().mockReturnValue(true) as unknown as typeof window.confirm;
    window.api = {
      readExtraTags: vi.fn().mockResolvedValue([]),
    } as unknown as Window["api"];
  });

  it("renders header with track count", () => {
    const tracks = [makeTrack(), makeTrack({ path: "/music/song2.mp3" })];
    render(
      <BatchExtraTagsEditor
        tracks={tracks}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByText("Batch Extra Tags")).toBeTruthy();
    expect(screen.getByText("2 files selected")).toBeTruthy();
  });

  it("shows singular file count for one track", () => {
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByText("1 file selected")).toBeTruthy();
  });

  it("starts with one empty row ready for input", async () => {
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    expect(await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)")).toBeTruthy();
    expect(screen.getByPlaceholderText("Value")).toBeTruthy();
  });

  it("loads and shows combined extra tags from all selected tracks", async () => {
    const readExtraTags = vi.fn()
      .mockResolvedValueOnce([
        { key: "MOOD", value: "Bright", source: "vorbis" },
        { key: "BARCODE", value: "111", source: "vorbis" },
      ])
      .mockResolvedValueOnce([
        { key: "MOOD", value: "Bright", source: "vorbis" },
        { key: "ISRC", value: "US-ABC-24-00001", source: "vorbis" },
      ]);
    window.api = { readExtraTags } as unknown as Window["api"];

    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack(), makeTrack({ path: "/music/song2.mp3" })]}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    expect(await screen.findByDisplayValue("BARCODE")).toBeTruthy();
    expect(screen.getByDisplayValue("111")).toBeTruthy();
    expect(screen.getByDisplayValue("ISRC")).toBeTruthy();
    expect(screen.getByDisplayValue("US-ABC-24-00001")).toBeTruthy();
    expect(screen.getByDisplayValue("MOOD")).toBeTruthy();
    expect(screen.getByDisplayValue("Bright")).toBeTruthy();
  });

  it("adds a new row when 'Add Tag' is clicked", async () => {
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    fireEvent.click(screen.getByText("Add Tag"));

    const keyInputs = screen.getAllByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    expect(keyInputs.length).toBe(2);
  });

  it("removes a row when the delete button is clicked (keeps at least one)", async () => {
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const keyInputs = screen.getAllByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    expect(keyInputs.length).toBe(1);

    // Add another row, then delete the first one
    fireEvent.click(screen.getByText("Add Tag"));
    const deleteButtons = screen.getAllByLabelText("Remove tag");
    expect(deleteButtons.length).toBe(2);
    fireEvent.click(deleteButtons[0]);

    // Should still have one row (min 1)
    const remaining = screen.getAllByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    expect(remaining.length).toBe(1);
  });

  it("calls onSave with only non-empty tags when Apply is clicked", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack(), makeTrack({ path: "/music/song2.mp3" })]}
        saving={false}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    // Fill in the tag fields
    const keyInput = await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const valueInput = screen.getByPlaceholderText("Value");
    fireEvent.change(keyInput, { target: { value: "BARCODE" } });
    fireEvent.change(valueInput, { target: { value: "ABC-123" } });

    fireEvent.click(screen.getByText("Apply to 2 files"));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith([
        { key: "BARCODE", value: "ABC-123" },
      ]);
    });
  });

  it("calls onSave with multiple tags when multiple rows are filled", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    // Fill first row
    await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const keyInputs = screen.getAllByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const valueInputs = screen.getAllByPlaceholderText("Value");
    fireEvent.change(keyInputs[0], { target: { value: "MOOD" } });
    fireEvent.change(valueInputs[0], { target: { value: "Bright" } });

    // Add and fill second row
    fireEvent.click(screen.getByText("Add Tag"));
    const keyInputs2 = screen.getAllByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const valueInputs2 = screen.getAllByPlaceholderText("Value");
    fireEvent.change(keyInputs2[1], { target: { value: "ISRC" } });
    fireEvent.change(valueInputs2[1], { target: { value: "US-ABC-24-00001" } });

    fireEvent.click(screen.getByText("Apply to 1 file"));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith([
        { key: "MOOD", value: "Bright" },
        { key: "ISRC", value: "US-ABC-24-00001" },
      ]);
    });
  });

  it("allows multiple ARTISTS rows to be applied to selected tracks", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <BatchExtraTagsEditor
        tracks={[
          makeTrack({ path: "/music/song1.flac", codec: "FLAC" }),
          makeTrack({ path: "/music/song2.flac", codec: "FLAC" }),
        ]}
        saving={false}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    fireEvent.click(screen.getByText("Add Tag"));

    const keyInputs = screen.getAllByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const valueInputs = screen.getAllByPlaceholderText("Value");
    fireEvent.change(keyInputs[0], { target: { value: "ARTISTS" } });
    fireEvent.change(valueInputs[0], { target: { value: "foo" } });
    fireEvent.change(keyInputs[1], { target: { value: "ARTISTS" } });
    fireEvent.change(valueInputs[1], { target: { value: "bar" } });

    fireEvent.click(screen.getByText("Apply to 2 files"));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith([
        { key: "ARTISTS", value: "foo" },
        { key: "ARTISTS", value: "bar" },
      ]);
    });
  });

  it("does not call onSave when all rows are empty", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    // Apply button should be disabled (has disabled attribute)
    await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const applyButton = screen.getByText("Apply to 1 file") as HTMLButtonElement;
    expect(applyButton.disabled).toBe(true);

    // Click it anyway to verify onSave is not called
    fireEvent.click(applyButton);
    await waitFor(() => expect(onSave).not.toHaveBeenCalled());
  });

  it("saves an empty tag list when existing rows are removed", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    window.api = {
      readExtraTags: vi.fn().mockResolvedValue([
        { key: "MOOD", value: "Bright", source: "vorbis" },
      ]),
    } as unknown as Window["api"];

    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack(), makeTrack({ path: "/music/song2.mp3" })]}
        saving={false}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    expect(await screen.findByDisplayValue("MOOD")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("Remove tag"));
    fireEvent.click(screen.getByText("Apply to 2 files"));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith([]);
    });
  });

  it("shows saving state and disables buttons while saving", () => {
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={true}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByText("Saving...")).toBeTruthy();
    const cancelBtn = screen.getByText("Cancel") as HTMLButtonElement;
    expect(cancelBtn.disabled).toBe(true);
  });

  it("resets rows after successful save", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={vi.fn()}
        onSave={onSave}
      />,
    );

    const keyInput = await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const valueInput = screen.getByPlaceholderText("Value");
    fireEvent.change(keyInput, { target: { value: "CATALOGNUMBER" } });
    fireEvent.change(valueInput, { target: { value: "CN-999" } });

    fireEvent.click(screen.getByText("Apply to 1 file"));

    await waitFor(() => {
      expect(onSave).toHaveBeenCalledWith([
        { key: "CATALOGNUMBER", value: "CN-999" },
      ]);
    });
  });

  it("asks for confirmation before closing with dirty changes", async () => {
    const onClose = vi.fn();
    const confirmMock = vi.fn().mockReturnValue(true);
    window.confirm = confirmMock as unknown as typeof window.confirm;

    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={onClose}
        onSave={vi.fn()}
      />,
    );

    // Fill both key and value to make the row dirty
    const keyInput = await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    const valueInput = screen.getByPlaceholderText("Value");
    fireEvent.change(keyInput, { target: { value: "MOOD" } });
    fireEvent.change(valueInput, { target: { value: "Bright" } });

    // Click Cancel
    fireEvent.click(screen.getByText("Cancel"));
    expect(confirmMock).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("closes without confirmation when clean", async () => {
    const onClose = vi.fn();
    const confirmMock = vi.fn();
    window.confirm = confirmMock as unknown as typeof window.confirm;

    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={onClose}
        onSave={vi.fn()}
      />,
    );

    await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    // Click Cancel without making changes
    fireEvent.click(screen.getByText("Cancel"));
    expect(confirmMock).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("renders the info banner about setting tags on all files", () => {
    render(
      <BatchExtraTagsEditor
        tracks={[makeTrack(), makeTrack({ path: "/music/song2.mp3" })]}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByText(/set on all 2 selected files/i)).toBeTruthy();
  });

  it("renders a datalist with common tag suggestions", async () => {
    const { container } = render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={vi.fn()}
        onSave={vi.fn()}
      />,
    );

    await screen.findByPlaceholderText("Tag key (e.g. MUSICBRAINZ_ALBUMID)");
    // Check datalist options in the DOM (option elements inside datalist)
    const options = container.querySelectorAll("datalist option");
    const optionValues = Array.from(options).map((opt) => opt.getAttribute("value"));
    expect(optionValues).toContain("MUSICBRAINZ_ALBUMID");
    expect(optionValues).toContain("BARCODE");
    expect(optionValues).toContain("ISRC");
    expect(optionValues).toContain("CATALOGNUMBER");
    expect(optionValues).toContain("RELEASETYPE");
    expect(optionValues).toContain("MEDIA");
    expect(optionValues).toContain("RATING");
    expect(optionValues).toContain("ASIN");
    expect(optionValues).toContain("LANGUAGE");
  });

  it("closes when clicking the backdrop overlay", () => {
    const onClose = vi.fn();
    const { container } = render(
      <BatchExtraTagsEditor
        tracks={[makeTrack()]}
        saving={false}
        onClose={onClose}
        onSave={vi.fn()}
      />,
    );

    // Click the dialog overlay (first child of the fixed backdrop)
    const backdrop = container.firstChild as HTMLElement;
    fireEvent.mouseDown(backdrop);
    expect(onClose).toHaveBeenCalled();
  });
});
