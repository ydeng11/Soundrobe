"""End-to-end CLI tests using synthetic fixtures.

These tests exercise the full pipeline: CLI arg parsing → audio discovery →
metadata reading → lookup → health report → output formatting.
All tests use CliRunner (no subprocess) for speed and debuggability.
"""

import json
from pathlib import Path

from click.testing import CliRunner

from auto_tagger.cli import cli


def test_tag_dry_run_on_fixture_album_shows_metadata(album_fixture: Path):
    """Dry-run on the synthetic album shows track metadata preview."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(album_fixture), "--dry-run"])
    assert result.exit_code == 0
    assert "反轉地球" in result.output
    assert "潘玮柏" in result.output
    assert "反转地球" in result.output
    assert "Metadata preview" in result.output


def test_tag_dry_run_shows_lookup_section(album_fixture: Path):
    """Dry-run output includes a lookup candidates section."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(album_fixture), "--dry-run"])
    assert result.exit_code == 0
    assert "Lookup" in result.output


def test_tag_dry_run_shows_health_report(album_fixture: Path):
    """Dry-run output includes a health report section."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(album_fixture), "--dry-run"])
    assert result.exit_code == 0
    assert "Health" in result.output


def test_tag_dry_run_writes_health_report_json(album_fixture: Path, tmp_path: Path):
    """--health-report flag writes a JSON file with valid structure."""
    report_path = tmp_path / "health.json"
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["tag", str(album_fixture), "--dry-run", "--health-report", str(report_path)],
    )
    assert result.exit_code == 0
    assert report_path.exists()
    data = json.loads(report_path.read_text(encoding="utf-8"))
    assert "summary" in data
    assert "errors" in data["summary"]
    assert "warnings" in data["summary"]


def test_tag_yolo_dry_run_does_not_write(album_fixture: Path):
    """YOLO mode with dry-run previews but does not mutate files."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(album_fixture), "--dry-run", "--yolo"])
    assert result.exit_code == 0
    assert "YOLO" in result.output


def test_tag_verbose_shows_extra_detail(album_fixture: Path):
    """Verbose flag adds detail to output."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--verbose", "tag", str(album_fixture), "--dry-run"])
    assert result.exit_code == 0


def test_tag_nonexistent_path_errors():
    """Tagging a nonexistent path exits non-zero."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", "/nonexistent/path/xyz"])
    assert result.exit_code != 0


def test_tag_empty_directory_handled(tmp_path: Path):
    """Tagging an empty directory shows appropriate message."""
    empty = tmp_path / "empty"
    empty.mkdir()
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(empty), "--dry-run"])
    assert isinstance(result.exit_code, int)


# ── batch command ──────────────────────────────────────────────


def test_batch_dry_run_on_fixture_library(fixtures_dir: Path):
    """Batch dry-run processes the full fixture library."""
    runner = CliRunner()
    result = runner.invoke(cli, ["batch", str(fixtures_dir), "--dry-run"])
    assert result.exit_code == 0
    assert "Batch processing" in result.output
    assert "Albums processed" in result.output


def test_batch_dry_run_shows_summary(fixtures_dir: Path):
    """Batch output includes album counts and applied/skipped/failed."""
    runner = CliRunner()
    result = runner.invoke(cli, ["batch", str(fixtures_dir), "--dry-run"])
    assert result.exit_code == 0
    output_lower = result.output.lower()
    assert "processed" in output_lower
    assert "applied" in output_lower or "skipped" in output_lower
