"""Tests for batch workflow album discovery and summaries."""

from pathlib import Path

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

        def run(self, path, dry_run, interactive=False, artist_mbid_map=None, artist_genre_map=None):
            if path == album_a:
                raise RuntimeError("boom")
            return type(
                "Result",
                (),
                {"applied_writes": 1, "skipped_writes": 0, "health_report": None,
                 "cover_art_fixed": False},
            )()

    summary = BatchWorkflow(Settings(), album_workflow_factory=FakeAlbumWorkflow).run(
        tmp_path,
        dry_run=True,
        parallel=1,
    )

    assert summary.processed == 2
    assert summary.failed == 1
    assert summary.applied == 1
