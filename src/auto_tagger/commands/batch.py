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
