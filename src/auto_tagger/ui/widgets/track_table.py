"""Track table widget — album browser + per-album track list."""

from __future__ import annotations

from pathlib import Path

from textual import events
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widget import Widget
from textual.widgets import Button, DataTable, Label, Static

from auto_tagger.core.metadata import format_position

# ── Constants ──────────────────────────────────────────────────────────────────

STATUS_ICONS: dict[str, str] = {
    "pending": "◌",
    "ok": "✅",
    "warning": "⚠️",
    "error": "❌",
}

ALBUM_COLUMNS: list[dict] = [
    {"key": "artist", "label": "Artist", "width": 20, "always": True},
    {"key": "album", "label": "Album", "width": 30, "always": True},
    {"key": "year", "label": "Year", "width": 6, "always": True},
    {"key": "tracks", "label": "Tracks", "width": 7, "always": True},
    {"key": "status", "label": "Status", "width": 8, "always": True},
]

TRACK_COLUMNS: list[dict] = [
    {"key": "track", "label": "#", "width": 4, "always": True},
    {"key": "filename", "label": "Path", "width": 40, "always": True},
    {"key": "title", "label": "Title", "width": 24, "always": True},
    {"key": "artist", "label": "Artist", "width": 20, "always": True},
    {"key": "album_artist", "label": "Album Artist", "width": 20, "always": True},
    {"key": "album", "label": "Album", "width": 24, "always": True},
    {"key": "year", "label": "Year", "width": 6, "always": True},
    {"key": "genre", "label": "Genre", "width": 14, "always": False},
    {"key": "bitrate", "label": "Bitrate", "width": 7, "always": False},
    {"key": "cover", "label": "Cover", "width": 6, "always": False},
    {"key": "status", "label": "Status", "width": 8, "always": True},
]

OPTIONAL_TRACK_KEYS: set[str] = {c["key"] for c in TRACK_COLUMNS if not c["always"]}


# ── Context Menu for Album Rows ──────────────────────────────────────────────


class AlbumContextMenu(ModalScreen[tuple[str, Path] | None]):
    """Right-click context menu for an album in the album browser."""

    DEFAULT_CSS = """
    AlbumContextMenu {
        align: center middle;
        background: $surface 80%;
    }

    #context-dialog {
        width: 36;
        height: auto;
        border: thick $primary;
        background: $surface;
    }

    #context-title {
        text-style: bold;
        padding: 1 2;
        background: $panel;
        text-align: center;
        height: 3;
    }

    #context-body {
        padding: 1 2;
        height: auto;
    }

    #context-body Button {
        width: 100%;
        margin-bottom: 1;
    }
    """

    def __init__(self, album_path: Path, artist: str, album: str) -> None:
        super().__init__()
        self.album_path = album_path
        self._header = f"{artist} — {album}"

    def compose(self) -> ComposeResult:
        with Vertical(id="context-dialog"):
            yield Static(self._header, id="context-title")
            with Vertical(id="context-body"):
                yield Button("▶ Auto-Tag This Album", id="auto-tag-album-btn", variant="primary")
                yield Button("✕ Cancel", id="cancel-btn", variant="default")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "auto-tag-album-btn":
            self.dismiss(("auto_tag", self.album_path))
        else:
            self.dismiss(None)


# ── Right-click-aware DataTable ──────────────────────────────────────────────


class _AlbumDataTable(DataTable):
    """DataTable that calls ``_on_album_right_clicked`` on its parent
    ``TrackTable`` when a row is right-clicked.

    Uses ``event.style.meta`` (the same mechanism DataTable's own
    ``_on_click`` uses) to resolve the clicked row.
    """

    def _on_mouse_down(self, event: events.MouseDown) -> None:
        """Intercept right-clicks to trigger a context menu on the parent."""
        if event.button == 3:
            meta = event.style.meta
            row_index = meta.get("row") if meta else None
            if row_index is not None and 0 <= row_index < self.row_count:
                ordered = list(self.ordered_rows)
                if row_index < len(ordered):
                    row_key = ordered[row_index].key
                    if row_key is not None:
                        album_path = Path(row_key.value)
                        parent = self.parent
                        if (parent is not None
                                and hasattr(parent, "_on_album_right_clicked")):
                            parent._on_album_right_clicked(album_path)
            event.stop()
            return
        super()._on_mouse_down(event)


# ── TrackTable Widget ──────────────────────────────────────────────────────────


class TrackTable(Widget):
    """Album browser + per-album track list.

    Two views, toggled by selecting/deselecting an album:
    - **Album browser**: one row per album with aggregate stats
    - **Track view**: one row per track, sorted and filterable
    """

    BINDINGS: list[Binding] = [
        Binding("ctrl+a", "select_all", "Select All"),
    ]

    DEFAULT_CSS = """
    TrackTable {
        width: 100%;
        height: 100%;
        layout: vertical;
    }

    #breadcrumb-bar {
        height: 1;
        width: 100%;
        display: none;
        background: $accent 15%;
        color: $foreground 70%;
        padding: 0 1;
    }

    #breadcrumb-bar.visible {
        display: block;
    }

    TrackTable DataTable {
        width: 100%;
        height: 100%;
    }

    #album-table {
        display: block;
    }

    #track-table-content {
        display: none;
    }
    """

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._tracks_data: list[dict] = []
        self._sort_col: str | None = None
        self._sort_reverse: bool = False
        self._visible_track_cols: set[str] = {
            "track", "filename", "title", "artist",
            "album_artist", "album", "year", "genre",
            "cover", "status",
        }
        self._ctrl_pressed: bool = False

    # ── Compose ──────────────────────────────────────────────────────────────

    def compose(self) -> None:
        yield Label(id="breadcrumb-bar")
        yield _AlbumDataTable(id="album-table", show_cursor=True, zebra_stripes=True)
        yield DataTable(id="track-table-content", show_cursor=True, zebra_stripes=True)

    def on_mount(self) -> None:
        self._init_table(self.track_table("album"), ALBUM_COLUMNS)
        self._rebuild_track_columns()
        # Set cursor type to "row" so Enter posts RowSelected (not CellSelected)
        self.track_table("album").cursor_type = "row"
        self.track_table("track").cursor_type = "row"
        self._show_browser_view()

    def track_table(self, which: str) -> DataTable:
        """Get album or track DataTable by short name ('album' | 'track')."""
        if which == "album":
            return self.query_one("#album-table", DataTable)
        return self.query_one("#track-table-content", DataTable)

    # ── Column management ────────────────────────────────────────────────────

    @staticmethod
    def _init_table(table: DataTable, columns: list[dict]) -> None:
        table.clear(columns=True)
        for col in columns:
            table.add_column(col["label"], key=col["key"], width=col.get("width"))

    def _rebuild_track_columns(self) -> None:
        table = self.track_table("track")
        table.clear(columns=True)
        for col in TRACK_COLUMNS:
            if col["key"] in self._visible_track_cols:
                table.add_column(col["label"], key=col["key"], width=col.get("width"))

    def toggle_track_column(self, column_key: str) -> None:
        if column_key in OPTIONAL_TRACK_KEYS:
            if column_key in self._visible_track_cols:
                self._visible_track_cols.discard(column_key)
            else:
                self._visible_track_cols.add(column_key)
            self._rebuild_track_columns()
            if not self.app.state.show_album_browser:
                self._populate_tracks()

    # ── View switching ──────────────────────────────────────────────────────

    def _show_browser_view(self) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        app_state.show_album_browser = True

        self.track_table("album").display = True
        self.track_table("track").display = False
        self.query_one("#breadcrumb-bar", Label).display = False
        self.track_table("album").focus()

    def _show_track_view(self) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        app_state.show_album_browser = False

        self.track_table("album").display = False
        self.track_table("track").display = True
        self.query_one("#breadcrumb-bar", Label).display = True
        self.track_table("track").focus()

    # ── Album browser data ──────────────────────────────────────────────────

    async def refresh_albums(self) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        table = self.track_table("album")
        table.clear()

        for album_path, album in sorted(
            app_state.albums.items(),
            key=lambda x: (x[1].artist_hint.lower(), x[1].album_hint.lower()),
        ):
            table.add_row(*self._make_album_row(album), key=str(album_path))

        if app_state.show_album_browser:
            self._show_browser_view()

    @staticmethod
    def _make_album_row(album) -> list[str]:
        return [
            album.artist_hint or "?",
            album.album_hint or "?",
            album.tracks[0].metadata.year if album.tracks and album.tracks_loaded else "",
            str(album.track_count),
            STATUS_ICONS.get(album.status, "◌"),
        ]

    @staticmethod
    def _format_track_number(meta) -> str:
        """Format track number with disc prefix when the track belongs to a
        multi-disc release (e.g. ``"1-01"`` for disc 1, track 1).
        Single-disc releases show the plain track number.
        """
        track_num = meta.track_number
        if track_num is None:
            return ""
        track_str = f"{track_num:02d}" if meta.track_total and meta.track_total > 9 else str(track_num)
        if meta.disc_number is not None:
            return f"{meta.disc_number}-{track_str}"
        if meta.track_total:
            return f"{track_num}/{meta.track_total}"
        return str(track_num)

    # ── Track view data ─────────────────────────────────────────────────────

    def select_album(self, album_path: Path) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        app_state.selected_album_path = album_path
        app_state.selected_track_paths.clear()
        app_state.show_album_browser = False

        album = app_state.albums.get(album_path)
        if album and not album.tracks_loaded:
            album.ensure_tracks_loaded()

        albums = app_state.albums
        artist = album.artist_hint if album else album_path.parent.name
        title = album.album_hint if album else album_path.name
        album_count = len(albums) if albums else 0
        bc = self.query_one("#breadcrumb-bar", Label)
        bc.update(
            f"🏠 Albums ({album_count}) › {artist} › {title}   "
            f"[Click a column header to sort | Esc to go back]"
        )

        self._populate_tracks()
        self._show_track_view()

    def _populate_tracks(self) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        album = app_state.selected_album
        if not album:
            return

        table = self.track_table("track")
        table.clear()
        self._tracks_data.clear()

        for track in album.tracks:
            row = self._make_track_row(track, album.artist_hint)
            key = str(track.path)
            self._tracks_data.append({"key": key, "row_data": row})
            table.add_row(*row, key=key)

        if self._sort_col:
            self._do_sort(self._sort_col, self._sort_reverse)

    def _relative_track_path(self, abs_path: Path) -> str:
        """Return the path relative to the library root, or just the file name."""
        try:
            app_state = self.app.state  # type: ignore[attr-defined]
            lib = app_state.library_path
            if lib:
                rel = abs_path.relative_to(lib)
                return str(rel)
        except (ValueError, AttributeError):
            pass
        return abs_path.name

    def _make_track_row(self, track, album_artist_hint: str = "") -> list[str]:
        meta = track.metadata
        track_str = self._format_track_number(meta)
        cover_status = "✅" if track.has_cover else "❌"

        # Use the album-level artist_hint (from directory structure) as the
        # authoritative "Album Artist" — the per-track tag may be missing or
        # inconsistent, while the directory structure is the reliable source
        # for which artist's album this belongs to.
        album_artist: str = album_artist_hint or meta.album_artist or meta.artist or ""

        row_map: dict[str, str] = {
            "track": track_str,
            "filename": self._relative_track_path(track.path),
            "title": meta.title or "",
            "artist": meta.artist or "",
            "album_artist": album_artist,
            "album": meta.album or "",
            "year": meta.year or "",
            "genre": meta.genre or "",
            "bitrate": f"{track.bitrate}k" if track.bitrate else "",
            "cover": cover_status,
            "status": STATUS_ICONS.get(track.status, "◌"),
        }

        return [row_map.get(c["key"], "") for c in TRACK_COLUMNS
                if c["key"] in self._visible_track_cols]

    # ── Sorting ─────────────────────────────────────────────────────────────

    def _do_sort(self, column_key: str, reverse: bool = False) -> None:
        self._sort_col = column_key
        self._sort_reverse = reverse
        self.track_table("track").sort(column_key, reverse=reverse)

    def sort_albums(self, column_key: str) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        table = self.track_table("album")

        def sort_key(item):
            _path, album = item
            if column_key == "artist":
                return album.artist_hint.lower()
            elif column_key == "album":
                return album.album_hint.lower()
            elif column_key == "year":
                if album.tracks and album.tracks_loaded:
                    return album.tracks[0].metadata.year or ""
                return ""
            elif column_key == "tracks":
                return album.track_count
            elif column_key == "status":
                return album.status
            return album.artist_hint.lower()

        sorted_items = sorted(app_state.albums.items(), key=sort_key)
        if self._sort_col == column_key:
            self._sort_reverse = not self._sort_reverse
            sorted_items = list(reversed(sorted_items))
        else:
            self._sort_col = column_key
            self._sort_reverse = False

        table.clear()
        for path, album in sorted_items:
            table.add_row(*self._make_album_row(album), key=str(path))

    # ── Event handlers ──────────────────────────────────────────────────────

    def on_data_table_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        """Cursor moves over a row — populate tag panel."""
        if event.data_table.id == "track-table-content":
            track_path = Path(event.row_key.value)
            self._select_track(track_path)

    def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        """Enter/click on a row — navigate or multi-select."""
        if event.data_table.id == "album-table":
            album_path = Path(event.row_key.value)
            self.select_album(album_path)
        elif event.data_table.id == "track-table-content":
            if self._ctrl_pressed:
                app_state = self.app.state  # type: ignore[attr-defined]
                track_path = Path(event.row_key.value)
                if track_path in app_state.selected_track_paths:
                    app_state.selected_track_paths.discard(track_path)
                else:
                    app_state.selected_track_paths.add(track_path)
                self._update_status_bar_selection()

    def on_data_table_header_selected(self, event: DataTable.HeaderSelected) -> None:
        """Click column header — sort by that column."""
        if event.data_table.id == "album-table":
            self.sort_albums(event.column_key)
        elif event.data_table.id == "track-table-content":
            reverse = (self._sort_col == event.column_key and not self._sort_reverse)
            self._do_sort(event.column_key, reverse)

    def _on_album_right_clicked(self, album_path: Path) -> None:
        """A right-click was detected on an album row — show context menu."""
        app_state = self.app.state  # type: ignore[attr-defined]
        album = app_state.albums.get(album_path)
        if not album:
            return

        # Ensure tracks are loaded so we can show artist/album info
        if not album.tracks_loaded:
            album.ensure_tracks_loaded()

        def on_menu_result(result: tuple[str, Path] | None) -> None:
            if result is None:
                return
            action, path = result
            if action == "auto_tag" and path in app_state.albums:
                screen = self.screen
                if hasattr(screen, "start_auto_tag_album"):
                    screen.start_auto_tag_album(path)

        self.app.push_screen(
            AlbumContextMenu(
                album_path=album_path,
                artist=album.artist_hint or "",
                album=album.album_hint or "",
            ),
            on_menu_result,
        )

    # ── Track selection ────────────────────────────────────────────────────

    def action_select_current(self) -> None:
        """Trigger RowSelected on the currently focused table."""
        state = self.app.state  # type: ignore[attr-defined]
        if state.show_album_browser:
            table = self.track_table("album")
        else:
            table = self.track_table("track")
        cursor_row = table.cursor_row
        if cursor_row is not None and cursor_row < table.row_count:
            ordered = list(table.ordered_rows)
            if cursor_row < len(ordered):
                row_key = ordered[cursor_row].key
                if row_key is not None:
                    table.action_select_cursor()

    def action_select_all(self) -> None:
        """Select all tracks in the current track view (Ctrl+A)."""
        app_state = self.app.state  # type: ignore[attr-defined]
        if app_state.show_album_browser:
            return
        album = app_state.selected_album
        if not album:
            return
        if not album.tracks_loaded:
            album.ensure_tracks_loaded()
        app_state.selected_track_paths = {t.path for t in album.tracks}
        self._update_status_bar_selection()
        # Populate tag panel with the first selected track
        tag_panel = self.screen.query_one("#tag-panel")
        if hasattr(tag_panel, "populate"):
            first_track = next(
                (t for t in album.tracks if t.path in app_state.selected_track_paths),
                None,
            )
            if first_track:
                tag_panel.populate(first_track.path)

    def _select_track(self, track_path: Path) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]

        if self._ctrl_pressed:
            # Ctrl+click: toggle multi-selection
            if track_path in app_state.selected_track_paths:
                app_state.selected_track_paths.discard(track_path)
            else:
                app_state.selected_track_paths.add(track_path)
        else:
            # Normal click: single selection
            app_state.selected_track_paths = {track_path}

        tag_panel = self.screen.query_one("#tag-panel")
        if hasattr(tag_panel, "populate"):
            tag_panel.populate(track_path)

        self._update_status_bar_selection()

    def _update_status_bar_selection(self) -> None:
        status_bar = self.screen.query_one("#status-bar")
        if hasattr(status_bar, "refresh_state"):
            status_bar.refresh_state()

    # ── Filtering ──────────────────────────────────────────────────────────

    def apply_filter(self, filter_text: str) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        if app_state.show_album_browser:
            return

        album = app_state.selected_album
        if not album:
            return

        table = self.track_table("track")
        table.clear()

        for td in self._tracks_data:
            if not filter_text:
                table.add_row(*td["row_data"], key=td["key"])
            else:
                match = any(
                    filter_text.lower() in str(cell).lower()
                    for cell in td["row_data"]
                )
                if match:
                    table.add_row(*td["row_data"], key=td["key"])

    # ── Status updates ─────────────────────────────────────────────────────

    def update_track_status(self, track_path: Path, status: str) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        album = app_state.selected_album
        if not album:
            return

        for track in album.tracks:
            if track.path == track_path:
                track.status = status
                break

        self._populate_tracks()

    def refresh_track_row(self, track_path: Path) -> None:
        app_state = self.app.state  # type: ignore[attr-defined]
        album = app_state.selected_album
        if not album:
            return

        for track in album.tracks:
            if track.path == track_path:
                break
        else:
            return

        row = self._make_track_row(track, album.artist_hint)
        key = str(track_path)
        table = self.track_table("track")

        try:
            table.remove_row(key)
        except Exception:
            pass

        table.add_row(*row, key=key)
        for td in self._tracks_data:
            if td["key"] == key:
                td["row_data"] = row
                break
