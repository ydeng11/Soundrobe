"""Tests for single-album workflow orchestration."""

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.core.metadata import TrackMetadata
from auto_tagger.workflows.album import AlbumWorkflow


def test_album_workflow_dry_run_collects_preview(monkeypatch, tmp_path: Path):
    """Album workflow reads metadata and returns a dry-run preview result."""
    audio = tmp_path / "01.flac"
    audio.touch()

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: [audio],
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: TrackMetadata(title="Song", artist="Artist", album="Album", track_number=1),
    )

    result = AlbumWorkflow(Settings()).run(tmp_path, dry_run=True)

    assert result.audio_files == [audio]
    assert result.dry_run is True
    assert result.applied_writes == 0
    assert result.planned_writes == 1
    assert result.metadata_by_path[audio].title == "Song"


def test_album_workflow_yolo_blocks_writes_on_health_errors(monkeypatch, tmp_path: Path):
    """YOLO mode still refuses writes when health report has errors."""
    from auto_tagger.quality.health import AlbumHealthReport, HealthIssue, HealthSeverity

    audio = tmp_path / "01.flac"
    audio.touch()

    monkeypatch.setattr(
        "auto_tagger.workflows.album.iter_audio_files",
        lambda path, recursive=False: [audio],
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.read_metadata",
        lambda path: TrackMetadata(title="Song", artist="Artist", album="Album", track_number=1),
    )
    monkeypatch.setattr(
        "auto_tagger.workflows.album.build_album_health_report",
        lambda album_path, audio_files, metadata_by_path, settings: AlbumHealthReport(
            album_path=album_path,
            tracks_checked=1,
            lrc_files_checked=0,
            issues=[
                HealthIssue("audio", HealthSeverity.ERROR, audio, "audio.bad", "Bad audio")
            ],
        ),
    )

    result = AlbumWorkflow(Settings(yolo=True)).run(tmp_path, dry_run=False)

    assert result.applied_writes == 0
    assert result.skipped_writes == 1
