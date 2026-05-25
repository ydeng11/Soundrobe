"""Tag editor panel — form fields + cover art preview with validation."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from textual.app import ComposeResult
from textual.containers import Horizontal, VerticalScroll
from textual.widget import Widget
from textual.widgets import Input, Label, Static

from auto_tagger.core.metadata import TrackMetadata

# ── Field definitions ──────────────────────────────────────────────────────────

FIELD_DEFS: list[dict[str, Any]] = [
    {"key": "title", "label": "Title", "required": True, "numeric": False},
    {"key": "artist", "label": "Artist", "required": True, "numeric": False},
    {"key": "artists", "label": "ARTISTS", "required": False, "numeric": False},
    {"key": "album", "label": "Album", "required": False, "numeric": False},
    {"key": "album_artist", "label": "Album Artist", "required": False, "numeric": False},
    {"key": "year", "label": "Year", "required": False, "numeric": True, "pattern": r"^\d{4}$"},
    {"key": "track", "label": "Track", "required": False, "numeric": True, "min": 1},
    {"key": "track_total", "label": "Track Total", "required": False, "numeric": True, "min": 1},
    {"key": "disc", "label": "Disc", "required": False, "numeric": True, "min": 1},
    {"key": "disc_total", "label": "Disc Total", "required": False, "numeric": True, "min": 1},
    {"key": "genre", "label": "Genre", "required": False, "numeric": False},
    {"key": "composer", "label": "Composer", "required": False, "numeric": False},
    {"key": "comment", "label": "Comment", "required": False, "numeric": False},
]

FIELD_META_MAP: dict[str, str] = {
    "track": "track_number",
    "track_total": "track_total",
    "disc": "disc_number",
    "disc_total": "disc_total",
}

KEEP_PLACEHOLDER = "<keep>"


# ── Validation ─────────────────────────────────────────────────────────────────


def validate_field(field_key: str, value: str) -> str | None:
    """Validate a field value. Returns None if valid, error message if invalid."""
    fd = _field_def(field_key)
    if not fd:
        return None

    stripped = value.strip()

    # Empty is allowed for non-required fields
    if not stripped:
        if fd.get("required"):
            return f"{fd['label']} is required"
        return None

    # Numeric validation
    if fd.get("numeric"):
        if fd.get("pattern"):
            import re
            if not re.match(fd["pattern"], stripped):
                return f"{fd['label']} must match {fd['pattern']}"
        else:
            try:
                val = int(stripped)
                min_val = fd.get("min")
                if min_val is not None and val < min_val:
                    return f"{fd['label']} must be ≥ {min_val}"
            except ValueError:
                return f"{fd['label']} must be a number"

    return None


def _field_def(field_key: str) -> dict[str, Any] | None:
    for fd in FIELD_DEFS:
        if fd["key"] == field_key:
            return fd
    return None


# ── _FieldRow widget ───────────────────────────────────────────────────────────


class _FieldRow(Widget):
    """A single labeled input field row with validation."""

    DEFAULT_CSS = """
    _FieldRow {
        height: 3;
        layout: horizontal;
    }

    _FieldRow > Label {
        width: 12;
        text-align: right;
        padding: 0 1 0 0;
        color: $foreground 60%;
    }

    _FieldRow > Input {
        width: 1fr;
    }

    _FieldRow > Input.changed {
        background: $success 20%;
    }

    _FieldRow > Input.flagged {
        background: $warning 20%;
    }

    _FieldRow > Input.error {
        background: $error 20%;
    }

    _FieldRow > Input.keep {
        color: $foreground 50%;
        text-style: italic;
    }
    """

    def __init__(self, field_key: str, value: str = "") -> None:
        super().__init__()
        self.field_key = field_key
        self._value = value

    def compose(self) -> ComposeResult:
        yield Label(self._label_text)
        yield Input(value=self._value, id=f"input-{self.field_key}")

    @property
    def _label_text(self) -> str:
        fd = _field_def(self.field_key)
        label = fd["label"] if fd else self.field_key.title()
        if fd and fd.get("required"):
            label += " *"
        return label

    def set_value(self, value: str) -> None:
        inp = self.query_one(Input)
        inp.value = value
        self._update_keep_style(value)

    def get_value(self) -> str:
        return self.query_one(Input).value

    @property
    def has_keep(self) -> bool:
        return self.get_value() == KEEP_PLACEHOLDER

    def mark_changed(self, changed: bool = True) -> None:
        self.query_one(Input).set_class(changed, "changed")

    def mark_flagged(self, flagged: bool = True) -> None:
        self.query_one(Input).set_class(flagged, "flagged")

    def mark_error(self, error: bool = True) -> None:
        self.query_one(Input).set_class(error, "error")

    def _update_keep_style(self, value: str) -> None:
        inp = self.query_one(Input)
        inp.set_class(value == KEEP_PLACEHOLDER, "keep")


# ── TagPanel widget ────────────────────────────────────────────────────────────


class TagPanel(Widget):
    """Left sidebar with metadata form fields and cover art preview."""

    DEFAULT_CSS = """
    TagPanel {
        width: 38;
        min-width: 30;
        background: $surface;
    }

    #panel-title {
        padding: 1;
        text-style: bold;
        background: $panel;
        height: 3;
    }

    #multi-track-badge {
        height: 1;
        display: none;
        background: $accent;
        color: $text;
        padding: 0 1;
    }

    #multi-track-badge.visible {
        display: block;
    }

    #form-scroll {
        height: 1fr;
        overflow-y: scroll;
    }

    #no-selection-msg {
        width: 100%;
        height: 100%;
        content-align: center middle;
        color: $foreground 40%;
    }

    #cover-container {
        height: 12;
        border-top: solid $border;
        padding: 0 1;
        overflow: hidden;
    }

    #cover-art {
        width: 100%;
        height: 100%;
        content-align: center middle;
    }

    #cover-art.present {
        color: $success;
    }

    #cover-art.missing {
        color: $error;
    }
    #validation-msg {
        height: 1;
        display: none;
        background: $error 20%;
        color: $error;
        padding: 0 1;
    }

    #validation-msg.visible {
        display: block;
    }
    """

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._fields: list[_FieldRow] = []
        self._editing_track: Path | None = None
        self._editing_multiple: bool = False
        self._populating: bool = False

    def compose(self) -> ComposeResult:
        yield Label("Metadata Editor", id="panel-title")
        yield Label("", id="multi-track-badge")
        with VerticalScroll(id="form-scroll"):
            yield Static("Select a track to edit", id="no-selection-msg")
        yield Label("", id="validation-msg")
        with Horizontal(id="cover-container"):
            yield Static("Cover Art\n(not available)", id="cover-art")

    def on_mount(self) -> None:
        """Pre-build all field rows but keep them hidden."""
        scroll = self.query_one("#form-scroll", VerticalScroll)
        for fd in FIELD_DEFS:
            row = _FieldRow(field_key=fd["key"])
            row.display = False
            self._fields.append(row)
            scroll.mount(row)

    # ── Populate ──────────────────────────────────────────────────────────────

    def populate(self, track_path: Path) -> None:
        """Populate form fields for a single track."""
        state = self.app.state  # type: ignore[attr-defined]
        album = state.selected_album
        if not album:
            return

        track = next((t for t in album.tracks if t.path == track_path), None)
        if not track:
            return

        self._editing_track = track_path
        self._editing_multiple = False
        self._populating = True
        meta = track.metadata

        self._hide_no_selection()
        self._hide_multi_badge()
        self._clear_validation()

        field_map = {f.field_key: f for f in self._fields}
        field_map["title"].set_value(meta.title or "")
        field_map["artist"].set_value(meta.artist or "")
        field_map["artists"].set_value(", ".join(meta.artists))
        field_map["album"].set_value(meta.album or "")
        field_map["album_artist"].set_value(meta.album_artist or "")
        field_map["year"].set_value(meta.year or "")
        field_map["track"].set_value(str(meta.track_number) if meta.track_number is not None else "")
        field_map["track_total"].set_value(str(meta.track_total) if meta.track_total is not None else "")
        field_map["disc"].set_value(str(meta.disc_number) if meta.disc_number is not None else "")
        field_map["disc_total"].set_value(str(meta.disc_total) if meta.disc_total is not None else "")
        field_map["genre"].set_value(meta.genre or "")
        field_map["composer"].set_value(meta.composer or "")
        field_map["comment"].set_value("")

        for f in self._fields:
            f.display = True
            f.mark_changed(False)
            f.mark_flagged(False)
            f.mark_error(False)

        self._apply_audit_flags(album, track_path)
        self._update_cover()
        self._populating = False

    def populate_multi(self, track_paths: list[Path]) -> None:
        """Populate form fields for multiple tracks — show <keep> for divergent values."""
        state = self.app.state  # type: ignore[attr-defined]
        album = state.selected_album
        if not album:
            return

        tracks = [t for t in album.tracks if t.path in track_paths]
        if not tracks:
            return

        self._editing_track = tracks[0].path  # primary track
        self._editing_multiple = True
        self._populating = True

        self._hide_no_selection()
        self._show_multi_badge(len(tracks))
        self._clear_validation()

        field_map = {f.field_key: f for f in self._fields}

        for fd in FIELD_DEFS:
            key = fd["key"]
            values = {self._value_for_field(t.metadata, key) for t in tracks}

            if len(values) == 1:
                val = values.pop()
            else:
                val = KEEP_PLACEHOLDER

            field_map[key].set_value(val)
            field_map[key].display = True

        for f in self._fields:
            f.mark_changed(False)
            f.mark_flagged(False)
            f.mark_error(False)

        self._update_cover()
        self._populating = False

    @staticmethod
    def _value_for_field(meta: TrackMetadata, field_key: str) -> str:
        """Extract a string value from TrackMetadata for a field key."""
        if field_key == "artists":
            return ", ".join(meta.artists)
        elif field_key == "track":
            return str(meta.track_number) if meta.track_number is not None else ""
        elif field_key == "track_total":
            return str(meta.track_total) if meta.track_total is not None else ""
        elif field_key == "disc":
            return str(meta.disc_number) if meta.disc_number is not None else ""
        elif field_key == "disc_total":
            return str(meta.disc_total) if meta.disc_total is not None else ""
        else:
            return getattr(meta, field_key, "") or ""

    # ── UI helpers ────────────────────────────────────────────────────────────

    def _hide_no_selection(self) -> None:
        self.query_one("#no-selection-msg", Static).display = False

    def _show_multi_badge(self, count: int) -> None:
        badge = self.query_one("#multi-track-badge", Label)
        badge.update(f"✏️ Editing {count} tracks")
        badge.display = True

    def _hide_multi_badge(self) -> None:
        self.query_one("#multi-track-badge", Label).display = False

    def _clear_validation(self) -> None:
        self.query_one("#validation-msg", Label).display = False

    def _show_validation(self, message: str) -> None:
        msg = self.query_one("#validation-msg", Label)
        msg.update(f"⚠ {message}")
        msg.display = True

    # ── Audit ─────────────────────────────────────────────────────────────────

    def _apply_audit_flags(self, album, track_path: Path) -> None:
        field_map = {f.field_key: f for f in self._fields}
        for result in album.audit_results:
            track = album.tracks[result.track_index] if album.tracks else None
            if track and track.path == track_path:
                field = field_map.get(result.field)
                if field:
                    if result.status == "error":
                        field.mark_error(True)
                    elif result.status == "warning":
                        field.mark_flagged(True)

    # ── Cover art ─────────────────────────────────────────────────────────────

    def _update_cover(self) -> None:
        """Update the cover art preview — render as coloured terminal blocks."""
        cover = self.query_one("#cover-art", Static)
        state = self.app.state  # type: ignore[attr-defined]
        album = state.selected_album

        if not album:
            cover.update("Cover Art\n(not available)")
            return

        # Estimate available render width from the widget
        available_width = self._cover_render_width()

        cover_bytes: bytes | None = None
        cover_source_label: str = ""

        # Priority 1: external cover file
        if album.cover_path and album.cover_path.exists():
            try:
                cover_bytes = album.cover_path.read_bytes()
                cover_source_label = album.cover_path.name
            except Exception:
                pass

        # Priority 2: embedded cover from first track
        if cover_bytes is None:
            cover_bytes = album.get_cover_bytes()
            if cover_bytes:
                cover_source_label = "embedded"

        if cover_bytes:
            from auto_tagger.ui.render_cover import render_cover_from_bytes

            rendered = render_cover_from_bytes(
                cover_bytes,
                max_width=available_width,
                max_height=10,
            )
            if rendered is not None:
                cover.update(rendered)
                return
            # Fallback: show text description
            cover.update(
                f"🎨 {cover_source_label}\n"
                f"({album.cover_source or 'embedded'})"
            )
            return

        # No cover available
        if any(t.has_cover for t in album.tracks if album._tracks_loaded):
            cover.update(
                "🎨 Embedded cover art\n"
                "(right-click for options)"
            )
        else:
            cover.update(
                "🎵 No cover art\n"
                "(right-click to add)"
            )

    def _cover_render_width(self) -> int:
        """Estimate available character width for cover art rendering.

        Returns the approximate number of columns available inside the
        cover container after accounting for padding and borders.
        """
        try:
            # The panel width is 38 (defined in CSS), minus padding/borders
            # yields roughly 30-34 usable characters.
            w = self.region.width if hasattr(self, "region") and self.region else 38
            return max(16, w - 8)  # subtract padding + border + scrollbar margin
        except Exception:
            return 30

    @staticmethod
    def _has_embedded_cover(album) -> bool:
        return any(t.has_cover for t in album.tracks)

    def clear(self) -> None:
        """Clear the form fields and reset to 'no selection' state."""
        self._editing_track = None
        self._editing_multiple = False
        self.query_one("#no-selection-msg", Static).display = True
        self._hide_multi_badge()
        self._clear_validation()

        for f in self._fields:
            f.display = False

        cover = self.query_one("#cover-art", Static)
        cover.update("Cover Art\n(not available)")

    # ── Right-click cover context menu ────────────────────────────────────────

    def on_static_clicked(self, event: Static.Click) -> None:
        """Handle right-click on the cover art."""
        if event.button != 3:
            return
        if event.widget.id != "cover-art":
            return
        event.stop()
        self._show_cover_menu()

    def _show_cover_menu(self) -> None:
        """Notify available cover actions."""
        self.app.notify(
            "Cover: [E]mbed, e[x]tract, [R]emove, [A]uto-fetch",
            severity="information",
            timeout=5,
        )

    def action_embed_cover(self) -> None:
        """Embed cover art from a file."""
        state = self.app.state  # type: ignore[attr-defined]
        album = state.selected_album
        if not album:
            return
        file_path = self._pick_cover_file()
        if file_path:
            self.app.notify(f"Embedding cover from {file_path.name}...", timeout=2)
            # TODO: implement cover embedding

    def action_extract_cover(self) -> None:
        """Extract embedded cover art to album folder."""
        self.app.notify("Extracting cover...", timeout=2)
        # TODO: implement cover extraction

    def action_remove_cover(self) -> None:
        """Remove embedded cover art."""
        self.app.notify("Removing cover...", timeout=2)
        # TODO: implement cover removal

    def action_fetch_cover(self) -> None:
        """Auto-fetch cover art from Cover Art Archive."""
        self.app.notify("Fetching cover art...", timeout=2)
        # TODO: implement cover fetching from subprocess

    @staticmethod
    def _pick_cover_file() -> Path | None:
        """Prompt user to select a cover image file."""
        # For now, use a placeholder
        return None

    # ── Input handling ───────────────────────────────────────────────────────

    def on_input_changed(self, event: Input.Changed) -> None:
        """Handle field edits — auto-save to file."""
        if self._populating:
            return
        if event.input.id.startswith("input-") and (self._editing_track or self._editing_multiple):
            self._on_field_edited(event)

    def _on_field_edited(self, event: Input.Changed) -> None:
        """Save field edit to disk immediately."""
        field_key = event.input.id.replace("input-", "")
        new_value = event.value

        state = self.app.state  # type: ignore[attr-defined]
        album = state.selected_album
        if not album:
            return

        # Validate
        error = validate_field(field_key, new_value)
        if error:
            self._show_validation(error)
            for f in self._fields:
                if f.field_key == field_key:
                    f.mark_error(True)
            return

        self._clear_validation()
        for f in self._fields:
            if f.field_key == field_key:
                f.mark_error(False)

        # Resolve tracks to edit
        if self._editing_multiple:
            tracks_to_edit = [
                t for t in album.tracks
                if t.path in state.selected_track_paths
            ]
        elif self._editing_track:
            tracks_to_edit = [
                t for t in album.tracks if t.path == self._editing_track
            ]
        else:
            return

        from dataclasses import replace
        from auto_tagger.ui.undo import TrackSnapshot

        for track in tracks_to_edit:
            # Skip if field is <keep> (don't change)
            if self._editing_multiple and new_value == KEEP_PLACEHOLDER:
                continue

            # Save pre-edit snapshot for undo
            snap = TrackSnapshot(path=track.path, metadata=track.metadata)
            self.app.undo_manager.push(  # type: ignore[attr-defined]
                f"Edit {field_key}: {track.path.name}", [snap]
            )

            # Convert value based on field type
            parsed = self._parse_value(field_key, new_value)
            meta_key = FIELD_META_MAP.get(field_key, field_key)
            track.metadata = replace(track.metadata, **{meta_key: parsed})
            track.changed_fields.add(field_key)

            # Write to file
            from auto_tagger.core.writer import write_metadata

            try:
                write_metadata(track.path, track.metadata)
            except Exception as exc:
                self.notify(f"Save failed: {exc}", severity="error")
                return

            if not self._editing_multiple:
                break  # only save once for single-track edit

        for f in self._fields:
            if f.field_key == field_key:
                f.mark_changed(True)

    @staticmethod
    def _parse_value(field_key: str, value: str) -> Any:
        """Parse a string value to the right type for TrackMetadata."""
        if not value.strip():
            return None

        numeric_keys = {"track", "track_total", "disc", "disc_total"}
        if field_key in numeric_keys:
            try:
                return int(value.strip())
            except ValueError:
                return None

        if field_key == "year":
            return value.strip()

        return value.strip()
