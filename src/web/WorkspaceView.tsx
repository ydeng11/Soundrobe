/**
 * Browser tagging workspace.
 *
 * The folder tree stays to the left of a music-first album view. Track details
 * dock beneath the table; all tag writes still go through the shared Rust writer.
 */

import React from "react";
import type { DirEntry, DirectoryData, LibraryRoot, TrackData } from "../shared/desktop-api";
import {
  listMonitoringReleases,
  markMonitoringReleaseReviewed,
  type MonitoredRelease,
} from "../shared/web-adapter";
import { DirectoryTree } from "./DirectoryTree";
import { TrackDetailEditor } from "./TrackDetailEditor";
import { TrackTable, type WebTableSort } from "./TrackTable";
import { TrackTools } from "./TrackTools";
import { WorkspaceSettingsDrawer } from "./WorkspaceSettingsDrawer";
import { WEB_TABLE_COLUMNS, fileNameOf, type WebTableColumnKey } from "./columns";
import { albumNameOf, needsReviewPaths } from "./review";

/** Keys of the browser-side presentation preferences. */
const DETAILS_KEY = "soundrobe.web.workspace.details";
const COLUMNS_KEY = "soundrobe.web.workspace.columns";
const ALL_COLUMNS: WebTableColumnKey[] = WEB_TABLE_COLUMNS.map((column) => column.key);

/** Preferences are presentation-only, so they live in this browser, not the service. */
function loadVisibleColumns(): WebTableColumnKey[] {
  try {
    const raw = window.localStorage.getItem(COLUMNS_KEY);
    if (!raw) return ALL_COLUMNS;
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return ALL_COLUMNS;
    const known = parsed.filter(
      (key): key is WebTableColumnKey => ALL_COLUMNS.includes(key as WebTableColumnKey),
    );
    // The file name identifies the row, so it is never dropped.
    return known.includes("fileName") ? known : ["fileName", ...known];
  } catch {
    return ALL_COLUMNS;
  }
}

function loadDetailsVisible(): boolean {
  try {
    return window.localStorage.getItem(DETAILS_KEY) !== "false";
  } catch {
    return true;
  }
}

const defaultMarkReviewed = (path: string) => markMonitoringReleaseReviewed(path);

async function readDirectoryDescendants(
  api: Window["api"],
  entries: readonly DirEntry[],
): Promise<DirectoryData[]> {
  const groups = await Promise.all(
    entries.map(async (entry) => {
      try {
        const data = await api.readDirectory(entry.path);
        return [data, ...(await readDirectoryDescendants(api, data.subdirs ?? []))];
      } catch {
        return [];
      }
    }),
  );
  return groups.flat();
}

function commonTrackValue(
  tracks: TrackData[],
  read: (track: TrackData) => string | null,
): string | null {
  const values = new Set(
    tracks.map((track) => read(track)?.trim() ?? "").filter((value) => value.length > 0),
  );
  return values.size === 1 ? [...values][0] ?? null : null;
}

function folderBrowserStartsOpen(): boolean {
  return window.matchMedia?.("(min-width: 901px)").matches ?? true;
}

function folderPathPrefixes(path: string): string[] {
  const separator = path.includes("\\") ? "\\" : "/";
  const parts = path.split(separator).filter(Boolean);
  let current = path.startsWith(separator) ? separator : "";
  return parts.map((part) => {
    if (current && !current.endsWith(separator)) current += separator;
    current += part;
    return current;
  });
}

export interface WorkspaceViewProps {
  /** Injected for tests; defaults to the installed `window.api` facade. */
  api?: Window["api"];
  /** The shell keeps this view mounted so its browsing context survives navigation. */
  active?: boolean;
  /** Albums background tagging classified; injected for tests. */
  loadReleases?: () => Promise<{ releases: MonitoredRelease[] }>;
  /** Acknowledge one reviewed album; injected for tests. */
  markReviewed?: (path: string) => Promise<unknown>;
  /**
   * An album to open on arrival, set when Monitoring sends the reader to the
   * album it just reported a problem with.
   */
  openAlbumPath?: string | null;
  /** Called once `openAlbumPath` has been loaded, so the caller can clear it. */
  onAlbumOpened?: () => void;
  /** Poll interval for durable review changes made by background workers. */
  reviewRefreshMs?: number;
}

interface DirectoryView {
  path: string;
  tracks: TrackData[];
  subdirs: DirEntry[];
  /** Lowercased file names in this directory, used for LRC detection. */
  names: ReadonlySet<string>;
  coverDataUrl: string | null;
}

export function WorkspaceView({
  api = window.api,
  active = true,
  loadReleases = listMonitoringReleases,
  markReviewed = defaultMarkReviewed,
  openAlbumPath = null,
  onAlbumOpened,
  reviewRefreshMs = 5_000,
}: WorkspaceViewProps) {
  const [roots, setRoots] = React.useState<LibraryRoot[]>([]);
  const [rootIndex, setRootIndex] = React.useState(0);
  const [directory, setDirectory] = React.useState<DirectoryView | null>(null);
  const [childrenByPath, setChildrenByPath] = React.useState<
    Record<string, DirEntry[] | undefined>
  >({});
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(new Set());
  const [selectedPaths, setSelectedPaths] = React.useState<string[]>([]);
  const [filterText, setFilterText] = React.useState("");
  const [sort, setSort] = React.useState<WebTableSort>({ key: "fileName", ascending: true });
  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [folderBrowserOpen, setFolderBrowserOpen] = React.useState(folderBrowserStartsOpen);
  const [editorOpen, setEditorOpen] = React.useState(true);
  const [detailsVisible, setDetailsVisible] = React.useState(loadDetailsVisible);
  const [visibleColumns, setVisibleColumns] =
    React.useState<WebTableColumnKey[]>(loadVisibleColumns);
  /** Albums the service's own classification left needing a person. */
  const [reviewPaths, setReviewPaths] = React.useState<ReadonlySet<string>>(new Set());
  const [acknowledgingPath, setAcknowledgingPath] = React.useState<string | null>(null);
  const reviewRefreshGeneration = React.useRef(0);
  const directoryGeneration = React.useRef(0);

  React.useEffect(() => {
    const mediaQuery = window.matchMedia?.("(min-width: 901px)");
    if (!mediaQuery) return undefined;
    const syncFolderDisclosure = () => setFolderBrowserOpen(mediaQuery.matches);
    syncFolderDisclosure();
    mediaQuery.addEventListener("change", syncFolderDisclosure);
    return () => mediaQuery.removeEventListener("change", syncFolderDisclosure);
  }, []);

  const root = roots[rootIndex] ?? null;

  /** Flagged albums under the mounted library, so the banner stays contextual. */
  const reviewInLibrary = React.useMemo(() => {
    if (!root) return [];
    // Compare against the separator too: a sibling library `/music-archive`
    // otherwise looks like a child of `/music` and would show the wrong
    // library's albums in this banner.
    const prefix = root.path.endsWith("/") ? root.path : `${root.path}/`;
    return [...reviewPaths].filter(
      (path) => path === root.path || path.startsWith(prefix),
    );
  }, [reviewPaths, root]);

  const refreshReviewPaths = React.useCallback(async () => {
    // Review state is the service's last classification, so it is re-read
    // rather than cached: a fix made in another app must clear the badge here.
    const generation = ++reviewRefreshGeneration.current;
    try {
      const { releases } = await loadReleases();
      if (generation === reviewRefreshGeneration.current) {
        setReviewPaths(new Set(needsReviewPaths(releases)));
      }
    } catch {
      // Keep the last durable classification visible through a transient
      // service error. Clearing it would falsely tell the operator that the
      // release no longer needs review.
    }
  }, [loadReleases]);

  React.useEffect(() => {
    if (!active) return undefined;
    void refreshReviewPaths();
    if (reviewRefreshMs <= 0) return;
    const timer = window.setInterval(() => void refreshReviewPaths(), reviewRefreshMs);
    return () => {
      reviewRefreshGeneration.current += 1;
      window.clearInterval(timer);
    };
  }, [active, refreshReviewPaths, reviewRefreshMs]);

  const acknowledgeReview = React.useCallback(
    async (path: string) => {
      setAcknowledgingPath(path);
      setError(null);
      try {
        await markReviewed(path);
        setReviewPaths((current) => {
          const next = new Set(current);
          next.delete(path);
          return next;
        });
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setAcknowledgingPath(null);
      }
    },
    [markReviewed],
  );

  const loadDirectory = React.useCallback(
    async (path: string, includeChildTracks = false) => {
      const generation = ++directoryGeneration.current;
      setLoading(true);
      setError(null);
      try {
        const [data, entries]: [DirectoryData, DirEntry[]] = await Promise.all([
          api.readDirectory(path),
          api.listDirectory(path),
        ]);
        // A mounted root is an artist folder; its child folders are albums.
        const childDirectories = includeChildTracks
          ? await readDirectoryDescendants(api, data.subdirs ?? [])
          : [];
        const tracks = [
          ...(data.tracks ?? []),
          ...childDirectories.flatMap((child) => child.tracks ?? []),
        ];
        const cover = await api.getCoverDataUrl(path, undefined).catch(() => null);
        // The retained workspace may receive a newer navigation while this read is pending.
        if (generation !== directoryGeneration.current) return false;
        setDirectory({
          path,
          tracks,
          subdirs: data.subdirs ?? [],
          names: new Set(
            entries.filter((entry) => !entry.isDirectory).map((entry) => entry.name.toLowerCase()),
          ),
          coverDataUrl: cover,
        });
        setChildrenByPath((current) => ({
          ...current,
          [path]: data.subdirs ?? [],
          ...Object.fromEntries(childDirectories.map((child) => [child.path, child.subdirs ?? []])),
        }));
        setExpanded((current) => new Set([...current, ...folderPathPrefixes(path)]));
        setSelectedPaths([]);
      } catch (reason) {
        if (generation === directoryGeneration.current) {
          setError(reason instanceof Error ? reason.message : String(reason));
          setDirectory(null);
        }
      } finally {
        if (generation === directoryGeneration.current) setLoading(false);
      }
      return generation === directoryGeneration.current;
    },
    [api],
  );

  React.useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const list = await api.listLibraryRoots();
        if (!active) return;
        if (list.length > 0) await loadDirectory(list[0].path, true);
        else setLoading(false);
        // Finish the initial tree cache before a pending Monitoring request can supersede it.
        if (active) setRoots(list);
      } catch (reason) {
        if (!active) return;
        setError(reason instanceof Error ? reason.message : String(reason));
        setLoading(false);
      }
    })();
    return () => {
      active = false;
    };
  }, [api, loadDirectory]);

  /**
   * Open the album Monitoring pointed at, once it is known where the album sits.
   *
   * The root list has to resolve first: a drilled-in path is absolute, and the
   * library selector and tree both read from `roots`. The callback clears the
   * request so a later re-render does not drag the reader back.
   */
  const drillInRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!openAlbumPath) {
      drillInRef.current = null;
      return;
    }
    if (roots.length === 0) return;
    if (drillInRef.current === openAlbumPath) return;
    drillInRef.current = openAlbumPath;
    const match = roots.findIndex((entry) => {
      const prefix = entry.path.endsWith("/") ? entry.path : `${entry.path}/`;
      return openAlbumPath === entry.path || openAlbumPath.startsWith(prefix);
    });
    if (match >= 0) setRootIndex(match);
    void loadDirectory(openAlbumPath).then((current) => {
      if (current) onAlbumOpened?.();
    });
  }, [openAlbumPath, onAlbumOpened, roots, loadDirectory]);

  const handleToggle = React.useCallback(
    (path: string) => {
      setExpanded((current) => {
        const next = new Set(current);
        if (next.has(path)) next.delete(path);
        else next.add(path);
        return next;
      });
    },
    [],
  );

  const selectedTrackPath = selectedPaths.length > 0 ? selectedPaths[selectedPaths.length - 1] : null;
  const selectedTrack =
    directory?.tracks.find((track) => track.path === selectedTrackPath) ?? null;
  const directoryTracks = directory?.tracks ?? [];
  const isLibraryRoot = Boolean(root && directory?.path === root.path);
  const albumArtist = commonTrackValue(
    directoryTracks,
    (track) =>
      track.albumArtist ||
      track.albumArtists.join(", ") ||
      track.artists.join(", ") ||
      track.artist,
  );
  const albumYear = commonTrackValue(directoryTracks, (track) => track.year);
  const isAlbumFolder = Boolean(
    directory && !isLibraryRoot && directoryTracks.length > 0 && directory.subdirs.length === 0,
  );
  const identityKind = isLibraryRoot ? "library" : isAlbumFolder ? "album" : "folder";
  const identityTitle = isLibraryRoot
    ? root?.name ?? "Library"
    : directory
      ? albumNameOf(directory.path)
      : root?.name ?? "Library";
  const trackCountLabel = isLibraryRoot
    ? `${directoryTracks.length} tracks across this folder`
    : `${directoryTracks.length} track${directoryTracks.length === 1 ? "" : "s"}`;
  const breadcrumbItems = React.useMemo(() => {
    if (!root || !directory) return [];
    const relative = directory.path
      .slice(root.path.length)
      .replace(/^[/\\]+/, "");
    const separator = root.path.includes("\\") ? "\\" : "/";
    let parent = root.path.replace(/[/\\]+$/, "");
    return [
      { name: root.name, path: root.path },
      ...relative.split(/[/\\]+/).filter(Boolean).map((name) => {
        parent = parent ? `${parent}${separator}${name}` : `${separator}${name}`;
        return { name, path: parent };
      }),
    ];
  }, [directory, root]);

  const visibleTracks = React.useMemo(() => {
    const tracks = directory?.tracks ?? [];
    const filtered = filterText
      ? tracks.filter((track) => {
          const haystack = [
            fileNameOf(track.path),
            track.title ?? "",
            track.artist ?? "",
            track.album ?? "",
            track.genre ?? "",
          ]
            .join(" ")
            .toLowerCase();
          return haystack.includes(filterText.toLowerCase());
        })
      : tracks;

    const direction = sort.ascending ? 1 : -1;
    return [...filtered].sort((a, b) => {
      const value = (track: TrackData): string => {
        switch (sort.key) {
          case "fileName":
            return fileNameOf(track.path);
          case "title":
            return track.title ?? "";
          case "artists":
            return track.artists.join(", ") || track.artist || "";
          case "albums":
            return track.album ?? "";
          case "albumArtist":
            return track.albumArtist ?? track.albumArtists.join(", ");
          case "lyrics":
            return track.lyrics ? "1" : "0";
          case "lrc":
            return directory?.names.has(
              `${fileNameOf(track.path).replace(/\.[^.]+$/, "")}.lrc`.toLowerCase(),
            )
              ? "1"
              : "0";
          case "year":
            return track.year ?? "";
          case "genres":
            return track.genre ?? "";
          default:
            return "";
        }
      };
      return value(a).localeCompare(value(b), undefined, { numeric: true }) * direction;
    });
  }, [directory, filterText, sort]);
  const selectedTrackPosition = selectedTrack
    ? directoryTracks.findIndex((track) => track.path === selectedTrack.path)
    : -1;

  const applyFields = React.useCallback(
    async (fields: Record<string, unknown>) => {
      if (!selectedTrackPath || !directory) return;
      setSaving(true);
      setError(null);
      try {
        await api.writeTrack(selectedTrackPath, fields);
        await loadDirectory(directory.path, directory.path === root.path);
        setSelectedPaths([selectedTrackPath]);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setSaving(false);
      }
    },
    [api, directory, loadDirectory, selectedTrackPath],
  );

  if (loading && !directory) {
    return (
      <div className="sr-empty" data-testid="workspace-loading">
        <p className="sr-empty__body">Loading library…</p>
      </div>
    );
  }

  if (!root) {
    return (
      <div className="sr-empty" data-testid="workspace-no-root">
        <h3 className="sr-empty__title">No library mounted</h3>
        <p className="sr-empty__body">
          Mount a music directory into the service to browse and tag files.
        </p>
      </div>
    );
  }

  return (
    <div className="sr-workspace" data-testid="workspace">
      <aside className="sr-side">
        <details
          className="sr-folder-disclosure"
          data-testid="folder-disclosure"
          open={folderBrowserOpen}
          onToggle={(event) => setFolderBrowserOpen(event.currentTarget.open)}
        >
          <summary className="sr-folder-disclosure__summary">
            <span>Folders</span>
            <span className="sr-folder-disclosure__current">{root.name}</span>
          </summary>
          <div className="sr-folder-disclosure__content">
            {roots.length > 1 && (
              <label className="sr-field sr-field--stacked">
                <span className="sr-field__label">Library</span>
                <select
                  className="sr-input"
                  aria-label="Library"
                  value={rootIndex}
                  onChange={(event) => {
                    const index = Number(event.target.value);
                    setRootIndex(index);
                    const nextRoot = roots[index];
                    if (nextRoot) void loadDirectory(nextRoot.path, true);
                  }}
                >
                  {roots.map((entry, index) => (
                    <option key={entry.id} value={index}>
                      {entry.name}
                    </option>
                  ))}
                </select>
              </label>
            )}

            <DirectoryTree
              rootLabel={root.name}
              rootPath={root.path}
              activePath={directory?.path ?? null}
              reviewPaths={reviewPaths}
              expanded={expanded}
              childrenByPath={childrenByPath}
              onToggle={handleToggle}
              onNavigate={(path) => {
                if (path !== directory?.path || !childrenByPath[path]) {
                  void loadDirectory(path, path === root.path);
                }
              }}
            />
          </div>
        </details>
      </aside>

      <section className="sr-pane sr-pane--workspace">
        <div className="sr-toolbar">
          <nav aria-label="Current folder" className="sr-crumbs">
            {breadcrumbItems.map((item, index) => (
              <React.Fragment key={item.path}>
                {index > 0 && <span className="sr-crumbs__sep" aria-hidden="true">/</span>}
                <button
                  type="button"
                  className="sr-crumbs__item"
                  aria-current={index === breadcrumbItems.length - 1 ? "page" : undefined}
                  onClick={() => void loadDirectory(item.path, item.path === root.path)}
                >
                  {item.name}
                </button>
              </React.Fragment>
            ))}
          </nav>

          <div className="sr-header__spacer" />

          <label className="sr-sr-only" htmlFor="sr-filter">
            Filter tracks
          </label>
          <input
            id="sr-filter"
            className="sr-input"
            placeholder="Filter tracks in this folder"
            value={filterText}
            onChange={(event) => setFilterText(event.target.value)}
          />

          <button
            type="button"
            className="sr-btn"
            aria-label="Workspace settings"
            onClick={() => setSettingsOpen(true)}
          >
            Settings
          </button>

          <button
            type="button"
            className="sr-btn"
            onClick={() => {
              const path = directory?.path ?? root.path;
              void loadDirectory(path, path === root.path);
            }}
          >
            Refresh
          </button>
        </div>

        {error && (
          <p className="sr-banner sr-banner--error" role="alert">
            {error}
          </p>
        )}

        {reviewInLibrary.length > 0 && (
          <details className="sr-review-queue" data-testid="review-queue">
            <summary className="sr-review-queue__summary" data-testid="review-banner">
              <strong data-testid="review-queue-summary">
                {reviewInLibrary.length === 1
                  ? "1 album needs review"
                  : `${reviewInLibrary.length} albums need review`}
              </strong>
              <span>Background tagging left these tags for a person to verify.</span>
              <span className="sr-review-queue__hint">Review albums</span>
            </summary>
            <ul className="sr-review-queue__list">
              {reviewInLibrary.map((path) => (
                <li className="sr-review-queue__item" key={path}>
                  <span className="sr-review-queue__name">{albumNameOf(path)}</span>
                  <span className="sr-badge sr-badge--warning">Needs review</span>
                  <button
                    type="button"
                    className="sr-btn"
                    data-testid={`review-open-${path}`}
                    onClick={() => void loadDirectory(path)}
                  >
                    Open album
                  </button>
                  <button
                    type="button"
                    className="sr-btn"
                    disabled={acknowledgingPath === path}
                    data-testid={`review-acknowledge-${path}`}
                    onClick={() => void acknowledgeReview(path)}
                  >
                    {acknowledgingPath === path ? "Acknowledging…" : "Acknowledge review"}
                  </button>
                </li>
              ))}
            </ul>
          </details>
        )}

        <section
          className="sr-album-hero"
          data-testid="workspace-identity"
          data-kind={identityKind}
          aria-labelledby="workspace-identity-title"
        >
          {isAlbumFolder && directory?.coverDataUrl ? (
            <img className="sr-album-hero__art" src={directory.coverDataUrl} alt={`${identityTitle} cover`} />
          ) : (
            <div
              className={`sr-album-hero__art${isAlbumFolder ? " sr-album-hero__art--empty" : " sr-album-hero__art--folder"}`}
              role="img"
              aria-label={isAlbumFolder ? "No artwork" : "Library folder"}
            >
              {isAlbumFolder ? "No artwork" : "Library folder"}
            </div>
          )}
          <div className="sr-album-hero__details">
            <p className="sr-album-hero__eyebrow">
              {identityKind === "library" ? "Library folder" : identityKind === "album" ? "Album" : "Folder"}
            </p>
            <h1 id="workspace-identity-title" className="sr-album-hero__title">{identityTitle}</h1>
            {isAlbumFolder && albumArtist && <p className="sr-album-hero__artist">{albumArtist}</p>}
            <div className="sr-album-hero__facts">
              {isAlbumFolder && albumYear && <span>{albumYear}</span>}
              <span>{trackCountLabel}</span>
            </div>
            <div className="sr-album-hero__actions">
              <TrackTools
                albumPath={isLibraryRoot ? null : directory?.path ?? null}
                tracks={directoryTracks}
                selectedPaths={selectedPaths}
                api={api}
                onApplied={async () => {
                  if (directory) await loadDirectory(directory.path, directory.path === root.path);
                  await refreshReviewPaths();
                }}
              />
            </div>
          </div>
        </section>

        {directory && visibleTracks.length === 0 ? (
          <div className="sr-empty" data-testid="workspace-empty">
            <h3 className="sr-empty__title">This folder has no files</h3>
            <p className="sr-empty__body">
              Switch directory, clear the search term, or refresh the index.
            </p>
          </div>
        ) : (
          <TrackTable
            tracks={visibleTracks}
            selectedPaths={selectedPaths}
            directoryNames={directory?.names}
            coverDataUrl={directory?.coverDataUrl ?? null}
            sort={sort}
            onSortChange={setSort}
            onSelectTrack={(path) => {
              setSelectedPaths([path]);
              setEditorOpen(true);
            }}
            onSelectionChange={(paths) => {
              setSelectedPaths(paths);
              if (paths.length > 0) setEditorOpen(true);
            }}
            visibleColumns={visibleColumns}
          />
        )}

        {detailsVisible && editorOpen && selectedTrack && (
          <section className="sr-editor-dock" data-testid="editor-dock" aria-label="Track editor">
            <TrackDetailEditor
              track={selectedTrack}
              coverDataUrl={directory?.coverDataUrl ?? null}
              hasLrc={
                directory?.names.has(
                  `${fileNameOf(selectedTrack.path).replace(/\.[^.]+$/, "")}.lrc`.toLowerCase(),
                ) ?? false
              }
              saving={saving}
              compact
              trackPosition={
                isAlbumFolder
                  ? selectedTrackPosition >= 0
                    ? selectedTrackPosition + 1
                    : undefined
                  : selectedTrack.trackNumber ?? undefined
              }
              albumTrackCount={isAlbumFolder ? directoryTracks.length : undefined}
              onApply={applyFields}
              onClose={() => setEditorOpen(false)}
            />
          </section>
        )}
      </section>

      {settingsOpen && (
        <WorkspaceSettingsDrawer
          detailsVisible={detailsVisible}
          visibleColumns={visibleColumns}
          onApply={(nextDetails, nextColumns) => {
            setDetailsVisible(nextDetails);
            setEditorOpen(nextDetails);
            setVisibleColumns([...nextColumns]);
            try {
              window.localStorage.setItem(DETAILS_KEY, String(nextDetails));
              window.localStorage.setItem(COLUMNS_KEY, JSON.stringify(nextColumns));
            } catch {
              /* preferences are a convenience; the session still applies them */
            }
            setSettingsOpen(false);
          }}
          onClose={() => setSettingsOpen(false)}
        />
      )}
    </div>
  );
}
