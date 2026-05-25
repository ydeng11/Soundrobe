"""Textual application entry point."""

from __future__ import annotations

from pathlib import Path
from typing import ClassVar

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.screen import Screen

from auto_tagger.ui.screens.main_screen import MainScreen
from auto_tagger.ui.state import AppState
from auto_tagger.ui.undo import UndoManager


class AutoTaggerApp(App):
    """Terminal UI for auto-tagger — browse, tag, audit, and fix."""

    TITLE: ClassVar[str] = "auto-tagger"
    SUB_TITLE: ClassVar[str] = "Intelligent Audio Tagging"
    CSS: ClassVar[str] = """
    Screen {
        background: $surface;
    }
    """

    BINDINGS: ClassVar[list[Binding]] = [
        Binding("ctrl+q", "quit", "Quit"),
        Binding("ctrl+o", "open_library", "Open Library"),
        Binding("ctrl+t", "auto_tag", "Auto-Tag"),
        Binding("ctrl+z", "undo", "Undo"),
        Binding("ctrl+f", "focus_filter", "Filter"),
        Binding("ctrl+s", "settings", "Settings"),
        Binding("escape", "go_back", "Back"),
        Binding("enter", "select_row", "Select"),
    ]

    def __init__(self, library_path: Path | None = None) -> None:
        super().__init__()
        self.state = AppState()
        self.undo_manager = UndoManager()
        self._library_path = library_path

    def on_mount(self) -> None:
        """Called when the app is mounted."""
        self.push_screen(MainScreen())

        if self._library_path:
            self.call_after_refresh(self._load_library_path)

    def _load_library_path(self) -> None:
        """Load the library path after mount."""
        screen = self.screen
        if hasattr(screen, "_load_library"):
            screen._load_library(self._library_path)

    def action_open_library(self) -> None:
        """Open a file picker to select a library directory."""
        screen = self.screen
        if hasattr(screen, "open_library_dialog"):
            screen.open_library_dialog()

    def action_auto_tag(self) -> None:
        """Trigger auto-tag on the current library."""
        screen = self.screen
        if hasattr(screen, "start_auto_tag"):
            screen.start_auto_tag()

    def action_undo(self) -> None:
        """Undo the last operation."""
        screen = self.screen
        if hasattr(screen, "undo_last"):
            screen.undo_last()

    def action_focus_filter(self) -> None:
        """Focus the filter input."""
        screen = self.screen
        if hasattr(screen, "focus_filter"):
            screen.focus_filter()

    def action_go_back(self) -> None:
        """Go back / deselect."""
        screen = self.screen
        if hasattr(screen, "go_back"):
            screen.go_back()

    def action_settings(self) -> None:
        """Open the settings modal."""
        screen = self.screen
        if hasattr(screen, "open_settings"):
            screen.open_settings()

    def action_select_row(self) -> None:
        """Select the current row / open album."""
        screen = self.screen
        track_table = getattr(screen, "query_one", None)
        if track_table:
            try:
                tw = screen.query_one("#track-table")
                if hasattr(tw, "action_select_current"):
                    tw.action_select_current()
            except Exception:
                pass
