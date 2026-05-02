# Phase 1 Execution Plan: Project Setup & Core Architecture

## Overview

**Goal**: Establish project structure, configuration, and basic CLI

**Duration**: ~2-3 hours

**Dependencies**: None (foundation phase)

**Success Criteria**:
- Project builds and installs successfully
- CLI entry point works (`auto-tag --help`)
- Configuration file loads correctly
- Logging outputs to console

---

## Wave 1.1: Project Structure & Build Config

**Objective**: Create Python project foundation with proper src layout and build configuration

### Task 1.1.1: Create directory structure

**Action**: Create src layout directories following PyPA recommendations

**Files to create**:
```
src/
  auto_tagger/
    __init__.py
    __main__.py
    cli.py
    exceptions.py
    commands/
      __init__.py
    core/
      __init__.py
    config/
      __init__.py
    utils/
      __init__.py
tests/
  __init__.py
  conftest.py
```

**Commands**:
```bash
mkdir -p src/auto_tagger/{commands,core,config,utils}
mkdir -p tests
touch src/auto_tagger/__init__.py
touch src/auto_tagger/__main__.py
touch src/auto_tagger/cli.py
touch src/auto_tagger/exceptions.py
touch src/auto_tagger/commands/__init__.py
touch src/auto_tagger/core/__init__.py
touch src/auto_tagger/config/__init__.py
touch src/auto_tagger/utils/__init__.py
touch tests/__init__.py
touch tests/conftest.py
```

**Verification**:
```bash
tree src -L 3
tree tests -L 1
```

**Rollback**:
```bash
rm -rf src tests
```

---

### Task 1.1.2: Initialize __init__.py with version

**Action**: Add version and package metadata

**File**: `src/auto_tagger/__init__.py`

**Content**:
```python
"""Auto Tagger - Intelligent audio file tagging CLI tool."""

__version__ = "0.1.0"
__author__ = "Auto Tagger Team"
__license__ = "MIT"
```

**Verification**:
```bash
python -c "from src.auto_tagger import __version__; print(__version__)"
```

**Rollback**:
```bash
echo "" > src/auto_tagger/__init__.py
```

---

### Task 1.1.3: Create pyproject.toml

**Action**: Create build configuration with dependencies

**File**: `pyproject.toml`

**Content**:
```toml
[build-system]
requires = ["hatchling>=1.26"]
build-backend = "hatchling.build"

[project]
name = "auto-tagger"
version = "0.1.0"
description = "Intelligent audio file tagging CLI tool"
readme = "README.md"
license = "MIT"
requires-python = ">=3.10"
authors = [
    { name = "Auto Tagger Team" }
]
classifiers = [
    "Development Status :: 3 - Alpha",
    "Environment :: Console",
    "Intended Audience :: Developers",
    "License :: OSI Approved :: MIT License",
    "Operating System :: OS Independent",
    "Programming Language :: Python :: 3",
    "Programming Language :: Python :: 3.10",
    "Programming Language :: Python :: 3.11",
    "Programming Language :: Python :: 3.12",
]
keywords = ["cli", "audio", "tagging", "metadata", "music"]

dependencies = [
    "click>=8.1.0",
    "rich>=13.0.0",
    "pydantic>=2.0.0",
    "pydantic-settings>=2.0.0",
    "pyyaml>=6.0",
]

[project.optional-dependencies]
dev = [
    "pytest>=7.0",
    "pytest-cov>=4.0",
    "mypy>=1.0",
    "ruff>=0.1.0",
]

[project.scripts]
auto-tag = "auto_tagger.cli:main"

[project.urls]
Homepage = "https://github.com/yourusername/auto-tagger"
Repository = "https://github.com/yourusername/auto-tagger.git"

[tool.hatch.build.targets.wheel]
packages = ["src/auto_tagger"]

[tool.hatch.build.targets.sdist]
include = ["src/", "README.md", "LICENSE"]

[tool.ruff]
line-length = 100
target-version = "py310"

[tool.ruff.lint]
select = ["E", "F", "I", "N", "W", "UP"]

[tool.mypy]
python_version = "3.10"
warn_return_any = true
warn_unused_configs = true
disallow_untyped_defs = true

[tool.pytest.ini_options]
testpaths = ["tests"]
python_files = ["test_*.py"]
python_classes = ["Test*"]
python_functions = ["test_*"]
```

**Verification**:
```bash
python -m pip install -e . --dry-run
```

**Rollback**:
```bash
rm pyproject.toml
```

---

### Task 1.1.4: Create .gitignore

**Action**: Create comprehensive Python gitignore

**File**: `.gitignore`

**Content**:
```
# Python
__pycache__/
*.py[cod]
*$py.class
*.so
.Python
build/
develop-eggs/
dist/
downloads/
eggs/
.eggs/
lib/
lib64/
parts/
sdist/
var/
wheels/
*.egg-info/
.installed.cfg
*.egg
MANIFEST

# Virtual environments
venv/
env/
ENV/
.venv

# IDE
.vscode/
.idea/
*.swp
*.swo
*~

# Testing
.pytest_cache/
.coverage
htmlcov/
.tox/
.hypothesis/

# Type checking
.mypy_cache/
.dmypy.json
dmypy.json

# Project specific
.env
*.log
.cache/
*.db
```

**Verification**:
```bash
cat .gitignore
```

**Rollback**:
```bash
rm .gitignore
```

---

### Task 1.1.5: Create README.md

**Action**: Create project README

**File**: `README.md`

**Content**:
```markdown
# Auto Tagger

Intelligent audio file tagging CLI tool.

## Installation

```bash
pip install auto-tagger
```

## Development

```bash
# Clone repository
git clone https://github.com/yourusername/auto-tagger.git
cd auto-tagger

# Create virtual environment
python -m venv .venv
source .venv/bin/activate  # or `.\.venv\Scripts\activate` on Windows

# Install in development mode
pip install -e ".[dev]"

# Run tests
pytest

# Run CLI
auto-tag --help
```

## Usage

```bash
# Tag a single album
auto-tag tag /path/to/album

# Batch process library
auto-tag batch /path/to/library

# View configuration
auto-tag config
```

## License

MIT
```

**Verification**:
```bash
cat README.md
```

**Rollback**:
```bash
rm README.md
```

---

### Task 1.1.6: Create LICENSE file

**Action**: Add MIT license

**File**: `LICENSE`

**Content**:
```
MIT License

Copyright (c) 2026 Auto Tagger Team

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

**Verification**:
```bash
cat LICENSE
```

**Rollback**:
```bash
rm LICENSE
```

---

### Task 1.1.7: Install package in development mode

**Action**: Install package with dev dependencies

**Commands**:
```bash
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
```

**Verification**:
```bash
pip list | grep auto-tagger
python -c "import auto_tagger; print(auto_tagger.__version__)"
```

**Rollback**:
```bash
rm -rf .venv
```

---

## Wave 1.2: Configuration System

**Objective**: Implement configuration management with Pydantic Settings

**Dependencies**: Wave 1.1 complete

### Task 1.2.1: Create custom exceptions

**Action**: Define custom exception hierarchy

**File**: `src/auto_tagger/exceptions.py`

**Content**:
```python
"""Custom exceptions for auto_tagger."""


class AutoTaggerError(Exception):
    """Base exception for auto_tagger."""
    exit_code = 1


class ConfigError(AutoTaggerError):
    """Configuration related errors."""
    exit_code = 2


class FileProcessingError(AutoTaggerError):
    """File processing errors."""
    exit_code = 3


class ValidationError(AutoTaggerError):
    """Validation errors."""
    exit_code = 4


class TaggingError(AutoTaggerError):
    """Tagging operation errors."""
    exit_code = 5
```

**Verification**:
```bash
python -c "from auto_tagger.exceptions import AutoTaggerError, ConfigError; print('OK')"
```

**Rollback**:
```bash
echo "" > src/auto_tagger/exceptions.py
```

---

### Task 1.2.2: Create settings model

**Action**: Define Pydantic Settings for configuration

**File**: `src/auto_tagger/config/settings.py`

**Content**:
```python
"""Configuration settings using Pydantic Settings."""

from pathlib import Path
from typing import Any

from pydantic import Field, field_validator
from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    """Application settings with multi-source configuration."""

    model_config = SettingsConfigDict(
        env_prefix="AUTO_TAG_",
        env_file=".env",
        env_file_encoding="utf-8",
        env_nested_delimiter="__",
        case_sensitive=False,
        extra="ignore",
    )

    output_format: str = Field(
        default="table",
        description="Output format: table, json, or plain",
    )
    verbose: bool = Field(
        default=False,
        description="Enable verbose output",
    )
    config_file: Path | None = Field(
        default=None,
        description="Path to configuration file",
    )

    recursive: bool = Field(
        default=False,
        description="Process directories recursively",
    )
    recursive_depth: int = Field(
        default=10,
        ge=0,
        le=50,
        description="Maximum recursion depth",
    )

    exclude_patterns: list[str] = Field(
        default=[".git", "__pycache__", "*.pyc", ".DS_Store"],
        description="Patterns to exclude from processing",
    )

    yolo: bool = Field(
        default=False,
        description="Auto-approve all changes without preview",
    )

    llm_api_key: str | None = Field(
        default=None,
        description="API key for LLM integration",
    )
    llm_endpoint: str = Field(
        default="https://openrouter.ai/api/v1",
        description="LLM API endpoint",
    )
    llm_model: str = Field(
        default="anthropic/claude-3.5-haiku",
        description="LLM model to use",
    )

    cache_enabled: bool = Field(
        default=True,
        description="Enable match caching",
    )
    cache_path: Path = Field(
        default=Path.home() / ".cache" / "auto-tagger" / "cache.db",
        description="Path to cache database",
    )

    @field_validator("output_format")
    @classmethod
    def validate_output_format(cls, v: str) -> str:
        """Validate output format is one of allowed values."""
        allowed = {"table", "json", "plain"}
        if v not in allowed:
            raise ValueError(f"output_format must be one of {allowed}, got {v}")
        return v

    @field_validator("config_file")
    @classmethod
    def validate_config_file(cls, v: Path | None) -> Path | None:
        """Validate config file exists if provided."""
        if v is not None and not v.exists():
            raise ValueError(f"Config file not found: {v}")
        return v

    def merge_with_cli_args(self, **cli_args: Any) -> "Settings":
        """Merge settings with CLI arguments (CLI takes precedence)."""
        update_dict = {
            k: v for k, v in cli_args.items() if v is not None
        }
        return self.model_copy(update=update_dict)
```

**Verification**:
```bash
python -c "from auto_tagger.config.settings import Settings; s = Settings(); print(s.output_format)"
```

**Rollback**:
```bash
rm src/auto_tagger/config/settings.py
```

---

### Task 1.2.3: Create config loader

**Action**: Implement YAML config file loading

**File**: `src/auto_tagger/config/loader.py`

**Content**:
```python
"""Configuration file loader."""

from pathlib import Path
from typing import Any

import yaml

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
```

**Verification**:
```bash
python -c "from auto_tagger.config.loader import find_config_file; print(find_config_file())"
```

**Rollback**:
```bash
rm src/auto_tagger/config/loader.py
```

---

### Task 1.2.4: Update config __init__.py

**Action**: Export config components

**File**: `src/auto_tagger/config/__init__.py`

**Content**:
```python
"""Configuration management for auto_tagger."""

from auto_tagger.config.loader import find_config_file, load_config_file, load_settings
from auto_tagger.config.settings import Settings

__all__ = [
    "Settings",
    "find_config_file",
    "load_config_file",
    "load_settings",
]
```

**Verification**:
```bash
python -c "from auto_tagger.config import Settings, load_settings; print('OK')"
```

**Rollback**:
```bash
echo "" > src/auto_tagger/config/__init__.py
```

---

### Task 1.2.5: Create example config file

**Action**: Create example configuration for users

**File**: `config.example.yaml`

**Content**:
```yaml
# Auto Tagger Configuration Example
# Copy to ~/.config/auto-tagger/config.yaml or ./auto-tagger.yaml

# Output format: table, json, or plain
output_format: table

# Enable verbose logging
verbose: false

# Process directories recursively
recursive: false
recursive_depth: 10

# Patterns to exclude from processing
exclude_patterns:
  - .git
  - __pycache__
  - "*.pyc"
  - .DS_Store
  - "*.tmp"

# Auto-approve all changes (use with caution!)
yolo: false

# LLM Configuration (for Phase 4)
llm:
  api_key: null  # Set via AUTO_TAG_LLM_API_KEY env var
  endpoint: https://openrouter.ai/api/v1
  model: anthropic/claude-3.5-haiku

# Caching Configuration
cache:
  enabled: true
  path: ~/.cache/auto-tagger/cache.db
```

**Verification**:
```bash
cat config.example.yaml
```

**Rollback**:
```bash
rm config.example.yaml
```

---

### Task 1.2.6: Create .env.example

**Action**: Create example environment file

**File**: `.env.example`

**Content**:
```bash
# Auto Tagger Environment Variables
# Copy to .env and fill in values

# LLM API Configuration
AUTO_TAG_LLM_API_KEY=your-api-key-here
AUTO_TAG_LLM_ENDPOINT=https://openrouter.ai/api/v1
AUTO_TAG_LLM_MODEL=anthropic/claude-3.5-haiku

# Output Configuration
AUTO_TAG_OUTPUT_FORMAT=table
AUTO_TAG_VERBOSE=false

# Processing Configuration
AUTO_TAG_RECURSIVE=false
AUTO_TAG_YOLO=false
```

**Verification**:
```bash
cat .env.example
```

**Rollback**:
```bash
rm .env.example
```

---

### Task 1.2.7: Write config tests

**Action**: Create tests for configuration system

**File**: `tests/test_config.py`

**Content**:
```python
"""Tests for configuration system."""

from pathlib import Path
from unittest.mock import patch

import pytest

from auto_tagger.config import Settings, find_config_file, load_config_file, load_settings
from auto_tagger.exceptions import ConfigError


def test_settings_defaults():
    """Test default settings values."""
    settings = Settings()
    assert settings.output_format == "table"
    assert settings.verbose is False
    assert settings.recursive is False
    assert settings.yolo is False
    assert settings.cache_enabled is True


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
```

**Verification**:
```bash
pytest tests/test_config.py -v
```

**Rollback**:
```bash
rm tests/test_config.py
```

---

## Wave 1.3: CLI Framework & Logging

**Objective**: Implement CLI with Typer and logging framework

**Dependencies**: Waves 1.1 and 1.2 complete

### Task 1.3.1: Create logging module

**Action**: Implement logging setup with Rich

**File**: `src/auto_tagger/utils/logging.py`

**Content**:
```python
"""Logging configuration for auto_tagger."""

import logging
import sys
from typing import TextIO

from rich.console import Console
from rich.logging import RichHandler


def setup_logging(
    verbose: bool = False,
    log_file: str | None = None,
    console_output: TextIO | None = None,
) -> logging.Logger:
    """Configure logging with Rich formatting.

    Args:
        verbose: Enable debug-level logging
        log_file: Optional file path for logging
        console_output: Optional console output stream (default: sys.stderr)

    Returns:
        Configured logger instance
    """
    level = logging.DEBUG if verbose else logging.INFO

    handlers: list[logging.Handler] = [
        RichHandler(
            console=Console(file=console_output or sys.stderr),
            rich_tracebacks=True,
            markup=True,
            show_time=False,
            show_path=verbose,
        )
    ]

    if log_file:
        file_handler = logging.FileHandler(log_file)
        file_handler.setLevel(level)
        file_handler.setFormatter(
            logging.Formatter(
                "%(asctime)s - %(name)s - %(levelname)s - %(message)s",
                datefmt="%Y-%m-%d %H:%M:%S",
            )
        )
        handlers.append(file_handler)

    logging.basicConfig(
        level=level,
        format="%(message)s",
        datefmt="[%X]",
        handlers=handlers,
        force=True,
    )

    logger = logging.getLogger("auto_tagger")
    logger.setLevel(level)

    return logger


def get_logger(name: str | None = None) -> logging.Logger:
    """Get a logger instance.

    Args:
        name: Optional logger name (default: auto_tagger)

    Returns:
        Logger instance
    """
    return logging.getLogger(name or "auto_tagger")
```

**Verification**:
```bash
python -c "from auto_tagger.utils.logging import setup_logging; logger = setup_logging(); logger.info('test')"
```

**Rollback**:
```bash
rm src/auto_tagger/utils/logging.py
```

---

### Task 1.3.2: Create output formatting module

**Action**: Implement Rich output helpers

**File**: `src/auto_tagger/utils/output.py`

**Content**:
```python
"""Output formatting utilities."""

from typing import Any

from rich.console import Console
from rich.table import Table


console = Console()


def print_success(message: str) -> None:
    """Print success message in green."""
    console.print(f"[green]✓[/green] {message}")


def print_error(message: str) -> None:
    """Print error message in red."""
    console.print(f"[red]✗[/red] {message}", style="red")


def print_warning(message: str) -> None:
    """Print warning message in yellow."""
    console.print(f"[yellow]![/yellow] {message}", style="yellow")


def print_info(message: str) -> None:
    """Print info message in blue."""
    console.print(f"[blue]ℹ[/blue] {message}")


def print_table(
    title: str,
    columns: list[str],
    rows: list[list[Any]],
    show_header: bool = True,
) -> None:
    """Print data as a table.

    Args:
        title: Table title
        columns: Column headers
        rows: Table rows (list of values)
        show_header: Whether to show column headers
    """
    table = Table(title=title, show_header=show_header)

    for column in columns:
        table.add_column(column)

    for row in rows:
        table.add_row(*[str(cell) for cell in row])

    console.print(table)


def print_json(data: Any) -> None:
    """Print data as formatted JSON."""
    import json

    from rich.syntax import Syntax

    json_str = json.dumps(data, indent=2, sort_keys=True)
    syntax = Syntax(json_str, "json", theme="monokai", line_numbers=False)
    console.print(syntax)


def print_panel(content: str, title: str | None = None, style: str = "blue") -> None:
    """Print content in a panel.

    Args:
        content: Panel content
        title: Optional panel title
        style: Panel border style
    """
    from rich.panel import Panel

    panel = Panel(content, title=title, border_style=style)
    console.print(panel)
```

**Verification**:
```bash
python -c "from auto_tagger.utils.output import print_success; print_success('Test')"
```

**Rollback**:
```bash
rm src/auto_tagger/utils/output.py
```

---

### Task 1.3.3: Update utils __init__.py

**Action**: Export utility functions

**File**: `src/auto_tagger/utils/__init__.py`

**Content**:
```python
"""Utility functions for auto_tagger."""

from auto_tagger.utils.logging import get_logger, setup_logging
from auto_tagger.utils.output import (
    console,
    print_error,
    print_info,
    print_json,
    print_panel,
    print_success,
    print_table,
    print_warning,
)

__all__ = [
    "console",
    "get_logger",
    "print_error",
    "print_info",
    "print_json",
    "print_panel",
    "print_success",
    "print_table",
    "print_warning",
    "setup_logging",
]
```

**Verification**:
```bash
python -c "from auto_tagger.utils import print_success, setup_logging; print('OK')"
```

**Rollback**:
```bash
echo "" > src/auto_tagger/utils/__init__.py
```

---

### Task 1.3.4: Create basic CLI with Typer

**Action**: Implement main CLI entry point

**File**: `src/auto_tagger/cli.py`

**Content**:
```python
"""Main CLI entry point for auto_tagger."""

import sys
from pathlib import Path
from typing import Optional

import click
from rich.console import Console

from auto_tagger import __version__
from auto_tagger.config import Settings, load_settings
from auto_tagger.exceptions import AutoTaggerError, ConfigError
from auto_tagger.utils import console, setup_logging

CONTEXT_SETTINGS = {
    "help_option_names": ["-h", "--help"],
    "max_content_width": 120,
}


@click.group(context_settings=CONTEXT_SETTINGS)
@click.option(
    "--config",
    "-c",
    type=click.Path(exists=True, path_type=Path),
    help="Path to configuration file",
)
@click.option(
    "--verbose",
    "-v",
    is_flag=True,
    help="Enable verbose output",
)
@click.option(
    "--output",
    "-o",
    type=click.Choice(["table", "json", "plain"]),
    help="Output format",
)
@click.version_option(version=__version__, prog_name="auto-tag")
@click.pass_context
def cli(ctx: click.Context, config: Optional[Path], verbose: bool, output: Optional[str]) -> None:
    """Auto Tagger - Intelligent audio file tagging CLI tool.

    Automatically tag audio files with metadata from MusicBrainz and LLM assistance.
    """
    try:
        cli_overrides = {}
        if verbose:
            cli_overrides["verbose"] = True
        if output:
            cli_overrides["output_format"] = output
        if config:
            cli_overrides["config_file"] = config

        settings = load_settings(config_file=config, **cli_overrides)

        setup_logging(verbose=settings.verbose)

        ctx.ensure_object(dict)
        ctx.obj["settings"] = settings
        ctx.obj["verbose"] = settings.verbose

    except ConfigError as e:
        console.print(f"[red]Configuration error:[/red] {e}")
        sys.exit(e.exit_code)
    except Exception as e:
        console.print(f"[red]Unexpected error:[/red] {e}")
        if verbose:
            console.print_exception()
        sys.exit(1)


@cli.command()
@click.argument("path", type=click.Path(exists=True, path_type=Path))
@click.option("--dry-run", is_flag=True, help="Preview changes without applying")
@click.option("--yolo", is_flag=True, help="Auto-approve all changes")
@click.pass_context
def tag(ctx: click.Context, path: Path, dry_run: bool, yolo: bool) -> None:
    """Tag a single album or directory.

    PATH: Path to album directory or audio file
    """
    from auto_tagger.commands.tag import execute

    settings: Settings = ctx.obj["settings"]

    if yolo:
        settings.yolo = True

    execute(settings, path, dry_run)


@cli.command()
@click.argument("path", type=click.Path(exists=True, path_type=Path))
@click.option("--dry-run", is_flag=True, help="Preview changes without applying")
@click.option("--yolo", is_flag=True, help="Auto-approve all changes")
@click.option(
    "--parallel",
    "-j",
    type=int,
    default=1,
    help="Number of parallel processes",
)
@click.pass_context
def batch(ctx: click.Context, path: Path, dry_run: bool, yolo: bool, parallel: int) -> None:
    """Batch process entire music library.

    PATH: Path to music library root
    """
    from auto_tagger.commands.batch import execute

    settings: Settings = ctx.obj["settings"]

    if yolo:
        settings.yolo = True

    execute(settings, path, dry_run, parallel)


@cli.command()
@click.argument("key", required=False)
@click.argument("value", required=False)
@click.pass_context
def config(ctx: click.Context, key: Optional[str], value: Optional[str]) -> None:
    """View or modify configuration.

    KEY: Configuration key to view or set
    VALUE: New value to set (optional)
    """
    from auto_tagger.commands.config_cmd import execute

    settings: Settings = ctx.obj["settings"]
    execute(settings, key, value)


@cli.command()
@click.pass_context
def version(ctx: click.Context) -> None:
    """Show version information."""
    console.print(f"[bold]auto-tag[/bold] version [cyan]{__version__}[/cyan]")


def main() -> None:
    """Main entry point."""
    try:
        cli()
    except AutoTaggerError as e:
        console.print(f"[red]Error:[/red] {e}")
        sys.exit(e.exit_code)
    except KeyboardInterrupt:
        console.print("\n[yellow]Interrupted by user[/yellow]")
        sys.exit(130)
    except Exception as e:
        console.print(f"[red]Unexpected error:[/red] {e}")
        console.print("Run with --verbose for details")
        sys.exit(1)


if __name__ == "__main__":
    main()
```

**Verification**:
```bash
python -m auto_tagger --help
auto-tag --help
```

**Rollback**:
```bash
echo "" > src/auto_tagger/cli.py
```

---

### Task 1.3.5: Create __main__.py entry point

**Action**: Enable running as module

**File**: `src/auto_tagger/__main__.py`

**Content**:
```python
"""Entry point for running as module: python -m auto_tagger"""

from auto_tagger.cli import main

if __name__ == "__main__":
    main()
```

**Verification**:
```bash
python -m auto_tagger --help
```

**Rollback**:
```bash
echo "" > src/auto_tagger/__main__.py
```

---

### Task 1.3.6: Create stub tag command

**Action**: Create placeholder tag command

**File**: `src/auto_tagger/commands/tag.py`

**Content**:
```python
"""Tag command implementation."""

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.utils import console, print_info, print_success


def execute(settings: Settings, path: Path, dry_run: bool) -> None:
    """Execute tag command.

    Args:
        settings: Application settings
        path: Path to album or file
        dry_run: Preview without changes
    """
    print_info(f"Tagging: {path}")
    console.print(f"  Dry run: {dry_run}")
    console.print(f"  YOLO mode: {settings.yolo}")
    console.print(f"  Output format: {settings.output_format}")

    print_success("Tag command not yet implemented (Phase 2)")
```

**Verification**:
```bash
python -m auto_tagger tag --help
```

**Rollback**:
```bash
rm src/auto_tagger/commands/tag.py
```

---

### Task 1.3.7: Create stub batch command

**Action**: Create placeholder batch command

**File**: `src/auto_tagger/commands/batch.py`

**Content**:
```python
"""Batch command implementation."""

from pathlib import Path

from auto_tagger.config import Settings
from auto_tagger.utils import console, print_info, print_success


def execute(settings: Settings, path: Path, dry_run: bool, parallel: int) -> None:
    """Execute batch command.

    Args:
        settings: Application settings
        path: Path to music library
        dry_run: Preview without changes
        parallel: Number of parallel processes
    """
    print_info(f"Batch processing: {path}")
    console.print(f"  Dry run: {dry_run}")
    console.print(f"  Parallel jobs: {parallel}")
    console.print(f"  YOLO mode: {settings.yolo}")
    console.print(f"  Output format: {settings.output_format}")

    print_success("Batch command not yet implemented (Phase 6)")
```

**Verification**:
```bash
python -m auto_tagger batch --help
```

**Rollback**:
```bash
rm src/auto_tagger/commands/batch.py
```

---

### Task 1.3.8: Create config command

**Action**: Implement config view/set command

**File**: `src/auto_tagger/commands/config_cmd.py`

**Content**:
```python
"""Config command implementation."""

from typing import Any

from auto_tagger.config import Settings, find_config_file
from auto_tagger.utils import console, print_info, print_json, print_table


def execute(settings: Settings, key: str | None, value: str | None) -> None:
    """Execute config command.

    Args:
        settings: Application settings
        key: Configuration key to view/set
        value: New value to set
    """
    if key is None:
        _show_all_config(settings)
    elif value is None:
        _show_config_key(settings, key)
    else:
        _set_config_key(settings, key, value)


def _show_all_config(settings: Settings) -> None:
    """Display all configuration settings."""
    print_info("Current configuration:")

    config_file = find_config_file()
    if config_file:
        console.print(f"\n[cyan]Config file:[/cyan] {config_file}")
    else:
        console.print("\n[yellow]No config file found (using defaults and env vars)[/yellow]")

    if settings.output_format == "json":
        config_dict = settings.model_dump(mode="json")
        print_json(config_dict)
    else:
        rows = [
            ["verbose", str(settings.verbose)],
            ["output_format", settings.output_format],
            ["recursive", str(settings.recursive)],
            ["recursive_depth", str(settings.recursive_depth)],
            ["yolo", str(settings.yolo)],
            ["cache_enabled", str(settings.cache_enabled)],
            ["cache_path", str(settings.cache_path)],
            ["llm_endpoint", settings.llm_endpoint],
            ["llm_model", settings.llm_model],
            ["llm_api_key", "***" if settings.llm_api_key else "None"],
        ]

        print_table("Configuration", ["Key", "Value"], rows)


def _show_config_key(settings: Settings, key: str) -> None:
    """Display a specific configuration key."""
    try:
        value = getattr(settings, key)
        console.print(f"[cyan]{key}[/cyan]: {value}")
    except AttributeError:
        console.print(f"[red]Unknown configuration key: {key}[/red]")
        console.print("\n[yellow]Valid keys:[/yellow]")
        valid_keys = [
            "verbose",
            "output_format",
            "recursive",
            "recursive_depth",
            "yolo",
            "cache_enabled",
            "cache_path",
            "llm_endpoint",
            "llm_model",
        ]
        for k in valid_keys:
            console.print(f"  - {k}")


def _set_config_key(settings: Settings, key: str, value: str) -> None:
    """Set a configuration key (in memory only, not persisted)."""
    console.print(
        "[yellow]Note:[/yellow] Configuration changes are not persisted yet. "
        "Edit your config file or use environment variables."
    )

    try:
        current_value = getattr(settings, key)
        console.print(f"[cyan]{key}[/cyan]: {current_value} → {value}")
        console.print("\n[yellow]To make this change permanent, add to your config file:[/yellow]")
        console.print(f"{key}: {value}")
    except AttributeError:
        console.print(f"[red]Unknown configuration key: {key}[/red]")
```

**Verification**:
```bash
python -m auto_tagger config
python -m auto_tagger config verbose
```

**Rollback**:
```bash
rm src/auto_tagger/commands/config_cmd.py
```

---

### Task 1.3.9: Update commands __init__.py

**Action**: Export commands

**File**: `src/auto_tagger/commands/__init__.py`

**Content**:
```python
"""CLI commands for auto_tagger."""

from auto_tagger.commands import batch, config_cmd, tag

__all__ = ["batch", "config_cmd", "tag"]
```

**Verification**:
```bash
python -c "from auto_tagger.commands import tag, batch, config_cmd; print('OK')"
```

**Rollback**:
```bash
echo "" > src/auto_tagger/commands/__init__.py
```

---

### Task 1.3.10: Write CLI tests

**Action**: Create tests for CLI functionality

**File**: `tests/test_cli.py`

**Content**:
```python
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
```

**Verification**:
```bash
pytest tests/test_cli.py -v
```

**Rollback**:
```bash
rm tests/test_cli.py
```

---

### Task 1.3.11: Update conftest.py

**Action**: Add pytest fixtures

**File**: `tests/conftest.py`

**Content**:
```python
"""Pytest configuration and fixtures."""

import pytest
from pathlib import Path
from auto_tagger.config import Settings


@pytest.fixture
def tmp_album(tmp_path: Path) -> Path:
    """Create a temporary album directory structure."""
    album_dir = tmp_path / "Test Artist" / "Test Album"
    album_dir.mkdir(parents=True)

    (album_dir / "01 - Track One.mp3").touch()
    (album_dir / "02 - Track Two.mp3").touch()

    return album_dir


@pytest.fixture
def tmp_library(tmp_path: Path) -> Path:
    """Create a temporary library structure."""
    library = tmp_path / "Music"

    albums = [
        ("Artist One", "Album One", ["01.mp3", "02.mp3"]),
        ("Artist Two", "Album Two", ["01.mp3", "02.mp3", "03.mp3"]),
    ]

    for artist, album, tracks in albums:
        album_dir = library / artist / album
        album_dir.mkdir(parents=True)
        for track in tracks:
            (album_dir / track).touch()

    return library


@pytest.fixture
def settings() -> Settings:
    """Create default settings instance."""
    return Settings()


@pytest.fixture
def verbose_settings() -> Settings:
    """Create verbose settings instance."""
    return Settings(verbose=True)


@pytest.fixture
def yolo_settings() -> Settings:
    """Create YOLO mode settings instance."""
    return Settings(yolo=True)
```

**Verification**:
```bash
pytest tests/conftest.py -v
```

**Rollback**:
```bash
echo "" > tests/conftest.py
```

---

### Task 1.3.12: Final verification and testing

**Action**: Run all tests and verify installation

**Commands**:
```bash
# Run all tests
pytest -v

# Run with coverage
pytest --cov=auto_tagger tests/

# Verify CLI works
auto-tag --help
auto-tag --version
auto-tag config
python -m auto_tagger --help

# Verify imports work
python -c "from auto_tagger import __version__; print(__version__)"
python -c "from auto_tagger.config import Settings; print(Settings())"
python -c "from auto_tagger.cli import main; print('OK')"
python -c "from auto_tagger.utils import print_success; print_success('Test')"
```

**Expected Output**:
- All tests pass
- CLI shows help text
- Version displayed correctly
- Config command shows settings
- All imports work without errors

**Rollback**: None needed - verification step

---

## Execution Order

### Dependency Graph

```
Wave 1.1 (No dependencies - can run in parallel):
  1.1.1 → 1.1.2 → 1.1.7 (install)
  1.1.3 → 1.1.4 → 1.1.5 → 1.1.6 (can be parallel with above)

Wave 1.2 (Depends on 1.1.7):
  1.2.1 → 1.2.2 → 1.2.3 → 1.2.4 → 1.2.7 (tests)
  1.2.5 → 1.2.6 (can be parallel with above)

Wave 1.3 (Depends on 1.2):
  1.3.1 → 1.3.2 → 1.3.3
  1.3.4 → 1.3.5 → 1.3.6 → 1.3.7 → 1.3.8 → 1.3.9 → 1.3.10 → 1.3.11 → 1.3.12
```

### Parallel Execution

Tasks that can run in parallel within each wave:

**Wave 1.1**:
- Task 1.1.3, 1.1.4, 1.1.5, 1.1.6 can run in parallel after 1.1.1
- Task 1.1.7 must run after 1.1.2

**Wave 1.2**:
- Task 1.2.5, 1.2.6 can run in parallel with 1.2.2, 1.2.3, 1.2.4
- Task 1.2.7 must run after 1.2.4

**Wave 1.3**:
- Task 1.3.1, 1.3.2 can run in parallel
- Task 1.3.4 through 1.3.12 must run sequentially

---

## Success Verification Checklist

Run these commands to verify Phase 1 completion:

### 1. Project Structure
```bash
tree src -L 3
tree tests -L 1
```
✓ All directories and files created

### 2. Package Installation
```bash
pip show auto-tagger
python -c "import auto_tagger; print(auto_tagger.__version__)"
```
✓ Package installs correctly
✓ Version is 0.1.0

### 3. CLI Entry Points
```bash
auto-tag --help
auto-tag --version
python -m auto_tagger --help
```
✓ CLI shows help
✓ Version command works
✓ Module entry point works

### 4. Configuration System
```bash
auto-tag config
auto-tag config verbose
python -c "from auto_tagger.config import Settings; s = Settings(verbose=True); print(s.verbose)"
```
✓ Config command displays settings
✓ Environment variables work
✓ Settings validation works

### 5. Logging Framework
```bash
python -c "from auto_tagger.utils import setup_logging; import logging; logger = setup_logging(verbose=True); logger.debug('test')"
```
✓ Logging outputs to console
✓ Rich formatting works

### 6. All Tests Pass
```bash
pytest -v
pytest --cov=auto_tagger
```
✓ All tests pass
✓ Coverage > 80%

### 7. Type Checking (Optional)
```bash
mypy src/auto_tagger
```
✓ No type errors

### 8. Linting (Optional)
```bash
ruff check src/auto_tagger
```
✓ No linting errors

---

## Notes

### Key Decisions
1. **Typer → Click**: Used Click directly for more control and simpler dependency chain (Typer builds on Click)
2. **pyproject.toml only**: No setup.py or requirements.txt needed (modern Python packaging)
3. **hatchling**: Modern build backend with good performance
4. **Rich for output**: Beautiful terminal output with minimal code

### Next Steps (Phase 2)
- Implement audio file reading/writing
- Add mutagen integration
- Create tag reader/writer abstraction
- Support multiple audio formats

### Potential Issues
1. **Click vs Typer**: If we want simpler CLI later, can migrate to Typer
2. **Pydantic v2**: Ensure all Pydantic code is v2 compatible
3. **Python 3.10+**: Using modern type hints (union with `|` instead of `Union`)

### Rollback Plan
If Phase 1 needs complete rollback:
```bash
rm -rf src tests .venv pyproject.toml README.md LICENSE .gitignore config.example.yaml .env.example
```
Then re-initialize with git checkout from clean state.