"""Main editor screen with tag panel, track table, toolbar, and status bar."""

from __future__ import annotations

import asyncio
from pathlib import Path

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal
from textual.screen import ModalScreen, Screen
from textual.widgets import DirectoryTree, Label

from auto_tagger.ui.widgets.status_bar import StatusBar
from auto_tagger.ui.widgets.tag_panel import TagPanel
from auto_tagger.ui.widgets.toolbar import Toolbar
from auto_tagger.ui.widgets.track_table import TrackTable


class MainScreen(Screen):
    """Main screen with the full tag editor layout.

    Layout structure::

        ┌──────────────────────────────────────────────────────┐
        │  Toolbar (dock: top)                                  │
        ├─────────────────────┬────────────────────────────────┤
        │  Tag Panel          │  Track Table                   │
        │  (fixed 38 cols)    │  (remaining width)             │
        │                     │                                │
        │  ┌─── Cover ────┐   │  DataTable                     │
        │  │  preview     │   │                                │
        │  └──────────────┘   │                                │
        ├─────────────────────┴────────────────────────────────┤
        │  Status Bar + Filter (dock: bottom)                  │
        └──────────────────────────────────────────────────────┘
    """

    BINDINGS: list[Binding] = [
        Binding("ctrl+s", "save", "Save"),
    ]

    CSS = """
    MainScreen {
        /* Toolbar docks top, StatusBar docks bottom.
           The middle area is a Horizontal container that fills 1fr. */
        layout: vertical;
    }

    #content-area {
        width: 100%;
        height: 1fr;
        layout: horizontal;
    }

    #tag-panel {
        width: 38;
        min-width: 30;
        max-width: 50;
        height: 100%;
        border-right: solid $border;
    }

    #track-table {
        width: 1fr;
        height: 100%;
    }

    /* Ensure tables fill their container */
    #track-table DataTable {
        width: 100%;
        height: 100%;
    }

    #album-table, #track-table-widget {
        width: 100%;
        height: 100%;
    }
    """

    def compose(self) -> ComposeResult:
        """Build the UI layout."""
        yield Toolbar(id="toolbar")
        with Horizontal(id="content-area"):
            yield TagPanel(id="tag-panel")
            yield TrackTable(id="track-table")
        yield StatusBar(id="status-bar")

    def on_mount(self) -> None:
        """Set up after mount."""
        self._update_title()

    def _update_title(self) -> None:
        """Update screen title based on state."""
        state = self.app.state  # type: ignore[attr-defined]
        if state.library_path:
            self.title = f"auto-tagger — {state.library_path.name}"
        else:
            self.title = "auto-tagger — No library loaded"

    # ── Library loading ─────────────────────────────────────────────

    def open_library_dialog(self) -> None:
        """Open a directory picker modal."""

        def on_dir_selected(path: Path) -> None:
            self._load_library(path)

        self.app.push_screen(
            DirectoryBrowser(title="Select Music Library"),
            on_dir_selected,
        )

    def _load_library(self, path: Path) -> None:
        """Load and scan a library directory."""
        state = self.app.state  # type: ignore[attr-defined]
        state.clear()
        state.library_path = path
        self._add_recent_workspace(path)
        self._update_title()
        self._scan_library(path)
        self.refresh_bindings()

    def _add_recent_workspace(self, path: Path) -> None:
        """Add a path to recent workspaces (max 5, no duplicates)."""
        state = self.app.state  # type: ignore[attr-defined]
        existing = [p for p in state.recent_workspaces if p.resolve() != path.resolve()]
        state.recent_workspaces = [path] + existing[:4]

    def _scan_library(self, path: Path) -> None:
        """Discover albums by scanning for audio files.

        Optimized with os.scandir for speed on large libraries
        on mounted/slow drives. Track metadata is read lazily.
        """
        import os

        from auto_tagger.core.audio import SUPPORTED_EXTENSIONS

        state = self.app.state  # type: ignore[attr-defined]
        audio_extensions = set(SUPPORTED_EXTENSIONS.keys())

        album_dirs: dict[Path, list[Path]] = {}

        # Use os.scandir with recursive walk for speed
        scan_queue = [path]
        scanned = 0
        max_scan = 100_000  # safety limit

        while scan_queue and scanned < max_scan:
            current = scan_queue.pop()
            try:
                with os.scandir(current) as it:
                    for entry in it:
                        if scanned >= max_scan:
                            break
                        scanned += 1
                        if entry.is_dir(follow_symlinks=False):
                            scan_queue.append(Path(entry.path))
                        elif entry.is_file():
                            p = Path(entry.path)
                            if p.suffix.lower() in audio_extensions:
                                parent = p.parent
                                if parent not in album_dirs:
                                    album_dirs[parent] = []
                                album_dirs[parent].append(p)
            except PermissionError:
                continue

        from auto_tagger.ui.state import AlbumData

        # ── Post-process: merge disc subdirectories into parent albums ──────
        # Detect directories named like CD1, CD2, Disc 1, disc01 etc. and
        # merge their tracks into the shared parent so multi-disc albums appear
        # as a single entry with the correct artist/album hints.
        import re

        _disc_re = re.compile(r"^(?:[Cc][Dd]|[Dd][Ii][Ss][CcKk])\s*\d+$")

        disc_parents: dict[Path, list[Path]] = {}
        for album_dir in list(album_dirs):
            if _disc_re.match(album_dir.name):
                parent = album_dir.parent
                disc_parents.setdefault(parent, []).append(album_dir)

        for parent, subdirs in disc_parents.items():
            merged_files: list[Path] = []
            for dd in subdirs:
                merged_files.extend(album_dirs.pop(dd))
            # Retain any tracks already under the parent itself
            existing = album_dirs.pop(parent, [])
            album_dirs[parent] = existing + merged_files

        # ── Create album entries ───────────────────────────────────────────
        for album_dir, audio_files in album_dirs.items():
            state.albums[album_dir] = AlbumData(
                path=album_dir,
                artist_hint=album_dir.parent.name,
                album_hint=album_dir.name,
                audio_file_paths=list(audio_files),
            )

        state.loaded = True
        state.audio_file_count = sum(len(files) for files in album_dirs.values())

    # ── UI refresh ──────────────────────────────────────────────────

    def refresh_bindings(self) -> None:
        """Update toolbar and status bar after state changes."""
        try:
            self.query_one("#toolbar", Toolbar).refresh_state()
            self.query_one("#status-bar", StatusBar).refresh_state()
            self.call_after_refresh(self._refresh_albums_async)
        except Exception:
            pass  # not all widgets may be mounted yet

    async def _refresh_albums_async(self) -> None:
        """Refresh the album table (async because DataTable operations need the event loop)."""
        try:
            await self.query_one("#track-table", TrackTable).refresh_albums()
        except Exception:
            pass

    def open_settings(self) -> None:
        """Open the settings modal."""

        def on_settings_dismissed(result: dict) -> None:
            if not result:
                return
            state = self.app.state  # type: ignore[attr-defined]
            for key, value in result.items():
                if value is not None and value != "":
                    setattr(state, key, value)
            self.notify("Settings saved", severity="information")
            self.refresh_bindings()

        from auto_tagger.ui.screens.settings_screen import SettingsScreen

        self.app.push_screen(SettingsScreen(self.app.state), on_settings_dismissed)

    # ── Actions ─────────────────────────────────────────────────────

    def start_auto_tag(self) -> None:
        """Start auto-tagging the loaded library."""
        state = self.app.state  # type: ignore[attr-defined]
        if not state.loaded or not state.library_path:
            self.notify("Load a library first", severity="warning")
            return
        if state.auto_tagging:
            self.notify("Auto-tag already running", severity="warning")
            return

        from auto_tagger.ui.workflow import run_auto_tag

        self._auto_tag_task = asyncio.create_task(run_auto_tag(self))

    def start_auto_tag_album(self, album_path: Path) -> None:
        """Start auto-tagging a single album (not the whole library)."""
        state = self.app.state  # type: ignore[attr-defined]
        if album_path not in state.albums:
            self.notify("Album not found", severity="error")
            return
        if state.auto_tagging:
            self.notify("Auto-tag already running", severity="warning")
            return

        from auto_tagger.ui.workflow import run_auto_tag

        self.notify(
            f"Starting per-album auto-tag: {album_path.name}...",
            severity="information",
        )
        self._auto_tag_task = asyncio.create_task(
            run_auto_tag(self, album_path=album_path)
        )

    def stop_auto_tag(self) -> None:
        """Stop the running auto-tag subprocess."""
        state = self.app.state  # type: ignore[attr-defined]
        if not state.auto_tagging:
            self.notify("Nothing to stop", severity="information")
            return

        from auto_tagger.ui.workflow import cancel_running

        cancel_running()
        self.notify("Stopping auto-tag...", severity="warning")
        state.auto_tagging = False
        self.refresh_bindings()

    def undo_last(self) -> None:
        """Undo the last operation via undo stack."""
        undo_manager = self.app.undo_manager  # type: ignore[attr-defined]
        op = undo_manager.pop()
        if op is None:
            self.notify("Nothing to undo", severity="information")
            return

        # Restore each track's metadata to disk
        from auto_tagger.core.writer import write_metadata

        for snapshot in op.snapshots:
            try:
                write_metadata(snapshot.path, snapshot.metadata)
            except Exception as exc:
                self.notify(f"Undo failed for {snapshot.path.name}: {exc}", severity="error")

        self.notify(f"Undone: {op.description}", severity="information")
        self.refresh_bindings()

    def focus_filter(self) -> None:
        """Focus the filter input in the status bar."""
        try:
            self.query_one("#status-bar", StatusBar).focus_filter()
        except Exception:
            pass

    def go_back(self) -> None:
        """Return to album browser from track view."""
        state = self.app.state  # type: ignore[attr-defined]
        if state.selected_album_path:
            state.selected_album_path = None
            state.selected_track_paths.clear()
            state.show_album_browser = True
            self.refresh_bindings()
            self.notify("Returned to album browser", severity="information")
        elif state.library_path:
            # If viewing album browser, unload library
            state.clear()
            self._update_title()
            self.refresh_bindings()


class DirectoryBrowser(ModalScreen[Path]):
    """Modal screen for selecting a music library directory."""

    def __init__(self, title: str = "Select Directory") -> None:
        super().__init__()
        self._dialog_title = title

    def compose(self) -> ComposeResult:
        yield Label(self._dialog_title, id="dialog-title")
        yield DirectoryTree("/", id="dir-tree")

    CSS = """
    DirectoryBrowser {
        align: center middle;
        background: $surface 90%;
    }

    #dialog-title {
        text-align: center;
        padding: 1;
        text-style: bold;
        width: 100%;
    }

    #dir-tree {
        width: 80;
        height: 80%;
        border: solid $primary;
    }
    """

    def on_directory_tree_directory_selected(
        self, event: DirectoryTree.DirectorySelected
    ) -> None:
        self.dismiss(event.path)
