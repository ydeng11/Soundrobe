/**
 * Search: find a release on MusicBrainz or Discogs and apply it to the album.
 *
 * This mirrors the desktop `SearchDialog` and `ConfirmWriteDialog`, which is
 * where the work actually is: the query has several narrowing fields, results
 * can be filtered and sorted before you commit to reading them, per-release
 * track counts are fetched on demand (they cost a provider request each), and
 * the mapping step lets you re-point any local file at a different provider
 * track and edit what will be written before anything is.
 *
 * The write goes through `album:search-apply-candidate`, the same native path
 * the desktop uses, so the mapping contract is shared rather than re-derived.
 */

import React from "react";
import type {
  AlbumCandidate,
  ProviderAlbum,
  ReleaseSearchPage,
  ReleaseSearchResult,
  TrackData,
  TrackEdit,
  TrackMappingRow,
} from "../shared/desktop-api";
import { ToolDrawer, useToolRun } from "./ToolDrawer";

type Provider = "musicbrainz" | "discogs";
type Phase = "form" | "results" | "mapping";

const PROVIDER_PAGE_SIZE = 100;
const RESULT_PAGE_SIZE = 10;

type ResultSort =
  | "closest"
  | "relevance"
  | "title-asc"
  | "title-desc"
  | "artist-asc"
  | "artist-desc"
  | "year-asc"
  | "year-desc"
  | "tracks-asc"
  | "tracks-desc";

const SORT_LABELS: Record<ResultSort, string> = {
  closest: "Tracks closest to local album",
  relevance: "Relevance",
  "title-asc": "Title (A-Z)",
  "title-desc": "Title (Z-A)",
  "artist-asc": "Artist (A-Z)",
  "artist-desc": "Artist (Z-A)",
  "year-asc": "Year (oldest first)",
  "year-desc": "Year (newest first)",
  "tracks-asc": "Tracks (fewest first)",
  "tracks-desc": "Tracks (most first)",
};

const PROVIDERS: Array<{ key: Provider; label: string }> = [
  { key: "musicbrainz", label: "MusicBrainz" },
  { key: "discogs", label: "Discogs" },
];

function normalized(value: string): string {
  return value.normalize("NFKC").toLowerCase();
}

function resultKey(result: ReleaseSearchResult): string {
  return `${result.provider}:${result.id}`;
}

/** One local file and the provider track it will be written from. */
interface MappingRow {
  localIndex: number;
  localTitle: string;
  localArtist: string;
  selectedRemoteIndex: number | null;
  editedTitle: string;
  editedArtist: string;
  editedArtists: string[];
  editedTrackNumber: string;
  editedTrackTotal: string;
  editedDiscNumber: string;
}

export interface SearchToolProps {
  albumPath: string;
  /** The album's tracks, used for a local count and the mapping preview. */
  tracks: TrackData[];
  api: Window["api"];
  onApplied: () => Promise<void> | void;
  onClose: () => void;
}

export function SearchTool({
  albumPath,
  tracks,
  api,
  onApplied,
  onClose,
}: SearchToolProps) {
  const [provider, setProvider] = React.useState<Provider>("musicbrainz");
  const [artist, setArtist] = React.useState(tracks[0]?.albumArtist ?? tracks[0]?.artist ?? "");
  const [album, setAlbum] = React.useState(tracks[0]?.album ?? "");
  const [year, setYear] = React.useState("");
  const [country, setCountry] = React.useState("");
  const [format, setFormat] = React.useState("");
  const [catalogNumber, setCatalogNumber] = React.useState("");
  const [barcode, setBarcode] = React.useState("");

  const [phase, setPhase] = React.useState<Phase>("form");
  const [page, setPage] = React.useState<ReleaseSearchPage | null>(null);
  const [providerPage, setProviderPage] = React.useState(1);
  const [counts, setCounts] = React.useState<Record<string, number | null>>({});
  const [countProgress, setCountProgress] = React.useState<{ loaded: number; total: number } | null>(
    null,
  );
  const [countError, setCountError] = React.useState<string | null>(null);
  const countGeneration = React.useRef(0);

  const [resultFilter, setResultFilter] = React.useState("");
  const [resultYear, setResultYear] = React.useState("");
  const [resultTrackCount, setResultTrackCount] = React.useState("");
  const [resultCountry, setResultCountry] = React.useState("");
  const [resultFormat, setResultFormat] = React.useState("");
  const [sameTrackCount, setSameTrackCount] = React.useState(false);
  const [resultSort, setResultSort] = React.useState<ResultSort>("relevance");
  const [resultPage, setResultPage] = React.useState(1);

  const [release, setRelease] = React.useState<ProviderAlbum | null>(null);
  const [mapping, setMapping] = React.useState<MappingRow[]>([]);
  const [candidate, setCandidate] = React.useState<AlbumCandidate | null>(null);
  const [localTrackCount, setLocalTrackCount] = React.useState<number | null>(null);

  const { busy, error, failures, message, run } = useToolRun();
  const canSearch = artist.trim().length > 0 || album.trim().length > 0;

  React.useEffect(() => {
    let active = true;
    api
      .readAlbum(albumPath)
      .then((detail) => {
        if (active) setLocalTrackCount(detail.tracks.length);
      })
      .catch(() => {
        // The album in hand is already known; a failed re-read only costs the
        // "same track count" convenience.
        if (active) setLocalTrackCount(tracks.length);
      });
    return () => {
      active = false;
    };
  }, [api, albumPath, tracks.length]);

  const trackCountOf = (result: ReleaseSearchResult): number | undefined => {
    const loaded = counts[resultKey(result)];
    return loaded === null ? undefined : (loaded ?? result.trackCount);
  };

  const search = (nextPage = 1) =>
    run(async () => {
      const pageResult = await api.searchReleases({
        provider,
        artist: artist.trim() || undefined,
        album: album.trim() || undefined,
        year: year.trim() || undefined,
        country: country.trim() || undefined,
        format: format.trim() || undefined,
        catalogNumber: catalogNumber.trim() || undefined,
        barcode: barcode.trim() || undefined,
        page: nextPage,
        pageSize: PROVIDER_PAGE_SIZE,
      });
      countGeneration.current += 1;
      setCounts({});
      setCountProgress(null);
      setCountError(null);
      setPage(pageResult);
      setProviderPage(nextPage);
      setResultPage(1);
      setRelease(null);
      setMapping([]);
      setCandidate(null);
      setPhase(pageResult.results.length > 0 ? "results" : "form");
      return pageResult.results.length > 0
        ? `${pageResult.results.length} releases found.`
        : "No releases matched.";
    });

  /** Track counts cost a provider request each, so they load on request. */
  const loadCounts = () =>
    run(async () => {
      if (!page) throw new Error("run a search first");
      const generation = (countGeneration.current += 1);
      const pending = page.results;
      setCountError(null);
      setCountProgress({ loaded: 0, total: pending.length });
      let loaded = 0;
      for (const result of pending) {
        if (countGeneration.current !== generation) return { message: "Counting cancelled." };
        try {
          const count = await api.releaseTrackCount(result.provider, result.id, result.kind);
          setCounts((current) => ({ ...current, [resultKey(result)]: count }));
        } catch (reason) {
          if (countGeneration.current !== generation) return { message: "Counting cancelled." };
          setCountError(reason instanceof Error ? reason.message : String(reason));
        }
        loaded += 1;
        setCountProgress({ loaded, total: pending.length });
      }
      setCountProgress(null);
      return { message: `Loaded ${loaded} track counts.` };
    });

  const editionResults = React.useMemo(() => {
    if (!page) return [];
    const text = normalized(resultFilter.trim());
    return page.results.filter((result) => {
      if (text) {
        const haystack = normalized(
          [result.title, result.artist, result.catalogNumber, result.barcode]
            .filter(Boolean)
            .join(" "),
        );
        if (!haystack.includes(text)) return false;
      }
      if (resultYear && result.year !== resultYear) return false;
      if (resultCountry && result.country !== resultCountry) return false;
      if (resultFormat && !result.formats.includes(resultFormat)) return false;
      return true;
    });
  }, [page, resultFilter, resultYear, resultCountry, resultFormat]);

  const filteredResults = React.useMemo(() => {
    const byCount = editionResults.filter(
      (result) => !resultTrackCount || String(trackCountOf(result)) === resultTrackCount,
    );
    if (resultSort === "closest") {
      if (localTrackCount === null) return byCount;
      return [...byCount].sort((left, right) => {
        const leftCount = trackCountOf(left);
        const rightCount = trackCountOf(right);
        if (leftCount === undefined) return rightCount === undefined ? 0 : 1;
        if (rightCount === undefined) return -1;
        return Math.abs(leftCount - localTrackCount) - Math.abs(rightCount - localTrackCount);
      });
    }
    if (resultSort === "relevance") return byCount;
    const descending = resultSort.endsWith("-desc");
    const field = resultSort.split("-")[0] as "title" | "artist" | "year" | "tracks";
    return byCount
      .map((result, index) => ({ result, index }))
      .sort(({ result: left, index: leftIndex }, { result: right, index: rightIndex }) => {
        const leftValue =
          field === "title"
            ? left.title
            : field === "artist"
              ? left.artist
              : field === "year"
                ? left.year
                : trackCountOf(left);
        const rightValue =
          field === "title"
            ? right.title
            : field === "artist"
              ? right.artist
              : field === "year"
                ? right.year
                : trackCountOf(right);
        if (leftValue === undefined && rightValue === undefined) return leftIndex - rightIndex;
        if (leftValue === undefined) return 1;
        if (rightValue === undefined) return -1;
        const comparison =
          typeof leftValue === "number" && typeof rightValue === "number"
            ? leftValue - rightValue
            : normalized(String(leftValue)).localeCompare(normalized(String(rightValue)));
        if (comparison !== 0) return descending ? -comparison : comparison;
        return leftIndex - rightIndex;
      })
      .map(({ result }) => result);
  }, [editionResults, resultTrackCount, resultSort, localTrackCount, counts]);

  const visibleResults = React.useMemo(() => {
    const start = (resultPage - 1) * RESULT_PAGE_SIZE;
    return filteredResults.slice(start, start + RESULT_PAGE_SIZE);
  }, [filteredResults, resultPage]);

  const resultPageCount = Math.max(1, Math.ceil(filteredResults.length / RESULT_PAGE_SIZE));
  const pageYears = React.useMemo(
    () => [...new Set((page?.results ?? []).map((r) => r.year).filter(Boolean))] as string[],
    [page],
  );
  const pageCountries = React.useMemo(
    () => [...new Set((page?.results ?? []).map((r) => r.country).filter(Boolean))] as string[],
    [page],
  );
  const pageFormats = React.useMemo(
    () => [...new Set((page?.results ?? []).flatMap((r) => r.formats))] as string[],
    [page],
  );

  /** Resolve the release, then map its tracks onto the local files. */
  const choose = (result: ReleaseSearchResult) =>
    run(async () => {
      const detail = await api.resolveRelease(result.provider, result.id, result.kind);
      const preview = await api.previewReleaseMatch({
        albumPath,
        release: detail,
        provider: result.provider,
      });
      setRelease(preview.release);
      setCandidate(preview.albumCandidate);
      setMapping(rowsFrom(preview.candidates, preview.release.tracks));
      setPhase("mapping");
      return null;
    });

  const setRemote = (localIndex: number, raw: string) => {
    setMapping((current) =>
      current.map((row) => {
        if (row.localIndex !== localIndex) return row;
        const parsed = raw === "" ? null : Number.parseInt(raw, 10);
        const remote =
          parsed !== null && release ? release.tracks[parsed] : undefined;
        // Re-pointing a file takes the provider track's values wholesale.
        return {
          ...row,
          selectedRemoteIndex: parsed,
          editedTitle: remote?.title ?? "",
          editedArtist: remote?.artist ?? "",
          editedArtists: remote?.artists ?? [],
          editedTrackNumber: remote?.trackNumber?.toString() ?? "",
          editedTrackTotal: remote?.trackTotal?.toString() ?? "",
          editedDiscNumber: remote?.discNumber?.toString() ?? "",
        };
      }),
    );
  };

  const editRow = (localIndex: number, field: keyof MappingRow, value: string) => {
    setMapping((current) =>
      current.map((row) => {
        if (row.localIndex !== localIndex) return row;
        const next = { ...row, [field]: value } as MappingRow;
        // The native boundary rebuilds ARTISTS from the display credit, so a
        // hand-edited artist has to carry its own list.
        if (field === "editedArtist") next.editedArtists = value ? [value] : [];
        return next;
      }),
    );
  };

  const mappedRows = mapping.filter((row) => row.selectedRemoteIndex !== null);
  const duplicateRemotes = React.useMemo(() => {
    const seen = new Map<number, number>();
    for (const row of mapping) {
      if (row.selectedRemoteIndex === null) continue;
      seen.set(row.selectedRemoteIndex, (seen.get(row.selectedRemoteIndex) ?? 0) + 1);
    }
    return new Set([...seen].filter(([, count]) => count > 1).map(([index]) => index));
  }, [mapping]);
  const unusedRemote = React.useMemo(() => {
    if (!release) return [];
    const used = new Set(mappedRows.map((row) => row.selectedRemoteIndex));
    return release.tracks.map((_, index) => index).filter((index) => !used.has(index));
  }, [release, mappedRows]);

  const apply = () =>
    run(async () => {
      if (!candidate || !release) throw new Error("choose a release first");
      if (mappedRows.length === 0) throw new Error("no local files are mapped to a provider track");
      // Positional: an unmapped row keeps a placeholder so the selected index
      // list still lines up with the album's own track order.
      const tracksEdit: TrackEdit[] = mapping.map((row) =>
        row.selectedRemoteIndex === null
          ? { artists: [] }
          : {
              title: row.editedTitle || undefined,
              artist: row.editedArtist || undefined,
              artists: row.editedArtists,
              track_number: row.editedTrackNumber
                ? Number.parseInt(row.editedTrackNumber, 10) || undefined
                : undefined,
              track_total: row.editedTrackTotal
                ? Number.parseInt(row.editedTrackTotal, 10) || undefined
                : undefined,
              disc_number: row.editedDiscNumber
                ? Number.parseInt(row.editedDiscNumber, 10) || undefined
                : undefined,
            },
      );
      const written = await api.searchApplyCandidate(
        albumPath,
        { ...candidate, tracks: tracksEdit },
        mappedRows.map((row) => row.localIndex),
      );
      await onApplied();
      setPhase("results");
      return written === 0
        ? "Nothing was written."
        : `Wrote ${written} track(s) from ${release.title ?? "the chosen release"}.`;
    });

  return (
    <ToolDrawer
      title="Search releases"
      busy={busy}
      error={error}
      errors={failures}
      onClose={onClose}
      testId="search-drawer"
    >
      <section className="sr-section">
        <h3 className="sr-section__title">Query</h3>
        <div className="sr-field">
          <label className="sr-field__label" htmlFor="search-provider">
            Provider
          </label>
          <select
            id="search-provider"
            className="sr-input"
            data-testid="search-provider"
            value={provider}
            onChange={(event) => setProvider(event.target.value as Provider)}
          >
            {PROVIDERS.map((candidate_) => (
              <option key={candidate_.key} value={candidate_.key}>
                {candidate_.label}
              </option>
            ))}
          </select>
        </div>

        {(
          [
            ["search-artist", "Artist", artist, setArtist, "Required, unless an album is given"],
            ["search-album", "Album", album, setAlbum, "Required, unless an artist is given"],
            ["search-year", "Year", year, setYear, ""],
            ["search-country", "Country", country, setCountry, ""],
            ["search-format", "Format", format, setFormat, "e.g. CD, Vinyl, Digital Media"],
            ["search-catalog", "Catalog number", catalogNumber, setCatalogNumber, ""],
            ["search-barcode", "Barcode", barcode, setBarcode, ""],
          ] as const
        ).map(([id, label, value, setter, hint]) => (
          <div className="sr-field" key={id}>
            <label className="sr-field__label" htmlFor={id}>
              {label}
            </label>
            <input
              id={id}
              className="sr-input"
              data-testid={id}
              value={value}
              onChange={(event) => setter(event.target.value)}
            />
            {hint && <p className="sr-field__static">{hint}</p>}
          </div>
        ))}

        <button
          type="button"
          className="sr-btn sr-btn--primary"
          data-testid="search-run"
          disabled={busy || !canSearch}
          onClick={() => void search(1)}
        >
          {busy ? "Searching…" : "Search"}
        </button>
      </section>

      {phase !== "form" && page && (
        <section className="sr-section">
          <h3 className="sr-section__title">Releases ({filteredResults.length})</h3>

          <div className="sr-field">
            <label className="sr-field__label" htmlFor="search-filter">
              Filter results
            </label>
            <input
              id="search-filter"
              className="sr-input"
              data-testid="search-filter"
              value={resultFilter}
              onChange={(event) => setResultFilter(event.target.value)}
            />
          </div>

          <div className="sr-field">
            <label className="sr-field__label" htmlFor="search-sort">
              Sort
            </label>
            <select
              id="search-sort"
              className="sr-input"
              data-testid="search-sort"
              value={resultSort}
              onChange={(event) => setResultSort(event.target.value as ResultSort)}
            >
              {localTrackCount !== null && <option value="closest">{SORT_LABELS.closest}</option>}
              {(Object.keys(SORT_LABELS) as ResultSort[])
                .filter((key) => key !== "closest")
                .map((key) => (
                  <option key={key} value={key}>
                    {SORT_LABELS[key]}
                  </option>
                ))}
            </select>
          </div>

          <div className="sr-chips">
            <label className="sr-field__label" htmlFor="search-count-filter">
              Tracks
            </label>
            <input
              id="search-count-filter"
              className="sr-input"
              data-testid="search-count-filter"
              value={resultTrackCount}
              onChange={(event) => setResultTrackCount(event.target.value)}
            />
            {pageYears.length > 0 && (
              <select
                className="sr-input"
                aria-label="Filter year"
                data-testid="search-year-filter"
                value={resultYear}
                onChange={(event) => setResultYear(event.target.value)}
              >
                <option value="">Any year</option>
                {pageYears.map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            )}
            {pageCountries.length > 0 && (
              <select
                className="sr-input"
                aria-label="Filter country"
                data-testid="search-country-filter"
                value={resultCountry}
                onChange={(event) => setResultCountry(event.target.value)}
              >
                <option value="">Any country</option>
                {pageCountries.map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            )}
            {pageFormats.length > 0 && (
              <select
                className="sr-input"
                aria-label="Filter format"
                data-testid="search-format-filter"
                value={resultFormat}
                onChange={(event) => setResultFormat(event.target.value)}
              >
                <option value="">Any format</option>
                {pageFormats.map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            )}
            <label className="sr-field__label">
              <input
                type="checkbox"
                data-testid="search-same-count"
                checked={sameTrackCount}
                disabled={localTrackCount === null || localTrackCount === 0}
                onChange={(event) => {
                  setSameTrackCount(event.target.checked);
                  setResultTrackCount(
                    event.target.checked && localTrackCount ? String(localTrackCount) : "",
                  );
                }}
              />{" "}
              Same track count as this album
            </label>
            {(resultFilter || resultYear || resultCountry || resultFormat || resultTrackCount) && (
              <button
                type="button"
                className="sr-btn"
                data-testid="search-clear-filters"
                onClick={() => {
                  setResultFilter("");
                  setResultYear("");
                  setResultCountry("");
                  setResultFormat("");
                  setResultTrackCount("");
                  setSameTrackCount(false);
                }}
              >
                Clear filters
              </button>
            )}
          </div>

          {countProgress ? (
            <p className="sr-field__static" data-testid="search-count-progress">
              Counting tracks: {countProgress.loaded}/{countProgress.total}{" "}
              <button
                type="button"
                className="sr-btn"
                data-testid="search-count-cancel"
                onClick={() => {
                  countGeneration.current += 1;
                  setCountProgress(null);
                }}
              >
                Cancel
              </button>
            </p>
          ) : (
            <button
              type="button"
              className="sr-btn"
              data-testid="search-load-counts"
              disabled={busy}
              onClick={() => void loadCounts()}
            >
              Load track counts
            </button>
          )}
          {countError && (
            <p className="sr-field__static" data-testid="search-count-error">
              {countError}
            </p>
          )}

          <ul className="sr-chips" data-testid="search-results">
            {visibleResults.map((result) => (
              <li key={resultKey(result)} className="sr-chip">
                {result.title}
                {result.artist ? ` — ${result.artist}` : ""}
                {result.year ? ` (${result.year})` : ""}
                {trackCountOf(result) !== undefined ? ` · ${trackCountOf(result)} tracks` : ""}
                {result.country ? ` · ${result.country}` : ""}
                {result.formats.length > 0 ? ` · ${result.formats.join("/")}` : ""}
                <button
                  type="button"
                  className="sr-btn"
                  data-testid={`search-choose-${result.id}`}
                  disabled={busy}
                  onClick={() => void choose(result)}
                >
                  Use release
                </button>
              </li>
            ))}
          </ul>

          {filteredResults.length > RESULT_PAGE_SIZE && (
            <div className="sr-chips">
              <button
                type="button"
                className="sr-btn"
                data-testid="search-page-prev"
                disabled={resultPage <= 1}
                onClick={() => setResultPage((current) => Math.max(1, current - 1))}
              >
                Previous
              </button>
              <span className="sr-field__static">
                Page {resultPage} of {resultPageCount}
              </span>
              <button
                type="button"
                className="sr-btn"
                data-testid="search-page-next"
                disabled={resultPage >= resultPageCount}
                onClick={() => setResultPage((current) => Math.min(resultPageCount, current + 1))}
              >
                Next
              </button>
            </div>
          )}

          {page.hasNext && (
            <button
              type="button"
              className="sr-btn"
              data-testid="search-more"
              disabled={busy}
              onClick={() => void search(providerPage + 1)}
            >
              Load more from {provider}
            </button>
          )}
        </section>
      )}

      {phase === "mapping" && release && (
        <section className="sr-section">
          <h3 className="sr-section__title">Apply {release.title ?? "release"}</h3>
          <p className="sr-field__static">
            {mappedRows.length} of {mapping.length} local file(s) mapped.
            {unusedRemote.length > 0
              ? ` ${unusedRemote.length} provider track(s) unused.`
              : ""}
          </p>

          <table className="sr-table" data-testid="search-mapping">
            <thead>
              <tr>
                <th>Local file</th>
                <th>Provider track</th>
                <th>Title</th>
                <th>Artist</th>
                <th>Track</th>
                <th>Disc</th>
              </tr>
            </thead>
            <tbody>
              {mapping.map((row) => (
                <tr key={row.localIndex} data-testid={`search-mapping-row-${row.localIndex}`}>
                  <td>{row.localTitle || `Track ${row.localIndex + 1}`}</td>
                  <td>
                    <select
                      className="sr-input"
                      aria-label={`Provider track for ${row.localTitle || row.localIndex + 1}`}
                      data-testid={`search-map-remote-${row.localIndex}`}
                      value={row.selectedRemoteIndex === null ? "" : String(row.selectedRemoteIndex)}
                      onChange={(event) => setRemote(row.localIndex, event.target.value)}
                    >
                      <option value="">Do not update</option>
                      {release.tracks.map((remote, index) => (
                        <option key={index} value={index}>
                          {`${index + 1}. ${remote.title ?? "Untitled"}${
                            remote.artist ? ` — ${remote.artist}` : ""
                          }`}
                        </option>
                      ))}
                    </select>
                    {row.selectedRemoteIndex !== null &&
                      duplicateRemotes.has(row.selectedRemoteIndex) && (
                        <span className="sr-badge sr-badge--warning">
                          Also mapped to another file
                        </span>
                      )}
                  </td>
                  <td>
                    <input
                      className="sr-input"
                      aria-label={`Title for ${row.localTitle || row.localIndex + 1}`}
                      data-testid={`search-map-title-${row.localIndex}`}
                      value={row.editedTitle}
                      onChange={(event) =>
                        editRow(row.localIndex, "editedTitle", event.target.value)
                      }
                    />
                  </td>
                  <td>
                    <input
                      className="sr-input"
                      aria-label={`Artist for ${row.localTitle || row.localIndex + 1}`}
                      data-testid={`search-map-artist-${row.localIndex}`}
                      value={row.editedArtist}
                      onChange={(event) =>
                        editRow(row.localIndex, "editedArtist", event.target.value)
                      }
                    />
                  </td>
                  <td>
                    <input
                      className="sr-input"
                      aria-label={`Track number for ${row.localTitle || row.localIndex + 1}`}
                      data-testid={`search-map-track-${row.localIndex}`}
                      value={row.editedTrackNumber}
                      onChange={(event) =>
                        editRow(row.localIndex, "editedTrackNumber", event.target.value)
                      }
                    />
                  </td>
                  <td>
                    <input
                      className="sr-input"
                      aria-label={`Disc number for ${row.localTitle || row.localIndex + 1}`}
                      data-testid={`search-map-disc-${row.localIndex}`}
                      value={row.editedDiscNumber}
                      onChange={(event) =>
                        editRow(row.localIndex, "editedDiscNumber", event.target.value)
                      }
                    />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          <button
            type="button"
            className="sr-btn"
            data-testid="search-back"
            onClick={() => setPhase("results")}
          >
            Back to results
          </button>
          <button
            type="button"
            className="sr-btn sr-btn--primary"
            data-testid="search-apply"
            disabled={busy || mappedRows.length === 0}
            onClick={() => void apply()}
          >
            {busy ? "Writing…" : `Write ${mappedRows.length} track(s)`}
          </button>
        </section>
      )}

      {message && (
        <p className="sr-banner sr-banner--notice" data-testid="search-result">
          {message}
        </p>
      )}
    </ToolDrawer>
  );
}

/** Seed the mapping from the native preview: one row per local file. */
function rowsFrom(
  candidates: TrackMappingRow[],
  remoteTracks: ProviderAlbum["tracks"],
): MappingRow[] {
  return candidates.map((match, index) => {
    const remoteIndex = match?.remoteIndex ?? null;
    const remote = remoteIndex !== null ? remoteTracks[remoteIndex] : undefined;
    return {
      localIndex: index,
      localTitle: match?.localTitle ?? "",
      localArtist: match?.localArtist ?? "",
      selectedRemoteIndex: remoteIndex,
      editedTitle: remote?.title ?? "",
      editedArtist: remote?.artist ?? "",
      editedArtists: remote?.artists ?? [],
      editedTrackNumber: remote?.trackNumber?.toString() ?? "",
      editedTrackTotal: remote?.trackTotal?.toString() ?? "",
      editedDiscNumber: remote?.discNumber?.toString() ?? "",
    };
  });
}
