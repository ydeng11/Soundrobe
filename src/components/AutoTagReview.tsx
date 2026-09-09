import React, { useEffect, useState } from "react";
import type {
  AutoTagReviewArtwork,
  AutoTagReviewDetail,
} from "../shared/desktop-api";

interface Props {
  reviewId: string;
  busy: boolean;
  onActing?: (acting: boolean) => void;
  onRetry: (albumPath: string) => void;
  onSearch: (albumPath: string) => void;
  onChanged: (review: AutoTagReviewDetail) => void | Promise<void>;
}
const buttonClass =
  "rounded border border-border px-3 py-1.5 text-xs hover:bg-surface-hover disabled:opacity-50";
function label(key: string) {
  return key
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/_/g, " ")
    .replace(/^./, (c) => c.toUpperCase());
}
function valueText(value: unknown): string {
  if (value === null || value === undefined || value === "") return "—";
  if (Array.isArray(value)) return value.map(valueText).join("\n") || "—";
  if (typeof value === "object")
    return Object.entries(value)
      .map(([key, item]) => `${label(key)}: ${valueText(item)}`)
      .join("\n");
  return String(value);
}
function Artwork({
  reviewId,
  art,
}: {
  reviewId: string;
  art: AutoTagReviewArtwork;
}) {
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState(art.error);
  const [expanded, setExpanded] = useState(false);
  const enlargeRef = React.useRef<HTMLButtonElement>(null);
  const closePreview = () => {
    setExpanded(false);
    enlargeRef.current?.focus();
  };
  useEffect(() => {
    let cancelled = false;
    window.api
      .getAutoTagReviewArtwork(reviewId, art.id)
      .then((result) => {
        if (!cancelled) setUrl(result);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [reviewId, art.id]);
  return (
    <figure className="rounded border border-border p-2 min-w-0">
      {url && (
        <button
          type="button"
          ref={enlargeRef}
          aria-label={`Enlarge ${art.label}`}
          onClick={() => setExpanded(true)}
        >
          <img src={url} alt={art.label} className="h-32 w-40 object-contain" />
        </button>
      )}
      {!url && !error && <p>Loading artwork…</p>}
      {error && (
        <p role="alert" className="text-red-600 break-words">
          Artwork unavailable: {error}
        </p>
      )}
      <figcaption className="break-all text-xs">
        {art.label}
        <br />
        {label(art.source)}
        {art.width && art.height ? ` · ${art.width} × ${art.height}` : ""}
      </figcaption>
      {expanded && url && (
        <div
          role="dialog"
          aria-label="Artwork preview"
          aria-modal="true"
          className="fixed inset-0 z-[70] bg-black/90 flex flex-col items-center justify-center p-6"
          onClick={(event) => {
            event.stopPropagation();
            closePreview();
          }}
          onKeyDown={(event) => {
            if (event.key === "Tab") {
              event.preventDefault();
              event.stopPropagation();
              event.currentTarget.querySelector("button")?.focus();
            }
            if (event.key === "Escape") {
              event.stopPropagation();
              closePreview();
            }
          }}
        >
          <button
            autoFocus
            type="button"
            className="text-white mb-3"
            onClick={closePreview}
          >
            Close artwork
          </button>
          <img
            src={url}
            alt={art.label}
            className="max-h-[85vh] max-w-full object-contain"
          />
        </div>
      )}
    </figure>
  );
}

export function AutoTagReview({
  reviewId,
  busy,
  onRetry,
  onSearch,
  onChanged,
  onActing,
}: Props) {
  const [review, setReview] = useState<AutoTagReviewDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [acting, setActing] = useState(false);
  const [allMetadata, setAllMetadata] = useState(false);
  const [artworkOpen, setArtworkOpen] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setReview(null);
    setError(null);
    setLoading(true);
    setAllMetadata(false);
    window.api
      .getAutoTagReview(reviewId)
      .then((result) => {
        if (!cancelled) setReview(result);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [reviewId]);
  async function decide(revert: boolean) {
    if (!review || acting || busy) return;
    if (
      revert &&
      !window.confirm(
        "Restore this album’s auto-tag changes, including lyrics and artwork? Later edits will prevent restoration.",
      )
    )
      return;
    setActing(true);
    onActing?.(true);
    setError(null);
    try {
      const updated = await (revert
        ? window.api.revertAutoTagReview(reviewId)
        : window.api.markAutoTagReviewed(reviewId));
      setReview(updated);
      await onChanged(updated);
    } catch (err) {
      setError(String(err));
    } finally {
      setActing(false);
      onActing?.(false);
    }
  }
  if (loading) return <p className="p-4">Loading album review…</p>;
  if (!review)
    return (
      <p role="alert" className="p-4 text-red-600">
        {error ?? "Review unavailable"}
      </p>
    );
  const skipped = review.outcome === "needs_review";
  const paths = Array.from(
    new Set(
      [...review.before.tracks, ...review.after.tracks].map(
        (track) => track.path,
      ),
    ),
  );
  return (
    <section className="p-4 space-y-4 text-sm" aria-label="Album review">
      <div>
        <h3 className="font-semibold break-all">{review.albumPath}</h3>
        <p>
          {label(review.outcome)} · {label(review.decision)}
        </p>
        {skipped && (
          <p className="text-amber-700">
            No changes written — no authoritative metadata match was available.
          </p>
        )}
        <p className="text-xs text-text-muted">
          Review and full undo are available until Soundrobe exits. Keeping
          changes marks this album reviewed and retains undo.
        </p>
      </div>
      {error && (
        <p role="alert" className="text-red-600">
          {error}
        </p>
      )}
      {[...review.errors, ...review.before.errors, ...review.after.errors].map(
        (item, i) => (
          <p role="alert" key={i} className="text-red-600 break-words">
            {item}
          </p>
        ),
      )}
      <div className="flex gap-2 flex-wrap">
        {review.decision !== "reverted" && (
          <button
            type="button"
            className={buttonClass}
            disabled={
              acting ||
              busy ||
              review.outcome === "running" ||
              review.decision === "kept"
            }
            onClick={() => void decide(false)}
          >
            {skipped ? "Keep current metadata" : "Keep changes"}
          </button>
        )}
        {review.canRevert && (
          <button
            type="button"
            className={buttonClass}
            disabled={acting || busy}
            onClick={() => void decide(true)}
          >
            Revert album
          </button>
        )}
        {skipped && (
          <>
            <button
              type="button"
              className={buttonClass}
              disabled={acting || busy}
              onClick={() => onRetry(review.albumPath)}
            >
              Retry auto-tag
            </button>
            <button
              type="button"
              className={buttonClass}
              disabled={acting || busy}
              onClick={() => onSearch(review.albumPath)}
            >
              Find release manually
            </button>
          </>
        )}
        {acting && <span role="status">Updating review…</span>}
      </div>
      {review.result && (
        <details open>
          <summary className="font-medium cursor-pointer">
            Matching evidence and diagnostics
          </summary>
          <dl className="grid grid-cols-[minmax(100px,1fr)_3fr] gap-2 mt-2">
            {Object.entries(review.result).map(([key, value]) => (
              <React.Fragment key={key}>
                <dt className="text-text-muted">{label(key)}</dt>
                <dd className="whitespace-pre-wrap break-words min-w-0">
                  {valueText(value)}
                </dd>
              </React.Fragment>
            ))}
          </dl>
        </details>
      )}
      <div className="flex gap-2">
        <button
          type="button"
          className={buttonClass}
          aria-pressed={!allMetadata}
          onClick={() => setAllMetadata(false)}
        >
          Changes only
        </button>
        <button
          type="button"
          className={buttonClass}
          aria-pressed={allMetadata}
          onClick={() => setAllMetadata(true)}
        >
          All metadata
        </button>
      </div>
      {review.decision === "reverted" && (
        <p>
          Restored. The comparison below records what this auto-tag run changed.
        </p>
      )}
      <div className="space-y-2">
        {paths.map((path) => {
          const before: Record<string, unknown> =
            review.before.tracks.find((track) => track.path === path) ?? {};
          const after: Record<string, unknown> =
            review.after.tracks.find((track) => track.path === path) ?? {};
          const keys = Array.from(
            new Set([...Object.keys(before), ...Object.keys(after)]),
          ).filter((key) => key !== "path");
          const comparable =
            review.before.tracks.some((track) => track.path === path) &&
            review.after.tracks.some((track) => track.path === path);
          const changed = comparable
            ? keys.filter(
                (key) =>
                  JSON.stringify(before[key] ?? null) !==
                  JSON.stringify(after[key] ?? null),
              )
            : [];
          const visible = allMetadata ? keys : changed;
          return (
            <details
              key={path}
              open={allMetadata || changed.length > 0}
              className="rounded border border-border p-2"
            >
              <summary className="cursor-pointer break-all">
                {path.split(/[\\/]/).pop()} · {changed.length} changed field
                {changed.length === 1 ? "" : "s"}
              </summary>
              <p className="text-xs text-text-muted break-all my-1">{path}</p>
              {!comparable && (
                <p className="text-amber-700">
                  Complete before/after comparison unavailable for this track.
                </p>
              )}
              {visible.length ? (
                <table className="w-full table-fixed text-xs">
                  <thead>
                    <tr>
                      <th className="text-left w-1/5">Field</th>
                      <th className="text-left">Before auto-tag</th>
                      <th className="text-left">After auto-tag</th>
                    </tr>
                  </thead>
                  <tbody>
                    {visible.map((key) => (
                      <tr
                        key={key}
                        className={
                          changed.includes(key) ? "bg-amber-500/10" : ""
                        }
                      >
                        <th className="text-left align-top p-1 font-normal">
                          {label(key)}
                        </th>
                        <td className="align-top p-1 whitespace-pre-wrap break-words">
                          {valueText(before[key])}
                        </td>
                        <td className="align-top p-1 whitespace-pre-wrap break-words">
                          {valueText(after[key])}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              ) : (
                <p>No metadata changes.</p>
              )}
            </details>
          );
        })}
      </div>
      <details onToggle={(event) => setArtworkOpen(event.currentTarget.open)}>
        <summary className="font-medium cursor-pointer">
          Artwork — before and after ({review.before.artworks.length} /{" "}
          {review.after.artworks.length})
        </summary>
        {artworkOpen && (
          <div className="grid grid-cols-2 gap-4 mt-2">
            {(["before", "after"] as const).map((side) => (
              <div key={side}>
                <h4 className="font-medium mb-2">{label(side)} auto-tag</h4>
                <div className="flex flex-wrap gap-2">
                  {review[side].artworks.length === 0 ? (
                    <p>No artwork found.</p>
                  ) : (
                    review[side].artworks.map((art, index) => (
                      <Artwork
                        key={`${art.id}-${index}`}
                        reviewId={reviewId}
                        art={art}
                      />
                    ))
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </details>
    </section>
  );
}
