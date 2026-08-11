import React, { useState, useCallback, useMemo } from "react";
import type {
  TrackData,
  PreviewMatchResult,
  AlbumCandidate,
  TrackEdit,
} from "../shared/desktop-api";

interface ConfirmWriteDialogProps {
  open: boolean;
  albumPath: string;
  albumTracks: TrackData[];
  previewResult: PreviewMatchResult | null;
  loading: boolean;
  writing: boolean;
  writeError: string | null;
  onConfirm: (candidate: AlbumCandidate) => void;
  onCancel: () => void;
}

interface RowState {
  localIndex: number;
  localTitle: string;
  localArtist: string;
  selectedRemoteIndex: number | null; // null = "Do not update"
  editedTitle: string;
  editedArtist: string;
  editedTrackNumber: string;
  editedTrackTotal: string;
  editedDiscNumber: string;
}

export function ConfirmWriteDialog({
  open,
  albumTracks,
  previewResult,
  loading,
  writing,
  writeError,
  onConfirm,
  onCancel,
}: ConfirmWriteDialogProps) {
  const [rows, setRows] = useState<RowState[]>([]);
  const [hasEdited, setHasEdited] = useState(false);

  // Initialize rows from preview result
  React.useEffect(() => {
    if (!previewResult) return;
    const initialRows: RowState[] = albumTracks.map((track, i) => {
      const match = previewResult.candidates[i];
      const remoteIdx = match?.remoteIndex ?? null;
      const remoteTrack = remoteIdx != null ? previewResult.release.tracks[remoteIdx] : null;
      return {
        localIndex: i,
        localTitle: track.title ?? "",
        localArtist: track.artist ?? "",
        selectedRemoteIndex: remoteIdx,
        editedTitle: remoteTrack?.title ?? "",
        editedArtist: remoteTrack?.artist ?? "",
        editedTrackNumber: remoteTrack?.trackNumber?.toString() ?? "",
        editedTrackTotal: remoteTrack?.trackTotal?.toString() ?? "",
        editedDiscNumber: remoteTrack?.discNumber?.toString() ?? "",
      };
    });
    setRows(initialRows);
    setHasEdited(false);
  }, [previewResult, albumTracks]);

  const handleRemoteSelect = useCallback((localIdx: number, remoteIdx: string) => {
    setRows((prev) => {
      const next = [...prev];
      const idx = parseInt(remoteIdx, 10);
      const newRemoteIdx = isNaN(idx) ? null : idx;
      const remoteTrack = newRemoteIdx != null && previewResult
        ? previewResult.release.tracks[newRemoteIdx]
        : null;
      next[localIdx] = {
        ...next[localIdx],
        selectedRemoteIndex: newRemoteIdx,
        editedTitle: remoteTrack?.title ?? "",
        editedArtist: remoteTrack?.artist ?? "",
        editedTrackNumber: remoteTrack?.trackNumber?.toString() ?? "",
        editedTrackTotal: remoteTrack?.trackTotal?.toString() ?? "",
        editedDiscNumber: remoteTrack?.discNumber?.toString() ?? "",
      };
      setHasEdited(true);
      return next;
    });
  }, [previewResult]);

  const handleFieldEdit = useCallback((localIdx: number, field: string, value: string) => {
    setRows((prev) => {
      const next = [...prev];
      (next[localIdx] as unknown as Record<string, unknown>)[field] = value;
      setHasEdited(true);
      return next;
    });
  }, []);

  const unusedRemoteTracks = useMemo(() => {
    if (!previewResult) return [];
    const usedIndices = new Set(
      rows.filter((r) => r.selectedRemoteIndex != null).map((r) => r.selectedRemoteIndex)
    );
    return previewResult.unusedRemoteIndices.filter((i) => !usedIndices.has(i));
  }, [previewResult, rows]);

  // Detect duplicate assignments
  const duplicateMap = useMemo(() => {
    const counts = new Map<number, number>();
    rows.forEach((r) => {
      if (r.selectedRemoteIndex != null) {
        counts.set(r.selectedRemoteIndex, (counts.get(r.selectedRemoteIndex) ?? 0) + 1);
      }
    });
    const duplicates = new Set<number>();
    counts.forEach((count, idx) => { if (count > 1) duplicates.add(idx); });
    return duplicates;
  }, [rows]);

  const matchedCount = rows.filter((r) => r.selectedRemoteIndex != null).length;

  const buildCandidate = useCallback((): AlbumCandidate | null => {
    if (!previewResult) return null;
    const tracks: TrackEdit[] = rows.map((r) => {
      if (r.selectedRemoteIndex == null) {
        // "Do not update" — leave the local file's per-track tags untouched.
        // Sentinel: all per-track fields empty; the native writer skips tracks
        // with no patchable per-track data while still applying album fields.
        return { artists: [] };
      }
      return {
        title: r.editedTitle || undefined,
        artist: r.editedArtist || undefined,
        artists: r.editedArtist ? [r.editedArtist] : [],
        // Native contract is snake_case (TrackCandidate); camelCase keys are
        // silently dropped by serde, so disc/track numbers never reached the
        // writer before.
        track_number: r.editedTrackNumber ? parseInt(r.editedTrackNumber, 10) || undefined : undefined,
        track_total: r.editedTrackTotal ? parseInt(r.editedTrackTotal, 10) || undefined : undefined,
        disc_number: r.editedDiscNumber ? parseInt(r.editedDiscNumber, 10) || undefined : undefined,
      };
    });
    return {
      ...previewResult.albumCandidate,
      tracks,
    };
  }, [previewResult, rows]);

  const handleConfirm = useCallback(() => {
    const candidate = buildCandidate();
    if (candidate) onConfirm(candidate);
  }, [buildCandidate, onConfirm]);

  if (!open) return null;

  const remoteOptions = previewResult
    ? previewResult.release.tracks.map((t, i) => ({
        index: i,
        label: `${i + 1}. ${t.title ?? "Untitled"}${t.artist ? ` — ${t.artist}` : ""}`,
      }))
    : [];

  return (
    <div
      role="dialog"
      aria-label="Confirm track mapping"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm"
      onClick={onCancel}
    >
      <div
        className="bg-white rounded-xl shadow-2xl border border-border w-full max-w-5xl max-h-[85vh] flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-border">
          <h2 className="text-sm font-semibold text-text-primary">
            Confirm track mapping
          </h2>
          <button
            onClick={onCancel}
            className="text-text-muted hover:text-text-primary transition-colors"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-auto p-5">
          {loading && (
            <div className="flex items-center justify-center py-8">
              <div className="w-5 h-5 border-2 border-accent/30 border-t-accent rounded-full animate-spin" />
              <span className="ml-3 text-[13px] text-text-muted">Loading release details…</span>
            </div>
          )}

          {writeError && (
            <div className="mb-4 p-3 bg-red-50 border border-red-200 rounded-lg text-[12px] text-red-700">
              {writeError}
            </div>
          )}

          {!loading && previewResult && (
            <>
              <div className="mb-4 flex items-center gap-3 text-[12px] text-text-muted">
                <span>Matched <strong className="text-text-primary">{matchedCount}</strong> of {albumTracks.length} tracks</span>
                {unusedRemoteTracks.length > 0 && (
                  <span className="text-amber-600">
                    ({unusedRemoteTracks.length} unused remote track{unusedRemoteTracks.length !== 1 ? "s" : ""})
                  </span>
                )}
              </div>

              <div className="overflow-x-auto">
                <table className="w-full text-[12px] border-collapse">
                  <thead>
                    <tr className="border-b border-border">
                      <th className="text-left py-2 px-2 text-text-muted font-medium w-8">#</th>
                      <th className="text-left py-2 px-2 text-text-muted font-medium">Local title</th>
                      <th className="text-left py-2 px-2 text-text-muted font-medium">Local artist</th>
                      <th className="text-left py-2 px-2 text-text-muted font-medium w-[200px]">Remote track</th>
                      <th className="text-left py-2 px-2 text-text-muted font-medium">Remote title</th>
                      <th className="text-left py-2 px-2 text-text-muted font-medium w-[200px]">Remote artist</th>
                      <th className="text-left py-2 px-2 text-text-muted font-medium w-16">#</th>
                      <th className="text-left py-2 px-2 text-text-muted font-medium w-16">Total</th>
                      <th className="text-left py-2 px-2 text-text-muted font-medium w-16">Disc</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((row) => {
                      const dupe = row.selectedRemoteIndex != null && duplicateMap.has(row.selectedRemoteIndex);
                      return (
                        <tr
                          key={row.localIndex}
                          className={`border-b border-border/60 hover:bg-surface-hover/50 transition-colors ${
                            row.selectedRemoteIndex == null ? "opacity-60" : ""
                          }`}
                        >
                          <td className="py-2 px-2 text-text-muted tabular-nums">{row.localIndex + 1}</td>
                          <td className="py-2 px-2 text-text-secondary truncate max-w-[150px]" title={row.localTitle}>
                            {row.localTitle || <span className="italic text-text-muted">untitled</span>}
                          </td>
                          <td className="py-2 px-2 text-text-secondary truncate max-w-[120px]" title={row.localArtist}>
                            {row.localArtist || "-"}
                          </td>
                          <td className="py-2 px-2">
                            <select
                              value={row.selectedRemoteIndex != null ? String(row.selectedRemoteIndex) : ""}
                              onChange={(e) => handleRemoteSelect(row.localIndex, e.target.value)}
                              className={`w-full h-7 px-2 text-[11px] border rounded-md outline-none bg-white ${
                                dupe ? "border-amber-400 bg-amber-50" : "border-border focus:border-accent/60"
                              }`}
                            >
                              <option value="">Do not update</option>
                              {remoteOptions.map((opt) => (
                                <option key={opt.index} value={String(opt.index)}>
                                  {opt.label}
                                </option>
                              ))}
                            </select>
                            {dupe && (
                              <div className="text-[10px] text-amber-600 mt-0.5">Assigned to multiple tracks</div>
                            )}
                          </td>
                          <td className="py-2 px-2">
                            <input
                              type="text"
                              value={row.editedTitle}
                              onChange={(e) => handleFieldEdit(row.localIndex, "editedTitle", e.target.value)}
                              disabled={row.selectedRemoteIndex == null}
                              placeholder="Title"
                              className="w-full h-7 px-2 text-[11px] border border-border rounded-md outline-none focus:border-accent/60 bg-white disabled:opacity-40"
                            />
                          </td>
                          <td className="py-2 px-2">
                            <input
                              type="text"
                              value={row.editedArtist}
                              onChange={(e) => handleFieldEdit(row.localIndex, "editedArtist", e.target.value)}
                              disabled={row.selectedRemoteIndex == null}
                              placeholder="Artist"
                              className="w-full h-7 px-2 text-[11px] border border-border rounded-md outline-none focus:border-accent/60 bg-white disabled:opacity-40"
                            />
                          </td>
                          <td className="py-2 px-2">
                            <input
                              type="text"
                              value={row.editedTrackNumber}
                              onChange={(e) => handleFieldEdit(row.localIndex, "editedTrackNumber", e.target.value)}
                              disabled={row.selectedRemoteIndex == null}
                              placeholder="#"
                              className="w-12 h-7 px-2 text-[11px] border border-border rounded-md outline-none focus:border-accent/60 bg-white disabled:opacity-40"
                            />
                          </td>
                          <td className="py-2 px-2">
                            <input
                              type="text"
                              value={row.editedTrackTotal}
                              onChange={(e) => handleFieldEdit(row.localIndex, "editedTrackTotal", e.target.value)}
                              disabled={row.selectedRemoteIndex == null}
                              placeholder="Total"
                              className="w-12 h-7 px-2 text-[11px] border border-border rounded-md outline-none focus:border-accent/60 bg-white disabled:opacity-40"
                            />
                          </td>
                          <td className="py-2 px-2">
                            <input
                              type="text"
                              value={row.editedDiscNumber}
                              onChange={(e) => handleFieldEdit(row.localIndex, "editedDiscNumber", e.target.value)}
                              disabled={row.selectedRemoteIndex == null}
                              placeholder="#"
                              className="w-12 h-7 px-2 text-[11px] border border-border rounded-md outline-none focus:border-accent/60 bg-white disabled:opacity-40"
                            />
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>

              {unusedRemoteTracks.length > 0 && (
                <details className="mt-3">
                  <summary className="text-[11px] text-text-muted cursor-pointer hover:text-text-secondary">
                    Unused remote tracks ({unusedRemoteTracks.length})
                  </summary>
                  <ul className="mt-1 space-y-0.5 pl-4">
                    {unusedRemoteTracks.map((idx) => {
                      const t = previewResult.release.tracks[idx];
                      return (
                        <li key={idx} className="text-[11px] text-text-muted">
                          {idx + 1}. {t?.title ?? "Untitled"}
                          {t?.artist ? ` — ${t.artist}` : ""}
                        </li>
                      );
                    })}
                  </ul>
                </details>
              )}
            </>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 px-5 py-3 border-t border-border">
          {writing && (
            <div className="flex items-center gap-2 text-[12px] text-text-muted">
              <div className="w-4 h-4 border-2 border-accent/30 border-t-accent rounded-full animate-spin" />
              Writing tags…
            </div>
          )}
          <button
            onClick={onCancel}
            disabled={writing}
            className="px-4 py-2 text-[12px] rounded-lg border border-border text-text-secondary hover:bg-surface-hover disabled:opacity-40 transition-all"
          >
            Cancel
          </button>
          <button
            onClick={handleConfirm}
            disabled={loading || writing || !previewResult}
            className="px-4 py-2 text-[12px] font-medium rounded-lg bg-accent text-white hover:bg-accent/90 active:scale-[0.98] disabled:opacity-40 transition-all"
          >
            {writing ? "Writing…" : "Confirm & Write"}
          </button>
        </div>
      </div>
    </div>
  );
}
