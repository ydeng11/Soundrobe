"""Tests for dataset command entry points (status and setup)."""

from click.testing import CliRunner

from auto_tagger.cli import cli


def test_dataset_status_when_not_installed():
    """dataset status command reports not-installed state."""
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["dataset", "status"],
        env={"AUTO_TAG_DATA_DIR": "/tmp/nonexistent-dataset-test"},
    )
    assert result.exit_code == 0
    assert "not installed" in result.output.lower()


def test_dataset_setup_dry_run_prints_plan():
    """dataset setup --dry-run prints plan without downloading."""
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["dataset", "setup", "--dry-run"],
    )
    assert result.exit_code == 0
    assert "Dataset setup" in result.output
    assert "dry run" in result.output.lower()


def test_dataset_setup_invalid_service_errors():
    """dataset setup with invalid service raises error."""
    runner = CliRunner()
    result = runner.invoke(
        cli,
        ["dataset", "setup", "--services", "invalid_service"],
    )
    assert result.exit_code != 0
