"""Tests for Tag Panel validation and multi-track features."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.ui.app import AutoTaggerApp
from auto_tagger.ui.state import AlbumData, TrackData
from auto_tagger.ui.widgets.tag_panel import (
    KEEP_PLACEHOLDER,
    FIELD_DEFS,
    validate_field,
    _field_def,
)
from auto_tagger.core.metadata import TrackMetadata


# ── Validation unit tests ──────────────────────────────────────────────────────


class TestValidateField:
    """Unit tests for field validation."""

    def test_year_valid(self):
        """4-digit year is valid."""
        assert validate_field("year", "2024") is None

    def test_year_empty(self):
        """Empty year is valid (not required)."""
        assert validate_field("year", "") is None

    def test_year_invalid(self):
        """Non-4-digit year shows error."""
        assert validate_field("year", "24") is not None
        assert validate_field("year", "abc") is not None
        assert validate_field("year", "12345") is not None

    def test_track_valid(self):
        """Positive integer track is valid."""
        assert validate_field("track", "1") is None
        assert validate_field("track", "12") is None

    def test_track_empty(self):
        """Empty track is valid."""
        assert validate_field("track", "") is None

    def test_track_invalid(self):
        """Non-numeric track shows error."""
        assert validate_field("track", "abc") is not None
        assert validate_field("track", "-1") is not None
        assert validate_field("track", "0") is not None  # min=1

    def test_title_required(self):
        """Title is required — empty shows error."""
        assert validate_field("title", "") is not None

    def test_title_valid(self):
        """Any non-empty title is valid."""
        assert validate_field("title", "Song Name") is None

    def test_artist_required(self):
        """Artist is required."""
        assert validate_field("artist", "") is not None
        assert validate_field("artist", "The Beatles") is None

    def test_comment_optional(self):
        """Comment is optional — any value ok."""
        assert validate_field("comment", "") is None
        assert validate_field("comment", "Great song") is None

    def test_all_fields_have_def(self):
        """Every FIELD_DEFS entry validates correctly."""
        for fd in FIELD_DEFS:
            assert _field_def(fd["key"]) is not None


# ── Integration tests ─────────────────────────────────────────────────────────


async def test_tag_panel_shows_no_selection_initially():
    """Tag panel shows 'no selection' message on startup."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        panel = app.screen.query_one("#tag-panel")
        no_msg = panel.query_one("#no-selection-msg")
        assert no_msg.display is True


async def test_tag_panel_populates_on_track_select():
    """Populating tag panel shows fields and hides no-selection."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        album_path = Path("/a")
        album = AlbumData(path=album_path, artist_hint="A", album_hint="B")
        album.tracks = [
            TrackData(
                path=Path("/t1.mp3"),
                metadata=TrackMetadata(title="Test Song", artist="Test Artist"),
            ),
        ]
        album._tracks_loaded = True
        app.state.albums[album_path] = album
        app.state.selected_album_path = album_path
        app.state.loaded = True

        panel = app.screen.query_one("#tag-panel")
        panel.populate(Path("/t1.mp3"))

        no_msg = panel.query_one("#no-selection-msg")
        assert no_msg.display is False

        field_map = {f.field_key: f for f in panel._fields}
        assert field_map["title"].get_value() == "Test Song"
        assert field_map["artist"].get_value() == "Test Artist"


async def test_tag_panel_multi_keep():
    """Multi-track editing shows <keep> for divergent values."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        album_path = Path("/a")
        album = AlbumData(path=album_path, artist_hint="A", album_hint="B")
        album.tracks = [
            TrackData(path=Path("/t1.mp3"), metadata=TrackMetadata(title="Song A")),
            TrackData(path=Path("/t2.mp3"), metadata=TrackMetadata(title="Song B")),
        ]
        album._tracks_loaded = True
        app.state.albums[album_path] = album
        app.state.selected_album_path = album_path
        app.state.selected_track_paths = {Path("/t1.mp3"), Path("/t2.mp3")}
        app.state.loaded = True

        panel = app.screen.query_one("#tag-panel")
        panel.populate_multi([Path("/t1.mp3"), Path("/t2.mp3")])

        field_map = {f.field_key: f for f in panel._fields}
        # Title diverges → show <keep>
        assert field_map["title"].get_value() == KEEP_PLACEHOLDER
        # artist is None for both → same, so show empty
        assert field_map["artist"].get_value() == ""
        # year is None for both → same
        assert field_map["year"].get_value() == ""


async def test_tag_panel_multi_badge_shows():
    """Multi-track badge shows editing count."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        album_path = Path("/a")
        album = AlbumData(path=album_path)
        album.tracks = [TrackData(path=Path("/t1.mp3"), metadata=TrackMetadata())]
        album._tracks_loaded = True
        app.state.albums[album_path] = album
        app.state.selected_album_path = album_path
        app.state.loaded = True

        panel = app.screen.query_one("#tag-panel")
        panel.populate_multi([Path("/t1.mp3")])

        badge = panel.query_one("#multi-track-badge")
        assert badge.display is True


async def test_tag_panel_clear_resets():
    """Clearing the panel hides fields and shows no-selection message."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        album_path = Path("/a")
        album = AlbumData(path=album_path)
        album.tracks = [TrackData(path=Path("/t1.mp3"), metadata=TrackMetadata(title="T"))]
        album._tracks_loaded = True
        app.state.albums[album_path] = album
        app.state.selected_album_path = album_path
        app.state.loaded = True

        panel = app.screen.query_one("#tag-panel")
        panel.populate(Path("/t1.mp3"))

        no_msg = panel.query_one("#no-selection-msg")
        assert no_msg.display is False  # fields showing

        panel.clear()
        assert no_msg.display is True  # no selection again
        for f in panel._fields:
            assert f.display is False

        badge = panel.query_one("#multi-track-badge")
        assert badge.display is False
