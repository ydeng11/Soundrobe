"""Configuration file loader."""

from pathlib import Path
from typing import Any

import yaml  # type: ignore[import-untyped]

from auto_tagger.config.settings import Settings
from auto_tagger.exceptions import ConfigError


def find_config_file() -> Path | None:
    """Find config file in standard locations."""
    locations = [
        Path.cwd() / "auto-tagger.yaml",
        Path.cwd() / "auto-tagger.yml",
        Path.cwd() / ".auto-tagger.yaml",
        Path.home() / ".config" / "auto-tagger" / "config.yaml",
        Path.home() / ".auto-tagger.yaml",
    ]

    for location in locations:
        if location.exists():
            return location

    return None


def load_config_file(config_path: Path | None = None) -> dict[str, Any]:
    """Load configuration from YAML file.

    Args:
        config_path: Path to config file, or None to auto-discover

    Returns:
        Configuration dictionary

    Raises:
        ConfigError: If config file cannot be loaded
    """
    if config_path is None:
        config_path = find_config_file()

    if config_path is None:
        return {}

    if not config_path.exists():
        raise ConfigError(f"Config file not found: {config_path}")

    try:
        with open(config_path) as f:
            config_data = yaml.safe_load(f)
            return config_data if config_data else {}
    except yaml.YAMLError as e:
        raise ConfigError(f"Invalid YAML in config file: {e}") from e
    except Exception as e:
        raise ConfigError(f"Failed to load config file: {e}") from e


def load_settings(config_file: Path | None = None, **cli_overrides: Any) -> Settings:
    """Load settings from all sources with proper priority.

    Priority order (highest to lowest):
    1. CLI arguments
    2. Environment variables
    3. Config file
    4. Defaults

    Args:
        config_file: Optional config file path
        **cli_overrides: CLI argument overrides

    Returns:
        Merged Settings instance
    """
    config_data = load_config_file(config_file)

    env_settings = Settings()

    merged_settings = env_settings.model_copy(update=config_data)

    if cli_overrides:
        merged_settings = merged_settings.merge_with_cli_args(**cli_overrides)

    return merged_settings
