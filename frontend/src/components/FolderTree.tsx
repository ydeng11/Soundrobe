import React, { useState, useCallback, useEffect } from "react";
import type { DirEntry } from "../shared/desktop-api";
import { basename } from "../utils/path";

interface FolderTreeProps {
  libraryPath: string | null;
  onSelectAlbum: (path: string | null) => void;
  onOpenLibrary: () => void;
  expanded: boolean;
  onClose: () => void;
}

export function FolderTree({
  libraryPath,
  onSelectAlbum,
  onOpenLibrary,
  expanded,
  onClose,
}: FolderTreeProps) {
  const [treeData, setTreeData] = useState<Record<string, DirEntry[]>>({});
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());
  const [loadingDirs, setLoadingDirs] = useState<Set<string>>(new Set());

  // Load root directory when library opens
  useEffect(() => {
    if (libraryPath && expanded) {
      loadDirectory(libraryPath);
    }
  }, [libraryPath, expanded]);

  const loadDirectory = useCallback(async (dirPath: string) => {
    if (treeData[dirPath] || loadingDirs.has(dirPath)) return;

    setLoadingDirs((prev) => new Set(prev).add(dirPath));
    try {
      const entries = await window.api.listDirectory(dirPath);
      setTreeData((prev) => ({ ...prev, [dirPath]: entries }));
    } catch {
      // ignore
    } finally {
      setLoadingDirs((prev) => {
        const next = new Set(prev);
        next.delete(dirPath);
        return next;
      });
    }
  }, [treeData, loadingDirs]);

  const toggleDir = useCallback(
    async (dirPath: string) => {
      if (expandedDirs.has(dirPath)) {
        setExpandedDirs((prev) => {
          const next = new Set(prev);
          next.delete(dirPath);
          return next;
        });
      } else {
        setExpandedDirs((prev) => new Set(prev).add(dirPath));
        await loadDirectory(dirPath);
      }
    },
    [expandedDirs, loadDirectory]
  );

  if (!expanded) return null;

  const renderDir = (dir: DirEntry, depth: number) => {
    const isExpanded = expandedDirs.has(dir.path);
    const children = treeData[dir.path] || [];
    const hasAudio = children.some((c) => !c.isDirectory);
    const hasSubdirs = children.some((c) => c.isDirectory);

    return (
      <div key={dir.path}>
        <button
          onClick={() => toggleDir(dir.path)}
          onDoubleClick={() => onSelectAlbum(dir.path)}
          className={`w-full flex items-center gap-1.5 px-2 py-1 rounded-md text-[12px] text-left transition-colors hover:bg-sidebar-hover ${
            hasAudio ? "text-text-primary font-medium" : "text-text-secondary"
          }`}
          style={{ paddingLeft: `${8 + depth * 14}px` }}
        >
          {/* Expand/collapse arrow */}
          <span className="w-3.5 h-3.5 flex items-center justify-center shrink-0">
            {loadingDirs.has(dir.path) ? (
              <svg
                width="10"
                height="10"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                className="animate-spin"
              >
                <line x1="12" y1="2" x2="12" y2="6" />
                <line x1="12" y1="18" x2="12" y2="22" />
              </svg>
            ) : hasSubdirs ? (
              <svg
                width="10"
                height="10"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                className={`transition-transform ${isExpanded ? "rotate-90" : ""}`}
              >
                <polyline points="9 18 15 12 9 6" />
              </svg>
            ) : (
              <span className="w-[10px]" />
            )}
          </span>

          {/* Folder icon */}
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            className="shrink-0"
          >
            {hasAudio ? (
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            ) : (
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            )}
          </svg>

          <span className="truncate">{dir.name}</span>

          {hasAudio && (
            <span className="ml-auto text-[9px] text-text-muted">
              {children.filter((c) => !c.isDirectory).length}
            </span>
          )}
        </button>

        {/* Children */}
        {isExpanded &&
          children
            .filter((c) => c.isDirectory)
            .map((child) => renderDir(child, depth + 1))}
      </div>
    );
  };

  const rootEntries = libraryPath && treeData[libraryPath]
    ? treeData[libraryPath]
    : [];

  return (
    <div className="px-2 pb-2" data-testid="folder-tree">
      {/* Header */}
      <div className="flex items-center gap-1.5 px-2 py-1.5">
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          className="text-text-muted"
        >
          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
        </svg>
        <span className="text-[11px] font-semibold uppercase tracking-wider text-text-muted flex-1">
          Folders
        </span>
        <button
          onClick={onClose}
          className="inline-flex items-center justify-center w-5 h-5 rounded text-text-muted hover:text-text-primary hover:bg-surface-hover transition-colors"
          title="Close folder tree"
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      {/* Root library item */}
      {libraryPath && (
        <button
          onClick={() => toggleDir(libraryPath)}
          className="w-full flex items-center gap-1.5 px-2 py-1 rounded-md text-[12px] text-left font-medium text-text-primary hover:bg-sidebar-hover transition-colors"
          style={{ paddingLeft: "8px" }}
        >
          <span className="w-3.5 h-3.5 flex items-center justify-center shrink-0">
            {loadingDirs.has(libraryPath) ? (
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="animate-spin">
                <line x1="12" y1="2" x2="12" y2="6" />
                <line x1="12" y1="18" x2="12" y2="22" />
              </svg>
            ) : (
              <svg
                width="10"
                height="10"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                className={`transition-transform ${expandedDirs.has(libraryPath) ? "rotate-90" : ""}`}
              >
                <polyline points="9 18 15 12 9 6" />
              </svg>
            )}
          </span>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" className="shrink-0">
            <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
          </svg>
          <span className="truncate">{basename(libraryPath) || libraryPath}</span>
        </button>
      )}

      {/* Directory tree */}
      <div className="mt-0.5">
        {!libraryPath ? (
          <div className="px-3 py-6 text-center">
            <div className="text-text-muted text-[11px] leading-relaxed">
              Open a library to browse folders
            </div>
            <button
              onClick={onOpenLibrary}
              className="mt-3 px-3 py-1.5 text-[11px] font-medium rounded-lg bg-accent text-white hover:bg-accent/90 transition-colors shadow-sm"
            >
              Open Library…
            </button>
          </div>
        ) : rootEntries.length === 0 && !loadingDirs.has(libraryPath) ? (
          <div className="px-3 py-3 text-center text-text-muted text-[11px]">
            No folders found
          </div>
        ) : (
          rootEntries
            .filter((e) => e.isDirectory)
            .map((dir) => renderDir(dir, 1))
        )}
      </div>
    </div>
  );
}
