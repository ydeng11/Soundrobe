"""Tests for CLI functionality."""

from click.testing import CliRunner
import pytest

from auto_tagger.cli import cli


def test_cli_help():
    """Test CLI help output."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--help"])
    assert result.exit_code == 0
    assert "Auto Tagger" in result.output
    assert "tag" in result.output
    assert "batch" in result.output


def test_cli_version():
    """Test version output."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--version"])
    assert result.exit_code == 0
    assert "0.1.0" in result.output


def test_tag_command_help():
    """Test tag command help."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", "--help"])
    assert result.exit_code == 0
    assert "Tag a single album" in result.output


def test_batch_command_help():
    """Test batch command help."""
    runner = CliRunner()
    result = runner.invoke(cli, ["batch", "--help"])
    assert result.exit_code == 0
    assert "Batch process" in result.output


def test_config_command():
    """Test config command."""
    runner = CliRunner()
    result = runner.invoke(cli, ["config"])
    assert result.exit_code == 0
    assert "Configuration" in result.output


def test_config_show_key():
    """Test showing config key."""
    runner = CliRunner()
    result = runner.invoke(cli, ["config", "verbose"])
    assert result.exit_code == 0
    assert "verbose" in result.output


def test_config_invalid_key():
    """Test showing invalid config key."""
    runner = CliRunner()
    result = runner.invoke(cli, ["config", "invalid_key"])
    assert result.exit_code == 0
    assert "Unknown configuration key" in result.output


def test_tag_command_with_path(tmp_path):
    """Test tag command with a valid path."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(tmp_path)])
    assert result.exit_code == 0
    assert "Tagging:" in result.output


def test_tag_command_dry_run(tmp_path):
    """Test tag command with dry-run flag."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", str(tmp_path), "--dry-run"])
    assert result.exit_code == 0
    assert "Dry run: True" in result.output


def test_verbose_flag(tmp_path):
    """Test verbose flag."""
    runner = CliRunner(mix_stderr=False)
    result = runner.invoke(cli, ["--verbose", "tag", str(tmp_path)])
    assert result.exit_code == 0


def test_output_format(tmp_path):
    """Test output format option."""
    runner = CliRunner()
    result = runner.invoke(cli, ["--output", "json", "config"])
    assert result.exit_code == 0


def test_tag_nonexistent_path():
    """Test tag command with nonexistent path."""
    runner = CliRunner()
    result = runner.invoke(cli, ["tag", "/nonexistent/path"])
    assert result.exit_code != 0