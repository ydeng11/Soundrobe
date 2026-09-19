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
import { albumNameOf } from "./monitoring";

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
  const [counting, setCounting] = React.useState(false);
  const [countMessage, setCountMessage] = React.useState<string | null>(null);
  const countGeneration = React.useRef(0);

  const [resultFilter, setResultFilter] = React.useState("");
  const [resultYear, setResultYear] = React.useState("");
  const [resultTrackCount, setResultTrackCount] = React.useState("");
  const [resultCountry, setResultCountry] = React.useState("");
  const [resultFormat, setResultFormat] = React.useState("");
  const [resultSort, setResultSort] = React.useState<ResultSort>("relevance");
  /** Once the reader picks an order, stop choosing one for them. */
  const [sortTouched, setSortTouched] = React.useState(false);
  const [resultPage, setResultPage] = React.useState(1);

  const [release, setRelease] = React.useState<ProviderAlbum | null>(null);
  const [mapping, setMapping] = React.useState<MappingRow[]>([]);
  const [candidate, setCandidate] = React.useState<AlbumCandidate | null>(null);
  const [localTrackCount, setLocalTrackCount] = React.useState<number | null>(null);
  /** Which release is being fetched, so the wait is attributed to a row. */
  const [resolvingId, setResolvingId] = React.useState<string | null>(null);

  const { busy, error, failures, message, run } = useToolRun();
  const canSearch = artist.trim().length > 0 || album.trim().length > 0;
  const albumLabel = React.useMemo(() => albumNameOf(albumPath), [albumPath]);

  /**
   * Any narrowing change returns to the first page.
   *
   * Without this, filtering while on page 3 of a list that now has five matches
   * slices from index 20 and shows nothing at all, with no way back — the pager
   * hides itself because the filtered list is shorter than a page.
   */
  const resetPage = React.useCallback(() => setResultPage(1), []);

  /*
   * Prefer the edition closest to this album, until the reader orders it
   * themselves.
   *
   * The provider's own order is not the album in hand: searching for a 10-track
   * album returned a 26-track edition first, which mapped none of its ten files,
   * while the 10-track edition mapped all ten. Deliberately different from the
   * desktop, which always starts on provider relevance.
   */
  React.useEffect(() => {
    if (!sortTouched && localTrackCount !== null && localTrackCount > 0) {
      setResultSort("closest");
    }
  }, [localTrackCount, sortTouched]);

  /**
   * Derived, never stored separately.
   *
   * As its own flag it could disagree with the Tracks field: tick the box, then
   * edit the number, and the box would still claim the album's own count. One
   * value, one truth.
   */
  const sameTrackCount =
    localTrackCount !== null && resultTrackCount === String(localTrackCount);

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

  /**
   * The count to compare against: the loaded one when there is one.
   *
   * A loaded `null` is meaningful - the provider reports no track count for that
   * release - and falls back to whatever the search result already carried.
   */
  const trackCountOf = (result: ReleaseSearchResult): number | undefined =>
    counts[resultKey(result)] ?? result.trackCount;

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
  const loadCounts = React.useCallback(async () => {
    if (!page) return;
    const generation = (countGeneration.current += 1);
    const pending = page.results;
    setCountError(null);
    setCountMessage(null);
    setCounting(true);
    setCountProgress({ loaded: 0, total: pending.length });
    let loaded = 0;
    for (const result of pending) {
      if (countGeneration.current !== generation) {
        setCounting(false);
        setCountProgress(null);
        setCountMessage("Counting cancelled.");
        return;
      }
      try {
        const count = await api.releaseTrackCount(result.provider, result.id, result.kind);
        setCounts((current) => ({ ...current, [resultKey(result)]: count }));
      } catch (reason) {
        if (countGeneration.current !== generation) {
          setCounting(false);
          setCountProgress(null);
          setCountMessage("Counting cancelled.");
          return;
        }
        setCountError(reason instanceof Error ? reason.message : String(reason));
      }
      loaded += 1;
      setCountProgress({ loaded, total: pending.length });
    }
    setCounting(false);
    setCountProgress(null);
    setCountMessage(`Loaded ${loaded} track count${loaded === 1 ? "" : "s"}.`);
  }, [api, page]);

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
      setResolvingId(resultKey(result));
      try {
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
      } finally {
        setResolvingId(null);
      }
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

  /** Arrow keys move between result rows, as a list of choices should. */
  const resultRefs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const handleResultsKeys = (event: React.KeyboardEvent<HTMLUListElement>) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const buttons = resultRefs.current.filter((entry): entry is HTMLButtonElement => entry !== null);
    if (buttons.length === 0) return;
    const current = buttons.findIndex((entry) => entry === document.activeElement);
    if (current === -1) return;
    event.preventDefault();
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? buttons.length - 1
          : event.key === "ArrowDown"
            ? Math.min(buttons.length - 1, current + 1)
            : Math.max(0, current - 1);
    buttons[next]?.focus();
  };

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
      title={`Search releases — ${albumLabel}`}
      busy={busy}
      error={error}
      errors={failures}
      onClose={onClose}
      testId="search-drawer"
    >
      {/*
        Announced and read first: an outcome reported below ten results is an
        outcome nobody sees.
      */}
      {message && (
        <p
          className="sr-banner sr-banner--notice"
          role="status"
          aria-live="polite"
          data-testid="search-result"
        >
          {message}
        </p>
      )}

      <section className="sr-section">
        <h3 className="sr-section__title">Query</h3>
        <p className="sr-field__static" data-testid="search-target">
          Searching for a release for {albumLabel}
          {localTrackCount !== null ? ` (${localTrackCount} local file${localTrackCount === 1 ? "" : "s"})` : ""}.
        </p>
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
              onKeyDown={(event) => {
                // Enter is how a form is submitted; a search field that ignores
                // it makes the keyboard user reach for the mouse.
                if (event.key === "Enter" && canSearch && !busy) {
                  event.preventDefault();
                  void search(1);
                }
              }}
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
          <h3 className="sr-section__title">Releases</h3>
          <p className="sr-field__static" data-testid="search-results-count" aria-live="polite">
            Showing {visibleResults.length} of {filteredResults.length} matching
            {page.results.length > 0 ? ` — ${page.results.length} from ${provider}` : ""}
            {page.hasNext ? ", more available" : ""}.
          </p>

          <div className="sr-field">
            <label className="sr-field__label" htmlFor="search-filter">
              Filter these results
            </label>
            <input
              id="search-filter"
              className="sr-input"
              data-testid="search-filter"
              placeholder="Title, artist, catalog number or barcode"
              value={resultFilter}
              onChange={(event) => {
                setResultFilter(event.target.value);
                resetPage();
              }}
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
              onChange={(event) => { setResultSort(event.target.value as ResultSort); setSortTouched(true); resetPage(); }}
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
              Tracks (exact)
            </label>
            <input
              id="search-count-filter"
              className="sr-input"
              data-testid="search-count-filter"
              value={resultTrackCount}
              onChange={(event) => { setResultTrackCount(event.target.value); resetPage(); }}
            />
            {pageYears.length > 0 && (
              <select
                className="sr-input"
                aria-label="Filter year"
                data-testid="search-year-filter"
                value={resultYear}
                onChange={(event) => { setResultYear(event.target.value); resetPage(); }}
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
                onChange={(event) => { setResultCountry(event.target.value); resetPage(); }}
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
                onChange={(event) => { setResultFormat(event.target.value); resetPage(); }}
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
                  setResultTrackCount(
                    event.target.checked && localTrackCount ? String(localTrackCount) : "",
                  );
                  resetPage();
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
                  resetPage();
                }}
              >
                Clear filters
              </button>
            )}
          </div>

          {counting && countProgress ? (
            <p className="sr-field__static" data-testid="search-count-progress" role="status">
              Counting tracks: {countProgress.loaded} of {countProgress.total}{" "}
              <button
                type="button"
                className="sr-btn"
                data-testid="search-count-cancel"
                onClick={() => {
                  countGeneration.current += 1;
                  setCounting(false);
                  setCountProgress(null);
                  setCountMessage("Counting cancelled.");
                }}
              >
                Cancel
              </button>
            </p>
          ) : (
            /*
              Counts cost one provider request each, and there can be a hundred
              of them. Running them through the shared busy flag disabled every
              "Use release" button for the duration, so a reader could not pick a
              release while the counts loaded.
            */
            <button
              type="button"
              className="sr-btn"
              data-testid="search-load-counts"
              disabled={counting}
              onClick={() => void loadCounts()}
            >
              Load track counts
            </button>
          )}
          {countMessage && (
            <p className="sr-field__static" data-testid="search-count-message" role="status">
              {countMessage}
            </p>
          )}
          {countError && (
            <p className="sr-field__static" data-testid="search-count-error" role="alert">
              {countError}
            </p>
          )}

          {/*
            The list container is always present, so "no matches" is an empty
            list plus an explanation rather than a missing region.
          */}
          <ul className="sr-results" data-testid="search-results" onKeyDown={handleResultsKeys}>
              {visibleResults.map((result, index) => {
                const key = resultKey(result);
                const resolving = resolvingId === key;
                return (
                  <li key={key}>
                    <button
                      type="button"
                      ref={(element) => {
                        resultRefs.current[index] = element;
                      }}
                      className="sr-result"
                      data-testid={`search-choose-${result.id}`}
                      disabled={busy}
                      aria-busy={resolving}
                      onClick={() => void choose(result)}
                    >
                      <span className="sr-result__main">
                        <span className="sr-result__title">{result.title || "Untitled release"}</span>
                        {result.artist && (
                          <span className="sr-result__artist">{result.artist}</span>
                        )}
                      </span>
                      <span className="sr-result__facts">
                        <Fact label="Year" value={result.year} />
                        <Fact label="Tracks" value={trackCountOf(result)} />
                        <Fact label="Country" value={result.country} />
                        <Fact label="Format" value={result.formats.join("/")} />
                        <Fact label="Catalog" value={result.catalogNumber} />
                      </span>
                      <span className="sr-result__action">
                        {resolving ? "Resolving…" : "Use release"}
                      </span>
                    </button>
                  </li>
              );
            })}
          </ul>
          {filteredResults.length === 0 && (
            <p className="sr-empty__body" data-testid="search-results-empty">
              No release in these results matches the filters above. Clear a filter, or search
              again with different terms.
            </p>
          )}

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
                <th scope="col">Local file</th>
                <th scope="col">Provider track</th>
                <th scope="col">Title</th>
                <th scope="col">Artist</th>
                <th scope="col">Track</th>
                <th scope="col">Total</th>
                <th scope="col">Disc</th>
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
                    {/*
                      The total is written from the provider track, so it has to be
                      visible: a value that is applied but not shown cannot be
                      corrected.
                    */}
                    <input
                      className="sr-input"
                      aria-label={`Track total for ${row.localTitle || row.localIndex + 1}`}
                      data-testid={`search-map-total-${row.localIndex}`}
                      value={row.editedTrackTotal}
                      onChange={(event) =>
                        editRow(row.localIndex, "editedTrackTotal", event.target.value)
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

    </ToolDrawer>
  );
}

/** One labelled fact about a release. An empty fact renders nothing. */
function Fact({ label, value }: { label: string; value: string | number | undefined | null }) {
  if (value === undefined || value === null || value === "") return null;
  return (
    <span className="sr-result__fact">
      <span className="sr-result__fact-label">{label}</span>
      <span className="sr-result__fact-value">{value}</span>
    </span>
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
