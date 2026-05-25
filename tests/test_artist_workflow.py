"""Tests for artist artwork workflow."""

from pathlib import Path

import pytest

from auto_tagger.config import Settings
from auto_tagger.workflows.artist import ArtistWorkflow


def test_artist_workflow_progress_callback(tmp_path: Path):
    """Progress callback is called with (current, total) for each artist dir."""
    for name in ("Artist One", "Artist Two", "Artist Three"):
        artist_dir = tmp_path / name
        artist_dir.mkdir()
        # Artist dirs need audio content to be discovered
        (artist_dir / "01.flac").touch()

    calls: list[tuple[int, int]] = []

    def _cb(current: int, total: int) -> None:
        calls.append((current, total))

    settings = Settings(
        artist_artwork_skip_dirs=["Compilations", "Various Artists", "compilations"],
        artist_artwork_enabled=False,
    )
    ArtistWorkflow(settings).run(
        tmp_path,
        dry_run=True,
        force=False,
        parallel=1,
        progress_callback=_cb,
    )

    assert calls == [(1, 3), (2, 3), (3, 3)]


def test_artist_workflow_progress_callback_no_artists(tmp_path: Path):
    """Progress callback is NOT called when no artist dirs exist."""
    calls: list[tuple[int, int]] = []

    def _cb(current: int, total: int) -> None:
        calls.append((current, total))

    settings = Settings(
        artist_artwork_skip_dirs=["Compilations", "Various Artists"],
        artist_artwork_enabled=False,
    )
    summary = ArtistWorkflow(settings).run(
        tmp_path,
        dry_run=True,
        force=False,
        parallel=1,
        progress_callback=_cb,
    )

    assert calls == []
    assert summary.total == 0


def test_artist_workflow_progress_callback_skips_nonaudio_dirs(tmp_path: Path):
    """Progress callback only fires for directories discovered as artist dirs."""
    (tmp_path / "Notes").mkdir()  # no audio, not an artist dir
    (tmp_path / "Real Artist").mkdir()
    (tmp_path / "Real Artist" / "Album").mkdir()
    (tmp_path / "Real Artist" / "Album" / "01.flac").touch()

    calls: list[tuple[int, int]] = []

    def _cb(current: int, total: int) -> None:
        calls.append((current, total))

    settings = Settings(
        artist_artwork_skip_dirs=[],
        artist_artwork_enabled=False,
    )
    summary = ArtistWorkflow(settings).run(
        tmp_path,
        dry_run=True,
        force=False,
        parallel=1,
        progress_callback=_cb,
    )

    assert calls == [(1, 1)]
    assert summary.total == 1 


def test_artist_workflow_no_progress_callback_does_not_crash(tmp_path: Path):
    """Calling run() without progress_callback works normally."""
    artist_dir = tmp_path / "Test Artist"
    artist_dir.mkdir()
    (artist_dir / "01.flac").touch()

    settings = Settings(
        artist_artwork_skip_dirs=["Compilations", "Various Artists"],
        artist_artwork_enabled=False,
    )
    summary = ArtistWorkflow(settings).run(
        tmp_path,
        dry_run=True,
        force=False,
        parallel=1,
    )

    assert summary.total == 1
