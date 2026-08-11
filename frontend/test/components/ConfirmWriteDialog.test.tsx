// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { ConfirmWriteDialog } from "../../src/components/ConfirmWriteDialog";
import type { PreviewMatchResult, AlbumCandidate, TrackMappingRow } from "../../src/shared/desktop-api";

afterEach(() => {
  cleanup();
});

function makePreviewResult(overrides?: Partial<PreviewMatchResult>): PreviewMatchResult {
  const candidates: TrackMappingRow[] = [
    { localIndex: 0, localTitle: "Track One", localArtist: "Artist A", remoteIndex: 0, remoteTitle: "Track One", remoteArtist: "Artist A", remoteTrackNumber: 1, evidence: "TagTitle" },
    { localIndex: 1, localTitle: "Track Two", localArtist: "Artist A", remoteIndex: 1, remoteTitle: "Track Two", remoteArtist: "Artist A", remoteTrackNumber: 2, evidence: "TagTitle" },
  ];
  return {
    release: {
      id: "mb-1",
      title: "Test Album",
      artist: "Artist A",
      artists: ["Artist A"],
      tracks: [
        { title: "Track One", matchTitles: [], artists: ["Artist A"], trackNumber: 1, trackTotal: 2, discNumber: 2, recordingId: "r1", length: 200 },
        { title: "Track Two", matchTitles: [], artists: ["Artist A"], trackNumber: 2, trackTotal: 2, discNumber: 2, recordingId: "r2", length: 200 },
      ],
    },
    candidates,
    unusedRemoteIndices: [],
    albumCandidate: {
      artist: "Artist A",
      artists: ["Artist A"],
      album: "Test Album",
      albumArtist: "Artist A",
      albumArtists: ["Artist A"],
      year: "2024",
      tracks: candidates.map((c) => ({
        title: c.remoteTitle,
        artist: c.remoteArtist,
        artists: c.remoteArtist ? [c.remoteArtist] : [],
        // Wire contract: the native TrackCandidate round-trips snake_case keys.
        track_number: c.remoteTrackNumber,
        track_total: 2,
        disc_number: 2,
      })),
    },
    ...overrides,
  };
}

describe("ConfirmWriteDialog", () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  const previewResult = makePreviewResult();

  const defaultProps = {
    open: true,
    previewResult,
    loading: false,
    writing: false,
    writeError: null,
    onConfirm,
    onCancel,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the mapping table with local and remote tracks", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    expect(screen.getByText("Track One")).toBeTruthy();
    expect(screen.getByText("Track Two")).toBeTruthy();
  });

  it("gives Remote Artist at least as much width as Remote Track", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    const remoteTrackHeader = screen.getByRole("columnheader", { name: "Remote track" });
    const remoteArtistHeader = screen.getByRole("columnheader", { name: "Remote artist" });

    expect(remoteTrackHeader.className).toContain("w-[200px]");
    expect(remoteArtistHeader.className).toContain("w-[200px]");
  });

  it("shows match count", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    expect(screen.getByText(/Matched/)).toBeTruthy();
  });

  it("calls onCancel when cancel button is clicked", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Cancel"));
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("calls onConfirm when confirm button is clicked", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Confirm & Write"));
    expect(onConfirm).toHaveBeenCalledOnce();
    const candidate = onConfirm.mock.calls[0][0] as AlbumCandidate;
    expect(candidate.album).toBe("Test Album");
    expect(candidate.tracks.length).toBe(2);
    expect(candidate.tracks[0].title).toBe("Track One");
  });

  it("emits snake_case track fields matching the native candidate contract", () => {
    // Regression: trackNumber/discNumber (camelCase) are silently dropped by
    // the native TrackCandidate deserializer, so the manual disc/track match
    // never reached the writer. The candidate must use snake_case keys.
    render(<ConfirmWriteDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Confirm & Write"));
    const candidate = onConfirm.mock.calls[0][0] as AlbumCandidate;
    const first = candidate.tracks[0] as Record<string, unknown>;
    expect(first.track_number).toBe(1);
    expect(first.disc_number).toBe(2);
    expect(first.track_total).toBe(2);
    expect(first.trackNumber).toBeUndefined();
    expect(first.discNumber).toBeUndefined();
    expect(first.trackTotal).toBeUndefined();
  });

  it("emits edited track total from the total input", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    // Total inputs follow the disc inputs in each row's input group.
    const totalInputs = document.querySelectorAll<HTMLInputElement>(
      "input[placeholder='Total']",
    );
    expect(totalInputs.length).toBe(2);
    fireEvent.change(totalInputs[0], { target: { value: "14" } });
    fireEvent.click(screen.getByText("Confirm & Write"));
    const candidate = onConfirm.mock.calls[0][0] as AlbumCandidate;
    expect(candidate.tracks[0].track_total).toBe(14);
  });

  it("does not call onConfirm when cancel is clicked on write-error dialog", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Cancel"));
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("shows loading state", () => {
    render(<ConfirmWriteDialog {...defaultProps} loading={true} previewResult={null} />);
    expect(screen.getByText(/Loading release details/)).toBeTruthy();
  });

  it("shows writing state and disables buttons", () => {
    render(<ConfirmWriteDialog {...defaultProps} writing={true} />);
    expect(screen.getByText("Writing tags…")).toBeTruthy();
    const cancelBtn = screen.getByText("Cancel") as HTMLButtonElement;
    expect(cancelBtn.disabled).toBe(true);
  });

  it("preserves a manual assignment while writing starts after a parent rerender", () => {
    const result = makePreviewResult({
      unusedRemoteIndices: [2],
      release: {
        ...previewResult.release,
        tracks: [
          ...previewResult.release.tracks,
          { title: "Track Three", matchTitles: [], artists: ["Artist A"], trackNumber: 3 },
        ],
      },
    });
    const { rerender } = render(
      <ConfirmWriteDialog {...defaultProps} previewResult={result} />,
    );
    const firstSelect = document.querySelectorAll<HTMLSelectElement>("select")[0];
    fireEvent.change(firstSelect, { target: { value: "2" } });

    rerender(
      <ConfirmWriteDialog
        {...defaultProps}
        previewResult={result}
        writing={true}
      />,
    );

    expect(document.querySelectorAll<HTMLSelectElement>("select")[0].value).toBe("2");
    expect(screen.getByText(/1\. Track One/, { selector: "li" })).toBeTruthy();
    expect(screen.getByText(/1 unused remote track/)).toBeTruthy();
  });

  it("shows write error", () => {
    render(<ConfirmWriteDialog {...defaultProps} writeError="Write failed" />);
    expect(screen.getByText("Write failed")).toBeTruthy();
  });

  it("renders 'Do not update' option in remote track dropdown", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    const selects = document.querySelectorAll("select");
    expect(selects.length).toBe(2);
    const firstOption = selects[0].querySelector("option");
    expect(firstOption?.textContent).toBe("Do not update");
  });

  it("shows unused remote tracks section", () => {
    const result = makePreviewResult({
      unusedRemoteIndices: [2],
      release: {
        id: "mb-1",
        title: "Test Album",
        artist: "Artist A",
        artists: ["Artist A"],
        tracks: [
          { title: "Track One", matchTitles: [], artists: ["Artist A"], trackNumber: 1, recordingId: "r1", length: 200 },
          { title: "Track Two", matchTitles: [], artists: ["Artist A"], trackNumber: 2, recordingId: "r2", length: 200 },
          { title: "Track Three", matchTitles: [], artists: ["Artist A"], trackNumber: 3, recordingId: "r3", length: 200 },
        ],
      },
    });
    render(<ConfirmWriteDialog {...defaultProps} previewResult={result} />);
    expect(screen.getByText(/1 unused remote track/)).toBeTruthy();
  });

  it("allows editing remote title field", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    const titleInputs = document.querySelectorAll<HTMLInputElement>(
      "input[type='text']",
    );
    // First title input
    if (titleInputs[0]) {
      fireEvent.change(titleInputs[0], { target: { value: "Edited Title" } });
      expect(titleInputs[0].value).toBe("Edited Title");
    }
  });

  it("serializes edited values into the candidate", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    const titleInputs = document.querySelectorAll<HTMLInputElement>(
      "input[type='text']",
    );
    if (titleInputs[0]) {
      fireEvent.change(titleInputs[0], { target: { value: "Edited Title" } });
    }
    fireEvent.click(screen.getByText("Confirm & Write"));
    const candidate = onConfirm.mock.calls[0][0] as AlbumCandidate;
    expect(candidate.tracks[0].title).toBe("Edited Title");
    expect(candidate.tracks[0].track_number).toBe(1);
    expect(candidate.tracks[0].track_total).toBe(2);
    expect(candidate.tracks[0].disc_number).toBe(2);
  });

  it("serializes a positional placeholder for a 'Do not update' row", () => {
    render(<ConfirmWriteDialog {...defaultProps} />);
    // Set first row to "Do not update"
    const selects = document.querySelectorAll("select");
    fireEvent.change(selects[0], { target: { value: "" } });
    fireEvent.click(screen.getByText("Confirm & Write"));
    const candidate = onConfirm.mock.calls[0][0] as AlbumCandidate;
    // "Do not update" rows send empty artists array as sentinel
    expect(candidate.tracks[0].artists).toEqual([]);
    expect(candidate.tracks[0].title).toBeUndefined();
    expect(candidate.tracks[0].track_number).toBeUndefined();
    expect(candidate.tracks[0].track_total).toBeUndefined();
    expect(candidate.tracks[0].disc_number).toBeUndefined();
    expect(onConfirm.mock.calls[0][1]).toEqual([1]);
  });

  it("reports selected rows separately when their edited metadata is empty", () => {
    const result = makePreviewResult({
      release: {
        ...previewResult.release,
        tracks: [
          { title: undefined, matchTitles: [], artists: [] },
          previewResult.release.tracks[1],
        ],
      },
    });
    render(<ConfirmWriteDialog {...defaultProps} previewResult={result} />);
    const selects = document.querySelectorAll<HTMLSelectElement>("select");
    fireEvent.change(selects[1], { target: { value: "" } });

    fireEvent.click(screen.getByText("Confirm & Write"));

    const candidate = onConfirm.mock.calls[0][0] as AlbumCandidate;
    expect(candidate.tracks[0]).toEqual({ artists: [] });
    expect(candidate.tracks[1]).toEqual({ artists: [] });
    expect(onConfirm.mock.calls[0][1]).toEqual([0]);
  });
});
