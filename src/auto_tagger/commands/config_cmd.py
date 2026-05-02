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