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