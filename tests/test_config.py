"""Tests for configuration system."""

from pathlib import Path
from unittest.mock import patch

import pytest

from auto_tagger.config import Settings, find_config_file, load_config_file
from auto_tagger.exceptions import ConfigError


def test_settings_defaults():
    """Test default settings values."""
    settings = Settings()
    assert settings.output_format == "table"
    assert settings.verbose is False
    assert settings.recursive is False
    assert settings.yolo is False
    assert settings.cache_enabled is True
    assert settings.ffprobe_path == "ffprobe"
    assert settings.ffprobe_timeout_seconds == 20
    assert settings.replaygain_command == "rgain3"
    assert settings.replaygain_timeout_seconds == 600
    assert settings.lrc_convert_encoding is False


def test_settings_env_override(monkeypatch):
    """Test environment variable overrides."""
    monkeypatch.setenv("AUTO_TAG_VERBOSE", "true")
    monkeypatch.setenv("AUTO_TAG_OUTPUT_FORMAT", "json")

    settings = Settings()
    assert settings.verbose is True
    assert settings.output_format == "json"


def test_settings_validation():
    """Test settings validation."""
    with pytest.raises(ValueError, match="output_format must be one of"):
        Settings(output_format="invalid")


def test_find_config_file_not_exists():
    """Test config file discovery when no file exists."""
    with patch.object(Path, "exists", return_value=False):
        result = find_config_file()
        assert result is None


def test_load_config_file_missing():
    """Test loading missing config file."""
    with pytest.raises(ConfigError, match="Config file not found"):
        load_config_file(Path("/nonexistent/config.yaml"))


def test_merge_with_cli_args():
    """Test merging CLI arguments with settings."""
    settings = Settings(output_format="table")
    merged = settings.merge_with_cli_args(output_format="json", verbose=True)

    assert merged.output_format == "json"
    assert merged.verbose is True


def test_settings_from_yaml(tmp_path):
    """Test loading settings from YAML file."""
    config_file = tmp_path / "config.yaml"
    config_file.write_text("output_format: json\nverbose: true\n")

    config_data = load_config_file(config_file)
    assert config_data["output_format"] == "json"
    assert config_data["verbose"] is True
