"""Toolbar widget with action buttons."""

from __future__ import annotations

from textual.app import ComposeResult
from textual.containers import Horizontal
from textual.widgets import Button, Label
from textual.widget import Widget


class Toolbar(Widget):
    """Top toolbar with Open, Auto-Tag, Stop, Undo, Filter buttons."""

    DEFAULT_CSS = """
    Toolbar {
        height: 3;
        background: $panel;
        border-bottom: solid $primary;
    }

    Toolbar > Horizontal {
        height: 100%;
        align: left middle;
    }

    Toolbar Button {
        margin: 0 1;
        min-width: 8;
    }

    .separator {
        width: 1;
        color: $foreground 30%;
        margin: 0 1;
    }

    .spacer {
        width: 1fr;
    }

    .info-label {
        padding: 0 1;
        color: $foreground 60%;
    }
    """

    def compose(self) -> ComposeResult:
        with Horizontal():
            yield Button("📂 Open", id="open-btn", variant="default")
            yield Button("▶ Auto-Tag", id="auto-tag-btn", variant="primary")
            yield Button("⏹ Stop", id="stop-btn", variant="error", disabled=True)
            yield Label("│", classes="separator")
            yield Button("↩ Undo", id="undo-btn", variant="default", disabled=True)
            yield Label("│", classes="separator")
            yield Button("🔍 Filter", id="filter-btn", variant="default")
            yield Label("", classes="spacer")
            yield Label("", id="library-info", classes="info-label")
            yield Button("⚙ Settings", id="settings-btn", variant="default")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        """Handle button presses."""
        button_id = event.button.id
        screen = self.screen

        if button_id == "open-btn":
            screen.open_library_dialog()
        elif button_id == "auto-tag-btn":
            screen.start_auto_tag()
        elif button_id == "stop-btn":
            screen.stop_auto_tag()
        elif button_id == "undo-btn":
            screen.undo_last()
        elif button_id == "filter-btn":
            screen.focus_filter()
        elif button_id == "settings-btn":
            screen.open_settings()

    def refresh_state(self) -> None:
        """Update button states based on app state."""
        state = self.app.state  # type: ignore[attr-defined]
        undo_manager = self.app.undo_manager  # type: ignore[attr-defined]

        is_busy = state.auto_tagging or state.auditing
        self.query_one("#stop-btn", Button).disabled = not is_busy
        self.query_one("#auto-tag-btn", Button).disabled = (
            is_busy or not state.loaded
        )
        self.query_one("#undo-btn", Button).disabled = not undo_manager.can_undo

        info = self.query_one("#library-info", Label)
        if state.library_path:
            info.update(f"📁 {state.library_path.name} • {state.audio_file_count} files")
        else:
            info.update("No library loaded")
