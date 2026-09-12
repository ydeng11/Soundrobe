import React, { useState, useCallback, useEffect, useMemo, useRef } from "react";
import type {
  ReleaseSearchResult,
  ReleaseSearchPage,
  ProviderAlbum,
} from "../shared/desktop-api";

interface SearchDialogProps {
  open: boolean;
  albumPath: string;
  onClose: () => void;
  onSelectRelease: (release: ProviderAlbum, provider: string) => void;
}

type Provider = "musicbrainz" | "discogs";
type Phase = "form" | "results" | "detail";

interface SearchCache {
  key: string;
  catalog?: ReleaseSearchPage;
}

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

function normalizedFilterText(value: string): string {
  return value.normalize("NFKC").toLowerCase();
}

function searchCacheKey(
  provider: Provider,
  fields: readonly (string | undefined)[],
): string {
  return JSON.stringify([provider, ...fields]);
}

export function SearchDialog({
  open,
  albumPath,
  onClose,
  onSelectRelease,
}: SearchDialogProps) {
  const [provider, setProvider] = useState<Provider>("musicbrainz");
  const [artist, setArtist] = useState("");
  const [album, setAlbum] = useState("");
  const [year, setYear] = useState("");
  const [country, setCountry] = useState("");
  const [format, setFormat] = useState("");
  const [catalogNumber, setCatalogNumber] = useState("");
  const [barcode, setBarcode] = useState("");

  const [phase, setPhase] = useState<Phase>("form");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchPage, setSearchPage] = useState<ReleaseSearchPage | null>(null);
  const [resultFilter, setResultFilter] = useState("");
  const [resultYear, setResultYear] = useState("");
  const [resultTrackCount, setResultTrackCount] = useState("");
  const [resultCountry, setResultCountry] = useState("");
  const [resultFormat, setResultFormat] = useState("");
  const [sameTrackCount, setSameTrackCount] = useState(false);
  const [localTrackCount, setLocalTrackCount] = useState<number | null>(null);
  const [localCountError, setLocalCountError] = useState<string | null>(null);
  const [countProgress, setCountProgress] = useState<{ loaded: number; total: number } | null>(null);
  const [countError, setCountError] = useState<string | null>(null);
  const countGeneration = useRef(0);
  const [resultSort, setResultSort] = useState<ResultSort>("relevance");
  const [resultPage, setResultPage] = useState(1);
  const [loadingProgress, setLoadingProgress] = useState<{
    loaded: number;
    total?: number;
  } | null>(null);
  const [detailAlbum, setDetailAlbum] = useState<ProviderAlbum | null>(null);
  const [detailTrackCount, setDetailTrackCount] = useState<number | undefined>(undefined);
  const searchGeneration = useRef(0);
  const searchCache = useRef<SearchCache | null>(null);
  const pendingSearch = useRef<{ key: string; generation: number } | null>(null);
  const canSearch = artist.trim().length > 0 || album.trim().length > 0;

  useEffect(() => {
    let active = true;
    countGeneration.current += 1;
    setCountProgress(null);
    setCountError(null);
    setLocalTrackCount(null);
    setLocalCountError(null);
    setSameTrackCount(false);
    setResultTrackCount("");
    setResultPage(1);
    if (open && albumPath) {
      window.api.readAlbum(albumPath).then((detail) => {
        if (active) setLocalTrackCount(detail.tracks.length);
      }).catch((err) => {
        if (active) setLocalCountError(err instanceof Error ? err.message : String(err));
      });
    }
    return () => {
      active = false;
      countGeneration.current += 1;
    };
  }, [open, albumPath]);

  useEffect(() => {
    if (!open) {
      searchGeneration.current += 1;
      pendingSearch.current = null;
      setLoading(false);
      setError(null);
      setSearchPage(null);
      setResultFilter("");
      setResultYear("");
      setResultTrackCount("");
      setResultCountry("");
      setResultFormat("");
      setSameTrackCount(false);
      setResultSort("relevance");
      setResultPage(1);
      setLoadingProgress(null);
      setDetailAlbum(null);
      setDetailTrackCount(undefined);
      setPhase("form");
    }
  }, [open]);

  const handleSearch = useCallback(async () => {
    if (!canSearch) return;
    const trimmedFields = [
      artist.trim() || undefined,
      album.trim() || undefined,
      year.trim() || undefined,
      country.trim() || undefined,
      format.trim() || undefined,
      catalogNumber.trim() || undefined,
      barcode.trim() || undefined,
    ] as const;
    const [
      requestArtist,
      requestAlbum,
      requestYear,
      requestCountry,
      requestFormat,
      requestCatalogNumber,
      requestBarcode,
    ] = trimmedFields;
    const cacheKey = searchCacheKey(provider, trimmedFields);
    if (searchCache.current?.key !== cacheKey) {
      searchCache.current = { key: cacheKey };
    }
    const pendingKey = cacheKey;
    if (pendingSearch.current?.key === pendingKey) return;
    const cachedCatalog = searchCache.current.catalog;

    countGeneration.current += 1;
    setCountProgress(null);
    setCountError(null);
    setError(null);
    setSearchPage(null);
    setResultFilter("");
    setResultYear("");
    setResultTrackCount("");
    setResultCountry("");
    setResultFormat("");
    setSameTrackCount(false);
    setResultSort("relevance");
    setResultPage(1);
    if (cachedCatalog) {
      setLoading(false);
      setLoadingProgress(null);
      setSearchPage(cachedCatalog);
      setPhase("results");
      return;
    }
    const generation = ++searchGeneration.current;
    pendingSearch.current = { key: pendingKey, generation };
    setLoading(true);
    setLoadingProgress({ loaded: 0 });

    try {
      const request = {
        provider,
        artist: requestArtist,
        album: requestAlbum,
        year: requestYear,
        country: requestCountry,
        format: requestFormat,
        catalogNumber: requestCatalogNumber,
        barcode: requestBarcode,
        pageSize: PROVIDER_PAGE_SIZE,
      };
      const results: ReleaseSearchResult[] = [];
      let providerPage = 1;
      let hasNext = true;
      let providerTotal: number | undefined;
      while (hasNext) {
        const page = await window.api.searchReleases({ ...request, page: providerPage });
        if (generation !== searchGeneration.current) return;
        results.push(...page.results);
        providerTotal ??= page.total;
        setLoadingProgress({ loaded: results.length, total: providerTotal });
        hasNext = page.hasNext;
        if (hasNext && page.results.length === 0) {
          throw new Error("Provider returned an empty page before the end of the results");
        }
        providerPage += 1;
      }
      const seen = new Set<string>();
      const uniqueResults = results.filter((result) => {
        const key = `${result.provider}:${result.kind ?? "release"}:${result.id}`;
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
      });
      const completedCatalog = {
        results: uniqueResults,
        page: 1,
        pageSize: RESULT_PAGE_SIZE,
        total: uniqueResults.length,
        hasNext: uniqueResults.length > RESULT_PAGE_SIZE,
      };
      if (searchCache.current?.key === cacheKey) {
        searchCache.current.catalog = completedCatalog;
      }
      setSearchPage(completedCatalog);
      setLoadingProgress(null);
      setPhase("results");
    } catch (err) {
      if (generation === searchGeneration.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (pendingSearch.current?.generation === generation) {
        pendingSearch.current = null;
      }
      if (generation === searchGeneration.current) setLoadingProgress(null);
      if (generation === searchGeneration.current) setLoading(false);
    }
  }, [provider, artist, album, year, country, format, catalogNumber, barcode, canSearch]);

  const editionResults = useMemo(() => {
    const query = normalizedFilterText(resultFilter.trim());
    return (searchPage?.results ?? []).filter((result) => {
      if (query && ![result.title, result.artist, result.catalogNumber, result.barcode]
        .some((value) => normalizedFilterText(value ?? "").includes(query))) return false;
      if (resultYear && result.year !== resultYear) return false;
      if (resultCountry && result.country !== resultCountry) return false;
      if (resultFormat && !result.formats.includes(resultFormat)) return false;
      return true;
    });
  }, [searchPage, resultFilter, resultYear, resultCountry, resultFormat]);

  const filteredResults = useMemo(() => {
    const filtered = editionResults.filter((result) =>
      !resultTrackCount || String(result.trackCount) === resultTrackCount);
    if (resultSort === "closest") {
      if (localTrackCount === null) return filtered;
      return filtered.sort((left, right) => {
        if (left.trackCount === undefined) return right.trackCount === undefined ? 0 : 1;
        if (right.trackCount === undefined) return -1;
        return Math.abs(left.trackCount - localTrackCount) - Math.abs(right.trackCount - localTrackCount);
      });
    }
    if (resultSort === "relevance") return filtered;
    const [field, direction] = resultSort.split("-") as [
      "title" | "artist" | "year" | "tracks",
      "asc" | "desc",
    ];
    const descending = direction === "desc";
    return filtered
      .map((result, index) => ({ result, index }))
      .sort(({ result: left, index: leftIndex }, { result: right, index: rightIndex }) => {
        const leftValue =
          field === "title"
            ? left.title
            : field === "artist"
              ? left.artist
              : field === "year"
                ? left.year
                : left.trackCount;
        const rightValue =
          field === "title"
            ? right.title
            : field === "artist"
              ? right.artist
              : field === "year"
                ? right.year
                : right.trackCount;
        if (leftValue === undefined && rightValue === undefined) return leftIndex - rightIndex;
        if (leftValue === undefined) return 1;
        if (rightValue === undefined) return -1;
        const comparison =
          typeof leftValue === "number" && typeof rightValue === "number"
            ? leftValue - rightValue
            : normalizedFilterText(String(leftValue)).localeCompare(
                normalizedFilterText(String(rightValue)),
              );
        if (comparison !== 0) return descending ? -comparison : comparison;
        return leftIndex - rightIndex;
      })
      .map(({ result }) => result);
  }, [editionResults, resultTrackCount, resultSort, localTrackCount]);

  const visibleResults = useMemo(() => {
    const start = (resultPage - 1) * RESULT_PAGE_SIZE;
    return filteredResults.slice(start, start + RESULT_PAGE_SIZE);
  }, [filteredResults, resultPage]);

  const resultPageCount = Math.max(1, Math.ceil(filteredResults.length / RESULT_PAGE_SIZE));
  const currentResultPage = resultPage;
  const showPagination = filteredResults.length > RESULT_PAGE_SIZE;
  const resultYears = useMemo(
    () => [...new Set((searchPage?.results ?? []).flatMap((result) => result.year ? [result.year] : []))]
      .sort((left, right) => right.localeCompare(left)),
    [searchPage],
  );
  const resultTrackCounts = useMemo(
    () => [...new Set((searchPage?.results ?? []).flatMap((result) => result.trackCount !== undefined ? [result.trackCount] : []))]
      .sort((left, right) => left - right),
    [searchPage],
  );
  const hasResultFilters = Boolean(
    resultFilter.trim() || resultYear || resultTrackCount || resultCountry || resultFormat,
  );

  const resultCountries = [...new Set((searchPage?.results ?? []).flatMap((result) => result.country ? [result.country] : []))].sort();
  const resultFormats = [...new Set((searchPage?.results ?? []).flatMap((result) => result.formats))].sort();
  const missingCountResults = editionResults.filter((result) => result.trackCount === undefined);

  const handleLoadCounts = async () => {
    const generation = ++countGeneration.current;
    const targets = missingCountResults;
    let failures = 0;
    setCountError(null);
    setCountProgress({ loaded: 0, total: targets.length });
    for (let index = 0; index < targets.length; index += 1) {
      if (generation !== countGeneration.current) return;
      const target = targets[index];
      try {
        const count = await window.api.releaseTrackCount(target.provider, target.id, target.kind);
        if (generation !== countGeneration.current) return;
        if (count === null || !Number.isInteger(count) || count < 0) {
          failures += 1;
        } else {
          const update = (catalog: ReleaseSearchPage): ReleaseSearchPage => ({
            ...catalog,
            results: catalog.results.map((result) =>
              result.provider === target.provider && result.kind === target.kind && result.id === target.id
                ? { ...result, trackCount: count } : result),
          });
          if (searchCache.current?.catalog) searchCache.current.catalog = update(searchCache.current.catalog);
          setSearchPage((catalog) => catalog ? update(catalog) : catalog);
        }
      } catch {
        if (generation !== countGeneration.current) return;
        failures += 1;
      }
      setCountProgress({ loaded: index + 1, total: targets.length });
      if (failures) setCountError(`${failures} count lookup${failures === 1 ? "" : "s"} failed or returned no count. Retry to try again.`);
    }
    setCountProgress(null);
  };

  const clearFilters = () => {
    setResultFilter("");
    setResultYear("");
    setResultCountry("");
    setResultFormat("");
    setResultTrackCount("");
    setSameTrackCount(false);
    setResultPage(1);
  };

  const handleOpenDetail = useCallback(async (result: ReleaseSearchResult) => {
    const generation = searchGeneration.current;
    setLoading(true);
    setError(null);
    setDetailTrackCount(undefined);
    try {
      const detail = await window.api.resolveRelease(
        result.provider,
        result.id,
        result.kind,
      );
      if (generation !== searchGeneration.current) return;
      setDetailAlbum(detail);
      setDetailTrackCount(result.trackCount);
      setPhase("detail");
    } catch (err) {
      if (generation === searchGeneration.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (generation === searchGeneration.current) setLoading(false);
    }
  }, []);

  const handleSelectDetail = useCallback(() => {
    if (detailAlbum) {
      onSelectRelease(detailAlbum, provider);
    }
  }, [detailAlbum, provider, onSelectRelease]);

  const handleBackToResults = useCallback(() => {
    setDetailAlbum(null);
    setDetailTrackCount(undefined);
    setPhase("results");
  }, []);

  const handleBackToForm = useCallback(() => {
    countGeneration.current += 1;
    setCountProgress(null);
    setCountError(null);
    searchGeneration.current += 1;
    pendingSearch.current = null;
    setLoading(false);
    setDetailAlbum(null);
    setDetailTrackCount(undefined);
    setSearchPage(null);
    setResultFilter("");
    setResultYear("");
    setResultTrackCount("");
    setResultCountry("");
    setResultFormat("");
    setSameTrackCount(false);
    setResultSort("relevance");
    setResultPage(1);
    setPhase("form");
  }, []);

  const handlePrevPage = useCallback(() => {
    setResultPage((page) => Math.max(1, page - 1));
  }, []);

  const handleNextPage = useCallback(() => {
    setResultPage((page) => Math.min(resultPageCount, page + 1));
  }, [resultPageCount]);

  const handleResultFilterChange = useCallback((value: string) => {
    setResultFilter(value);
    setResultPage(1);
  }, []);

  const handleResultYearChange = useCallback((value: string) => {
    setResultYear(value);
    setResultPage(1);
  }, []);

  const handleResultTrackCountChange = useCallback((value: string) => {
    setSameTrackCount(false);
    setResultTrackCount(value);
    setResultPage(1);
  }, []);

  const handleResultSortChange = useCallback((value: ResultSort) => {
    setResultSort(value);
    setResultPage(1);
  }, []);

  const handleClose = useCallback(() => {
    countGeneration.current += 1;
    searchGeneration.current += 1;
    pendingSearch.current = null;
    onClose();
  }, [onClose]);

  if (!open) return null;

  return (
    <div
      role="dialog"
      aria-label="Search releases"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm"
      onClick={handleClose}
    >
      <div
        className="bg-white rounded-xl shadow-2xl border border-border w-full max-w-2xl max-h-[85vh] flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-border">
          <div className="flex items-center gap-3">
            {phase === "detail" && (
              <button
                onClick={handleBackToResults}
                className="text-text-muted hover:text-text-primary transition-colors"
                title="Back to results"
              >
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="m15 18-6-6 6-6" />
                </svg>
              </button>
            )}
            {phase === "results" && (
              <button
                onClick={handleBackToForm}
                className="text-text-muted hover:text-text-primary transition-colors"
                title="Back to search"
              >
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="m15 18-6-6 6-6" />
                </svg>
              </button>
            )}
            <h2 className="text-sm font-semibold text-text-primary">
              {phase === "form" && "Search releases"}
              {phase === "results" && searchPage && (
                hasResultFilters
                  ? `Results (${filteredResults.length} of ${searchPage.results.length})`
                  : `Results (${searchPage.results.length})`
              )}
              {phase === "detail" && (detailAlbum?.title ?? "Release detail")}
            </h2>
          </div>
          <button
            onClick={handleClose}
            className="text-text-muted hover:text-text-primary transition-colors"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </div>

        {localTrackCount !== null && (
          <p className="px-5 pt-3 text-[12px] text-text-secondary">Local album: {localTrackCount} tracks</p>
        )}
        {localCountError && <p role="status" className="px-5 pt-3 text-[12px] text-red-700">Local track count unavailable: {localCountError}</p>}
        {/* Body */}
        <div className="flex-1 overflow-y-auto p-5">
          {error && (
            <div className="mb-4 p-3 bg-red-50 border border-red-200 rounded-lg text-[12px] text-red-700 flex items-start gap-2">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="mt-0.5 shrink-0">
                <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" />
              </svg>
              <span className="flex-1">{error}</span>
              <button onClick={() => setError(null)} className="text-red-400 hover:text-red-600">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg>
              </button>
            </div>
          )}

          {/* Phase: Form */}
          {phase === "form" && (
            <div className="space-y-3">
              <div>
                <label className="block text-[11px] font-medium text-text-muted mb-1">Provider</label>
                <div className="flex gap-2">
                  <button
                    onClick={() => setProvider("musicbrainz")}
                    className={`px-3 py-1.5 text-[12px] rounded-lg border transition-all ${
                      provider === "musicbrainz"
                        ? "border-accent bg-accent/5 text-accent font-medium"
                        : "border-border text-text-secondary hover:border-accent/40"
                    }`}
                  >
                    MusicBrainz
                  </button>
                  <button
                    onClick={() => setProvider("discogs")}
                    className={`px-3 py-1.5 text-[12px] rounded-lg border transition-all ${
                      provider === "discogs"
                        ? "border-accent bg-accent/5 text-accent font-medium"
                        : "border-border text-text-secondary hover:border-accent/40"
                    }`}
                  >
                    Discogs
                  </button>
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">Artist <span className="text-text-muted/60 font-normal">(required if no album)</span></label>
                  <input
                    type="text"
                    value={artist}
                    onChange={(e) => setArtist(e.target.value)}
                    placeholder="Artist name"
                    className="w-full h-8 px-2.5 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 focus:shadow-[0_0_0_2px_rgba(0,122,255,0.12)] bg-white"
                    onKeyDown={(e) => { if (e.key === "Enter") handleSearch(); }}
                  />
                </div>
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">Album <span className="text-text-muted/60 font-normal">(required if no artist)</span></label>
                  <input
                    type="text"
                    value={album}
                    onChange={(e) => setAlbum(e.target.value)}
                    placeholder="Album title"
                    className="w-full h-8 px-2.5 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 focus:shadow-[0_0_0_2px_rgba(0,122,255,0.12)] bg-white"
                    onKeyDown={(e) => { if (e.key === "Enter") handleSearch(); }}
                  />
                </div>
              </div>

              <p className="text-[10.5px] text-text-muted/70 -mt-1.5">Artist or Album is required. All other fields are optional.</p>

              <div className="grid grid-cols-3 gap-3">
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">Year <span className="text-text-muted/50 font-normal">(optional)</span></label>
                  <input
                    type="text"
                    value={year}
                    onChange={(e) => setYear(e.target.value)}
                    placeholder="e.g. 2004"
                    className="w-full h-8 px-2.5 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 bg-white"
                  />
                </div>
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">Country <span className="text-text-muted/50 font-normal">(optional)</span></label>
                  <input
                    type="text"
                    value={country}
                    onChange={(e) => setCountry(e.target.value)}
                    placeholder="e.g. US"
                    className="w-full h-8 px-2.5 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 bg-white"
                  />
                </div>
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">Format <span className="text-text-muted/50 font-normal">(optional)</span></label>
                  <input
                    type="text"
                    value={format}
                    onChange={(e) => setFormat(e.target.value)}
                    placeholder="e.g. CD"
                    className="w-full h-8 px-2.5 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 bg-white"
                  />
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">Catalog Number <span className="text-text-muted/50 font-normal">(optional)</span></label>
                  <input
                    type="text"
                    value={catalogNumber}
                    onChange={(e) => setCatalogNumber(e.target.value)}
                    placeholder="e.g. CDP 7243..."
                    className="w-full h-8 px-2.5 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 bg-white"
                  />
                </div>
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">Barcode <span className="text-text-muted/50 font-normal">(optional)</span></label>
                  <input
                    type="text"
                    value={barcode}
                    onChange={(e) => setBarcode(e.target.value)}
                    placeholder="UPC/EAN"
                    className="w-full h-8 px-2.5 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 bg-white"
                  />
                </div>
              </div>

              <div className="pt-2">
                <button
                  onClick={() => handleSearch()}
                  disabled={!canSearch || loading}
                  className={`w-full h-9 rounded-lg text-[13px] font-medium transition-all ${
                    !canSearch || loading
                      ? "bg-gray-100 text-text-muted/60 cursor-not-allowed"
                      : "bg-accent text-white hover:bg-accent/90 active:scale-[0.98]"
                  }`}
                >
                  {loading
                    ? loadingProgress?.total
                      ? `Loading ${loadingProgress.loaded} of ${loadingProgress.total}…`
                      : "Searching…"
                    : "Search"}
                </button>
              </div>
            </div>
          )}

          {/* Phase: Results */}
          {phase === "results" && searchPage && (
            <div className="space-y-2">
              {searchPage.results.length > 0 && (
                <div className="space-y-2 mb-3">
                  <input
                    type="search"
                    value={resultFilter}
                    onChange={(event) => handleResultFilterChange(event.target.value)}
                    placeholder="Filter title, artist, catalog number or barcode"
                    aria-label="Filter title, artist, catalog number or barcode"
                    className="w-full h-8 px-2.5 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 focus:shadow-[0_0_0_2px_rgba(0,122,255,0.12)] bg-white"
                  />
                  <div className="grid grid-cols-3 gap-2">
                    <select
                      value={resultYear}
                      onChange={(event) => handleResultYearChange(event.target.value)}
                      aria-label="Filter year"
                      className="h-8 px-2 text-[11px] border border-border rounded-lg outline-none focus:border-accent/60 bg-white text-text-secondary"
                    >
                      <option value="">All years</option>
                      {resultYears.map((value) => <option key={value} value={value}>{value}</option>)}
                    </select>
                    <select
                      value={resultTrackCount}
                      onChange={(event) => handleResultTrackCountChange(event.target.value)}
                      aria-label="Filter track count"
                      disabled={resultTrackCounts.length === 0 && !resultTrackCount}
                      className="h-8 px-2 text-[11px] border border-border rounded-lg outline-none focus:border-accent/60 bg-white text-text-secondary disabled:bg-gray-50 disabled:text-text-muted/60"
                    >
                      <option value="">All track counts</option>
                      {resultTrackCount && !resultTrackCounts.includes(Number(resultTrackCount)) && (
                        <option value={resultTrackCount}>{resultTrackCount} tracks</option>
                      )}
                      {resultTrackCounts.map((value) => <option key={value} value={value}>{value} tracks</option>)}
                    </select>
                    <select
                      value={resultSort}
                      onChange={(event) => handleResultSortChange(event.target.value as ResultSort)}
                      aria-label="Sort results"
                      className="h-8 px-2 text-[11px] border border-border rounded-lg outline-none focus:border-accent/60 bg-white text-text-secondary"
                    >
                      <option value="relevance">Sort: relevance</option>
                      {localTrackCount !== null && <option value="closest">Tracks: closest to local album</option>}
                      <option value="title-asc">Title: A–Z</option>
                      <option value="title-desc">Title: Z–A</option>
                      <option value="artist-asc">Artist: A–Z</option>
                      <option value="artist-desc">Artist: Z–A</option>
                      <option value="year-desc">Year: newest</option>
                      <option value="year-asc">Year: oldest</option>
                      {resultTrackCounts.length > 0 && (
                        <>
                          <option value="tracks-desc">Tracks: most</option>
                          <option value="tracks-asc">Tracks: fewest</option>
                        </>
                      )}
                    </select>
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    <select aria-label="Filter country" value={resultCountry} onChange={(event) => { setResultCountry(event.target.value); setResultPage(1); }} className="h-8 px-2 text-[11px] border border-border rounded-lg bg-white">
                      <option value="">All countries</option>
                      {resultCountries.map((value) => <option key={value} value={value}>{value}</option>)}
                    </select>
                    <select aria-label="Filter format" value={resultFormat} onChange={(event) => { setResultFormat(event.target.value); setResultPage(1); }} className="h-8 px-2 text-[11px] border border-border rounded-lg bg-white">
                      <option value="">All formats</option>
                      {resultFormats.map((value) => <option key={value} value={value}>{value}</option>)}
                    </select>
                  </div>
                  <div className="flex items-center justify-between gap-2 text-[12px]">
                    <label className="flex items-center gap-2">
                      <input type="checkbox" checked={sameTrackCount} disabled={localTrackCount === null || localTrackCount === 0}
                        onChange={(event) => {
                          setSameTrackCount(event.target.checked);
                          setResultTrackCount(event.target.checked ? String(localTrackCount) : "");
                          setResultPage(1);
                        }} />
                      Same track count ({localTrackCount ?? "unknown"})
                    </label>
                    {hasResultFilters && <button onClick={clearFilters} className="text-accent">Clear filters</button>}
                  </div>
                  <p role="status" className="text-[11px] text-text-muted">
                    Track counts loaded: {editionResults.length - missingCountResults.length} of {editionResults.length}.
                    {missingCountResults.length > 0 && " Unknown counts are excluded from track-count matches."}
                  </p>
                  {resultTrackCounts.length === 0 && (
                    <p className="text-[11px] text-text-muted">No track counts available yet. Narrow the editions, then load their counts.</p>
                  )}
                  {countProgress ? (
                    <div className="flex items-center gap-3 text-[12px]">
                      <span>Loading track counts {countProgress.loaded} of {countProgress.total}…</span>
                      <button className="text-accent" onClick={() => { countGeneration.current += 1; setCountProgress(null); }}>Cancel count loading</button>
                    </div>
                  ) : missingCountResults.length > 0 && (
                    <button className="text-[12px] text-accent" onClick={handleLoadCounts}>
                      Load track counts for {missingCountResults.length} {missingCountResults.length === 1 ? "release" : "releases"}
                    </button>
                  )}
                  {countError && <p role="status" className="text-[11px] text-red-700">{countError}</p>}
                </div>
              )}
              {searchPage.results.length === 0 ? (
                <div className="text-center py-10 text-text-muted text-[13px]">
                  No releases found. Try different search terms.
                </div>
              ) : filteredResults.length === 0 ? (
                <div className="text-center py-10 text-text-muted text-[13px]">
                  No cached releases match these filters.
                </div>
              ) : (
                <>
                  {visibleResults.map((result) => (
                    <button
                      key={`${result.provider}-${result.kind ?? "release"}-${result.id}`}
                      onClick={() => handleOpenDetail(result)}
                      className="w-full text-left p-3 rounded-lg border border-border hover:border-accent/40 hover:bg-surface-hover transition-all"
                    >
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="text-[13px] font-medium text-text-primary truncate">
                            {result.title}
                          </div>
                          <div className="text-[11px] text-text-muted mt-0.5">
                            {result.artist && <span>{result.artist}</span>}
                            {result.artist && result.year && <span> · </span>}
                            {result.year && <span>{result.year}</span>}
                            {(result.artist || result.year) && <span> · </span>}
                            <span>{result.trackCount !== undefined ? `${result.trackCount} tracks` : "Track count unknown"}</span>
                            {result.country && <span> · {result.country}</span>}
                            {result.catalogNumber && <span> · <span>{result.catalogNumber}</span></span>}
                          </div>
                          {result.formats.length > 0 && (
                            <div className="flex gap-1 mt-1 flex-wrap">
                              {result.formats.map((fmt, i) => (
                                <span key={i} className="text-[10px] px-1.5 py-0.5 bg-surface-alt rounded text-text-muted">
                                  {fmt}
                                </span>
                              ))}
                            </div>
                          )}
                        </div>
                        <span className="shrink-0 text-[10px] px-2 py-0.5 rounded-full bg-accent/10 text-accent font-medium">
                          {result.provider === "musicbrainz" ? "MB" : "DG"}
                          {result.kind === "master" ? " (master)" : ""}
                        </span>
                      </div>
                    </button>
                  ))}
                </>
              )}

              {/* Pagination */}
              {showPagination && (
                <div className="flex items-center justify-center gap-4 pt-3">
                  <button
                    onClick={handlePrevPage}
                    disabled={currentResultPage <= 1}
                    className="px-3 py-1 text-[12px] rounded-lg border border-border text-text-secondary hover:bg-surface-hover disabled:opacity-30 disabled:cursor-not-allowed transition-all"
                  >
                    &lt; Prev
                  </button>
                  <span className="text-[12px] text-text-muted">
                    Page {currentResultPage} of {resultPageCount}
                  </span>
                  <button
                    onClick={handleNextPage}
                    disabled={resultPage >= resultPageCount}
                    className="px-3 py-1 text-[12px] rounded-lg border border-border text-text-secondary hover:bg-surface-hover disabled:opacity-30 disabled:cursor-not-allowed transition-all"
                  >
                    Next &gt;
                  </button>
                </div>
              )}
            </div>
          )}

          {/* Phase: Detail */}
          {phase === "detail" && detailAlbum && (
            <div className="space-y-4">
              <div>
                <h3 className="text-[15px] font-semibold text-text-primary">{detailAlbum.title}</h3>
                <p className="text-[12px] text-text-muted">
                  {detailAlbum.artist}
                  {detailAlbum.artist && detailAlbum.year && " · "}
                  {detailAlbum.year}
                </p>
                {detailAlbum.genre && (
                  <span className="inline-block mt-1 text-[10px] px-2 py-0.5 bg-surface-alt rounded text-text-muted">
                    {detailAlbum.genre}
                  </span>
                )}
              </div>

              <div className="border-t border-border pt-3">
                <h4 className="text-[11px] font-semibold text-text-muted uppercase tracking-wide mb-2">Tracks ({detailTrackCount ?? detailAlbum.tracks.length})</h4>
                <div className="space-y-1">
                  {detailAlbum.tracks.map((track, i) => (
                    <div key={i} className="flex items-center gap-3 text-[12px] text-text-secondary py-1 px-2 rounded hover:bg-surface-hover">
                      <span className="w-6 text-right text-text-muted tabular-nums">
                        {track.trackNumber ?? i + 1}
                      </span>
                      <span className="flex-1 truncate">{track.title ?? `Track ${i + 1}`}</span>
                      {track.artist && track.artist !== detailAlbum.artist && (
                        <span className="text-text-muted truncate max-w-[120px]">{track.artist}</span>
                      )}
                      {track.length && (
                        <span className="text-text-muted tabular-nums w-12 text-right">
                          {Math.floor(track.length / 60)}:{(track.length % 60).toString().padStart(2, "0")}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              </div>

              <div className="flex gap-3 pt-3 border-t border-border">
                <button
                  onClick={handleBackToResults}
                  className="px-4 py-2 text-[12px] rounded-lg border border-border text-text-secondary hover:bg-surface-hover transition-all"
                >
                  Back
                </button>
                <button
                  onClick={handleSelectDetail}
                  className="flex-1 px-4 py-2 text-[12px] font-medium rounded-lg bg-accent text-white hover:bg-accent/90 active:scale-[0.98] transition-all"
                >
                  Select this release
                </button>
              </div>
            </div>
          )}

          {loading && (
            <div className="flex items-center justify-center py-8">
              <div className="w-5 h-5 border-2 border-accent/30 border-t-accent rounded-full animate-spin" />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
