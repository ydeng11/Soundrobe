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