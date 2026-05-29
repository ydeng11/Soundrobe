import React from "react";
import type { AlbumInfo } from "../../electron/preload";

interface SidebarProps {
  albums: AlbumInfo[];
  libraryPath: string | null;
  activeAlbumPath: string | null;
  onSelectAlbum: (path: string | null) => void;
  onOpenLibrary: () => void;
}

export function Sidebar({
  albums,
  libraryPath,
  activeAlbumPath,
  onSelectAlbum,
  onOpenLibrary,
}: SidebarProps) {
  return (
    <div className="w-[220px] min-w-[180px] h-full flex flex-col bg-sidebar-DEFAULT backdrop-blur-xl border-r border-sidebar-border select-none overflow-hidden">
      {/* App header */}
      <div className="px-4 pt-3 pb-2 flex items-center gap-2.5">
        <div className="w-6 h-6 rounded-md bg-accent flex items-center justify-center shadow-sm">
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="white"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M9 18V5l12-2v13" />
            <circle cx="6" cy="18" r="3" />
            <circle cx="18" cy="16" r="3" />
          </svg>
        </div>
        <span className="text-[13px] font-semibold text-text-primary tracking-tight">
          AudioTag Pro
        </span>
      </div>

      {/* Divider */}
      <div className="mx-4 my-2 h-px bg-sidebar-border" />

      {/* Section header */}
      <div className="px-4 py-1">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-text-muted">
          Albums
        </span>
      </div>

      {/* Albums List */}
      <div className="flex-1 overflow-y-auto px-2 pb-2">
        {!libraryPath ? (
          <div className="px-3 py-6 text-center">
            <div className="text-text-muted text-[11px] leading-relaxed">
              Open a music library
              <br />
              to browse albums
            </div>
            <button
              onClick={onOpenLibrary}
              className="mt-3 px-3 py-1.5 text-[11px] font-medium rounded-lg bg-accent text-white hover:bg-accent/90 transition-colors shadow-sm"
            >
              Open Library…
            </button>
          </div>
        ) : albums.length === 0 ? (
          <div className="px-3 py-6 text-center text-text-muted text-[11px]">
            No albums found
          </div>
        ) : (
          <div className="space-y-0.5">
            {/* "All Files" item */}
            <AlbumRow
              name="All Files"
              count={0}
              active={activeAlbumPath === null}
              onClick={() => onSelectAlbum(null)}
            />
            {albums.map((album) => (
              <AlbumRow
                key={album.path}
                name={album.name}
                artist={album.artistHint || album.albumHint}
                count={album.trackCount}
                active={activeAlbumPath === album.path}
                onClick={() => onSelectAlbum(album.path)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Bottom metadata */}
      {libraryPath && (
        <div className="px-4 py-2 border-t border-sidebar-border">
          <div className="flex items-center gap-1.5 text-[10px] text-text-muted truncate">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            <span className="truncate">{libraryPath}</span>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Sub-components ──────────────────────────────────────────────

function AlbumRow({
  name,
  artist,
  count,
  active,
  onClick,
}: {
  name: string;
  artist?: string;
  count?: number;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[12px] transition-all duration-100 text-left ${
        active
          ? "bg-sidebar-active text-accent font-medium"
          : "text-text-secondary hover:bg-sidebar-hover hover:text-text-primary"
      }`}
    >
      <span className="w-4 h-4 shrink-0 flex items-center justify-center">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
          <path d="M9 18V5l12-2v13" />
          <circle cx="6" cy="18" r="3" />
          <circle cx="18" cy="16" r="3" />
        </svg>
      </span>
      <div className="flex-1 min-w-0">
        <div className="truncate">{name}</div>
        {artist && (
          <div className="text-[10px] text-text-muted truncate">{artist}</div>
        )}
      </div>
      {count != null && count > 0 && (
        <span className="text-[10px] text-text-muted tabular-nums">{count}</span>
      )}
    </button>
  );
}
