/**
 * Browser tagging workspace.
 *
 * Mirrors the reference layout: a left column holding the folder tree and the
 * track detail editor, and a main pane holding the directory toolbar and the
 * ten-column file table. All tag writes go through the shared Rust writer.
 */

import React from "react";
import type { DirEntry, DirectoryData, LibraryRoot, TrackData } from "../shared/desktop-api";
import {
  listMonitoringReleases,
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

export interface WorkspaceViewProps {
  /** Injected for tests; defaults to the installed `window.api` facade. */
  api?: Window["api"];
  /** Albums background tagging classified; injected for tests. */
  loadReleases?: () => Promise<{ releases: MonitoredRelease[] }>;
  /**
   * An album to open on arrival, set when Monitoring sends the reader to the
   * album it just reported a problem with.
   */
  openAlbumPath?: string | null;
  /** Called once `openAlbumPath` has been loaded, so the caller can clear it. */
  onAlbumOpened?: () => void;
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
  loadReleases = listMonitoringReleases,
  openAlbumPath = null,
  onAlbumOpened,
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
  const [detailsVisible, setDetailsVisible] = React.useState(loadDetailsVisible);
  const [visibleColumns, setVisibleColumns] =
    React.useState<WebTableColumnKey[]>(loadVisibleColumns);
  /** Albums the service's own classification left needing a person. */
  const [reviewPaths, setReviewPaths] = React.useState<ReadonlySet<string>>(new Set());

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
    try {
      const { releases } = await loadReleases();
      setReviewPaths(new Set(needsReviewPaths(releases)));
    } catch {
      // A workspace that cannot read review state still lists files; the
      // banner simply stays empty rather than blocking the file browser.
      setReviewPaths(new Set());
    }
  }, [loadReleases]);

  React.useEffect(() => {
    void refreshReviewPaths();
  }, [refreshReviewPaths]);

  const loadDirectory = React.useCallback(
    async (path: string, includeChildTracks = false) => {
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
        setSelectedPaths([]);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
        setDirectory(null);
      } finally {
        setLoading(false);
      }
    },
    [api],
  );

  React.useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const list = await api.listLibraryRoots();
        if (!active) return;
        setRoots(list);
        if (list.length > 0) await loadDirectory(list[0].path, true);
        else setLoading(false);
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
    if (!openAlbumPath || roots.length === 0) return;
    if (drillInRef.current === openAlbumPath) return;
    drillInRef.current = openAlbumPath;
    const match = roots.findIndex((entry) => {
      const prefix = entry.path.endsWith("/") ? entry.path : `${entry.path}/`;
      return openAlbumPath === entry.path || openAlbumPath.startsWith(prefix);
    });
    if (match > 0) setRootIndex(match);
    void loadDirectory(openAlbumPath).then(() => onAlbumOpened?.());
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

        {detailsVisible && (
          <TrackDetailEditor
            track={selectedTrack}
            coverDataUrl={directory?.coverDataUrl ?? null}
            hasLrc={
              selectedTrack
                ? directory?.names.has(
                    `${fileNameOf(selectedTrack.path).replace(/\.[^.]+$/, "")}.lrc`.toLowerCase(),
                  ) ?? false
                : false
            }
            saving={saving}
            onApply={applyFields}
          />
        )}
      </aside>

      <section className="sr-pane">
        <div className="sr-toolbar">
          <nav aria-label="Current folder" className="sr-crumbs">
            <button
              type="button"
              className="sr-crumbs__item"
              aria-current={directory?.path === root.path ? "page" : undefined}
              onClick={() => void loadDirectory(root.path, true)}
            >
              {root.name}
            </button>
          </nav>

          <div className="sr-header__spacer" />

          <label className="sr-sr-only" htmlFor="sr-filter">
            Filter files
          </label>
          <input
            id="sr-filter"
            className="sr-input"
            style={{ width: 240 }}
            placeholder="Search files or folders"
            value={filterText}
            onChange={(event) => setFilterText(event.target.value)}
          />

          <TrackTools
            albumPath={directory?.path ?? null}
            tracks={directory?.tracks ?? []}
            selectedPaths={selectedPaths}
            api={api}
            onApplied={async () => {
              if (directory) await loadDirectory(directory.path, directory.path === root.path);
              await refreshReviewPaths();
            }}
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
          <div className="sr-banner sr-banner--notice sr-banner--review" data-testid="review-banner">
            <strong>
              {reviewInLibrary.length === 1
                ? "1 album needs review"
                : `${reviewInLibrary.length} albums need review`}
            </strong>{" "}
            — background tagging left these without an authoritative match. Open one,
            then use Search, Number, or Convert.{" "}
            {reviewInLibrary.map((path) => (
              <button
                key={path}
                type="button"
                className="sr-btn"
                data-testid={`review-open-${path}`}
                onClick={() => void loadDirectory(path)}
              >
                {albumNameOf(path)}
              </button>
            ))}
          </div>
        )}

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
            onSelectTrack={(path) => setSelectedPaths([path])}
            onSelectionChange={setSelectedPaths}
            visibleColumns={visibleColumns}
          />
        )}
      </section>

      {settingsOpen && (
        <WorkspaceSettingsDrawer
          detailsVisible={detailsVisible}
          visibleColumns={visibleColumns}
          onApply={(nextDetails, nextColumns) => {
            setDetailsVisible(nextDetails);
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
