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
  pages: Map<number, ReleaseSearchPage>;
}

const PROVIDER_PAGE_SIZE = 100;
const RESULT_PAGE_SIZE = 10;

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
  const [localCatalog, setLocalCatalog] = useState(false);
  const [resultFilter, setResultFilter] = useState("");
  const [resultPage, setResultPage] = useState(1);
  const [detailAlbum, setDetailAlbum] = useState<ProviderAlbum | null>(null);
  const searchGeneration = useRef(0);
  const searchCache = useRef<SearchCache | null>(null);
  const pendingSearch = useRef<{ key: string; generation: number } | null>(null);
  const canSearch = artist.trim().length > 0 || album.trim().length > 0;

  useEffect(() => {
    if (!open) {
      searchGeneration.current += 1;
      pendingSearch.current = null;
      setLoading(false);
      setError(null);
      setSearchPage(null);
      setLocalCatalog(false);
      setResultFilter("");
      setResultPage(1);
      setDetailAlbum(null);
      setPhase("form");
    }
  }, [open]);

  const handleSearch = useCallback(async (pageNum = 1) => {
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
    const shouldCacheCatalog = provider === "musicbrainz" && requestArtist !== undefined;
    const cacheKey = searchCacheKey(provider, trimmedFields);
    if (searchCache.current?.key !== cacheKey) {
      searchCache.current = { key: cacheKey, pages: new Map() };
    }
    const pendingKey = JSON.stringify([
      cacheKey,
      shouldCacheCatalog ? "catalog" : pageNum,
    ]);
    if (pendingSearch.current?.key === pendingKey) return;
    const cachedPage = shouldCacheCatalog
      ? searchCache.current.catalog
      : searchCache.current.pages.get(pageNum);

    setError(null);
    if (pageNum === 1) {
      setSearchPage(null);
      setLocalCatalog(false);
      setResultFilter("");
      setResultPage(1);
    }
    if (cachedPage) {
      setLoading(false);
      setSearchPage(cachedPage);
      setLocalCatalog(shouldCacheCatalog);
      setPhase("results");
      return;
    }
    const generation = ++searchGeneration.current;
    pendingSearch.current = { key: pendingKey, generation };
    setLoading(true);

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
        pageSize: shouldCacheCatalog ? PROVIDER_PAGE_SIZE : RESULT_PAGE_SIZE,
      };
      if (!shouldCacheCatalog) {
        const page = await window.api.searchReleases({ ...request, page: pageNum });
        if (generation !== searchGeneration.current) return;
        if (searchCache.current?.key === cacheKey) {
          searchCache.current.pages.set(pageNum, page);
        }
        setSearchPage(page);
        setLocalCatalog(false);
        setPhase("results");
        return;
      }
      const results: ReleaseSearchResult[] = [];
      let providerPage = 1;
      let hasNext = true;
      while (hasNext) {
        const page = await window.api.searchReleases({ ...request, page: providerPage });
        if (generation !== searchGeneration.current) return;
        results.push(...page.results);
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
      setLocalCatalog(true);
      setPhase("results");
    } catch (err) {
      if (generation === searchGeneration.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (pendingSearch.current?.generation === generation) {
        pendingSearch.current = null;
      }
      if (generation === searchGeneration.current) setLoading(false);
    }
  }, [provider, artist, album, year, country, format, catalogNumber, barcode, canSearch]);

  const filteredResults = useMemo(() => {
    const query = normalizedFilterText(resultFilter.trim());
    if (!searchPage || !query) return searchPage?.results ?? [];
    return searchPage.results.filter((result) =>
      normalizedFilterText(result.title).includes(query),
    );
  }, [searchPage, resultFilter]);

  const visibleResults = useMemo(() => {
    if (!localCatalog) return filteredResults;
    const start = (resultPage - 1) * RESULT_PAGE_SIZE;
    return filteredResults.slice(start, start + RESULT_PAGE_SIZE);
  }, [filteredResults, localCatalog, resultPage]);

  const resultPageCount = localCatalog
    ? Math.max(1, Math.ceil(filteredResults.length / RESULT_PAGE_SIZE))
    : Math.max(
        1,
        Math.ceil((searchPage?.total ?? 0) / (searchPage?.pageSize ?? RESULT_PAGE_SIZE)),
      );
  const currentResultPage = localCatalog ? resultPage : (searchPage?.page ?? 1);
  const showPagination = localCatalog
    ? filteredResults.length > RESULT_PAGE_SIZE
    : (searchPage?.total ?? 0) > (searchPage?.pageSize ?? RESULT_PAGE_SIZE);

  const handleOpenDetail = useCallback(async (result: ReleaseSearchResult) => {
    setLoading(true);
    setError(null);
    try {
      const detail = await window.api.resolveRelease(
        result.provider,
        result.id,
        result.kind,
      );
      setDetailAlbum(detail);
      setPhase("detail");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  const handleSelectDetail = useCallback(() => {
    if (detailAlbum) {
      onSelectRelease(detailAlbum, provider);
    }
  }, [detailAlbum, provider, onSelectRelease]);

  const handleBackToResults = useCallback(() => {
    setDetailAlbum(null);
    setPhase("results");
  }, []);

  const handleBackToForm = useCallback(() => {
    searchGeneration.current += 1;
    pendingSearch.current = null;
    setLoading(false);
    setDetailAlbum(null);
    setSearchPage(null);
    setLocalCatalog(false);
    setResultFilter("");
    setResultPage(1);
    setPhase("form");
  }, []);

  const handlePrevPage = useCallback(() => {
    if (localCatalog) {
      setResultPage((page) => Math.max(1, page - 1));
    } else if (searchPage && searchPage.page > 1) {
      handleSearch(searchPage.page - 1);
    }
  }, [localCatalog, searchPage, handleSearch]);

  const handleNextPage = useCallback(() => {
    if (localCatalog) {
      setResultPage((page) => Math.min(resultPageCount, page + 1));
    } else if (searchPage?.hasNext) {
      handleSearch(searchPage.page + 1);
    }
  }, [localCatalog, resultPageCount, searchPage, handleSearch]);

  const handleResultFilterChange = useCallback((value: string) => {
    setResultFilter(value);
    setResultPage(1);
  }, []);

  const handleClose = useCallback(() => {
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
                localCatalog && resultFilter.trim()
                  ? `Results (${filteredResults.length} of ${searchPage.results.length})`
                  : `Results (${localCatalog ? searchPage.results.length : (searchPage.total ?? "?")})`
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
                  {loading ? "Searching…" : "Search"}
                </button>
              </div>
            </div>
          )}

          {/* Phase: Results */}
          {phase === "results" && searchPage && (
            <div className="space-y-2">
              {searchPage.results.length > 0 && (
                <input
                  type="search"
                  value={resultFilter}
                  onChange={(event) => handleResultFilterChange(event.target.value)}
                  placeholder="Filter release titles"
                  className="w-full h-8 px-2.5 mb-2 text-[12px] border border-border rounded-lg outline-none focus:border-accent/60 focus:shadow-[0_0_0_2px_rgba(0,122,255,0.12)] bg-white"
                />
              )}
              {searchPage.results.length === 0 ? (
                <div className="text-center py-10 text-text-muted text-[13px]">
                  No releases found. Try different search terms.
                </div>
              ) : filteredResults.length === 0 ? (
                <div className="text-center py-10 text-text-muted text-[13px]">
                  {localCatalog
                    ? "No cached releases match this title."
                    : "No releases on this page match this title."}
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
                    disabled={localCatalog ? resultPage >= resultPageCount : !searchPage.hasNext}
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
                <h4 className="text-[11px] font-semibold text-text-muted uppercase tracking-wide mb-2">Tracks</h4>
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
