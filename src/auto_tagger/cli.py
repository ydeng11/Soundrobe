"""Main CLI entry point for auto_tagger."""

import sys
from pathlib import Path
from typing import Any

import click

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
def cli(ctx: click.Context, config: Path | None, verbose: bool, output: str | None) -> None:
    """Auto Tagger - Intelligent audio file tagging CLI tool.

    Automatically tag audio files with metadata from MusicBrainz and LLM assistance.
    """
    try:
        cli_overrides: dict[str, Any] = {}
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
@click.option("--interactive", is_flag=True, help="Prompt before applying album changes")
@click.option(
    "--health-report",
    type=click.Path(dir_okay=False, path_type=Path),
    help="Write album health report JSON to this path",
)
@click.pass_context
def tag(
    ctx: click.Context,
    path: Path,
    dry_run: bool,
    yolo: bool,
    interactive: bool,
    health_report: Path | None,
) -> None:
    """Tag a single album or directory.

    PATH: Path to album directory or audio file
    """
    from auto_tagger.commands.tag import execute

    settings: Settings = ctx.obj["settings"]

    if yolo:
        settings.yolo = True

    execute(settings, path, dry_run, health_report, interactive)


@cli.command()
@click.argument("path", type=click.Path(exists=True, path_type=Path))
@click.option("--dry-run", is_flag=True, help="Preview changes without applying")
@click.option("--yolo", is_flag=True, help="Auto-approve all changes")
@click.option("--interactive", is_flag=True, help="Prompt before applying each album")
@click.option(
    "--parallel",
    "-j",
    type=int,
    default=1,
    help="Number of parallel processes",
)
@click.option(
    "--health-report",
    type=click.Path(dir_okay=False, path_type=Path),
    help="Write combined health report JSON for all albums",
)
@click.pass_context
def batch(
    ctx: click.Context,
    path: Path,
    dry_run: bool,
    yolo: bool,
    interactive: bool,
    parallel: int,
    health_report: Path | None,
) -> None:
    """Batch process entire music library.

    PATH: Path to music library root
    """
    from auto_tagger.commands.batch import execute

    settings: Settings = ctx.obj["settings"]

    if yolo:
        settings.yolo = True

    execute(settings, path, dry_run, parallel, interactive, health_report)


@cli.command()
@click.argument("key", required=False)
@click.argument("value", required=False)
@click.pass_context
def config(ctx: click.Context, key: str | None, value: str | None) -> None:
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


@cli.group()
@click.pass_context
def dataset(ctx: click.Context) -> None:
    """Manage the local MusicMoveArr dataset index."""


@dataset.command("status")
@click.pass_context
def dataset_status(ctx: click.Context) -> None:
    """Show local dataset setup status."""
    from auto_tagger.commands.dataset import execute_status

    settings: Settings = ctx.obj["settings"]
    execute_status(settings)


@dataset.command("setup")
@click.option(
    "--service",
    "services",
    multiple=True,
    type=click.Choice(["musicbrainz", "spotify", "tidal", "deezer"]),
    help="Dataset service to install; can be passed multiple times",
)
@click.option("--dry-run", is_flag=True, help="Show setup plan without downloading")
@click.pass_context
def dataset_setup(ctx: click.Context, services: tuple[str, ...], dry_run: bool) -> None:
    """Download the dataset and build a local SQLite lookup index."""
    from auto_tagger.commands.dataset import execute_setup

    settings: Settings = ctx.obj["settings"]
    if not services and not dry_run and sys.stdin.isatty():
        answer = click.prompt(
            "Services to install (comma-separated)",
            default=",".join(settings.dataset_services),
            show_default=True,
        )
        services = tuple(item.strip() for item in answer.split(",") if item.strip())
    execute_setup(settings, services, dry_run)


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
