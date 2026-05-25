"""Status bar with filter input and file statistics."""

from __future__ import annotations

from pathlib import Path

from textual.app import ComposeResult
from textual.widgets import Input, Label
from textual.widget import Widget


DEBOUNCE_MS = 150


class StatusBar(Widget):
    """Bottom bar with filter input, file count, and selection info."""

    DEFAULT_CSS = """
    StatusBar {
        height: 3;
        background: $panel;
        border-top: solid $primary;
    }

    StatusBar > Horizontal {
        height: 100%;
    }

    #status-label {
        padding: 0 1;
        min-width: 30;
        color: $foreground 70%;
    }

    #filter-input {
        width: 1fr;
        margin: 0 1;
    }

    #filter-input:focus {
        border: solid $accent;
    }

    .spacer {
        width: 1;
    }
    """

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._debounce_timer = None

    def compose(self) -> ComposeResult:
        from textual.containers import Horizontal

        with Horizontal():
            yield Label("", id="status-label")
            yield Label("", classes="spacer")
            yield Input(placeholder="🔍 Filter... (supports regex)", id="filter-input")

    def on_input_changed(self, event: Input.Changed) -> None:
        """Handle filter text changes with debounce."""
        if event.input.id == "filter-input":
            state = self.app.state  # type: ignore[attr-defined]
            state.filter_text = event.value

            # Cancel previous debounce timer
            if self._debounce_timer is not None:
                self._debounce_timer.reset(DEBOUNCE_MS)
            else:
                self._debounce_timer = self.set_timer(DEBOUNCE_MS, self._apply_filter)

    def _apply_filter(self) -> None:
        """Apply the current filter to the track table."""
        self._debounce_timer = None
        state = self.app.state  # type: ignore[attr-defined]
        track_table = self.screen.query_one("#track-table")
        if hasattr(track_table, "apply_filter"):
            track_table.apply_filter(state.filter_text)

    def focus_filter(self) -> None:
        """Focus the filter input."""
        self.query_one("#filter-input", Input).focus()

    def refresh_state(self) -> None:
        """Update status label with current stats."""
        state = self.app.state  # type: ignore[attr-defined]
        label = self.query_one("#status-label", Label)

        parts: list[str] = []
        if state.audio_file_count:
            parts.append(f"{state.audio_file_count} files")

        if state.total_duration_seconds:
            minutes = int(state.total_duration_seconds // 60)
            parts.append(f"{minutes}m")

        if state.selected_track_paths:
            parts.append(f"{len(state.selected_track_paths)} selected")

        if state.auto_tagging:
            if state.auto_tag_total > 0:
                pct = int(state.auto_tag_progress / state.auto_tag_total * 100)
                parts.append(f"⏳ Auto-tagging... {state.auto_tag_progress}/{state.auto_tag_total} ({pct}%)")
            else:
                parts.append("⏳ Auto-tagging...")
        elif state.auditing:
            if state.audit_total > 0:
                pct = int(state.audit_progress / state.audit_total * 100)
                parts.append(f"🔍 Auditing... {state.audit_progress}/{state.audit_total} ({pct}%)")
            else:
                parts.append("🔍 Auditing...")

        if state.albums:
            errors = sum(a.error_count for a in state.albums.values())
            warnings = sum(a.warning_count for a in state.albums.values())
            if errors or warnings:
                parts.append(f"⚠ {errors} err / {warnings} warn")

        label.update(" • ".join(parts) if parts else "Ready")
