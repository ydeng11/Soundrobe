"""Tests for batch workflow album discovery and summaries."""

from pathlib import Path

import pytest

from auto_tagger.config import Settings
from auto_tagger.workflows.batch import BatchWorkflow, discover_album_paths


def test_discover_album_paths_groups_audio_by_directory(tmp_path: Path):
    """Batch discovery returns sorted album directories with supported audio."""
    (tmp_path / "Artist" / "Album A").mkdir(parents=True)
    (tmp_path / "Artist" / "Album B").mkdir(parents=True)
    (tmp_path / "Artist" / "Album A" / "01.flac").touch()
    (tmp_path / "Artist" / "Album B" / "01.mp3").touch()
    (tmp_path / "Artist" / "Album B" / "notes.txt").touch()

    albums = discover_album_paths(tmp_path)

    assert albums == [
        tmp_path / "Artist" / "Album A",
        tmp_path / "Artist" / "Album B",
    ]


def test_batch_workflow_continues_after_album_failure(tmp_path: Path):
    """Batch workflow records failures and keeps processing later albums."""
    album_a = tmp_path / "A"
    album_b = tmp_path / "B"
    album_a.mkdir()
    album_b.mkdir()
    (album_a / "01.flac").touch()
    (album_b / "01.flac").touch()

    class FakeAlbumWorkflow:
        def __init__(self, settings):
            self.settings = settings

        def run(self, path, dry_run, interactive=False, force=False, artist_mbid_map=None, artist_genre_map=None):
            if path == album_a:
                raise RuntimeError("boom")
            return type(
                "Result",
                (),
                {"applied_writes": 1, "skipped_writes": 0, "health_report": None,
                 "cover_art_fixed": False, "metadata_by_path": {}},
            )()

    summary = BatchWorkflow(Settings(), album_workflow_factory=FakeAlbumWorkflow).run(
        tmp_path,
        dry_run=True,
        parallel=1,
    )

    assert summary.processed == 2
    assert summary.failed == 1
    assert summary.applied == 1


def test_batch_workflow_progress_callback(tmp_path: Path):
    """Progress callback is called with (current, total) for each album."""
    album_a = tmp_path / "A"
    album_b = tmp_path / "B"
    album_c = tmp_path / "C"
    for d in (album_a, album_b, album_c):
        d.mkdir()
        (d / "01.flac").touch()

    class SilentAlbumWorkflow:
        def __init__(self, settings):
            self.settings = settings

        def run(self, path, dry_run, interactive=False, force=False, artist_mbid_map=None, artist_genre_map=None):
            return type(
                "Result",
                (),
                {"applied_writes": 1, "skipped_writes": 0, "health_report": None,
                 "cover_art_fixed": False, "metadata_by_path": {}},
            )()

    calls: list[tuple[int, int]] = []

    def _cb(current: int, total: int) -> None:
        calls.append((current, total))

    BatchWorkflow(Settings(), album_workflow_factory=SilentAlbumWorkflow).run(
        tmp_path,
        dry_run=True,
        parallel=1,
        progress_callback=_cb,
    )

    assert calls == [(1, 3), (2, 3), (3, 3)]


def test_batch_workflow_progress_callback_on_failure(tmp_path: Path):
    """Progress callback is still called when an album fails."""
    album_a = tmp_path / "A"
    album_b = tmp_path / "B"
    album_a.mkdir()
    album_b.mkdir()
    (album_a / "01.flac").touch()
    (album_b / "01.flac").touch()

    class FailingAlbumWorkflow:
        def __init__(self, settings):
            self.settings = settings

        def run(self, path, dry_run, interactive=False, force=False, artist_mbid_map=None, artist_genre_map=None):
            if path == album_a:
                raise RuntimeError("boom")
            return type(
                "Result",
                (),
                {"applied_writes": 1, "skipped_writes": 0, "health_report": None,
                 "cover_art_fixed": False, "metadata_by_path": {}},
            )()

    calls: list[tuple[int, int]] = []

    def _cb(current: int, total: int) -> None:
        calls.append((current, total))

    BatchWorkflow(Settings(), album_workflow_factory=FailingAlbumWorkflow).run(
        tmp_path,
        dry_run=True,
        parallel=1,
        progress_callback=_cb,
    )

    assert calls == [(1, 2), (2, 2)]


def test_batch_workflow_no_progress_callback_does_not_crash(tmp_path: Path):
    """Calling run() without progress_callback works normally."""
    d = tmp_path / "Album"
    d.mkdir()
    (d / "01.flac").touch()

    class SimpleAlbumWorkflow:
        def __init__(self, settings):
            self.settings = settings

        def run(self, path, dry_run, interactive=False, force=False, artist_mbid_map=None, artist_genre_map=None):
            return type(
                "Result",
                (),
                {"applied_writes": 0, "skipped_writes": 0, "health_report": None,
                 "cover_art_fixed": False, "metadata_by_path": {}},
            )()

    summary = BatchWorkflow(Settings(), album_workflow_factory=SimpleAlbumWorkflow).run(
        tmp_path,
        dry_run=True,
        parallel=1,
    )

    assert summary.processed == 1
    assert summary.failed == 0
