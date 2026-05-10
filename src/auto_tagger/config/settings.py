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
    llm_fallback_model: str = Field(
        default="google/gemini-flash-1.5-8b",
        description="Fallback LLM model to use",
    )
    llm_max_candidates: int = Field(
        default=5,
        ge=1,
        le=20,
        description="Maximum lookup candidates to include in LLM prompts",
    )
    llm_max_tokens: int = Field(
        default=800,
        ge=64,
        le=8000,
        description="Maximum LLM completion tokens",
    )
    llm_temperature: float = Field(
        default=0.0,
        ge=0.0,
        le=2.0,
        description="LLM sampling temperature",
    )
    llm_cost_per_1k_prompt_tokens: float = Field(
        default=0.001,
        ge=0.0,
        description="Estimated prompt token cost per 1K tokens",
    )
    llm_cost_per_1k_completion_tokens: float = Field(
        default=0.002,
        ge=0.0,
        description="Estimated completion token cost per 1K tokens",
    )

    cache_enabled: bool = Field(
        default=True,
        description="Enable match caching",
    )
    cache_path: Path = Field(
        default=Path(".planning") / "cache.db",
        description="Path to cache database",
    )
    ffprobe_path: str = Field(
        default="ffprobe",
        description="Path or command name for ffprobe validation",
    )
    ffprobe_timeout_seconds: int = Field(
        default=20,
        ge=1,
        le=300,
        description="Per-file ffprobe timeout in seconds",
    )
    replaygain_command: str = Field(
        default="rgain3",
        description="ReplayGain command to use for calculation",
    )
    replaygain_timeout_seconds: int = Field(
        default=600,
        ge=1,
        le=7200,
        description="ReplayGain album calculation timeout in seconds",
    )
    lrc_convert_encoding: bool = Field(
        default=False,
        description="Convert non-UTF-8 LRC files when applying changes",
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
