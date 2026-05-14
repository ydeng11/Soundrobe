"""Tests for command entry point execute() functions."""

from pathlib import Path

from auto_tagger.commands.tag import execute as tag_execute
from auto_tagger.config import Settings


def test_tag_execute_with_fixture_album(album_fixture: Path):
    """tag.execute() runs without raising on a valid album."""
    settings = Settings(data_dir=Path("/tmp/auto-tagger-test"))
    tag_execute(
        settings=settings,
        path=album_fixture,
        dry_run=True,
        interactive=False,
        health_report_path=None,
    )


def test_tag_execute_with_empty_dir(tmp_path: Path):
    """tag.execute() handles empty directory gracefully."""
    empty = tmp_path / "empty"
    empty.mkdir()
    settings = Settings(data_dir=Path("/tmp/auto-tagger-test"))
    tag_execute(
        settings=settings,
        path=empty,
        dry_run=True,
        interactive=False,
        health_report_path=None,
    )


def test_tag_execute_yolo_mode(album_fixture: Path):
    """tag.execute() runs in yolo mode with dry-run (no mutation)."""
    settings = Settings(data_dir=Path("/tmp/auto-tagger-test"), yolo=True)
    tag_execute(
        settings=settings,
        path=album_fixture,
        dry_run=True,
        interactive=False,
        health_report_path=None,
    )
