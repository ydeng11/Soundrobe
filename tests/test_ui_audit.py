"""Tests for LLM audit system: prompts, schemas, and command."""

from __future__ import annotations

import json
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from auto_tagger.commands.audit import _audit_album, _emit_json
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.llm.prompts import build_audit_messages
from auto_tagger.llm.schemas import AuditResponse, AuditTrackResult


# ── Schema tests ──────────────────────────────────────────────────────────────


class TestAuditSchema:
    """AuditResponse schema enforces structure."""

    def test_valid_audit_track_result(self):
        r = AuditTrackResult(index=0, field="artist", status="error", message="Missing", suggestion="Radiohead")
        assert r.index == 0
        assert r.field == "artist"
        assert r.status == "error"
        assert r.suggestion == "Radiohead"

    def test_valid_audit_warning(self):
        r = AuditTrackResult(index=1, field="title", status="warning", message="Possible typo")
        assert r.status == "warning"
        assert r.suggestion is None

    def test_valid_audit_correct(self):
        r = AuditTrackResult(index=2, field="album", status="correct", message="Looks good")
        assert r.status == "correct"

    def test_audit_response_with_tracks(self):
        resp = AuditResponse(tracks=[
            AuditTrackResult(index=0, field="artist", status="error", message="Missing"),
            AuditTrackResult(index=1, field="title", status="warning", message="Typo", suggestion="Fix"),
        ])
        assert len(resp.tracks) == 2
        assert resp.tracks[0].field == "artist"
        assert resp.tracks[1].suggestion == "Fix"

    def test_audit_response_empty(self):
        resp = AuditResponse(tracks=[])
        assert len(resp.tracks) == 0


# ── Prompt tests ──────────────────────────────────────────────────────────────


class TestBuildAuditMessages:
    """Prompt builder produces correct messages."""

    def test_basic_structure(self):
        tracks = [
            TrackMetadata(title="Song A", artist="Artist A", album="Album X"),
            TrackMetadata(title="Song B", artist="Artist A", album="Album X"),
        ]
        filenames = ["01-song-a.flac", "02-song-b.flac"]
        messages = build_audit_messages("Artist A", "Album X", tracks, filenames)

        assert len(messages) == 2
        assert messages[0]["role"] == "system"
        assert messages[1]["role"] == "user"

        payload = json.loads(messages[1]["content"])
        assert payload["album_folder"] == "Album X"
        assert payload["artist_folder"] == "Artist A"
        assert len(payload["tracks"]) == 2
        assert payload["tracks"][0]["title"] == "Song A"
        assert payload["tracks"][1]["path"] == "02-song-b.flac"

    def test_empty_metadata(self):
        tracks = [TrackMetadata(), TrackMetadata()]
        filenames = ["empty.flac", "also-empty.flac"]
        messages = build_audit_messages(None, None, tracks, filenames)
        payload = json.loads(messages[1]["content"])
        assert payload["tracks"][0]["title"] == ""
        assert payload["artist_folder"] == ""

    def test_artists_field_included(self):
        tracks = [TrackMetadata(title="Song", artist="A", album="X", artists=["A", "B"])]
        filenames = ["01-song.flac"]
        messages = build_audit_messages("A", "X", tracks, filenames)
        payload = json.loads(messages[1]["content"])
        assert payload["tracks"][0]["artists"] == "A, B"

    def test_system_has_examples(self):
        tracks = [TrackMetadata(title="Song", artist="A")]
        filenames = ["song.flac"]
        messages = build_audit_messages("A", "X", tracks, filenames)
        sys_msg = messages[0]["content"]
        assert "Examples" in sys_msg
        assert "correct" in sys_msg.lower()
        assert "error" in sys_msg.lower() or "warning" in sys_msg.lower()


# ── Command tests ──────────────────────────────────────────────────────────────


class TestAuditCommandHelpers:
    """Unit tests for the audit command internals."""

    def test_emit_json(self, capsys):
        _emit_json({"type": "test", "value": 42})
        captured = capsys.readouterr()
        assert json.loads(captured.out.strip()) == {"type": "test", "value": 42}

    def test_audit_album_no_metadata_returns_empty(self):
        """Albums with no meaningful metadata return empty results."""
        settings = MagicMock()
        settings.llm_api_key = "test-key"
        client = MagicMock()
        album_path = Path("/empty/album")
        audio_files = []

        result, fixed_count = _audit_album(settings, client, album_path, audio_files)
        assert result == []
        assert fixed_count == 0

    def test_audit_album_skips_correct_results(self):
        """_audit_album filters out 'correct' status entries, only returning warnings/errors."""
        settings = MagicMock()
        client = MagicMock()

        # Mock the complete_json response to return mixed results
        mock_response = MagicMock()
        mock_response.data = {
            "tracks": [
                {"index": 0, "field": "artist", "status": "correct", "message": "ok"},
                {"index": 1, "field": "title", "status": "warning", "message": "typo", "suggestion": "Fix"},
                {"index": 2, "field": "album", "status": "error", "message": "wrong", "suggestion": "Album X"},
            ]
        }
        client.complete_json.return_value = mock_response

        # Create tracks with valid metadata so the early-return check is skipped
        from auto_tagger.core.metadata import TrackMetadata
        tracks_meta = [TrackMetadata(title="Song A", artist="Artist A")]
        filenames = ["01-song-a.flac"]

        with patch("auto_tagger.commands.audit.load_audio_file") as mock_load, \
             patch("auto_tagger.commands.audit.read_tags") as mock_read:
            mock_af = MagicMock()
            mock_load.return_value = mock_af
            mock_read.return_value = TrackMetadata(title="Song A", artist="Artist A")

            import tempfile
            with tempfile.TemporaryDirectory() as tmpdir:
                album_path = Path(tmpdir)
                audio_files = [album_path / "01-test.flac"]
                audio_files[0].write_text("fake audio")

                result, fixed_count = _audit_album(settings, client, album_path, audio_files)

        # Only warning and error should remain
        assert len(result) == 2
        assert fixed_count == 0  # not using --fix
        assert all(r["status"] in ("warning", "error") for r in result)
        assert result[0]["field"] == "title"
        assert result[1]["field"] == "album"

    def test_audit_album_empty_files_returns_empty(self):
        """Album with unreadable files returns empty."""
        settings = MagicMock()
        client = MagicMock()

        import tempfile
        with tempfile.TemporaryDirectory() as tmpdir:
            album_path = Path(tmpdir)
            # Try to read a non-audio file — should fail gracefully
            audio_files = [album_path / "not-audio.txt"]
            audio_files[0].write_text("not audio")

            result, fixed_count = _audit_album(settings, client, album_path, audio_files)

        # No LLM call should be made for empty metadata
        assert result == []
        assert fixed_count == 0
        client.complete_json.assert_not_called()


# ── Fix tests ─────────────────────────────────────────────────────────────────


class TestAuditFix:
    """Tests for the --fix path in audit, especially collaborative songs."""

    def _mock_llm_with_collaborative_result(self, settings, client):
        """Helper: mock LLM to return a collaborative song audit result."""
        mock_response = MagicMock()
        mock_response.data = {
            "tracks": [
                {
                    "index": 0,
                    "field": "artists",
                    "status": "warning",
                    "message": (
                        "Track '古古惑惑(清清楚楚系我)' has artist='陈小春/郑伊健' "
                        "indicating collaboration, but artists field is empty"
                    ),
                    "suggestion": "陈小春, 郑伊健",
                    "corrected": {
                        "title": "古古惑惑(清清楚楚系我)",
                        "artist": "陈小春/郑伊健",
                        "artists": ["陈小春", "郑伊健"],
                    },
                },
            ]
        }
        client.complete_json.return_value = mock_response
        return mock_response

    def test_corrected_field_preserved_in_results(self):
        """The 'corrected' key is preserved from LLM response in audit results."""
        settings = MagicMock()
        client = MagicMock()

        mock_response = MagicMock()
        mock_response.data = {
            "tracks": [
                {
                    "index": 0,
                    "field": "artists",
                    "status": "warning",
                    "message": "artists field empty for collaborative song",
                    "suggestion": "陈小春, 郑伊健",
                    "corrected": {
                        "artists": ["陈小春", "郑伊健"],
                    },
                },
            ]
        }
        client.complete_json.return_value = mock_response

        import tempfile
        with tempfile.TemporaryDirectory() as tmpdir:
            album_path = Path(tmpdir)
            audio_file = album_path / "01-test.flac"
            audio_file.write_text("fake flac")
            with (
                patch("auto_tagger.commands.audit.load_audio_file") as mock_load,
                patch("auto_tagger.commands.audit.read_tags") as mock_read,
            ):
                mock_af = MagicMock()
                mock_load.return_value = mock_af
                mock_read.return_value = TrackMetadata(
                    title="古古惑惑(清清楚楚系我)",
                    artist="陈小春/郑伊健",
                )

                result, fixed_count = _audit_album(
                    settings, client, album_path, [audio_file], fix=False,
                )

        assert len(result) == 1
        assert fixed_count == 0
        # corrected dict should be preserved
        assert result[0].get("corrected") == {"artists": ["陈小春", "郑伊健"]}

    def test_fix_artists_from_corrected(self):
        """--fix applies artists from CorrectedTrack to file metadata."""
        settings = MagicMock()
        client = MagicMock()

        mock_response = MagicMock()
        mock_response.data = {
            "tracks": [
                {
                    "index": 0,
                    "field": "artists",
                    "status": "warning",
                    "message": "Collaborative track needs artists field",
                    "suggestion": "陈小春, 郑伊健",
                    "corrected": {
                        "title": "古古惑惑(清清楚楚系我)",
                        "artist": "陈小春/郑伊健",
                        "artists": ["陈小春", "郑伊健"],
                    },
                },
            ]
        }
        client.complete_json.return_value = mock_response

        import tempfile
        with tempfile.TemporaryDirectory() as tmpdir:
            album_path = Path(tmpdir)
            audio_file = album_path / "01-古古惑惑.flac"
            audio_file.write_text("fake flac data")

            with (
                patch("auto_tagger.commands.audit.load_audio_file") as mock_load,
                patch("auto_tagger.commands.audit.read_tags") as mock_read,
                patch("auto_tagger.commands.audit.write_metadata") as mock_write,
            ):
                mock_af = MagicMock()
                mock_load.return_value = mock_af
                mock_read.return_value = TrackMetadata(
                    title="古古惑惑(清清楚楚系我)",
                    artist="陈小春/郑伊健",
                    album="友情岁月",
                    artists=[],
                )

                result, fixed_count = _audit_album(
                    settings, client, album_path, [audio_file], fix=True,
                )

        assert fixed_count == 1
        # Verify write_metadata was called with artists populated
        # "陈小春/郑伊健" = 2 singers → 2 entries in artists field
        mock_write.assert_called_once()
        call_args = mock_write.call_args[0]
        written_meta = call_args[1]
        assert len(written_meta.artists) == 2
        assert written_meta.artist == "陈小春/郑伊健"
        assert written_meta.artists == ["陈小春", "郑伊健"]
        assert written_meta.title == "古古惑惑(清清楚楚系我)"

    def test_fix_artists_via_suggestion(self):
        """--fix applies per-field suggestion when corrected is missing."""
        settings = MagicMock()
        client = MagicMock()

        mock_response = MagicMock()
        mock_response.data = {
            "tracks": [
                {
                    "index": 0,
                    "field": "artists",
                    "status": "warning",
                    "message": "Collaborative track needs artists field",
                    "suggestion": "陈小春, 郑伊健",
                    # No corrected field — fallback to suggestion
                },
            ]
        }
        client.complete_json.return_value = mock_response

        import tempfile
        with tempfile.TemporaryDirectory() as tmpdir:
            album_path = Path(tmpdir)
            audio_file = album_path / "01-古古惑惑.flac"
            audio_file.write_text("fake flac data")

            with (
                patch("auto_tagger.commands.audit.load_audio_file") as mock_load,
                patch("auto_tagger.commands.audit.read_tags") as mock_read,
                patch("auto_tagger.commands.audit.write_metadata") as mock_write,
            ):
                mock_af = MagicMock()
                mock_load.return_value = mock_af
                mock_read.return_value = TrackMetadata(
                    title="古古惑惑(清清楚楚系我)",
                    artist="陈小春/郑伊健",
                    album="友情岁月",
                    artists=[],
                )

                result, fixed_count = _audit_album(
                    settings, client, album_path, [audio_file], fix=True,
                )

        assert fixed_count == 1
        mock_write.assert_called_once()
        call_args = mock_write.call_args[0]
        written_meta = call_args[1]
        # Suggestion is wrapped into a list for artists field
        assert written_meta.artists == ["陈小春, 郑伊健"]

    def test_fix_skips_when_no_suggestion_or_corrected(self):
        """--fix skips tracks that have neither suggestion nor corrected."""
        settings = MagicMock()
        client = MagicMock()

        mock_response = MagicMock()
        mock_response.data = {
            "tracks": [
                {
                    "index": 0,
                    "field": "artist",
                    "status": "warning",
                    "message": "Possible typo",
                    # No suggestion and no corrected — no fix
                },
            ]
        }
        client.complete_json.return_value = mock_response

        import tempfile
        with tempfile.TemporaryDirectory() as tmpdir:
            album_path = Path(tmpdir)
            audio_file = album_path / "01-track.flac"
            audio_file.write_text("fake flac data")

            with (
                patch("auto_tagger.commands.audit.load_audio_file") as mock_load,
                patch("auto_tagger.commands.audit.read_tags") as mock_read,
                patch("auto_tagger.commands.audit.write_metadata") as mock_write,
            ):
                mock_af = MagicMock()
                mock_load.return_value = mock_af
                mock_read.return_value = TrackMetadata(title="Song", artist="Artist")

                result, fixed_count = _audit_album(
                    settings, client, album_path, [audio_file], fix=True,
                )

        assert fixed_count == 0
        mock_write.assert_not_called()


# ── Integration test ──────────────────────────────────────────────────────────


async def test_handle_audit_event_updates_track_status():
    """Audit events from subprocess update track state via existing handler."""
    from auto_tagger.ui.workflow import _handle_audit_event
    from auto_tagger.ui.app import AutoTaggerApp
    from auto_tagger.ui.state import AlbumData, TrackData

    app = AutoTaggerApp()
    album_path = Path("/test/Album")
    album = AlbumData(path=album_path)
    album.tracks = [
        TrackData(path=Path("/test/Album/01.flac"), metadata=TrackMetadata(title="Song A")),
        TrackData(path=Path("/test/Album/02.flac"), metadata=TrackMetadata(title="Song B")),
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
                {"index": 0, "field": "artist", "status": "error", "message": "Missing", "suggestion": "Radiohead"},
                {"index": 1, "field": "title", "status": "warning", "message": "Possible typo", "suggestion": "Song B (Remastered)"},
            ],
        })
        assert len(album.audit_results) == 2
        assert album.tracks[0].status == "error"
        assert album.tracks[1].status == "warning"
