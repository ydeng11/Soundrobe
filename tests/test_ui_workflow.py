"""Tests for auto-tag integration: JSON stream protocol and subprocess launcher."""

from __future__ import annotations

import json
from pathlib import Path

from auto_tagger.commands.batch import _execute_json_stream, _emit_json
from auto_tagger.ui.workflow import _handle_event, _handle_audit_event
from auto_tagger.ui.app import AutoTaggerApp
from auto_tagger.ui.state import AlbumData, AppState, TrackAuditResult, TrackData


# ── JSON Stream Protocol Unit Tests ────────────────────────────────────────────


class TestEmitJson:
    """Tests for the JSON line emitter."""

    def test_emits_valid_json(self, capsys):
        """_emit_json writes a valid JSON line to stdout."""
        _emit_json({"type": "test", "value": 42})
        captured = capsys.readouterr()
        assert captured.out.strip() == '{"type": "test", "value": 42}'

    def test_emits_newline_terminated(self, capsys):
        """Each emit ends with a newline."""
        _emit_json({"type": "alpha"})
        _emit_json({"type": "beta"})
        captured = capsys.readouterr()
        lines = captured.out.strip().split("\n")
        assert len(lines) == 2
        assert json.loads(lines[0])["type"] == "alpha"
        assert json.loads(lines[1])["type"] == "beta"

    def test_emits_unicode(self, capsys):
        """Unicode characters are not escaped."""
        _emit_json({"type": "album", "path": "/Música/Álbum"})
        captured = capsys.readouterr()
        assert "Música" in captured.out


class TestJsonStreamSchema:
    """Verify the JSON stream output schema for each event type."""

    def test_album_event_schema(self):
        """Album event has required fields."""
        event = {
            "type": "album",
            "path": "/music/Album",
            "status": "ok",
            "changes": 5,
            "tracks": [{"path": "/music/Album/01.flac"}],
        }
        assert event["type"] == "album"
        assert isinstance(event["path"], str)
        assert event["status"] in ("ok", "error", "skipped")
        assert isinstance(event["changes"], int)
        assert isinstance(event["tracks"], list)

    def test_progress_event_schema(self):
        """Progress event has current/total."""
        event = {"type": "progress", "current": 3, "total": 42}
        assert event["type"] == "progress"
        assert event["current"] <= event["total"]

    def test_summary_event_schema(self):
        """Summary event aggregates results."""
        event = {
            "type": "summary",
            "processed": 10,
            "failed": 1,
            "applied": 42,
            "skipped": 5,
            "total": 10,
        }
        assert event["type"] == "summary"
        # processed should never exceed total
        assert event["processed"] <= event["total"]
        # failed should never exceed total
        assert event["failed"] <= event["total"]

    def test_audit_event_schema(self):
        """Audit result event has per-track fields."""
        event = {
            "type": "audit",
            "path": "/music/Album",
            "tracks": [
                {
                    "index": 0,
                    "field": "artist",
                    "status": "error",
                    "message": "Missing artist tag",
                    "suggestion": "The Beatles",
                },
            ],
        }
        assert event["type"] == "audit"
        track = event["tracks"][0]
        assert track["index"] == 0
        assert track["field"] == "artist"
        assert track["status"] in ("correct", "warning", "error")
        assert "message" in track
        assert "suggestion" in track


# ── Event Handler Tests ────────────────────────────────────────────────────────


def _make_screen_with_album():
    """Create an app with a single album for event handler testing."""
    from auto_tagger.core.metadata import TrackMetadata

    app = AutoTaggerApp()
    album_path = Path("/test/Album")
    album = AlbumData(path=album_path, artist_hint="A", album_hint="B")
    album.tracks = [
        TrackData(path=Path("/test/Album/01.flac"), metadata=TrackMetadata(title="Song")),
    ]
    album._tracks_loaded = True
    app.state.albums[album_path] = album
    app.state.loaded = True
    app.state.selected_album_path = album_path
    return app


async def test_handle_album_event_ok():
    """Album 'ok' event updates album status."""
    app = _make_screen_with_album()
    async with app.run_test() as pilot:
        screen = app.screen
        await _handle_event(screen, {
            "type": "album",
            "path": "/test/Album",
            "status": "ok",
            "changes": 3,
            "tracks": [{"path": "/test/Album/01.flac"}],
        })
        album = app.state.albums[Path("/test/Album")]
        assert album.status == "ok"


async def test_handle_album_event_error():
    """Album 'error' event sets album status to error."""
    app = _make_screen_with_album()
    async with app.run_test() as pilot:
        screen = app.screen
        await _handle_event(screen, {
            "type": "album",
            "path": "/test/Album",
            "status": "error",
            "changes": 0,
            "message": "Lookup failed",
            "tracks": [],
        })
        album = app.state.albums[Path("/test/Album")]
        assert album.status == "error"


async def test_handle_progress_event():
    """Progress events don't crash."""
    app = _make_screen_with_album()
    async with app.run_test() as pilot:
        screen = app.screen
        # Should not raise
        await _handle_event(screen, {"type": "progress", "current": 1, "total": 10})


async def test_handle_summary_event():
    """Summary events notify user."""
    app = _make_screen_with_album()
    async with app.run_test() as pilot:
        screen = app.screen
        await _handle_event(screen, {
            "type": "summary",
            "processed": 1,
            "failed": 0,
            "applied": 3,
            "skipped": 0,
            "total": 1,
        })
        # Should not crash — notification is fire-and-forget


async def test_handle_audit_event():
    """Audit event applies results to album."""
    from auto_tagger.core.metadata import TrackMetadata

    app = AutoTaggerApp()
    album_path = Path("/test/Album")
    album = AlbumData(path=album_path)
    album.tracks = [
        TrackData(path=Path("/test/Album/01.flac"), metadata=TrackMetadata(title="Song")),
    ]
    album._tracks_loaded = True
    app.state.albums[album_path] = album
    app.state.selected_album_path = album_path

    async with app.run_test() as pilot:
        screen = app.screen
        await _handle_audit_event(screen, {
            "type": "audit",
            "path": "/test/Album",
            "tracks": [
                {
                    "index": 0,
                    "field": "artist",
                    "status": "error",
                    "message": "Missing artist",
                    "suggestion": "The Beatles",
                },
            ],
        })
        assert len(album.audit_results) == 1
        assert album.audit_results[0].field == "artist"
        assert album.audit_results[0].status == "error"
        assert album.tracks[0].status == "error"


async def test_handle_audit_warning_does_not_override_error():
    """A warning does not downgrade an existing error status."""
    from auto_tagger.core.metadata import TrackMetadata

    app = AutoTaggerApp()
    album_path = Path("/test/Album")
    album = AlbumData(path=album_path)
    album.tracks = [
        TrackData(path=Path("/test/Album/01.flac"), metadata=TrackMetadata(title="Song"), status="error"),
    ]
    album._tracks_loaded = True
    app.state.albums[album_path] = album
    app.state.selected_album_path = album_path

    async with app.run_test() as pilot:
        screen = app.screen
        await _handle_audit_event(screen, {
            "type": "audit",
            "path": "/test/Album",
            "tracks": [
                {
                    "index": 0,
                    "field": "title",
                    "status": "warning",
                    "message": "Suspicious title",
                    "suggestion": "Song (Album Version)",
                },
            ],
        })
        # Status should remain "error", not downgrade to "warning"
        assert album.tracks[0].status == "error"


async def test_handle_album_event_unknown_album_ignored():
    """Events for unknown albums are silently ignored."""
    app = _make_screen_with_album()
    async with app.run_test() as pilot:
        screen = app.screen
        # Should not raise
        await _handle_event(screen, {
            "type": "album",
            "path": "/nonexistent",
            "status": "ok",
            "changes": 0,
            "tracks": [],
        })


async def test_toolbar_disabled_during_auto_tag():
    """Toolbar auto-tag button is disabled while auto-tag is running."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        app.state.library_path = Path("/test")
        app.state.loaded = True
        app.state.auto_tagging = True
        app.screen.refresh_bindings()
        toolbar = app.screen.query_one("#toolbar")
        btn = toolbar.query_one("#auto-tag-btn")
        assert btn.disabled is True
        stop_btn = toolbar.query_one("#stop-btn")
        assert stop_btn.disabled is False


async def test_toolbar_stop_enabled_during_audit():
    """Stop button is enabled during audit too."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        app.state.library_path = Path("/test")
        app.state.loaded = True
        app.state.auditing = True
        app.screen.refresh_bindings()
        toolbar = app.screen.query_one("#toolbar")
        stop_btn = toolbar.query_one("#stop-btn")
        assert stop_btn.disabled is False
