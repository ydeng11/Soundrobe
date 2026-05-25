"""Tests for UI state models and undo system."""

from pathlib import Path

from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.ui.state import AlbumData, AppState, TrackAuditResult, TrackData
from auto_tagger.ui.undo import TrackSnapshot, UndoManager


class TestAppState:
    """Tests for the in-memory application state."""

    def test_initial_state(self):
        """State starts empty."""
        state = AppState()
        assert state.library_path is None
        assert state.albums == {}
        assert state.loaded is False
        assert state.auto_tagging is False
        assert state.audio_file_count == 0
        assert state.show_album_browser is True

    def test_clear_resets_state(self):
        """Clear restores initial state."""
        state = AppState()
        state.library_path = Path("/music")
        state.albums[Path("/a")] = AlbumData(path=Path("/a"))
        state.loaded = True
        state.auto_tagging = True
        state.audio_file_count = 42

        state.clear()

        assert state.library_path is None
        assert state.albums == {}
        assert state.loaded is False
        assert state.auto_tagging is False
        assert state.audio_file_count == 0

    def test_selected_album_none_when_no_path(self):
        """selected_album returns None when nothing selected."""
        state = AppState()
        assert state.selected_album is None

    def test_selected_album_returns_data(self):
        """selected_album returns the correct album."""
        state = AppState()
        album = AlbumData(path=Path("/music/artist/album"))
        state.albums[Path("/music/artist/album")] = album
        state.selected_album_path = Path("/music/artist/album")
        assert state.selected_album is album

    def test_selected_tracks_empty_no_selection(self):
        """selected_tracks returns empty when nothing selected."""
        state = AppState()
        assert state.selected_tracks == []

    def test_selected_tracks_empty_no_album(self):
        """selected_tracks returns empty when no album selected."""
        state = AppState()
        state.selected_track_paths = {Path("/a/1.mp3")}
        assert state.selected_tracks == []


class TestAlbumData:
    """Tests for album-level data."""

    def test_empty_counts(self):
        """Empty album has zero counts."""
        album = AlbumData(path=Path("/a"))
        assert album.track_count == 0
        assert album.error_count == 0
        assert album.warning_count == 0

    def test_counts_by_status(self):
        """Counts reflect track statuses."""
        album = AlbumData(path=Path("/a"))
        album.tracks = [
            TrackData(path=Path("/a/1.mp3"), metadata=TrackMetadata(), status="ok"),
            TrackData(path=Path("/a/2.mp3"), metadata=TrackMetadata(), status="warning"),
            TrackData(path=Path("/a/3.mp3"), metadata=TrackMetadata(), status="error"),
        ]
        album._tracks_loaded = True
        assert album.track_count == 3
        assert album.error_count == 1
        assert album.warning_count == 1

    def test_get_cover_bytes_from_path(self, tmp_path):
        """get_cover_bytes returns bytes from external cover file."""
        from PIL import Image as PILImage

        # Create a small cover image
        cover_path = tmp_path / "cover.jpg"
        img = PILImage.new("RGB", (10, 10), (255, 0, 0))
        img.save(str(cover_path), format="JPEG")

        album = AlbumData(path=tmp_path)
        album.cover_path = cover_path
        album.cover_source = "external"

        result = album.get_cover_bytes()
        assert result is not None
        assert isinstance(result, bytes)
        # JPEG files start with FF D8 FF
        assert result[:3] == b"\xff\xd8\xff"

    def test_get_cover_bytes_no_cover(self):
        """get_cover_bytes returns None when no cover exists."""
        album = AlbumData(path=Path("/nonexistent"))
        album.cover_path = None
        assert album.get_cover_bytes() is None

    def test_get_cover_bytes_missing_file_returns_none(self):
        """get_cover_bytes returns None when cover_path file is missing."""
        album = AlbumData(path=Path("/tmp"))
        album.cover_path = Path("/nonexistent/cover.jpg")
        album.cover_source = "external"
        assert album.get_cover_bytes() is None


class TestExtractEmbeddedCover:
    """Tests for the _extract_embedded_cover helper using mock objects."""

    def test_extract_from_mock_mp3_apic(self):
        """Extract cover bytes from a mock MP3 with APIC."""
        from auto_tagger.ui.state import _extract_embedded_cover
        from auto_tagger.core.audio import AudioFormat

        cover_data = b"\xff\xd8\xff\x00fake_jpeg_data"

        # Create a minimal mock that mimics mutagen ID3 tags with APIC
        class MockAPIC:
            data = cover_data

        class MockID3Tags:
            def getall(self, key):
                if key == "APIC":
                    return [MockAPIC()]
                return []

        class MockTags:
            tags = MockID3Tags()

        result = _extract_embedded_cover(AudioFormat.MP3, MockTags())
        assert result == cover_data

    def test_extract_from_mock_m4a_covr(self):
        """Extract cover bytes from a mock M4A with covr atom."""
        from auto_tagger.ui.state import _extract_embedded_cover
        from auto_tagger.core.audio import AudioFormat

        cover_data = b"\x89PNG\r\n\x1a\nfake_png"

        class MockM4ATags:
            def get(self, key, default=None):
                if key == "covr":
                    return [cover_data]
                return default

        result = _extract_embedded_cover(AudioFormat.M4A, MockM4ATags())
        assert result == cover_data

    def test_extract_from_mock_flac_pictures(self):
        """Extract cover bytes from a mock FLAC with pictures."""
        from auto_tagger.ui.state import _extract_embedded_cover
        from auto_tagger.core.audio import AudioFormat

        cover_data = b"mock_flac_cover_data"

        class MockPicture:
            data = cover_data

        class MockFLACTags:
            pictures = [MockPicture()]

        result = _extract_embedded_cover(AudioFormat.FLAC, MockFLACTags())
        assert result == cover_data

    def test_extract_returns_none_when_no_cover(self):
        """Extract returns None when no cover is embedded."""
        from auto_tagger.ui.state import _extract_embedded_cover
        from auto_tagger.core.audio import AudioFormat

        class MockEmptyTags:
            tags = None

            def getall(self, key):
                return []

        # MP3 without tags attribute
        result = _extract_embedded_cover(AudioFormat.MP3, MockEmptyTags())
        assert result is None

        # FLAC without pictures
        class MockFLACNoPic:
            pictures = []

        result = _extract_embedded_cover(AudioFormat.FLAC, MockFLACNoPic())
        assert result is None

        # M4A without covr
        class MockM4ANoCovr:
            def get(self, key, default=None):
                return default

        result = _extract_embedded_cover(AudioFormat.M4A, MockM4ANoCovr())
        assert result is None

    def test_extract_handles_exception_gracefully(self):
        """Extract returns None when an exception occurs."""
        from auto_tagger.ui.state import _extract_embedded_cover
        from auto_tagger.core.audio import AudioFormat

        class BrokenTags:
            @property
            def tags(self):
                raise RuntimeError("broken")

        result = _extract_embedded_cover(AudioFormat.MP3, BrokenTags())
        assert result is None


class TestTrackAuditResult:
    """Tests for audit result data."""

    def test_minimal_creation(self):
        """Audit result can be created with just required fields."""
        result = TrackAuditResult(track_index=0, field="title", status="correct")
        assert result.status == "correct"
        assert result.message is None
        assert result.suggestion is None

    def test_full_creation(self):
        """Audit result with all fields."""
        result = TrackAuditResult(
            track_index=2,
            field="artist",
            status="error",
            message="Artist name appears incorrect",
            suggestion="Pink Floyd",
        )
        assert result.track_index == 2
        assert result.field == "artist"
        assert result.message == "Artist name appears incorrect"
        assert result.suggestion == "Pink Floyd"


class TestUndoManager:
    """Tests for the undo stack."""

    def test_empty_stack(self):
        """Fresh undo manager has no undo."""
        um = UndoManager()
        assert um.can_undo is False
        assert um.current_description is None
        assert um.pop() is None

    def test_push_and_pop(self):
        """Push adds an operation, pop removes it."""
        um = UndoManager()
        snap = [TrackSnapshot(path=Path("/t.mp3"), metadata=TrackMetadata(title="T"))]
        um.push("Edit title", snap)
        assert um.can_undo is True
        assert um.current_description == "Edit title"

        op = um.pop()
        assert op is not None
        assert op.description == "Edit title"
        assert len(op.snapshots) == 1
        assert um.can_undo is False

    def test_multiple_operations(self):
        """Stack maintains LIFO order."""
        um = UndoManager()
        snap1 = [TrackSnapshot(path=Path("/a.mp3"), metadata=TrackMetadata(title="A"))]
        snap2 = [TrackSnapshot(path=Path("/b.mp3"), metadata=TrackMetadata(title="B"))]

        um.push("First", snap1)
        um.push("Second", snap2)

        assert um.current_description == "Second"
        op2 = um.pop()
        assert op2 is not None and op2.description == "Second"

        assert um.current_description == "First"
        op1 = um.pop()
        assert op1 is not None and op1.description == "First"

        assert um.can_undo is False

    def test_max_depth(self):
        """Stack respects maximum depth."""
        um = UndoManager(max_depth=3)
        snap = [TrackSnapshot(path=Path("/t.mp3"), metadata=TrackMetadata(title="T"))]

        for i in range(5):
            um.push(f"Op {i}", snap)

        assert len(um) == 3
        op = um.pop()
        assert op is not None and op.description == "Op 4"

    def test_clear(self):
        """Clear removes all operations."""
        um = UndoManager()
        snap = [TrackSnapshot(path=Path("/t.mp3"), metadata=TrackMetadata(title="T"))]
        um.push("Test", snap)
        um.clear()
        assert um.can_undo is False
        assert len(um) == 0
