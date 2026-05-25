"""Settings modal screen — edit application preferences."""

from __future__ import annotations

from pathlib import Path

from textual.app import ComposeResult
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.screen import ModalScreen
from textual.widgets import Button, Checkbox, Input, Label, Select, Static

from auto_tagger.ui.state import AppState


class SettingsScreen(ModalScreen[dict]):
    """Modal overlay for editing application settings.

    Returns a dict of changed settings when dismissed, or None if cancelled.
    """

    DEFAULT_CSS = """
    SettingsScreen {
        align: center middle;
        background: $surface 85%;
    }

    #settings-dialog {
        width: 50;
        height: 70%;
        min-height: 20;
        border: thick $primary;
        background: $surface;
    }

    #settings-title {
        text-style: bold;
        padding: 1 2;
        background: $panel;
        text-align: center;
        height: 3;
    }

    #settings-body {
        height: 1fr;
        padding: 1 2;
        overflow-y: scroll;
    }

    #settings-body > Label {
        margin-top: 1;
        color: $foreground 70%;
    }

    #settings-body > Input, #settings-body > Select {
        margin-bottom: 1;
    }

    #settings-footer {
        height: 3;
        padding: 0 1;
        align: right middle;
    }

    #settings-footer Button {
        margin: 0 1;
        min-width: 10;
    }
    """

    def __init__(self, state: AppState) -> None:
        super().__init__()
        self._state = state
        self._parallel_jobs: str = str(getattr(state, "parallel_jobs", 4))
        self._auto_audit: bool = state.auto_audit_enabled
        self._llm_model: str = getattr(state, "llm_model", "")
        self._output_format: str = "table"
        self._debug_enabled: bool = state.debug_enabled

    def compose(self) -> ComposeResult:
        with Vertical(id="settings-dialog"):
            yield Static("⚙ Settings", id="settings-title")
            with VerticalScroll(id="settings-body"):
                yield Label("Auto-Audit After Tag")
                yield Checkbox("Run LLM audit after auto-tag completes", value=self._auto_audit, id="auto-audit-toggle")

                yield Label("Debug Logging")
                yield Checkbox("Enable detailed debug logging (metadata tracing)", value=self._debug_enabled, id="debug-toggle")

                yield Label("LLM Model")
                yield Input(value=self._llm_model, id="llm-model-input", placeholder="e.g. anthropic/claude-3.5-haiku")

                yield Label("Output Format")
                yield Select(
                    [(f, f) for f in ["table", "json", "plain"]],
                    prompt="Select format",
                    value=self._output_format,
                    id="output-format-select",
                )

                yield Label("Recent Workspaces")
                recent = getattr(self._state, "recent_workspaces", [])
                if recent:
                    for rp in recent:
                        yield Static(f"  📁 {rp.name}", classes="recent-workspace")
                else:
                    yield Static("  (none)", classes="recent-workspace")

            with Horizontal(id="settings-footer"):
                yield Button("Cancel", id="cancel-btn", variant="default")
                yield Button("Save", id="save-btn", variant="primary")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "cancel-btn":
            self.dismiss({})
        elif event.button.id == "save-btn":
            self._save()

    def _save(self) -> None:
        """Collect values and dismiss with result."""
        result = {
            "auto_audit_enabled": self.query_one("#auto-audit-toggle", Checkbox).value,
            "llm_model": self.query_one("#llm-model-input", Input).value.strip(),
            "output_format": self.query_one("#output-format-select", Select).value,
            "debug_enabled": self.query_one("#debug-toggle", Checkbox).value,
        }
        self.dismiss(result)

    def on_mount(self) -> None:
        """Focus the first interactive field."""
        self.query_one("#auto-audit-toggle", Checkbox).focus()
