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