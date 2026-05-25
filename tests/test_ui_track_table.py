"""Integration tests for Track Table widget features."""

from __future__ import annotations

from pathlib import Path

from auto_tagger.ui.app import AutoTaggerApp
from auto_tagger.ui.state import AlbumData, TrackData
from auto_tagger.core.metadata import TrackMetadata


async def test_app_initial_view_is_browser():
    """App starts in album browser view."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        table_widget = app.screen.query_one("#track-table")
        assert table_widget.track_table("album").display is True
        assert table_widget.track_table("track").display is False
        assert app.state.show_album_browser is True


async def test_view_switching():
    """Calling _show_track_view / _show_browser_view toggles correctly."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        tw = app.screen.query_one("#track-table")

        tw._show_track_view()
        assert tw.track_table("album").display is False
        assert tw.track_table("track").display is True
        assert app.state.show_album_browser is False

        tw._show_browser_view()
        assert tw.track_table("album").display is True
        assert tw.track_table("track").display is False
        assert app.state.show_album_browser is True


async def test_album_selection_loads_tracks():
    """Selecting an album triggers lazy loading and populates track table."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        album_path = Path("/music/Artist/Album")
        album = AlbumData(
            path=album_path,
            artist_hint="Artist",
            album_hint="Album",
            audio_file_paths=[],
        )
        app.state.albums[album_path] = album
        app.state.loaded = True

        tw = app.screen.query_one("#track-table")
        tw.select_album(album_path)

        assert app.state.selected_album_path == album_path
        assert app.state.show_album_browser is False
        assert tw.track_table("album").display is False
        assert tw.track_table("track").display is True

        bc = app.screen.query_one("#breadcrumb-bar")
        assert bc.display is True


async def test_multi_select_toggle():
    """Ctrl+click toggles multi-selection of tracks."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        album_path = Path("/music/Artist/Album")
        album = AlbumData(path=album_path, artist_hint="A", album_hint="B")
        album.tracks = [
            TrackData(path=Path("/t1.mp3"), metadata=TrackMetadata(title="T1")),
            TrackData(path=Path("/t2.mp3"), metadata=TrackMetadata(title="T2")),
        ]
        album._tracks_loaded = True
        app.state.albums[album_path] = album
        app.state.loaded = True

        tw = app.screen.query_one("#track-table")
        tw.select_album(album_path)

        # Ctrl+click on row 1
        tw._ctrl_pressed = True
        tw._select_track(Path("/t1.mp3"))
        assert Path("/t1.mp3") in app.state.selected_track_paths

        # Ctrl+click row 2
        tw._select_track(Path("/t2.mp3"))
        assert len(app.state.selected_track_paths) == 2

        # Click without Ctrl replaces selection
        tw._ctrl_pressed = False
        tw._select_track(Path("/t1.mp3"))
        assert app.state.selected_track_paths == {Path("/t1.mp3")}


async def test_toggle_optional_columns():
    """Toggling optional columns adds/removes them from the track table."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        tw = app.screen.query_one("#track-table")

        assert "bitrate" not in tw._visible_track_cols

        tw.toggle_track_column("bitrate")
        assert "bitrate" in tw._visible_track_cols

        tw.toggle_track_column("bitrate")
        assert "bitrate" not in tw._visible_track_cols


async def test_sort_tracks():
    """Sorting the track table triggers DataTable.sort."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        album_path = Path("/music/Artist/Album")
        album = AlbumData(path=album_path, artist_hint="A", album_hint="B")
        album.tracks = [
            TrackData(path=Path("/z.mp3"), metadata=TrackMetadata(title="Zulu")),
            TrackData(path=Path("/a.mp3"), metadata=TrackMetadata(title="Alpha")),
        ]
        album._tracks_loaded = True
        app.state.albums[album_path] = album
        app.state.loaded = True

        tw = app.screen.query_one("#track-table")
        tw.select_album(album_path)

        tw._do_sort("title")
        assert tw._sort_col == "title"
        assert tw._sort_reverse is False

        tw._do_sort("title", reverse=True)
        assert tw._sort_reverse is True


async def test_sort_albums():
    """Sorting the album browser re-orders rows."""
    app = AutoTaggerApp()
    async with app.run_test() as pilot:
        app.state.albums = {
            Path("/b/AlbumB"): AlbumData(path=Path("/b/AlbumB"), artist_hint="B"),
            Path("/a/AlbumA"): AlbumData(path=Path("/a/AlbumA"), artist_hint="A"),
        }
        app.state.loaded = True

        tw = app.screen.query_one("#track-table")
        tw.sort_albums("artist")

        assert tw._sort_col == "artist"
        assert tw._sort_reverse is False


async def test_track_data_attributes():
    """TrackData holds all the data needed for table display."""
    track = TrackData(
        path=Path("/test/01 Song.flac"),
        metadata=TrackMetadata(
            title="Song Title",
            artist="Artist",
            album_artist="Album Artist",
            album="Album Name",
            year="2024",
            genre="Rock",
            track_number=1,
            track_total=10,
        ),
        status="ok",
        has_cover=True,
        bitrate=320,
    )

    assert track.status == "ok"
    assert track.has_cover is True
    assert track.bitrate == 320
    assert track.metadata.title == "Song Title"
    assert track.metadata.track_number == 1
    assert track.metadata.track_total == 10
